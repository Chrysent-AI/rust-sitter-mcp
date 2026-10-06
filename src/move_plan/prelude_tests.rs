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
        let found = shadows("probe.rs", tree.root_node(), source, controls).unwrap();
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
    let found = shadows("probe.rs", tree.root_node(), source, controls).unwrap();
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
