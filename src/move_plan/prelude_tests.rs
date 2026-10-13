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
fn unknown_derives_keep_their_own_witness_and_veto_unrelated_prelude_names() {
    let flag = AtomicBool::new(false);
    let controls = (Instant::now() + Duration::from_secs(10), &flag);
    for (derives, unknown) in [
        ("Inject", "Inject"),
        ("custom::Debug", "custom::Debug"),
        ("my::Macro", "my::Macro"),
        ("serde::Serialize", "serde::Serialize"),
        ("Debug, Inject", "Inject"),
        ("Serialize, Inject", "Inject"),
    ] {
        let attribute = format!("#[derive({derives})]");
        let source = format!("// original coordinates\n{attribute} struct Other;");
        let tree = crate::trivia::parse(&source, controls.0, controls.1)
            .unwrap()
            .unwrap();
        let found = shadows("probe.rs", tree.root_node(), &source, controls, None).unwrap();
        for index in (0..STANDARD_PRELUDE.len()).chain([STD_ROOT]) {
            assert!(found.refuses(index), "{derives}: {index}");
            let bases = found.basis_for(index, false);
            assert_eq!(bases.len(), 1, "{derives}: {bases:?}");
            assert_eq!(bases[0].class, "conditional_context");
            assert!(bases[0].name.is_none());
            assert_eq!(bases[0].anchor.path, "probe.rs");
            let range = bases[0].anchor.range.as_ref().unwrap();
            assert_eq!(&source[range.start_byte..range.end_byte], attribute);
        }
        // An unbounded unknown expansion also prevents a mixed-list builtin proof.
        assert!(found.refuses_derive_provider(0));
        let basis = found
            .refusal_basis
            .iter()
            .find(|b| {
                b.class == "derive_veto"
                    && b.anchor
                        .range
                        .as_ref()
                        .is_some_and(|range| &source[range.start_byte..range.end_byte] == unknown)
            })
            .unwrap();
        assert_eq!(basis.anchor.path, "probe.rs");
    }
}

#[test]
fn bare_registrar_derives_share_helper_spelling_trust_without_discharge() {
    let flag = AtomicBool::new(false);
    let controls = (Instant::now() + Duration::from_secs(10), &flag);
    for derives in [
        "Serialize",
        "Deserialize",
        "JsonSchema",
        "r#Serialize",
        "Debug, Serialize, Deserialize, JsonSchema",
    ] {
        // Admission does not require a matching helper on this declaration.
        let source = format!("#[derive({derives})] struct Other;");
        let tree = crate::trivia::parse(&source, controls.0, controls.1)
            .unwrap()
            .unwrap();
        let found = shadows("probe.rs", tree.root_node(), &source, controls, None).unwrap();
        for index in (0..STANDARD_PRELUDE.len()).chain([STD_ROOT]) {
            assert!(!found.refuses(index), "{derives}: {index}");
            assert!(found.basis_for(index, false).is_empty());
        }
        assert!(!found.refuses_derive_provider(0));
        assert!(found.refusal_basis.iter().all(|b| b.class == "derive_veto"));
        assert_eq!(
            found.refusal_basis.len(),
            if derives.starts_with("Debug,") { 3 } else { 1 }
        );
    }
}
#[test]
fn registered_helper_companions_do_not_veto_unrelated_prelude_names() {
    let flag = AtomicBool::new(false);
    let controls = (Instant::now() + Duration::from_secs(10), &flag);
    for companion in [
        "#[schemars(crate = \"rmcp::schemars\")]",
        "#[serde(rename_all = \"snake_case\")]",
    ] {
        let source =
            format!("#[derive(Debug, Clone, Serialize, JsonSchema)]\n{companion}\nstruct Other;");
        let tree = crate::trivia::parse(&source, controls.0, controls.1)
            .unwrap()
            .unwrap();
        let found = shadows("probe.rs", tree.root_node(), &source, controls, None).unwrap();
        for index in (0..STANDARD_PRELUDE.len()).chain([STD_ROOT]) {
            assert!(!found.refuses(index), "{companion}: {index}");
            assert!(
                found.basis_for(index, false).is_empty(),
                "{companion}: {index}"
            );
        }
        for index in [0, 1] {
            assert!(found.refuses_derive(index), "{companion}");
            let basis = found.basis_for(index, true);
            assert_eq!(basis.len(), 1);
            assert_eq!(basis[0].class, "conditional_context");
            assert_eq!(
                basis[0].name.as_deref(),
                Some(items::BUILTIN_DERIVES[index].0)
            );
            let range = basis[0].anchor.range.as_ref().unwrap();
            assert_eq!(&source[range.start_byte..range.end_byte], companion);
        }
    }
    // A bare cfg control conditions the item but cannot inject bindings.
    let source = "#[derive(Debug, Clone)]\n#[cfg(feature = \"unknown\")]\nstruct Other;";
    let tree = crate::trivia::parse(source, controls.0, controls.1)
        .unwrap()
        .unwrap();
    let found = shadows("probe.rs", tree.root_node(), source, controls, None).unwrap();
    for index in (0..STANDARD_PRELUDE.len()).chain([STD_ROOT]) {
        assert!(!found.refuses(index), "{index}");
    }
}

#[test]
fn unclassified_companion_attributes_veto_prelude_names_with_the_witness() {
    let flag = AtomicBool::new(false);
    let controls = (Instant::now() + Duration::from_secs(10), &flag);
    for (declares, companion) in [
        ("Debug", "#[serde(default)]"),
        ("Debug", "#[unknown_provider]"),
        ("Debug", "#[injector::inject]"),
        ("Debug", "#[custom::inline]"),
        ("Serialize", "#[schemars(crate = \"rmcp::schemars\")]"),
        (
            "Debug",
            "#[cfg_attr(feature = \"inject\", injector::inject)]",
        ),
        (
            "Debug",
            "#[cfg_attr(test, serde(rename = \"derive(Inject)\"))]",
        ),
    ] {
        let source = format!("#[derive({declares})]\n{companion}\nstruct Other;");
        let tree = crate::trivia::parse(&source, controls.0, controls.1)
            .unwrap()
            .unwrap();
        let found = shadows("probe.rs", tree.root_node(), &source, controls, None).unwrap();
        for index in (0..STANDARD_PRELUDE.len()).chain([STD_ROOT]) {
            assert!(found.refuses(index), "{companion}: {index}");
            let basis = found.basis_for(index, false);
            let witness = basis.iter().find(|b| b.name.is_none()).unwrap();
            assert_eq!(witness.class, "conditional_context");
            assert_eq!(witness.anchor.path, "probe.rs");
            let range = witness.anchor.range.as_ref().unwrap();
            assert_eq!(&source[range.start_byte..range.end_byte], companion);
        }
    }
    // A registrar derive makes the matching helper attributable and inert.
    let source = "#[derive(Debug, Serialize)]\n#[serde(default)]\nstruct Other;";
    let tree = crate::trivia::parse(source, controls.0, controls.1)
        .unwrap()
        .unwrap();
    let found = shadows("probe.rs", tree.root_node(), source, controls, None).unwrap();
    for index in (0..STANDARD_PRELUDE.len()).chain([STD_ROOT]) {
        assert!(!found.refuses(index), "{index}");
    }
}

#[test]
fn cfg_attr_custom_derive_payloads_refuse_with_the_outer_attribute_witness() {
    let flag = AtomicBool::new(false);
    let controls = (Instant::now() + Duration::from_secs(10), &flag);
    for attribute in [
        "#[cfg_attr(feature = \"inject\", derive(Inject))]",
        "#[cfg_attr(test, r#derive(Inject))]",
        "#[r#cfg_attr(test, derive(Inject))]",
        "#[cfg_attr(test, r#cfg_attr(test, derive(Inject)))]",
        "#[cfg_attr(feature = \"inject\", doc(hidden), derive(Debug, Inject))]",
        "#[cfg_attr(feature = \"inject\", cfg_attr(test, derive(custom::Debug)))]",
        "#[cfg_attr(test, derive(Debug /* not bare */ :: Inject))]",
        "#[cfg_attr(test, derive())]",
    ] {
        let source = format!("#[derive(Debug)]\n{attribute}\nstruct Other;");
        let tree = crate::trivia::parse(&source, controls.0, controls.1)
            .unwrap()
            .unwrap();
        let found = shadows("probe.rs", tree.root_node(), &source, controls, None).unwrap();
        for index in (0..STANDARD_PRELUDE.len()).chain([STD_ROOT]) {
            assert!(found.refuses(index), "{attribute}: {index}");
            let basis = found.basis_for(index, false);
            let witness = basis.iter().find(|b| b.name.is_none()).unwrap();
            assert_eq!(witness.class, "conditional_context");
            assert_eq!(witness.anchor.path, "probe.rs");
            let range = witness.anchor.range.as_ref().unwrap();
            assert_eq!(&source[range.start_byte..range.end_byte], attribute);
        }
    }
}

#[test]
fn cfg_attr_builtin_payloads_keep_the_scoped_provider_audit() {
    let flag = AtomicBool::new(false);
    let controls = (Instant::now() + Duration::from_secs(10), &flag);
    for attribute in [
        "#[cfg_attr(test, derive(Debug, Clone,))]",
        "#[cfg_attr(test, r#derive(Debug))]",
        "#[cfg_attr(test, cfg_attr(feature = \"nested\", derive(r#Debug /* comment */)))]",
        "#[cfg_attr(test, doc = \"derive(Inject)\")]",
    ] {
        for shadow in ["", "use custom::Debug;"] {
            let source = format!("{shadow}\n{attribute}\nstruct Other;");
            let tree = crate::trivia::parse(&source, controls.0, controls.1)
                .unwrap()
                .unwrap();
            let found = shadows("probe.rs", tree.root_node(), &source, controls, None).unwrap();
            let has_derive = !found.derives.is_empty();
            for index in (0..STANDARD_PRELUDE.len()).chain([STD_ROOT]) {
                assert_eq!(
                    found.refuses(index),
                    !shadow.is_empty() && has_derive,
                    "{attribute}: {shadow}: {index}"
                );
            }
            for ((_, start, end), index) in &found.derives {
                assert!(found.refuses_derive(*index));
                assert_eq!(
                    &source[*start..*end],
                    if *index == 1 {
                        "Clone"
                    } else if attribute.contains("r#Debug") {
                        "r#Debug"
                    } else {
                        "Debug"
                    }
                );
            }
        }
    }
}

#[test]
fn cfg_attr_custom_derives_skip_only_provably_inactive_payloads() {
    let flag = AtomicBool::new(false);
    let controls = (Instant::now() + Duration::from_secs(10), &flag);
    let cfg = items::DeclaredCfg::from([("feature".into(), Some("inject".into()))]);
    for (attribute, refuses) in [
        ("#[cfg_attr(feature = \"inject\", derive(Inject))]", true),
        ("#[cfg_attr(feature = \"unlisted\", derive(Inject))]", true),
        (
            "#[cfg_attr(not(feature = \"inject\"), derive(Inject))]",
            false,
        ),
        ("#[cfg_attr(false, derive(Inject))]", false),
        (
            "#[cfg_attr(feature = \"unlisted\", cfg_attr(not(feature = \"inject\"), derive(Inject)))]",
            false,
        ),
        (
            "#[cfg_attr(any(feature = \"inject\", unknown), derive(Inject))]",
            true,
        ),
    ] {
        let source = format!("#[derive(Debug)]\n{attribute}\nstruct Other;");
        let tree = crate::trivia::parse(&source, controls.0, controls.1)
            .unwrap()
            .unwrap();
        let found = shadows("probe.rs", tree.root_node(), &source, controls, Some(&cfg)).unwrap();
        assert_eq!(found.refuses(0), refuses, "{attribute}");
    }
}

#[test]
fn documentary_companions_do_not_make_builtin_derives_conditional() {
    let flag = AtomicBool::new(false);
    let controls = (Instant::now() + Duration::from_secs(10), &flag);
    for companion in [
        "",
        "#[doc(hidden)]",
        "#[doc = \"documented\"]",
        "#[doc=\"documented\"]",
        "/// documented",
        "/** documented */",
    ] {
        let source = format!("#[derive(Debug, Clone)]\n{companion}\nstruct Other;");
        let tree = crate::trivia::parse(&source, controls.0, controls.1)
            .unwrap()
            .unwrap();
        let found = shadows("probe.rs", tree.root_node(), &source, controls, None).unwrap();
        assert!(!found.refuses_derive(0), "{companion}");
        assert!(!found.refuses_derive(1), "{companion}");
        assert!(!found.refuses(0), "{companion}");
        assert!(found.refusal_basis.is_empty(), "{companion}");
    }
    for text in [
        "#[custom::doc(hidden)]",
        "#[documentary]",
        "#[cfg_attr(test, doc(hidden))]",
    ] {
        assert!(!items::context_independent_attribute(text), "{text}");
    }
    assert!(items::context_independent_attribute(
        "#![doc = \"module docs\"]"
    ));
}

#[test]
fn conditional_derives_do_not_erase_genuine_same_spelling_shadows() {
    let flag = AtomicBool::new(false);
    let controls = (Instant::now() + Duration::from_secs(10), &flag);
    for (index, (name, _)) in STANDARD_PRELUDE.iter().enumerate() {
        let source = format!("#[derive(Debug, Serialize)] #[serde(default)] struct {name};");
        let tree = crate::trivia::parse(&source, controls.0, controls.1)
            .unwrap()
            .unwrap();
        let found = shadows("probe.rs", tree.root_node(), &source, controls, None).unwrap();
        assert!(found.refuses(index), "{name}");
        let basis = found.basis_for(index, false);
        assert_eq!(basis.len(), 1);
        assert_eq!(basis[0].class, "shadow");
        assert_eq!(basis[0].name.as_deref(), Some(*name));
        let range = basis[0].anchor.range.as_ref().unwrap();
        assert_eq!(&source[range.start_byte..range.end_byte], *name);
    }
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
fn provenance_filters_unrelated_names_and_keeps_visible_derive_provider_shadows() {
    let mut found = Shadows::default();
    found.name("String", "other.rs", None);
    assert!(!found.refuses(0));
    assert!(found.basis_for(0, false).is_empty());
    found.name("Debug", "other.rs", None);
    assert!(!found.refuses(0));
    assert!(found.basis_for(0, false).is_empty());
    found.derives.insert(("probe.rs".into(), 0, 5), 0);
    assert!(found.refuses(0));
    assert!(found.refuses_derive(0));
    for derive in [false, true] {
        let basis = found.basis_for(0, derive);
        assert_eq!(basis.len(), 1);
        assert_eq!(basis[0].name.as_deref(), Some("Debug"));
    }
}

#[test]
fn shadowed_builtin_derive_providers_veto_types_and_constructors_not_just_derives() {
    let flag = AtomicBool::new(false);
    let controls = (Instant::now() + Duration::from_secs(10), &flag);
    for (derive, (name, _)) in items::BUILTIN_DERIVES.iter().enumerate() {
        let source = format!("use custom::{name}; #[derive({name})] struct Other;");
        let tree = crate::trivia::parse(&source, controls.0, controls.1)
            .unwrap()
            .unwrap();
        let found = shadows("probe.rs", tree.root_node(), &source, controls, None).unwrap();
        assert!(found.refuses_derive(derive), "{name}");
        for index in (0..STANDARD_PRELUDE.len()).chain([STD_ROOT]) {
            assert!(found.refuses(index), "{name}: {index}");
            let basis = found.basis_for(index, false);
            assert_eq!(basis.len(), 1, "{name}: {index}: {basis:?}");
            assert_eq!(basis[0].class, "shadow");
            assert_eq!(basis[0].name.as_deref(), Some(*name));
            let range = basis[0].anchor.range.as_ref().unwrap();
            assert_eq!(
                &source[range.start_byte..range.end_byte],
                format!("custom::{name}")
            );
        }
    }
}
