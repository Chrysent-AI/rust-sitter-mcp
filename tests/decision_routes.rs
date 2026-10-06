#[allow(dead_code)]
#[path = "support/fixture_gen.rs"]
mod fixture_gen;
#[allow(dead_code)]
#[path = "support/move_artifacts.rs"]
mod move_artifacts;
#[path = "support/stdio_client.rs"]
mod stdio_client;
use fixture_gen::{Fixture, observe};
use move_artifacts::anchor;
use serde_json::{Value, json};
use std::collections::BTreeSet;
use stdio_client::Client;

fn fixture(source: &str) -> Fixture {
    let repo = Fixture::generate();
    repo.write("cases/layout/lib.rs", "mod source;\nmod destination;\n");
    repo.write("cases/layout/source.rs", source);
    repo.write("cases/layout/destination.rs", "fn collision() {}\n");
    repo
}
fn request(repo: &Fixture, selected: &[&str]) -> Value {
    json!({"repo_path":repo.0,"crate_root":"cases/layout/lib.rs","paths":["cases/layout"],"limits":{"text_bytes":0},"moves":selected.iter().map(|text| json!({"item":anchor(repo,"cases/layout/source.rs",text),"destination":{"kind":"existing","path":"cases/layout/destination.rs"}})).collect::<Vec<_>>()})
}
fn groups(value: &Value) {
    let decisions = value["decisions"].as_array().unwrap();
    let mut seen = BTreeSet::new();
    for group in value["decision_groups"].as_array().unwrap() {
        let ids: Vec<String> = group["decision_ids"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|id| {
                if let Some(id) = id.as_str() {
                    return vec![id.to_owned()];
                }
                let first = id["first_id"]
                    .as_str()
                    .unwrap()
                    .strip_prefix("d/")
                    .unwrap()
                    .parse::<usize>()
                    .unwrap();
                (first..first + id["count"].as_u64().unwrap() as usize)
                    .map(|n| format!("d/{n}"))
                    .collect()
            })
            .collect();
        assert_eq!(group["count"].as_u64().unwrap() as usize, ids.len());
        for id in ids {
            assert!(seen.insert(id.clone()));
            let d = decisions.iter().find(|d| d["id"] == id).unwrap();
            for field in ["category", "reason", "blocks_applicability"] {
                assert_eq!(group[field], d[field]);
            }
            assert_eq!(group["route"], d["action"]["route"]);
            assert!(!d["anchors"].as_array().unwrap().is_empty());
        }
    }
    assert_eq!(seen.len(), decisions.len());
}
#[test]
fn blocked_batch_exposes_three_routes_and_choice_replay_clears_only_its_cause() {
    let selected = [
        "fn selected() {}",
        "fn collision() {}",
        "fn member(x: u8) { x.foo(); x.bar(); }",
    ];
    let repo = fixture(&format!(
        "{}\n{}\n{}\nfn caller() {{ selected(); }}\n",
        selected[0], selected[1], selected[2]
    ));
    let before = observe(&repo.0);
    let mut client = Client::new();
    let args = request(&repo, &selected);
    let preview = client.call("move_item", args.clone());
    let import = preview["plan"]["rewrites"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["kind"] == "import_insert" && r["target"]["path"] == "cases/layout/source.rs")
        .unwrap();
    let mut rejected = args;
    rejected["limits"]["diagnostic_count"] = json!(64);
    rejected["rewrite_overrides"] = json!([{"target":import["target"],"action":"retain"}]);
    let result = client.call("move_item", rejected.clone());
    assert_eq!(result["schema_version"], 2);
    let plan = &result["plan"];
    assert_eq!(plan["applicable"], false);
    for field in ["edits", "created_files", "patch"] {
        assert!(plan[field].is_null());
    }
    groups(plan);
    let decisions = plan["decisions"].as_array().unwrap();
    let retained = decisions
        .iter()
        .find(|d| {
            d["reason"] == "required_rewrite_retained" && d["action"]["target"] == import["target"]
        })
        .unwrap();
    assert_eq!(retained["action"]["route"], "request_field");
    assert_eq!(retained["action"]["field"], "rewrite_overrides[]");
    assert_eq!(retained["action"]["tool"], "move_item");
    assert_eq!(retained["action"]["purpose"], "resolve_decision");
    assert_eq!(
        retained["action"]["choices"],
        json!(["accept_default", "replace"])
    );
    let collision = decisions
        .iter()
        .find(|d| d["reason"] == "destination_binding_conflict")
        .unwrap();
    assert_eq!(collision["action"]["route"], "selection_change_required");
    assert!(
        collision["action"]["fields"]
            .as_array()
            .unwrap()
            .contains(&json!("moves[index].destination"))
    );
    assert_eq!(
        collision["anchors"][0],
        anchor(&repo, "cases/layout/destination.rs", "fn collision() {}")
    );
    let members: Vec<_> = decisions
        .iter()
        .filter(|d| d["reason"] == "member_or_constructor_unproved")
        .collect();
    assert_eq!(members.len(), 2);
    for (d, text) in members.iter().zip(["x.foo", "x.bar"]) {
        assert_eq!(d["action"]["route"], "unsupported_in_engine");
        assert_eq!(d["action"]["construct"], "member_or_constructor_unproved");
        assert_eq!(d["resolution"], "request_change_required");
        assert!(d["supported_choices"].as_array().unwrap().is_empty());
        assert_eq!(
            d["anchors"][0],
            anchor(&repo, "cases/layout/source.rs", text)
        );
    }
    let mut counts_only = rejected.clone();
    counts_only["limits"]["diagnostic_count"] = json!(0);
    let summary = client.call("move_item", counts_only);
    assert_eq!(summary["plan"]["decisions"], json!([]));
    assert_eq!(summary["counts"]["omissions"]["decisions"], decisions.len());
    assert_eq!(summary["plan"]["decision_groups"], plan["decision_groups"]);
    for group in summary["plan"]["decision_groups"].as_array().unwrap() {
        let action = &group["actions"][0];
        assert_eq!(action["route"], group["route"]);
        match action["route"].as_str().unwrap() {
            "request_field" => {
                assert_eq!(action["field"], "rewrite_overrides[]");
                assert_eq!(action["purpose"], "resolve_decision");
                assert_eq!(action["choices"], json!(["accept_default", "replace"]));
                assert!(action["target"].is_null());
            }
            "selection_change_required" => assert!(action["fields"].is_array()),
            "unsupported_in_engine" => assert!(action["construct"].is_string()),
            _ => panic!("unexpected route"),
        }
    }
    rejected["rewrite_overrides"][0]["action"] = json!("accept_default");
    let replay = client.call("move_item", rejected);
    assert_eq!(replay["plan"]["applicable"], false);
    assert!(
        !replay["plan"]["decisions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["reason"] == "required_rewrite_retained")
    );
    assert_eq!(
        replay["plan"]["decisions"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|d| d["reason"] == "member_or_constructor_unproved")
            .count(),
        2
    );
    assert_eq!(observe(&repo.0), before);
}
#[test]
fn failed_alias_and_private_visibility_alternatives_link_the_real_target() {
    let repo = fixture("fn selected() {}\nfn taken() {}\nfn caller() { selected(); }\n");
    let mut client = Client::new();
    let args = request(&repo, &["fn selected() {}"]);
    let baseline = client.call("move_item", args.clone());
    assert_eq!(baseline["plan"]["applicable"], true);
    for (kind, text, reason) in [
        (
            "import_insert",
            "use crate::destination::selected as taken;",
            "final_alias_conflict",
        ),
        ("visibility", "", "visibility_scope_unproved"),
    ] {
        let r = baseline["plan"]["rewrites"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["kind"] == kind)
            .unwrap();
        let mut choice = args.clone();
        choice["rewrite_overrides"] =
            json!([{"target":r["target"],"action":"replace","replacement_text":text}]);
        let blocked = client.call("move_item", choice);
        let d = blocked["plan"]["decisions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|d| d["reason"] == reason)
            .unwrap();
        assert_eq!(d["action"]["route"], "request_field");
        assert_eq!(d["action"]["target"], r["target"]);
        groups(&blocked["plan"]);
    }
}
#[test]
fn banners_root_scope_and_cross_group_routes_are_honest_analysis_or_review() {
    let repo = fixture(
        "fn keep() {}\n// --- banner ---\n\nfn alpha_one() { beta_one(); }\nfn beta_one() {}\nfn alpha_two() {}\n",
    );
    repo.write("cases/layout/main.rs", "fn main() {}\n");
    let mut client = Client::new();
    let advice = json!({"repo_path":repo.0,"crate_root":"cases/layout/lib.rs","source_path":"cases/layout/source.rs","paths":["cases/layout"],"limits":{"text_bytes":0}});
    let split = client.call("suggest_split", advice.clone());
    groups(&split);
    let cross = split["decisions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["reason"] == "cross_group_reference_review")
        .unwrap();
    assert_eq!(cross["action"]["field"], "moves");
    assert_eq!(cross["action"]["purpose"], "submit_for_analysis");
    assert!(cross["action"]["target"].is_null());
    let moved = client.call("move_item", request(&repo, &["fn alpha_two() {}"]));
    let banner = moved["plan"]["decisions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["reason"] == "ordinary_trivia_choice")
        .unwrap();
    assert_eq!(banner["action"]["purpose"], "review_default");
    assert_eq!(banner["blocks_applicability"], false);
    assert_eq!(banner["action"]["trivia"], banner["anchors"][0]);
    assert_eq!(
        banner["action"]["target_item"],
        anchor(&repo, "cases/layout/source.rs", "fn alpha_two() {}")
    );
    let mut wrong = advice.clone();
    wrong["crate_root"] = json!("cases/layout/main.rs");
    let wrong = client.call("suggest_split", wrong);
    groups(&wrong);
    let d = wrong["decisions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["reason"] == "module_chain_failure")
        .unwrap();
    assert_eq!(d["action"]["field"], "crate_root");
    assert_eq!(d["action"]["purpose"], "submit_for_analysis");
    let mut scoped = advice;
    scoped["paths"] = json!(["cases/layout/lib.rs", "cases/layout/source.rs"]);
    repo.write("cases/layout/lib.rs", "mod middle;\n");
    repo.write("cases/layout/middle.rs", "mod source;\n");
    repo.write(
        "cases/layout/middle/source.rs",
        "fn first() {}\nfn second() {}\n",
    );
    scoped["source_path"] = json!("cases/layout/middle/source.rs");
    scoped["paths"] = json!(["cases/layout/lib.rs", "cases/layout/middle/source.rs"]);
    let unadmitted = client.call("suggest_split", scoped);
    let chain = unadmitted["decisions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["reason"] == "module_chain_failure")
        .unwrap();
    assert_eq!(chain["action"]["field"], "paths");
}

#[test]
fn overflow_preserves_complete_membership_and_root_and_schemas_publish_all_routes() {
    let schema = serde_json::to_value(rmcp::schemars::schema_for!(
        rust_sitter_mcp::move_plan::MoveEnvelope
    ))
    .unwrap();
    let schema = schema.to_string();
    for field in [
        "decision_groups",
        "request_field",
        "selection_change_required",
        "unsupported_in_engine",
        "review_default",
        "submit_for_analysis",
        "removal_gap_choice",
        "default_disposition",
    ] {
        assert!(schema.contains(field));
    }
    let source = (0..180)
        .map(|i| format!("fn item_{i}(x: u8) {{ x.foo(); x.bar(); }}\n"))
        .collect::<String>();
    let repo = fixture(&source);
    let mut client = Client::new();
    let selected: Vec<_> = source.lines().collect();
    let mut args = request(&repo, &selected);
    args["limits"]["response_bytes"] = json!(65536);
    args["limits"]["diagnostic_count"] = json!(100_000);
    let result = client.call("move_item", args);
    assert_eq!(result["status"], "partial");
    assert!(result["plan"]["decisions"].as_array().unwrap().is_empty());
    assert!(result["root"].is_string() && result["snapshot_id"].is_string());
    let blocking_group = result["plan"]["decision_groups"]
        .as_array()
        .unwrap()
        .iter()
        .find(|g| g["blocks_applicability"] == true)
        .unwrap();
    assert_eq!(blocking_group["count"], 360);
    assert_eq!(
        blocking_group["decision_ids"],
        json!([{"first_id":"d/0","count":360}])
    );
    assert_eq!(
        result["plan"]["decision_groups"].as_array().unwrap().len(),
        2
    );
    assert!(result["counts"]["omissions"]["decision_groups"].is_null());
    assert_eq!(result["counts"]["omissions"]["decisions"], 361);
    let advice = client.call("suggest_split", json!({"repo_path":repo.0,"crate_root":"cases/layout/lib.rs","source_path":"cases/layout/source.rs","paths":["cases/layout"],"limits":{"text_bytes":0,"response_bytes":65536}}));
    assert_eq!(advice["status"], "partial");
    assert!(advice["decisions"].as_array().unwrap().is_empty());
    assert!(advice["root"].is_string() && advice["snapshot_id"].is_string());
    assert!(advice["counts"]["omissions"]["decision_groups"].is_null());
    assert_eq!(advice["decision_groups"][0]["count"], 360);
    assert!(advice["drafts"].as_array().unwrap().is_empty());
    assert!(!advice["draft_summaries"].as_array().unwrap().is_empty());
    for summary in advice["draft_summaries"].as_array().unwrap() {
        let members: BTreeSet<_> = summary["groups"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|g| g["item_ids"].as_array().unwrap())
            .map(|id| id.as_str().unwrap())
            .collect();
        assert_eq!(members.len(), 180);
    }
}
