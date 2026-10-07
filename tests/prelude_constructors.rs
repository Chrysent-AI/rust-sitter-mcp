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

const SELECTED: &str = "fn selected() { let _ = Some(1); let _ = None; }";

fn request(repo: &Fixture, prefix: &str, text: &str, enabled: bool) -> Value {
    repo.write("cases/layout/lib.rs", "mod source; mod destination;");
    repo.write("cases/layout/source.rs", &format!("{prefix}{text}"));
    repo.write("cases/layout/destination.rs", "");
    json!({"repo_path":repo.0,"crate_root":"cases/layout/lib.rs","paths":["cases/layout"],
        "assume_standard_prelude":enabled,
        "moves":[{"item":move_artifacts::anchor(repo,"cases/layout/source.rs",text),"destination":{"kind":"existing","path":"cases/layout/destination.rs"}}]})
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
fn constructors(value: &Value) -> Vec<&Value> {
    value["plan"]["binding_proofs"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|p| p["class"] == "standard_prelude_constructor")
        .collect()
}

#[test]
fn bare_value_constructors_are_explicit_unshadowed_assumptions_not_semantic_proofs() {
    let repo = Fixture::generate();
    let args = request(&repo, "", SELECTED, true);
    let value = run(&repo, args.clone());
    assert_eq!(value["plan"]["applicable"], true, "{value}");
    assert_eq!(value["coverage"]["standard_prelude_constructor"], 2);
    assert_eq!(value["plan"]["integrity"]["semantic"], "not_performed");
    assert!(value["coverage"]["standard_prelude"].is_null());
    let proofs = constructors(&value);
    assert_eq!(proofs.len(), 2);
    for proof in proofs {
        let name = proof["anchor"]["expected_text"].as_str().unwrap();
        assert!(matches!(name, "Some" | "None"));
        assert_eq!(
            proof["standard_path"],
            format!("std::option::Option::{name}")
        );
        assert!(
            proof["basis"]
                .as_str()
                .unwrap()
                .contains("constructor identity assumed with the type; not macro hygiene proved")
        );
    }
    let copy = move_artifacts::apply(&repo, &value);
    let moved = std::fs::read_to_string(copy.0.join("cases/layout/destination.rs")).unwrap();
    assert!(moved.contains(SELECTED));
    assert!(!moved.contains("use std::"));
    let mut off = args.clone();
    off["assume_standard_prelude"] = json!(false);
    let off_result = run(&repo, off);
    assert_eq!(off_result["plan"]["applicable"], false);
    assert!(off_result["plan"]["patch"].is_null());
    assert!(constructors(&off_result).is_empty());
    let mut default = args;
    default
        .as_object_mut()
        .unwrap()
        .remove("assume_standard_prelude");
    assert_eq!(run(&repo, default), off_result);
}

#[test]
fn option_or_constructor_shadows_and_expansion_controls_still_refuse_assumptions() {
    for (prefix, text, destination) in [
        ("struct Option; ", SELECTED, ""),
        (
            "",
            "fn selected() { struct Option; let _ = Some(1); let _ = None; }",
            "",
        ),
        ("", SELECTED, "struct Option;"),
        ("", SELECTED, "const Some: u32 = 0; const None: u32 = 0;"),
        (
            "",
            "fn selected(Some: u32, None: u32) { let _ = Some; let _ = None; }",
            "",
        ),
        ("use unknown::*; ", SELECTED, ""),
        ("#![no_implicit_prelude]\n", SELECTED, ""),
        (
            "",
            "fn selected() { let _ = Some(1); let _ = None; introduce!(); }",
            "",
        ),
        (
            "",
            "fn selected() { #[derive(Custom)] struct Local; let _ = Some(1); let _ = None; }",
            "",
        ),
    ] {
        let repo = Fixture::generate();
        let args = request(&repo, prefix, text, true);
        repo.write("cases/layout/destination.rs", destination);
        let value = run(&repo, args);
        assert!(
            constructors(&value).is_empty(),
            "{prefix} {text} {destination}: {value}"
        );
        assert!(value["coverage"]["standard_prelude_constructor"].is_null());
    }
}

#[test]
fn qualified_paths_and_patterns_are_never_assumed_prelude_constructors() {
    for text in [
        "fn selected() { let _ = Option::Some(1); let _ = Option::None; }",
        "fn selected() { let Some(value) = unknown else { return; }; }",
        "fn selected() { match unknown { None => (), Some(value) => () } }",
    ] {
        let repo = Fixture::generate();
        let args = request(&repo, "", text, true);
        let value = run(&repo, args);
        assert!(constructors(&value).is_empty(), "{text}: {value}");
        assert_eq!(value["plan"]["applicable"], false, "{value}");
        assert!(value["plan"]["patch"].is_null());
    }
}
