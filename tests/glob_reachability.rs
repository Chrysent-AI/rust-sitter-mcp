#[path = "support/fixture_gen.rs"]
mod fixture_gen;
#[path = "support/move_artifacts.rs"]
mod move_artifacts;
use fixture_gen::{Fixture, observe};
use move_artifacts::anchor;
use rust_sitter_mcp::{engine::Engine, move_plan::MoveRequest, split::SuggestSplitRequest};
use serde_json::{Value, json};
use std::sync::atomic::AtomicBool;

fn moved(repo: &Fixture, source: &str, root: &str) -> Value {
    let before = observe(&repo.0);
    let request: MoveRequest = serde_json::from_value(json!({
        "repo_path": repo.0, "crate_root": root, "paths": ["cases/layout"],
        "moves": [{"item": anchor(repo, source, "fn selected() {}"),
            "destination": {"kind": "new_sibling", "path": "cases/layout/target.rs", "parent_path": root}}],
        "limits": {"diagnostic_count": 100000}
    })).unwrap();
    let result = serde_json::to_value(
        Engine::new(repo.0.clone())
            .unwrap()
            .move_item(request, &AtomicBool::new(false)),
    )
    .unwrap();
    assert_eq!(observe(&repo.0), before);
    assert_eq!(result["plan"]["integrity"]["semantic"], "not_performed");
    result
}
fn glob_anchors(result: &Value) -> Vec<&str> {
    result["plan"]["decisions"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|d| d["reason"] == "glob_binding_unproved")
        .map(|d| d["anchors"][0]["expected_text"].as_str().unwrap())
        .collect()
}

#[test]
fn unrelated_globs_do_not_inherit_aliases_from_other_files() {
    let repo = Fixture::generate();
    repo.write("cases/layout/lib.rs", "mod source; mod other;\n");
    repo.write("cases/layout/source.rs", "fn selected() {}\n");
    repo.write(
        "cases/layout/other.rs",
        "mod tests; mod syntax; pub enum TextRole { Plain }\n",
    );
    repo.write("cases/layout/other/tests.rs", "use super::*;\n");
    repo.write(
        "cases/layout/other/syntax.rs",
        "use super::TextRole; fn role() { use TextRole::*; }\n",
    );
    // With this caller-selected root, the old fallback collected every crate import's spelling.
    repo.write(
        "cases/layout/aliases.rs",
        "use crate::unrelated::{super, TextRole};\n",
    );
    let result = moved(&repo, "cases/layout/source.rs", "cases/layout/source.rs");
    assert!(glob_anchors(&result).is_empty(), "{result}");
    assert!(result["plan"]["applicable"].as_bool().unwrap(), "{result}");
    let copy = move_artifacts::apply(&repo, &result);
    assert!(
        std::fs::read_to_string(copy.0.join("cases/layout/target.rs"))
            .unwrap()
            .contains("fn selected() {}")
    );
    assert!(
        result["coverage"]["glob_exclusions"]["different_written_module"]
            .as_u64()
            .unwrap()
            >= 1
    );
    assert!(
        result["coverage"]["glob_exclusions"]["enum_variants"]
            .as_u64()
            .unwrap()
            >= 1
    );
}

#[test]
fn same_named_modules_in_unrelated_subtrees_are_not_glob_consumers() {
    let repo = Fixture::generate();
    repo.write("cases/layout/lib.rs", "mod source; mod other;\n");
    repo.write("cases/layout/source.rs", "fn selected() {}\n");
    repo.write(
        "cases/layout/other.rs",
        "mod source; use self::source::*;\n",
    );
    repo.write("cases/layout/other/source.rs", "fn independent() {}\n");
    let result = moved(&repo, "cases/layout/source.rs", "cases/layout/lib.rs");
    assert!(glob_anchors(&result).is_empty(), "{result}");
    assert_eq!(result["plan"]["applicable"], true, "{result}");
}

#[test]
fn reachable_and_uncertain_globs_still_refuse_moves() {
    for import in [
        "use crate::source::*;",
        "use crate::{source::{*}};",
        "use crate::source as alias; use alias::*;",
        "mod child { use super::source::*; }",
        "use crate::source as alias; use alias::unknown::*;",
    ] {
        let repo = Fixture::generate();
        repo.write("cases/layout/lib.rs", &format!("mod source; {import}\n"));
        repo.write("cases/layout/source.rs", "fn selected() {}\n");
        let result = moved(&repo, "cases/layout/source.rs", "cases/layout/lib.rs");
        assert!(!glob_anchors(&result).is_empty(), "{import}: {result}");
        assert_eq!(result["plan"]["applicable"], false);
        assert!(result["plan"]["patch"].is_null());
    }
}

#[test]
fn indirect_and_local_alias_globs_are_not_discharged_as_unrelated() {
    for forwarded in ["pub use crate::source::selected;", "use crate::source::*;"] {
        let repo = Fixture::generate();
        repo.write("cases/layout/lib.rs", "mod source; mod other;\n");
        repo.write("cases/layout/source.rs", "fn selected() {}\n");
        repo.write(
            "cases/layout/other.rs",
            "mod source; use self::source::*;\n",
        );
        repo.write("cases/layout/other/source.rs", forwarded);
        let result = moved(&repo, "cases/layout/source.rs", "cases/layout/lib.rs");
        assert!(!glob_anchors(&result).is_empty(), "{result}");
        assert_eq!(result["plan"]["applicable"], false);
    }
    let repo = Fixture::generate();
    repo.write("cases/layout/lib.rs", "mod source; mod other;\n");
    repo.write("cases/layout/source.rs", "fn selected() {}\n");
    repo.write(
        "cases/layout/other.rs",
        "enum TextRole { Plain } fn role() { use crate::source as TextRole; use TextRole::*; }\n",
    );
    let result = moved(&repo, "cases/layout/source.rs", "cases/layout/lib.rs");
    assert!(!glob_anchors(&result).is_empty(), "{result}");
}

#[test]
fn advice_recognizes_alias_globs_reaching_the_same_test_parent() {
    let repo = Fixture::generate();
    repo.write("cases/layout/lib.rs", "mod source;\n");
    repo.write("cases/layout/source.rs", "fn selected() {}\n#[cfg(test)] mod tests { use super::{self as parent}; use parent::*; fn check() { selected(); } }\n");
    let request: SuggestSplitRequest = serde_json::from_value(json!({"repo_path": repo.0,
        "crate_root": "cases/layout/lib.rs", "source_path": "cases/layout/source.rs",
        "paths": ["cases/layout"], "limits": {"diagnostic_count": 100000}}))
    .unwrap();
    let result = serde_json::to_value(
        Engine::new(repo.0.clone())
            .unwrap()
            .suggest_split(request, &AtomicBool::new(false)),
    )
    .unwrap();
    assert!(
        result["signals"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["kind"] == "cfg_test_consumer"),
        "{result}"
    );
}
