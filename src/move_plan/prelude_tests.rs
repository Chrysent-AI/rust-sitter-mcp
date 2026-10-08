use super::*;
use std::time::Duration;

#[test]
fn veto_provenance_names_each_class_at_original_coordinates() {
    let flag = AtomicBool::new(false);
    let controls = (Instant::now() + Duration::from_secs(10), &flag);
    for (source, class, witness) in [
        ("introduce!();", "chain_macro_statement", "introduce!()"),
        ("struct Option;", "shadow", "Option"),
        (
            "#[cfg_attr(test, derive(Clone))] struct Option;",
            "shadow",
            "Option",
        ),
        ("use unknown::*;", "glob_import", "use unknown::*;"),
        ("fn broken( {", "syntax_recovery", "fn broken( {"),
    ] {
        let tree = crate::trivia::parse(source, controls.0, controls.1)
            .unwrap()
            .unwrap();
        let found = shadows("probe.rs", tree.root_node(), source, controls, None).unwrap();
        assert!(found.refuses(0), "{source}");
        let basis = found
            .basis_for(0, false)
            .into_iter()
            .find(|b| b.class == class)
            .unwrap();
        assert_eq!(basis.anchor.path, "probe.rs");
        let range = basis.anchor.range.unwrap();
        assert_eq!(&source[range.start_byte..range.end_byte], witness);
        if class == "shadow" {
            assert_eq!(basis.name.as_deref(), Some("Option"));
        }
    }
}

#[test]
fn unknown_derive_keeps_its_own_witness_without_vetoing_unrelated_type_names() {
    let source = "#[derive(custom::Debug)] struct Other;";
    let flag = AtomicBool::new(false);
    let controls = (Instant::now() + Duration::from_secs(10), &flag);
    let tree = crate::trivia::parse(source, controls.0, controls.1)
        .unwrap()
        .unwrap();
    let found = shadows("probe.rs", tree.root_node(), source, controls, None).unwrap();
    assert!(!found.refuses(0));
    assert!(found.basis_for(0, false).is_empty());
    let basis = found
        .refusal_basis
        .iter()
        .find(|b| b.class == "derive_veto")
        .unwrap();
    let range = basis.anchor.range.as_ref().unwrap();
    assert_eq!(&source[range.start_byte..range.end_byte], "custom::Debug");
}
#[test]
fn scoped_prelude_controls_do_not_escape_to_parent_or_sibling() {
    let source = "struct Parent { option: Option<u8> } mod isolated { #![no_implicit_prelude] struct Disabled { option: Option<u8> } mod inner { struct DisabledChild { option: Option<u8> } } } mod sibling { struct Sibling { option: Option<u8> } }";
    let flag = AtomicBool::new(false);
    let controls = (Instant::now() + Duration::from_secs(10), &flag);
    let tree = crate::trivia::parse(source, controls.0, controls.1)
        .unwrap()
        .unwrap();
    let scoped =
        ScopedShadows::collect("probe.rs", tree.root_node(), source, controls, None).unwrap();
    for ((at, _), refuses) in source
        .match_indices("Option")
        .zip([false, true, true, false])
    {
        let node = tree
            .root_node()
            .named_descendant_for_byte_range(at, at + "Option".len())
            .unwrap();
        let found = scoped.visible(Some(node));
        assert_eq!(found.refuses(0), refuses, "offset {at}");
        assert_eq!(
            found
                .basis_for(0, false)
                .iter()
                .any(|b| b.class == "prelude_disabled"),
            refuses
        );
    }
    assert!(!scoped.root.chain_context(true).refuses(0));
    // A file-root directive does inherit into filesystem children; crate-only
    // controls retain the existing unbounded policy even in an inline scan.
    for (source, scoped_control) in [
        ("#![no_implicit_prelude]", true),
        ("#![no_std]", false),
        ("#![no_core]", false),
        ("#![prelude_import]", false),
    ] {
        let tree = crate::trivia::parse(source, controls.0, controls.1)
            .unwrap()
            .unwrap();
        let found = shadows("probe.rs", tree.root_node(), source, controls, None).unwrap();
        assert!(found.chain_context(true).refuses(0), "{source}");
        assert_eq!(
            found.chain_context(false).refuses(0),
            !scoped_control,
            "{source}"
        );
    }
}

#[test]
fn retained_local_need_discloses_the_definite_declaration_not_the_use() {
    // The ordinary dependency scan can omit proven locals entirely. Supply a
    // retained need directly to exercise the fallback's definite-shadow branch.
    for (source, declaration) in [
        (
            "fn selected<Option: Copy>(input: Option) -> Option { input }",
            "Option: Copy",
        ),
        (
            "fn selected() { struct Option; let value: Option; }",
            "struct Option;",
        ),
    ] {
        let flag = AtomicBool::new(false);
        let controls = (Instant::now() + Duration::from_secs(10), &flag);
        let files: BTreeMap<String, FileSnapshot> = BTreeMap::from([
            (
                "probe.rs".into(),
                FileSnapshot {
                    path: "probe.rs".into(),
                    source: source.into(),
                    mode: 0o100644,
                },
            ),
            (
                "destination.rs".into(),
                FileSnapshot {
                    path: "destination.rs".into(),
                    source: String::new(),
                    mode: 0o100644,
                },
            ),
        ]);
        let mut observed = 0;
        let parsed: BTreeMap<_, _> = files
            .iter()
            .map(|(path, file)| {
                (
                    path.clone(),
                    items::parse(file, usize::MAX, controls.0, controls.1, &mut observed).unwrap(),
                )
            })
            .collect();
        let item = parsed["probe.rs"].items[0].clone();
        let at = source.rfind("Option").unwrap();
        let node = parsed["probe.rs"]
            .tree
            .root_node()
            .named_descendant_for_byte_range(at, at + "Option".len())
            .unwrap();
        let mut needs = vec![Need {
            attribute_range: None,
            reason: DecisionReason::ExternalOrMissingBinding,
            choice_target: None,
            lexical_uncertainty: None,
            refusal_basis: vec![],
            category: "unsupported_dependency_form",
            path: "probe.rs".into(),
            range: crate::result::ByteRange {
                start_byte: node.start_byte(),
                end_byte: node.end_byte(),
            },
            message: "retained local type".into(),
            item_ids: vec![item.id.clone()],
        }];
        let contexts = BTreeMap::from([
            (
                "probe.rs".into(),
                ModuleEvidence {
                    crate_root: "probe.rs".into(),
                    module_segments: vec![],
                    declaration_anchors: vec![],
                    filesystem_paths: vec!["probe.rs".into()],
                    assumptions: vec![],
                    unresolved: vec![],
                },
            ),
            (
                "destination.rs".into(),
                ModuleEvidence {
                    crate_root: "probe.rs".into(),
                    module_segments: vec!["destination".into()],
                    declaration_anchors: vec![],
                    filesystem_paths: vec!["probe.rs".into(), "destination.rs".into()],
                    assumptions: vec![],
                    unresolved: vec![],
                },
            ),
        ]);
        let proofs = discharge(
            &files,
            &parsed,
            &[("probe.rs".into(), item, "destination.rs".into())],
            &contexts,
            &contexts,
            &[],
            &mut needs,
            controls,
            None,
        )
        .unwrap();
        assert!(proofs.is_empty());
        assert_eq!(needs.len(), 1);
        let basis = needs[0]
            .refusal_basis
            .iter()
            .find(|b| b.class == "shadow" && b.name.as_deref() == Some("Option"))
            .unwrap();
        let range = basis.anchor.range.as_ref().unwrap();
        assert_eq!(basis.anchor.path, "probe.rs");
        assert_eq!(&source[range.start_byte..range.end_byte], declaration);
        assert!(range.end_byte <= needs[0].range.start_byte);
    }
}

#[test]
fn unresolved_chain_has_a_path_not_fabricated_source_offsets() {
    let found = chain_shadows(
        "missing.rs",
        None,
        &BTreeMap::new(),
        &BTreeMap::new(),
        false,
    );
    assert!(found.refuses(0));
    let value = serde_json::to_value(&found.basis_for(0, false)[0]).unwrap();
    assert_eq!(
        value,
        serde_json::json!({"class":"unresolved_chain", "anchor":{"path":"missing.rs"}})
    );
}

#[test]
fn provenance_filters_unrelated_names_and_keeps_contextual_derive_shadow() {
    let mut found = Shadows::default();
    found.name("String", "other.rs", None);
    assert!(!found.refuses(0));
    assert!(found.basis_for(0, false).is_empty());
    found.name("Debug", "other.rs", None);
    found.derives.insert(("probe.rs".into(), 0, 5), 0);
    assert!(found.refuses(0));
    let basis = found.basis_for(0, false);
    assert_eq!(basis.len(), 1);
    assert_eq!(basis[0].name.as_deref(), Some("Debug"));
}
