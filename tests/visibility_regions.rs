#[path = "support/fixture_gen.rs"]
mod fixture_gen;
#[path = "support/move_artifacts.rs"]
mod move_artifacts;
use fixture_gen::{Fixture, observe};
use move_artifacts::{anchor, apply};
use rust_sitter_mcp::{engine::Engine, move_plan::MoveRequest};
use serde_json::{Value, json};
use std::{process::Command, sync::atomic::AtomicBool};

const SOURCE: &str = "cases/layout/scheduler/work/source.rs";
const FIRST: &str = "fn first(&self) -> u8 { self.read() }";
const SECOND: &str = "fn second(&self) -> u8 { self.read() }";

fn fixture(visibility: &str) -> Fixture {
    let repo = Fixture::generate();
    repo.write("cases/layout/lib.rs", "mod scheduler; mod outside;\n");
    repo.write(
        "cases/layout/scheduler.rs",
        "mod work; mod observe; struct Core;\n",
    );
    repo.write(
        "cases/layout/scheduler/work.rs",
        "mod source; mod target;\n",
    );
    repo.write("cases/layout/scheduler/observe.rs", "mod target;\n");
    repo.write("cases/layout/outside.rs", "mod target;\n");
    repo.write(SOURCE, &format!("use crate::scheduler::Core;\nimpl Core {{\n    {visibility}fn read(&self) -> u8 {{ 1 }}\n    {FIRST}\n    {SECOND}\n}}\n"));
    for path in [
        "cases/layout/scheduler/work/target.rs",
        "cases/layout/scheduler/observe/target.rs",
        "cases/layout/outside/target.rs",
    ] {
        repo.write(path, "");
    }
    repo
}
fn movement(repo: &Fixture, method: &str, destination: &str) -> Value {
    json!({"item":anchor(repo,SOURCE,method),"enclosing_impl":anchor(repo,SOURCE,"impl Core "),"destination":{"kind":"existing","path":destination}})
}
fn request(repo: &Fixture, second: Option<&str>) -> Value {
    let mut moves = vec![movement(
        repo,
        FIRST,
        "cases/layout/scheduler/work/target.rs",
    )];
    if let Some(destination) = second {
        moves.push(movement(repo, SECOND, destination));
    }
    json!({"repo_path":repo.0,"crate_root":"cases/layout/lib.rs","paths":["cases/layout"],"moves":moves,"resolve_semantic":true,"semantic_configuration":{"crates":[{"name":"fixture","root_file":"cases/layout/lib.rs","edition":"2024","features":[],"cfg":[],"dependencies":[]}]},"limits":{"time_budget_ms":30000,"diagnostic_count":1000}})
}
fn run(repo: &Fixture, args: Value) -> Value {
    let before = observe(&repo.0);
    let result = Engine::new(repo.0.clone()).unwrap().move_item(
        serde_json::from_value::<MoveRequest>(args).unwrap(),
        &AtomicBool::new(false),
    );
    assert_eq!(observe(&repo.0), before);
    serde_json::to_value(result).unwrap()
}
fn compile(repo: &Fixture) {
    let copy = repo.copy();
    copy.write("Cargo.toml", "[package]\nname = \"visibility-fixture\"\nversion = \"0.0.0\"\nedition = \"2024\"\n[lib]\npath = \"cases/layout/lib.rs\"\n");
    let result = Command::new("cargo")
        .current_dir(&copy.0)
        .args(["check", "--offline", "--quiet"])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
}
fn nested_anchor(repo: &Fixture, outer: &str, occurrence: &str) -> Value {
    let mut result = anchor(repo, SOURCE, outer);
    let start =
        result["range"]["start_byte"].as_u64().unwrap() as usize + outer.find(occurrence).unwrap();
    result["range"] = json!({"start_byte":start,"end_byte":start + occurrence.len()});
    result["expected_text"] = json!(occurrence);
    result
}
fn read_repair(result: &Value) -> &Value {
    let repairs: Vec<_> = result["plan"]["rewrites"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|r| {
            r["kind"] == "visibility"
                && (r["target"]["path"] == SOURCE || r["target"]["anchor"]["path"] == SOURCE)
                && (r["target"]["binding"] == "crate::scheduler::work::source::read"
                    || r["before_text"]
                        .as_str()
                        .is_some_and(|s| s.starts_with("pub(in")))
        })
        .collect();
    assert_eq!(repairs.len(), 1, "{result}");
    repairs[0]
}

#[test]
fn inherent_call_uses_parent_mid_region_or_crate_as_required_by_all_callers() {
    for (second, expected) in [
        (None, "pub(super) "),
        (
            Some("cases/layout/scheduler/observe/target.rs"),
            "pub(in crate::scheduler) ",
        ),
        (Some("cases/layout/outside/target.rs"), "pub(crate) "),
    ] {
        let repo = fixture("");
        compile(&repo);
        let args = request(&repo, second);
        let result = run(&repo, args.clone());
        assert_eq!(result["plan"]["applicable"], true, "{result}");
        let repair = read_repair(&result);
        assert_eq!(repair["after_text"], expected);
        assert_eq!(
            repair["item_ids"].as_array().unwrap().len(),
            if second.is_some() { 2 } else { 1 }
        );
        let visibility = &repair["visibility"];
        assert_eq!(
            visibility["original_defining_region"],
            json!(["scheduler", "work", "source"])
        );
        assert_eq!(
            visibility["final_defining_region"],
            visibility["original_defining_region"]
        );
        assert_eq!(
            visibility["preserved_access_region"],
            visibility["original_defining_region"]
        );
        let expected_region = match second {
            None => json!(["scheduler", "work"]),
            Some("cases/layout/scheduler/observe/target.rs") => json!(["scheduler"]),
            Some(_) => json!([]),
        };
        assert_eq!(visibility["narrowest_covering_region"], expected_region);
        let consumers = visibility["consumers"].as_array().unwrap();
        assert_eq!(consumers.len(), if second.is_some() { 2 } else { 1 });
        for (index, consumer) in consumers.iter().enumerate() {
            assert_eq!(
                consumer["anchor"],
                nested_anchor(&repo, if index == 0 { FIRST } else { SECOND }, "self.read")
            );
            assert_eq!(consumer["access_reason"], "same_type_inherent_access");
            assert_eq!(
                consumer["original_module_region"],
                json!(["scheduler", "work", "source"])
            );
            let final_region = if index == 0 || second.is_none() {
                json!(["scheduler", "work", "target"])
            } else if second == Some("cases/layout/scheduler/observe/target.rs") {
                json!(["scheduler", "observe", "target"])
            } else {
                json!(["outside", "target"])
            };
            assert_eq!(consumer["final_module_region"], final_region);
        }
        if second == Some("cases/layout/outside/target.rs") {
            let type_repair = result["plan"]["rewrites"]
                .as_array()
                .unwrap()
                .iter()
                .find(|r| {
                    r["kind"] == "visibility" && r["target"]["binding"] == "crate::scheduler::Core"
                })
                .unwrap();
            let callers = type_repair["visibility"]["consumers"].as_array().unwrap();
            assert_eq!(callers.len(), 2);
            for caller in callers {
                assert_eq!(caller["anchor"], nested_anchor(&repo, "impl Core ", "Core"));
                assert_eq!(caller["access_reason"], "impl_self_type");
            }
            assert_eq!(
                type_repair["visibility"]["narrowest_covering_region"],
                json!([])
            );
        }
        compile(&apply(&repo, &result));
        // Caller order must not change the covering region or create overlapping splices.
        if second.is_some() {
            let mut reversed = args;
            reversed["moves"].as_array_mut().unwrap().reverse();
            let reverse_result = run(&repo, reversed);
            assert_eq!(read_repair(&reverse_result)["after_text"], expected);
            assert_eq!(
                read_repair(&reverse_result)["visibility"],
                repair["visibility"]
            );
            compile(&apply(&repo, &reverse_result));
        }
    }
}

#[test]
fn insufficient_restriction_escalates_and_insufficient_override_blocks() {
    let repo = fixture("pub(in crate::scheduler::work) ");
    compile(&repo);
    let args = request(&repo, Some("cases/layout/scheduler/observe/target.rs"));
    let result = run(&repo, args.clone());
    assert_eq!(result["plan"]["applicable"], true, "{result}");
    let repair = read_repair(&result);
    assert_eq!(repair["after_text"], "pub(in crate::scheduler)");
    assert_eq!(
        repair["visibility"]["preserved_access_region"],
        json!(["scheduler", "work"])
    );
    assert_eq!(
        repair["visibility"]["narrowest_covering_region"],
        json!(["scheduler"])
    );
    // Include the caller already covered by the original restriction, not only the widening trigger.
    assert_eq!(
        repair["visibility"]["consumers"].as_array().unwrap().len(),
        2
    );
    compile(&apply(&repo, &result));
    for text in [
        "pub(super)",
        "pub(in crate::scheduler::work)",
        "pub(self)",
        "",
    ] {
        let mut alternative = args.clone();
        alternative["rewrite_overrides"] =
            json!([{"target":repair["target"],"action":"replace","replacement_text":text}]);
        let blocked = run(&repo, alternative);
        assert_eq!(blocked["plan"]["applicable"], false, "{blocked}");
        for field in ["edits", "created_files", "patch"] {
            assert!(blocked["plan"][field].is_null());
        }
        assert!(
            blocked["plan"]["decisions"]
                .as_array()
                .unwrap()
                .iter()
                .any(|d| d["reason"] == "visibility_scope_unproved"
                    && d["blocks_applicability"] == true)
        );
    }
    for text in ["pub(in crate::scheduler)", "pub(crate)"] {
        let mut alternative = args.clone();
        alternative["rewrite_overrides"] =
            json!([{"target":repair["target"],"action":"replace","replacement_text":text}]);
        let accepted = run(&repo, alternative);
        assert_eq!(
            read_repair(&accepted)["visibility"]["narrowest_covering_region"],
            json!(["scheduler"])
        );
        compile(&apply(&repo, &accepted));
    }
}

#[test]
fn relative_retained_restriction_can_escalate_without_relocating_its_scope() {
    let repo = fixture("pub(super) ");
    let result = run(
        &repo,
        request(&repo, Some("cases/layout/scheduler/observe/target.rs")),
    );
    assert_eq!(result["plan"]["applicable"], true, "{result}");
    let repairs: Vec<_> = result["plan"]["rewrites"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|r| r["kind"] == "visibility" && r["before_text"] == "pub(super)")
        .collect();
    assert_eq!(repairs.len(), 1);
    assert_eq!(repairs[0]["after_text"], "pub(in crate::scheduler)");
    compile(&apply(&repo, &result));
}

#[test]
fn sufficient_written_access_is_preserved_without_visibility_repair() {
    for visibility in ["pub(in crate::scheduler) ", "pub(crate) "] {
        let repo = fixture(visibility);
        let result = run(
            &repo,
            request(&repo, Some("cases/layout/scheduler/observe/target.rs")),
        );
        assert_eq!(result["plan"]["applicable"], true, "{result}");
        assert!(
            !result["plan"]["rewrites"]
                .as_array()
                .unwrap()
                .iter()
                .any(|r| r["kind"] == "visibility" && r["before_text"] == visibility.trim())
        );
        compile(&apply(&repo, &result));
    }
}

#[test]
fn new_module_declaration_uses_parent_region_and_rechecks_its_override() {
    let repo = fixture("");
    let selected = "pub(in crate::scheduler) fn selected() {}";
    repo.write(SOURCE, selected);
    repo.write("cases/layout/scheduler/work.rs", "pub(super) mod source;\n");
    repo.write(
        "cases/layout/scheduler/observe/target.rs",
        "fn caller() { crate::scheduler::work::source::selected(); }\n",
    );
    compile(&repo);
    let args = json!({"repo_path":repo.0,"crate_root":"cases/layout/lib.rs","paths":["cases/layout"],"moves":[{"item":anchor(&repo,SOURCE,selected),"destination":{"kind":"new_sibling","path":"cases/layout/scheduler/work/moved.rs","parent_path":"cases/layout/scheduler/work.rs"}}]});
    let result = run(&repo, args.clone());
    compile(&apply(&repo, &result));
    let file = &result["plan"]["created_files"][0];
    let repair = result["plan"]["rewrites"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["id"] == file["declaration_visibility_rewrite_id"])
        .unwrap();
    assert_eq!(repair["after_text"], "pub(super) ");
    let visibility = &repair["visibility"];
    assert!(visibility["original_defining_region"].is_null());
    assert_eq!(
        visibility["final_defining_region"],
        json!(["scheduler", "work"])
    );
    assert_eq!(
        visibility["narrowest_covering_region"],
        json!(["scheduler"])
    );
    assert_eq!(visibility["consumers"][0]["access_reason"], "written_path");
    assert_eq!(
        visibility["consumers"][0]["anchor"],
        anchor(
            &repo,
            "cases/layout/scheduler/observe/target.rs",
            "crate::scheduler::work::source::selected"
        )
    );
    for text in ["pub(super) ", "pub(in crate::scheduler) "] {
        let mut alternative = args.clone();
        alternative["rewrite_overrides"] =
            json!([{"target":repair["target"],"action":"replace","replacement_text":text}]);
        compile(&apply(&repo, &run(&repo, alternative)));
    }
    let mut alternative = args;
    alternative["rewrite_overrides"] =
        json!([{"target":repair["target"],"action":"replace","replacement_text":"pub(self) "}]);
    let blocked = run(&repo, alternative);
    assert_eq!(blocked["plan"]["applicable"], false, "{blocked}");
    assert!(blocked["plan"]["patch"].is_null());
}

#[test]
fn visibility_overrides_reject_public_nonancestor_and_injected_tokens() {
    let repo = fixture("");
    let args = request(&repo, None);
    let result = run(&repo, args.clone());
    let repair = read_repair(&result);
    for text in [
        "pub",
        "pub(in crate::outside)",
        "pub(crate) unsafe",
        "pub(in cr ate::scheduler)",
        "pub(/* keep */ super)",
    ] {
        let mut alternative = args.clone();
        alternative["rewrite_overrides"] =
            json!([{"target":repair["target"],"action":"replace","replacement_text":text}]);
        let rejected = run(&repo, alternative);
        assert_eq!(rejected["status"], "failed", "{rejected}");
        assert_eq!(
            rejected["error"]["code"], "INVALID_REWRITE_OVERRIDE",
            "{rejected}"
        );
    }
    // Synthesis targets are in original coordinates, but relative choices use the final scope.
    let moved = result["plan"]["rewrites"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| {
            r["kind"] == "visibility"
                && r["target"]["binding"] == "crate::scheduler::work::target::first"
        })
        .unwrap();
    let mut alternative = args;
    alternative["rewrite_overrides"] =
        json!([{"target":moved["target"],"action":"replace","replacement_text":"pub(super) "}]);
    compile(&apply(&repo, &run(&repo, alternative)));
}

#[test]
fn private_member_preservation_reports_no_new_observed_caller() {
    let repo = fixture("");
    let method = "fn isolated(&self) -> u8 { 1 }";
    let mut args = request(&repo, None);
    repo.write(
        SOURCE,
        &format!("use crate::scheduler::Core; impl Core {{ {method} }}"),
    );
    args["moves"] = json!([movement(
        &repo,
        method,
        "cases/layout/scheduler/work/target.rs"
    )]);
    compile(&repo);
    let result = run(&repo, args);
    assert_eq!(result["plan"]["applicable"], true, "{result}");
    let repair = result["plan"]["rewrites"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| {
            r["kind"] == "visibility"
                && r["target"]["binding"] == "crate::scheduler::work::target::isolated"
        })
        .unwrap();
    let explanation = &repair["visibility"];
    assert_eq!(explanation["consumers"], json!([]));
    assert_eq!(
        explanation["original_defining_region"],
        json!(["scheduler", "work", "source"])
    );
    assert_eq!(
        explanation["final_defining_region"],
        json!(["scheduler", "work", "target"])
    );
    assert_eq!(
        explanation["preserved_original_access_regions"],
        json!([["scheduler", "work", "source"]])
    );
    assert_eq!(
        explanation["narrowest_covering_region"],
        json!(["scheduler", "work"])
    );
    assert!(
        repair["rationale"]
            .as_str()
            .unwrap()
            .contains("no new observed caller")
    );
    assert_eq!(repair["after_text"], "pub(super) ");
    compile(&apply(&repo, &result));
}

#[test]
fn free_binding_visibility_names_the_exact_moving_consumer() {
    let repo = fixture("");
    let selected = "fn selected() -> u8 { helper() }";
    repo.write(SOURCE, &format!("fn helper() -> u8 {{ 1 }}\n{selected}\n"));
    let args = json!({"repo_path":repo.0,"crate_root":"cases/layout/lib.rs","paths":["cases/layout"],"moves":[{"item":anchor(&repo,SOURCE,selected),"destination":{"kind":"existing","path":"cases/layout/scheduler/work/target.rs"}}]});
    let result = run(&repo, args);
    assert_eq!(result["plan"]["applicable"], true, "{result}");
    let repair = result["plan"]["rewrites"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| {
            r["kind"] == "visibility"
                && r["target"]["binding"] == "crate::scheduler::work::source::helper"
        })
        .unwrap();
    assert_eq!(
        repair["visibility"]["consumers"],
        json!([{
            "anchor":nested_anchor(&repo, selected, "helper"),
            "access_reason":"written_binding",
            "original_module_region":["scheduler","work","source"],
            "final_module_region":["scheduler","work","target"]
        }])
    );
    assert_eq!(
        repair["visibility"]["narrowest_covering_region"],
        json!(["scheduler", "work"])
    );
    assert_eq!(repair["after_text"], "pub(super) ");
    assert_eq!(result["plan"]["integrity"]["semantic"], "not_performed");
    assert!(
        result["plan"]["rewrites"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|r| r["kind"] != "visibility")
            .all(|r| r.get("visibility").is_none())
    );
    compile(&apply(&repo, &result));
}
