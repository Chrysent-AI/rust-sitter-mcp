//! Checked-in replica of a binary importing a separate ordinary library module tree.
use crate::fixture_gen::Fixture;
use serde_json::Value;

pub const BIN_ROOT: &str = "cases/chain/src/main.rs";
pub const LIB_ROOT: &str = "cases/chain/src/lib.rs";
pub const SOURCE: &str = "cases/chain/src/scheduler/work.rs";
pub const PARENT: &str = "cases/chain/src/scheduler/mod.rs";
pub const DESTINATION: &str = "cases/chain/src/scheduler/destination.rs";
pub const SELECTED: &str = "fn selected() {}";

pub fn install(repo: &Fixture) {
    repo.write(BIN_ROOT, "use fixture_library::scheduler;\nfn main() {}\n");
    repo.write(LIB_ROOT, "mod scheduler;\n");
    repo.write(PARENT, "mod work;\nmod destination;\nmod dangling;\n");
    repo.write(SOURCE, "fn selected() {}\nfn retained() {}\n");
    repo.write(DESTINATION, "fn keep() {}\n");
}

pub const MACRO_ROOT: &str = include_str!("../fixtures/chain-refusals/macro-root.rs");
pub const MACRO_NESTED_INPUT: &str =
    include_str!("../fixtures/chain-refusals/macro-nested-input.rs");
pub const MACRO_NESTED_OUTPUT: &str =
    include_str!("../fixtures/chain-refusals/macro-nested-output.rs");
pub const INNER_ATTR_ROOT: &str = include_str!("../fixtures/chain-refusals/inner-attr-root.rs");
pub const INNER_ATTR_NESTED: &str = include_str!("../fixtures/chain-refusals/inner-attr-nested.rs");

pub fn named_refusal(diagnostics: &Value, decisions: &Value, reason: &str, cause: &str) {
    linked(diagnostics, decisions);
    let diagnostic = diagnostics
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["reason"] == reason && d["role"] == "source")
        .unwrap_or_else(|| panic!("missing {reason}: {diagnostics}"));
    let decision = decisions
        .as_array()
        .unwrap()
        .iter()
        .find(|d| {
            d["chain_diagnostic_ids"]
                .as_array()
                .unwrap()
                .contains(&diagnostic["id"])
        })
        .unwrap();
    assert_eq!(decision["action"]["route"], "unsupported_in_engine");
    assert_eq!(decision["action"]["construct"], reason);
    for message in ["unresolved_consequence", "next_action"] {
        let text = decision[message].as_str().unwrap();
        assert!(text.contains(reason) && text.contains(cause), "{text}");
        assert!(
            text.contains("advice-only") && text.contains("acknowledgment"),
            "{text}"
        );
        assert!(text.contains(diagnostic["at_file_path"].as_str().unwrap()));
    }
}

pub fn linked(diagnostics: &Value, decisions: &Value) {
    let diagnostics = diagnostics.as_array().unwrap();
    for (index, diagnostic) in diagnostics.iter().enumerate() {
        assert_eq!(diagnostic["id"], format!("chain/{index}"));
        // Diagnostic locations are text-free coordinates, not execution anchors.
        let text = diagnostic.to_string();
        assert!(!text.contains("expected_text") && !text.contains("text_bytes"));
    }
    for decision in decisions.as_array().unwrap() {
        assert!(decision["id"].as_str().is_some_and(|id| !id.is_empty()));
        for id in decision["chain_diagnostic_ids"].as_array().unwrap() {
            assert!(diagnostics.iter().any(|d| d["id"] == *id));
        }
    }
    for diagnostic in diagnostics {
        assert!(decisions.as_array().unwrap().iter().any(|d| {
            d["category"] == "module_context"
                && d["chain_diagnostic_ids"]
                    .as_array()
                    .unwrap()
                    .contains(&diagnostic["id"])
        }));
    }
}
