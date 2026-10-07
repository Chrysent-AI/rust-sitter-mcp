#[allow(dead_code)]
#[path = "support/fixture_gen.rs"]
mod fixture_gen;
#[allow(dead_code)]
#[path = "support/move_artifacts.rs"]
mod move_artifacts;
use fixture_gen::{Fixture, observe};
use move_artifacts::{anchor, apply};
use rust_sitter_mcp::{engine::Engine, move_plan::MoveRequest};
use serde_json::{Value, json};
use std::{fs, sync::atomic::AtomicBool};

const SOURCE: &str = "cases/layout/operations.rs";
const CONSUMER: &str = "cases/layout/operations/verification_flow/settlement.rs";
const SELECTED: &str = "pub(crate) fn selected() {}";

fn fixture(consumer: &str) -> Fixture {
    let repo = Fixture::generate();
    repo.write(
        "cases/layout/lib.rs",
        "mod operations;\nmod unrelated;\nmod forwarding;\nmod explicit;\n",
    );
    repo.write(
        SOURCE,
        &format!("mod verification_flow;\n{SELECTED}\nfn retained() {{ selected(); }}\n"),
    );
    repo.write(
        "cases/layout/operations/verification_flow.rs",
        "mod settlement;\n",
    );
    repo.write(CONSUMER, consumer);
    repo.write("cases/layout/unrelated.rs", "pub(crate) fn other() {}\n");
    repo.write("cases/layout/forwarding.rs", "\n");
    repo.write("cases/layout/explicit.rs", "\n");
    repo
}
fn run(repo: &Fixture, acknowledge: bool) -> Value {
    run_selected(repo, acknowledge, SELECTED)
}
fn run_selected(repo: &Fixture, acknowledge: bool, selected: &str) -> Value {
    let before = observe(&repo.0);
    let args = json!({"repo_path":repo.0,"crate_root":"cases/layout/lib.rs","paths":["cases/layout"],
        "acknowledge_test_consumers":acknowledge,"limits":{"diagnostic_count":512},
        "moves":[{"item":anchor(repo,SOURCE,selected),"destination":{"kind":"new_sibling","path":"cases/layout/projections.rs","parent_path":"cases/layout/lib.rs"}}]});
    let result = Engine::new(repo.0.clone()).unwrap().move_item(
        serde_json::from_value::<MoveRequest>(args).unwrap(),
        &AtomicBool::new(false),
    );
    assert_eq!(observe(&repo.0), before);
    serde_json::to_value(result).unwrap()
}
fn glob_decisions(result: &Value) -> Vec<&Value> {
    result["plan"]["decisions"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|d| d["reason"] == "glob_consumer_unrepaired")
        .collect()
}
fn assert_blocked(repo: &Fixture, result: &Value, path: &str, glob: &str) {
    assert_eq!(result["status"], "complete", "{result}");
    assert_eq!(result["plan"]["applicable"], false, "{result}");
    for field in ["edits", "created_files", "patch"] {
        assert!(result["plan"][field].is_null());
    }
    let decisions = glob_decisions(result);
    let decision = decisions
        .iter()
        .find(|d| d["anchors"][0]["path"] == path)
        .unwrap_or_else(|| panic!("{result}"));
    assert_eq!(decision["blocks_applicability"], true);
    assert_eq!(decision["action"]["route"], "selection_change_required");
    let source = fs::read_to_string(repo.0.join(path)).unwrap();
    let at = source.rfind("selected();").unwrap();
    assert_eq!(
        decision["anchors"][0],
        json!({"path":path,"range":{"start_byte":at,"end_byte":at+"selected".len()},"expected_text":"selected"})
    );
    assert!(
        decision["anchors"]
            .as_array()
            .unwrap()
            .iter()
            .any(|a| a == &anchor(repo, path, glob))
    );
    assert!(
        decision["refusal_basis"]
            .as_array()
            .unwrap()
            .iter()
            .any(|b| b["class"] == "glob_import"
                && b["anchor"]["range"] == anchor(repo, path, glob)["range"])
    );
}

#[test]
fn chain_glob_settlement_call_is_anchored_and_cannot_be_acknowledged() {
    // Unicode and CRLF ensure the exposed coordinates are bytes, not columns.
    let repo = fixture("use super::super::*;\r\n// é\r\nfn settle() { selected(); }\r\n");
    repo.write(SOURCE, "mod verification_flow;\nfn selected() {}\n");
    for acknowledge in [false, true] {
        let result = run_selected(&repo, acknowledge, "fn selected() {}");
        assert_blocked(&repo, &result, CONSUMER, "use super::super::*;");
        assert_eq!(glob_decisions(&result).len(), 1);
    }
}

#[test]
fn grouped_alias_block_inline_and_forwarded_glob_matrix() {
    for (consumer, glob) in [
        (
            "use crate::operations::*;\nfn settle() { selected(); }",
            "use crate::operations::*;",
        ),
        (
            "use super::super as parent;\nuse parent::{*};\nfn settle() { selected(); }",
            "use parent::{*};",
        ),
        (
            "fn settle() { use super::super::*; selected(); }",
            "use super::super::*;",
        ),
        (
            "mod nested { use super::super::super::*; fn settle() { selected(); } }",
            "use super::super::super::*;",
        ),
        (
            "use crate::forwarding::*;\nfn settle() { selected(); }",
            "use crate::forwarding::*;",
        ),
    ] {
        let repo = fixture(consumer);
        repo.write(
            "cases/layout/forwarding.rs",
            "pub(crate) use crate::operations::*;\n",
        );
        assert_blocked(&repo, &run(&repo, false), CONSUMER, glob);
    }
}

#[test]
fn mixed_files_keep_explicit_and_same_file_repair_previews() {
    let repo = fixture("use super::super::*;\nfn settle() { selected(); }");
    repo.write(
        "cases/layout/explicit.rs",
        "use crate::operations::selected;\nfn explicit() { selected(); }\n",
    );
    let result = run(&repo, false);
    assert_blocked(&repo, &result, CONSUMER, "use super::super::*;");
    for path in [SOURCE, "cases/layout/explicit.rs"] {
        assert!(
            result["plan"]["rewrites"]
                .as_array()
                .unwrap()
                .iter()
                .any(|r| {
                    (r["target"]["path"] == path || r["target"]["anchor"]["path"] == path)
                        && r["after_text"]
                            .as_str()
                            .is_some_and(|t| t.contains("crate::projections::selected"))
                }),
            "{result}"
        );
    }
}

#[test]
fn explicit_consumer_and_unrelated_glob_remain_applicable() {
    let repo = fixture(
        "use crate::unrelated::*;\nuse crate::operations::selected;\nfn settle() { selected(); }",
    );
    let compile = |repo: &Fixture| {
        let output = std::process::Command::new("rustc")
            .args(["--crate-type=lib", "--edition=2024"])
            .arg(repo.0.join("cases/layout/lib.rs"))
            .arg("-o")
            .arg(repo.0.join("consumer-control.rlib"))
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    };
    compile(&repo);
    let result = run(&repo, false);
    assert!(glob_decisions(&result).is_empty(), "{result}");
    assert_eq!(result["plan"]["applicable"], true, "{result}");
    let copy = apply(&repo, &result);
    compile(&copy);
    for path in [SOURCE, CONSUMER] {
        assert!(
            fs::read_to_string(copy.0.join(path))
                .unwrap()
                .contains("crate::projections::selected")
        );
    }
}

#[test]
fn type_only_and_conditional_names_do_not_hide_glob_value_consumers() {
    for consumer in [
        "use super::super::*;\ntype selected = u8;\nfn settle() { selected(); }",
        "use super::super::*;\n#[cfg(any())]\nfn selected() {}\nfn settle() { selected(); }",
        "use super::super::*;\n#[cfg(any())]\nuse crate::unrelated::other as selected;\nfn settle() { selected(); }",
        "use super::super::*;\nuse crate::unrelated::Alias as selected;\nfn settle() { selected(); }",
    ] {
        let repo = fixture(consumer);
        repo.write(
            "cases/layout/unrelated.rs",
            "pub(crate) type Alias = u8;\npub(crate) fn other() {}\n",
        );
        assert_blocked(&repo, &run(&repo, false), CONSUMER, "use super::super::*;");
    }
}

#[test]
fn explicit_import_beats_reachable_chain_glob() {
    let repo = fixture(
        "use super::super::*;\nuse crate::operations::selected;\nfn settle() { selected(); }",
    );
    let result = run(&repo, false);
    assert!(glob_decisions(&result).is_empty(), "{result}");
    assert_eq!(result["plan"]["applicable"], true, "{result}");
}

#[test]
fn out_of_scope_globs_and_independent_locals_do_not_become_consumers() {
    for consumer in [
        "mod sibling { use super::super::super::*; }\nfn settle() {}",
        "use super::super::*;\nfn settle(selected: fn()) { selected(); }",
        "use crate::unrelated::*;\nfn selected() {}\nfn settle() { selected(); }",
        "use crate::unrelated::*;\nfn settle() {}",
        "use super::super::*;\nfn selected() {}\nfn settle() { selected(); }",
    ] {
        let repo = fixture(consumer);
        let result = run(&repo, false);
        assert!(glob_decisions(&result).is_empty(), "{result}");
    }
}
