#[allow(dead_code)]
#[path = "support/fixture_gen.rs"]
mod fixture_gen;
#[allow(dead_code)]
#[path = "support/move_artifacts.rs"]
mod move_artifacts;
use fixture_gen::{Fixture, observe};
use rust_sitter_mcp::{engine::Engine, move_plan::MoveRequest};
use serde_json::{Value, json};
use std::sync::atomic::AtomicBool;

const SELECTED: &str = "fn selected(value: crate::Outcome) -> crate::Outcome { match value { crate::Outcome::Pass => crate::Outcome::Pass, crate::Outcome::Fail(number) => crate::Outcome::Fail(number) } }";

fn fixture(declarations: &str) -> Fixture {
    let repo = Fixture::generate();
    repo.write(
        "cases/layout/lib.rs",
        &format!("mod source;\n{declarations}\n"),
    );
    repo.write("cases/layout/source.rs", SELECTED);
    repo
}
fn request(repo: &Fixture) -> Value {
    json!({"repo_path":repo.0,"crate_root":"cases/layout/lib.rs","paths":["cases/layout"],
        "resolve_semantic":true,"limits":{"time_budget_ms":30000,"diagnostic_count":1000},
        "semantic_configuration":{"crates":[{"name":"fixture","root_file":"cases/layout/lib.rs","edition":"2024","features":[],"cfg":[],"dependencies":[]}]},
        "moves":[{"item":move_artifacts::anchor(repo,"cases/layout/source.rs",SELECTED),"destination":{"kind":"new_sibling","path":"cases/layout/moved.rs","parent_path":"cases/layout/lib.rs"}}]})
}
fn run(repo: &Fixture, args: Value) -> Value {
    let before = observe(&repo.0);
    let result = Engine::new(repo.0.clone()).unwrap().move_item(
        serde_json::from_value::<MoveRequest>(args).unwrap(),
        &AtomicBool::new(false),
    );
    assert_eq!(observe(&repo.0), before, "analysis modified caller source");
    serde_json::to_value(result).unwrap()
}

#[test]
fn variant_identity_survives_own_and_unrelated_additive_derives() {
    for declarations in [
        "#[derive(Clone)] pub enum Outcome { Pass, Fail(u32) }",
        "#[derive(Clone)] pub enum Outcome { Pass, Fail(u32) } #[derive(Custom)] struct Other;",
        "#[derive(Custom)] pub enum Outcome { Pass, Fail(u32) }",
        "#[derive(custom::Serialize, r#Custom,)] pub enum Outcome { Pass, Fail(u32) }",
        "#[cfg_attr(all(), derive(custom::Serialize))] pub enum Outcome { Pass, Fail(u32) }",
    ] {
        let repo = fixture(declarations);
        let result = run(&repo, request(&repo));
        assert_eq!(result["status"], "complete", "{result}");
        assert_eq!(
            result["plan"]["applicable"], true,
            "{declarations}: {result}"
        );
        for artifact in ["edits", "created_files", "patch"] {
            assert!(!result["plan"][artifact].is_null());
        }
        let variants: Vec<_> = result["plan"]["binding_proofs"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|p| p["classification"] == "variant_path")
            .collect();
        assert_eq!(variants.len(), 4, "{result}");
        for proof in variants {
            assert_eq!(proof["class"], "ra_resolved");
            assert_eq!(proof["source_access"], true);
            assert_eq!(proof["final_access"], true);
            assert_eq!(
                proof["declaration"]["anchor"]["expected_text"],
                proof["final_declaration"]["anchor"]["expected_text"]
            );
            assert_eq!(
                proof["declaration"]["crate_origin"],
                proof["final_declaration"]["crate_origin"]
            );
            assert_eq!(
                proof["original_receiver"]["declaration"]["anchor"]["expected_text"],
                "Outcome"
            );
            assert_eq!(proof["adjusted_receiver"], proof["original_receiver"]);
            assert!(proof["basis"].as_str().unwrap().contains("nominal identity via written declaration; derive emits additional items only; generated-item facts retain their veto"));
        }
        let evaluations = result["plan"]["resolution_coverage"]["context_evaluations"]
            .as_array()
            .unwrap();
        for revision in ["original", "final"] {
            assert!(
                evaluations.iter().any(|e| e["revision"] == revision
                    && e["status"] == "admitted"
                    && e["fact_class"] == "nominal_identity"
                    && e["basis"]
                        .as_str()
                        .unwrap()
                        .contains("derive emits additional items only")),
                "{result}"
            );
        }
        // Exercise exact published bytes, not just a true applicability bit.
        let copy = move_artifacts::apply(&repo, &result);
        assert!(
            std::fs::read_to_string(copy.0.join("cases/layout/moved.rs"))
                .unwrap()
                .contains(SELECTED)
        );
    }
}

#[test]
fn unknown_conditions_attributes_missing_enums_and_malformed_derives_still_block() {
    for declarations in [
        "",
        "pub enum Outcome { #[cfg(undeclared)] Pass, Fail(u32) }",
        "pub enum Outcome { Pass, Fail(#[cfg_attr(undeclared, allow(dead_code))] u32) }",
        "pub enum Outcome<T> { Pass, Fail(T) }",
        "#[cfg_attr(undeclared, derive(Custom))] pub enum Outcome { Pass, Fail(u32) }",
        "#[arbitrary] pub enum Outcome { Pass, Fail(u32) }",
        "#[derive(Custom Other)] pub enum Outcome { Pass, Fail(u32) }",
        "#[derive()] pub enum Outcome { Pass, Fail(u32) }",
        "#[derive(custom::)] pub enum Outcome { Pass, Fail(u32) }",
        "#[derive(custom: :Serialize)] pub enum Outcome { Pass, Fail(u32) }",
        "#[derive(,Custom)] pub enum Outcome { Pass, Fail(u32) }",
    ] {
        let repo = fixture(declarations);
        let result = run(&repo, request(&repo));
        assert_eq!(
            result["plan"]["applicable"], false,
            "{declarations}: {result}"
        );
        for artifact in ["edits", "created_files", "patch"] {
            assert!(result["plan"][artifact].is_null());
        }
        assert!(
            !result["plan"]["binding_proofs"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|p| p["classification"] == "variant_path"),
            "{result}"
        );
        if declarations.contains("cfg_attr") {
            assert!(
                result["plan"]["resolution_coverage"]["context_evaluations"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|e| e["reason"] == "undeclared_cfg_atom"
                        && e["fact_class"] == "nominal_identity"),
                "{result}"
            );
        }
    }
}

#[test]
fn final_variant_shadow_or_unknown_cfg_attribute_is_not_original_identity() {
    for destination in [
        "pub enum Outcome { Pass, Fail(u32) }",
        "#[cfg_attr(undeclared, derive(Custom))] struct Other;",
    ] {
        let repo = fixture("pub enum Outcome { Pass, Fail(u32) }");
        repo.write(
            "cases/layout/lib.rs",
            "mod source; mod destination; pub enum Outcome { Pass, Fail(u32) }",
        );
        repo.write("cases/layout/destination.rs", destination);
        // The explicit crate path does not see an unrelated same-name destination
        // enum; a local path does, and needs must compare their actual identities.
        let text = SELECTED.replace("crate::Outcome", "Outcome");
        let mut args = request(&repo);
        repo.write(
            "cases/layout/source.rs",
            &format!("use crate::Outcome; {text}"),
        );
        args["moves"][0]["item"] = move_artifacts::anchor(&repo, "cases/layout/source.rs", &text);
        args["moves"][0]["destination"] =
            json!({"kind":"existing","path":"cases/layout/destination.rs"});
        let result = run(&repo, args);
        assert_eq!(
            result["plan"]["applicable"], false,
            "{destination}: {result}"
        );
        assert!(result["plan"]["patch"].is_null());
    }
}
