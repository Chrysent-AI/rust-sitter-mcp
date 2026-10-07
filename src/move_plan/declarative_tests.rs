use super::*;

const MACRO: &str = "macro_rules! make { ($name:ident) => { pub struct $name(Opaque); }; }";
const SELECTED: &str = "fn selected(value: crate::model::ItemId) {}";
fn config() -> Configuration {
    serde_json::from_value(serde_json::json!({"crates":[{
        "name":"fixture","root_file":"lib.rs","edition":"2024","features":[],"cfg":[{"key":"enabled","value":null}],"dependencies":[]
    }]})).unwrap()
}
fn coverage(config: &Configuration) -> ResolutionCoverage {
    ResolutionCoverage {
        decisions: 0,
        configuration: config.clone(),
        snapshot_id: String::new(),
        semantic_input_digest: String::new(),
        final_overlay_digest: String::new(),
        analyzer: ANALYZER.into(),
        statement: String::new(),
        omissions: Vec::new(),
        context_evaluations: Vec::new(),
    }
}
fn need() -> Need {
    let at = SELECTED.find("crate::model::ItemId").unwrap();
    Need {
        path: "source.rs".into(),
        range: range(at, at + "crate::model::ItemId".len()),
        item_ids: Vec::new(),
        reason: DecisionReason::ExternalOrMissingBinding,
        category: "test",
        message: "identity unproved".into(),
        attribute_range: None,
        choice_target: None,
        lexical_uncertainty: None,
        refusal_basis: Vec::new(),
    }
}
fn texts(definition: &str) -> BTreeMap<String, String> {
    BTreeMap::from([
        ("lib.rs".into(), "mod source; mod model;".into()),
        ("source.rs".into(), SELECTED.into()),
        ("model.rs".into(), format!("{definition} make!(ItemId);")),
    ])
}

#[test]
fn definition_bytes_and_invocation_origins_are_part_of_both_overlay_identity() {
    let flag = AtomicBool::new(false);
    let controls = (Instant::now() + Duration::from_secs(30), &flag);
    let config = config();
    let old = Inputs::new(texts(MACRO), &config, controls).unwrap();
    let original = texts(MACRO)["model.rs"].clone();
    let mut shifted = texts(MACRO);
    shifted.insert("model.rs".into(), format!("\n{original}"));
    let new = Inputs::new(shifted, &config, controls).unwrap();
    let origins = vec![MoveOrigin {
        id: "gap".into(),
        source_path: "model.rs".into(),
        source_range: range(0, original.len()),
        output_path: "model.rs".into(),
        output_range: range(1, original.len() + 1),
        role: "gap".into(),
    }];
    let mut needs = vec![need()];
    let proofs = evaluate(
        &old,
        &new,
        &mut needs,
        &origins,
        &coverage(&config),
        controls,
    )
    .unwrap();
    assert_eq!(proofs.len(), 1);
    assert_eq!(
        normalize(&proofs[0].final_declaration, &origins, &old),
        Some(proofs[0].declaration.clone())
    );
    assert!(needs.is_empty());
    for (definition, class) in [
        (MACRO.to_string(), None),
        (
            MACRO.replace("Opaque", "Opaque2"),
            Some("semantic_identity_unproved"),
        ),
        (format!(" {MACRO}"), Some("semantic_identity_unproved")),
        (
            format!("#[cfg(enabled)] {MACRO}"),
            Some("semantic_final_fact_unproved"),
        ),
    ] {
        let new = Inputs::new(texts(&definition), &config, controls).unwrap();
        let mut needs = vec![need()];
        let proofs = evaluate(&old, &new, &mut needs, &[], &coverage(&config), controls).unwrap();
        if let Some(class) = class {
            assert!(proofs.is_empty(), "{definition}");
            assert_eq!(needs[0].refusal_basis.last().unwrap().class, class);
        } else {
            assert_eq!(proofs.len(), 1);
            assert!(matches!(
                proofs[0].class,
                ProofClass::DeclarativeMacroIdentity
            ));
            assert_eq!(proofs[0].declaration, proofs[0].final_declaration);
            assert!(needs.is_empty());
        }
    }
}

#[test]
fn excluded_or_external_macro_definitions_and_conditional_parent_modules_refuse() {
    let flag = AtomicBool::new(false);
    let controls = (Instant::now() + Duration::from_secs(30), &flag);
    for kind in [
        "excluded",
        "external",
        "conditional_parent",
        "conditional_invocation",
        "literal_name",
    ] {
        let mut config = config();
        let mut texts = texts(MACRO);
        match kind {
            "excluded" => {
                texts.insert("model.rs".into(), "make!(ItemId);".into());
            }
            "external" => {
                texts.insert("dep.rs".into(), format!("#[macro_export] {MACRO}"));
                texts.insert("model.rs".into(), "dep::make!(ItemId);".into());
                config.crates[0].dependencies.push(DependencyInput {
                    name: "dep".into(),
                    crate_name: "dependency".into(),
                });
                config.crates.push(CrateInput {
                    name: "dependency".into(),
                    root_file: "dep.rs".into(),
                    edition: "2024".into(),
                    features: Vec::new(),
                    cfg: Vec::new(),
                    dependencies: Vec::new(),
                });
            }
            "conditional_parent" => {
                texts.insert(
                    "lib.rs".into(),
                    "mod source; #[cfg(enabled)] mod model;".into(),
                );
            }
            "literal_name" => {
                texts.insert(
                    "model.rs".into(),
                    "macro_rules! make { ($name:ident) => { pub struct ItemId; }; } make!(ItemId);"
                        .into(),
                );
            }
            "conditional_invocation" => {
                texts.insert(
                    "model.rs".into(),
                    format!("{MACRO} #[cfg(enabled)] make!(ItemId);"),
                );
            }
            _ => unreachable!(),
        }
        let old = Inputs::new(texts.clone(), &config, controls).unwrap();
        let new = Inputs::new(texts, &config, controls).unwrap();
        let mut needs = vec![need()];
        let proofs = evaluate(&old, &new, &mut needs, &[], &coverage(&config), controls).unwrap();
        assert!(proofs.is_empty(), "{kind}");
        assert_eq!(
            needs[0].refusal_basis.last().unwrap().class,
            "semantic_source_fact_unproved"
        );
        if kind != "excluded" && kind != "literal_name" {
            assert!(
                old.context
                    .borrow()
                    .values()
                    .any(|e| e.kind == "declarative_macro" && e.status == "skipped"),
                "{kind}"
            );
        }
    }
}
