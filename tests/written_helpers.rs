#[path = "support/fixture_gen.rs"]
mod fixture_gen;
#[path = "support/move_artifacts.rs"]
mod move_artifacts;
use fixture_gen::Fixture;
use rust_sitter_mcp::{engine::Engine, move_plan::MoveRequest};
use serde_json::{Value, json};
use std::sync::atomic::AtomicBool;

fn run(repo: &Fixture, text: &str, helpers: Option<bool>) -> Value {
    let mut args = json!({"repo_path":repo.0,"crate_root":"cases/layout/lib.rs","paths":["cases/layout"],"resolve_semantic":true,
        "semantic_configuration":{"crates":[{"name":"fixture","root_file":"cases/layout/lib.rs","edition":"2024","features":[],"cfg":[],"dependencies":[]}]},
        "moves":[{"item":move_artifacts::anchor(repo,"cases/layout/source.rs",text),"destination":{"kind":"new_sibling","path":"cases/layout/moved.rs","parent_path":"cases/layout/lib.rs"}}]});
    if let Some(value) = helpers {
        args["assume_declared_helpers"] = json!(value);
    }
    serde_json::to_value(Engine::new(repo.0.clone()).unwrap().move_item(
        serde_json::from_value::<MoveRequest>(args).unwrap(),
        &AtomicBool::new(false),
    ))
    .unwrap()
}
fn fixture(model: &str, text: &str) -> Fixture {
    let repo = Fixture::generate();
    repo.write("cases/layout/lib.rs", "mod source; pub mod model;");
    repo.write("cases/layout/model.rs", model);
    repo.write("cases/layout/source.rs", text);
    repo
}
#[test]
fn written_helpers_require_assertion_and_keep_both_overlay_assumptions_separate() {
    let text = "fn selected(value: crate::model::Record) -> crate::model::Record { value }";
    for attr in [
        "#[serde(transparent)]",
        "#[serde(rename_all = \"snake_case\")]",
        "#[serde(deny_unknown_fields)]",
        "#[provider::helper(value)]",
    ] {
        let repo = fixture(
            &format!("#[derive(Serialize)] {attr} pub struct Record;"),
            text,
        );
        let omitted = run(&repo, text, None);
        assert_eq!(omitted, run(&repo, text, Some(false)));
        assert_eq!(omitted["plan"]["applicable"], false);
        let value = run(&repo, text, Some(true));
        assert_eq!(value["plan"]["applicable"], true, "{value}");
        assert_eq!(value["coverage"]["assumed_declared_identity"], 2);
        assert!(value["coverage"].get("ra_resolved").is_none());
        for proof in value["plan"]["binding_proofs"].as_array().unwrap() {
            assert_eq!(proof["class"], "assumed_declared_identity");
            assert_eq!(
                proof["declaration"]["anchor"]["expected_text"],
                proof["final_declaration"]["anchor"]["expected_text"]
            );
            assert!(
                proof["declaration"]["anchor"]["expected_text"]
                    .as_str()
                    .is_some_and(|text| text == "Record"
                        || text == attr
                        || text == "#[derive(Serialize)]")
            );
            assert!(
                proof["basis"]
                    .as_str()
                    .unwrap()
                    .contains("not engine-classified")
            );
        }
        let contexts = value["plan"]["resolution_coverage"]["context_evaluations"]
            .as_array()
            .unwrap();
        for revision in ["original", "final"] {
            assert!(contexts.iter().any(|e| e["revision"] == revision
                && e["anchor"]["expected_text"] == attr
                && e["reason"] == "assumed_declared_helpers"));
        }
        move_artifacts::apply(&repo, &value);
    }
}
#[test]
fn written_assumption_does_not_admit_control_attributes_or_generated_item_facts() {
    let text = "fn selected(value: crate::model::Record) { let _ = value.read(); }";
    let repo = fixture(
        "#[derive(Serialize)] #[serde(transparent)] pub struct Record; impl Record { pub fn read(&self) {} }",
        text,
    );
    assert_eq!(run(&repo, text, Some(true))["plan"]["applicable"], false);
    let text = "fn selected(value: crate::model::Record) {}";
    for attr in [
        "#[cfg(unknown)]",
        "#[cfg_attr(unknown, serde(transparent))]",
        "#[no_implicit_prelude]",
    ] {
        let repo = fixture(
            &format!("{attr} #[serde(transparent)] pub struct Record;"),
            text,
        );
        assert_eq!(run(&repo, text, Some(true))["plan"]["applicable"], false);
    }
}
