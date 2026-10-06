#[allow(dead_code)]
#[path = "support/fixture_gen.rs"]
mod fixture_gen;
#[allow(dead_code)]
#[path = "support/move_artifacts.rs"]
mod move_artifacts;
use fixture_gen::{Fixture, observe};
use move_artifacts::anchor;
use rust_sitter_mcp::{engine::Engine, move_plan::MoveRequest};
use serde_json::{Value, json};
use std::{fs, sync::atomic::AtomicBool};

fn run(repo: &Fixture, args: Value) -> Value {
    let before = observe(&repo.0);
    let result = Engine::new(repo.0.clone()).unwrap().move_item(
        serde_json::from_value::<MoveRequest>(args).unwrap(),
        &AtomicBool::new(false),
    );
    assert_eq!(observe(&repo.0), before);
    serde_json::to_value(result).unwrap()
}
fn request(repo: &Fixture, items: &[&str]) -> Value {
    json!({"repo_path":repo.0,"crate_root":"cases/layout/lib.rs","paths":["cases/layout"],
        "assume_standard_prelude":true,"limits":{"diagnostic_count":100000},
        "moves":items.iter().map(|text| json!({"item":anchor(repo,"cases/layout/source.rs",text),
            "destination":{"kind":"new_sibling","path":"cases/layout/moved.rs","parent_path":"cases/layout/lib.rs"}})).collect::<Vec<_>>()})
}
fn witnessed(repo: &Fixture, result: &Value, class: &str, path: &str, text: &str) -> bool {
    result["plan"]["decisions"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|d| d["refusal_basis"].as_array().into_iter().flatten())
        .any(|b| {
            if b["class"] != class || b["anchor"]["path"] != path {
                return false;
            }
            let range = &b["anchor"]["range"];
            let source = fs::read_to_string(repo.0.join(path)).unwrap();
            match (range["start_byte"].as_u64(), range["end_byte"].as_u64()) {
                (Some(start), Some(end)) => &source[start as usize..end as usize] == text,
                _ => false,
            }
        })
}

#[test]
fn zero_prelude_proofs_have_exact_source_destination_and_chain_veto_anchors() {
    let selected = "struct Record { option: Option<u8> }";
    for (prefix, class, witness) in [
        ("introduce!();", "chain_macro_statement", "introduce!()"),
        (
            "#[derive(custom::Debug)] struct Other;",
            "derive_veto",
            "custom::Debug",
        ),
        ("struct Option;", "shadow", "Option"),
        (
            "#[cfg(test)] mod tests;",
            "conditional_context",
            "#[cfg(test)]",
        ),
        ("use unknown::*;", "glob_import", "use unknown::*;"),
    ] {
        let repo = Fixture::generate();
        repo.write(
            "cases/layout/lib.rs",
            &format!("mod source; mod destination; {prefix}"),
        );
        repo.write("cases/layout/source.rs", selected);
        repo.write("cases/layout/destination.rs", "");
        let result = run(&repo, request(&repo, &[selected]));
        assert_eq!(result["plan"]["applicable"], false, "{result}");
        assert!(result["plan"]["patch"].is_null());
        assert!(result["coverage"].get("standard_prelude").is_none());
        let option = result["plan"]["decisions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|d| d["anchors"][0]["expected_text"] == "Option")
            .unwrap();
        assert!(
            option["refusal_basis"]
                .as_array()
                .unwrap()
                .iter()
                .any(|b| b["class"] == class),
            "{option}"
        );
        assert!(
            witnessed(&repo, &result, class, "cases/layout/lib.rs", witness),
            "{result}"
        );
    }
}

#[test]
fn probe_a_shape_discloses_conditional_and_derive_causes_on_retained_option() {
    let repo = Fixture::generate();
    repo.write("cases/layout/lib.rs", "mod source; #[cfg(test)] mod tests;");
    let a = "fn claim_review_status(outcome: Outcome) -> Status { match outcome { Outcome::Pass => Status::Pass } }";
    let b = "fn single_transition(effects: &[Effect]) -> Option<&Transition> { let [Effect::RecordTransition(transition)] = effects else { return None; }; Some(transition) }";
    repo.write("cases/layout/source.rs", &format!("enum Outcome {{ Pass }} enum Status {{ Pass }} struct Transition; #[derive(custom::Debug)] enum Effect {{ RecordTransition(Transition) }} {a} {b}"));
    let result = run(&repo, request(&repo, &[a, b]));
    assert_eq!(result["plan"]["applicable"], false, "{result}");
    let option = result["plan"]["decisions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["anchors"][0]["expected_text"] == "Option")
        .unwrap();
    for class in ["conditional_context", "derive_veto"] {
        assert!(
            option["refusal_basis"]
                .as_array()
                .unwrap()
                .iter()
                .any(|b| b["class"] == class),
            "{option}"
        );
    }
    for decision in result["plan"]["decisions"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|d| d["blocks_applicability"] == true)
    {
        assert!(
            !decision["refusal_basis"].as_array().unwrap().is_empty(),
            "{decision}"
        );
    }
    assert!(witnessed(
        &repo,
        &result,
        "conditional_context",
        "cases/layout/lib.rs",
        "#[cfg(test)]"
    ));
    assert!(witnessed(
        &repo,
        &result,
        "derive_veto",
        "cases/layout/source.rs",
        "custom::Debug"
    ));
    assert!(result["plan"].get("binding_proofs").is_none(), "{result}");
}

#[test]
fn incomplete_destination_chain_and_synthetic_names_have_path_only_anchors() {
    let selected = "struct Record { option: Option<u8> }";
    let repo = Fixture::generate();
    repo.write("cases/layout/lib.rs", "mod source;");
    repo.write("cases/layout/source.rs", selected);
    repo.write("cases/layout/destination.rs", "");
    let mut args = request(&repo, &[selected]);
    args["moves"][0]["destination"] =
        json!({"kind":"existing","path":"cases/layout/destination.rs"});
    let result = run(&repo, args);
    assert_eq!(result["plan"]["applicable"], false);
    assert!(result["plan"]["decisions"].as_array().unwrap().iter().flat_map(|d| d["refusal_basis"].as_array().into_iter().flatten()).any(|b| {
        b == &json!({"class":"unresolved_chain","anchor":{"path":"cases/layout/destination.rs"}})
    }), "{result}");
    let mut args = request(&repo, &[selected]);
    args["moves"][0]["destination"]["path"] = json!("cases/layout/Option.rs");
    let result = run(&repo, args);
    assert!(result["plan"]["decisions"].as_array().unwrap().iter().flat_map(|d| d["refusal_basis"].as_array().into_iter().flatten()).any(|b| {
        b == &json!({"class":"shadow","name":"Option","anchor":{"path":"cases/layout/Option.rs"}})
    }), "{result}");
}
