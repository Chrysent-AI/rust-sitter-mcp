#[allow(dead_code)]
#[path = "support/fixture_gen.rs"]
mod fixture_gen;
#[allow(dead_code)]
#[path = "support/move_artifacts.rs"]
mod move_artifacts;
#[path = "support/stdio_client.rs"]
mod stdio_client;
use fixture_gen::{Fixture, observe};
use move_artifacts::{anchor, apply};
use rust_sitter_mcp::{engine::Engine, move_plan::MoveRequest};
use serde_json::{Value, json};
use std::sync::atomic::AtomicBool;

fn fixture(source: &str) -> Fixture {
    let repo = Fixture::generate();
    repo.write("cases/layout/lib.rs", "mod source;\nmod destination;\n");
    repo.write("cases/layout/source.rs", source);
    repo.write("cases/layout/destination.rs", "fn keep() {}\n");
    repo
}
fn request(repo: &Fixture, selected: &[&str]) -> Value {
    json!({"repo_path":repo.0,"crate_root":"cases/layout/lib.rs","paths":["cases/layout"],"moves":selected.iter().map(|text| json!({"item":anchor(repo,"cases/layout/source.rs",text),"destination":{"kind":"existing","path":"cases/layout/destination.rs"}})).collect::<Vec<_>>()})
}
fn rewrite_batch(blocked: bool) -> (Fixture, Value) {
    let selected: Vec<_> = (0..70)
        .map(|i| {
            let body = if blocked { "x.foo();" } else { "" };
            format!("pub(crate) fn selected_{i}(x: u8) {{ {body} }}")
        })
        .collect();
    let callers: Vec<_> = (0..70)
        .map(|i| format!("fn caller_{i}() {{ crate::source::selected_{i}(0); }}"))
        .collect();
    let repo = fixture(&format!(
        "{}\n{}\n",
        selected.join("\n"),
        callers.join("\n")
    ));
    let texts: Vec<_> = selected.iter().map(String::as_str).collect();
    let args = request(&repo, &texts);
    (repo, args)
}
fn call_error(client: &mut stdio_client::Client, name: &str, args: Value) -> Value {
    let result = client.rpc("tools/call", json!({"name":name,"arguments":args}));
    assert_eq!(result["isError"], true);
    let fallback: Value =
        serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(fallback, result["structuredContent"]);
    result["structuredContent"].clone()
}
fn run(repo: &Fixture, args: Value) -> Value {
    let before = observe(&repo.0);
    let result = Engine::new(repo.0.clone()).unwrap().move_item(
        serde_json::from_value::<MoveRequest>(args).unwrap(),
        &AtomicBool::new(false),
    );
    assert!(result.wire_bytes() <= result.limits.response_bytes);
    assert_eq!(observe(&repo.0), before);
    serde_json::to_value(result).unwrap()
}

#[test]
fn blocked_details_are_counts_first_deduplicated_and_explicitly_expandable() {
    let selected: Vec<_> = (0..30)
        .map(|i| {
            format!(
                "fn selected_{i}(x: u8) {{ {} }}",
                "x.foo(); x.bar(); ".repeat(6)
            )
        })
        .collect();
    let repo = fixture(&selected.join("\n"));
    let texts: Vec<_> = selected.iter().map(String::as_str).collect();
    let args = request(&repo, &texts);
    let result = run(&repo, args.clone());
    assert_eq!(result["status"], "complete");
    assert_eq!(result["plan"]["applicable"], false);
    assert_eq!(result["plan"]["integrity"]["semantic"], "not_performed");
    for artifact in ["edits", "created_files", "patch"] {
        assert!(result["plan"][artifact].is_null());
    }
    let decisions = result["plan"]["decisions"].as_array().unwrap();
    assert_eq!(decisions.len(), 1);
    assert_eq!(result["counts"]["omissions"]["decisions"], 360 - 1);
    assert_eq!(result["counts"]["omissions"]["moves"], 30 - 4);
    assert_eq!(result["plan"]["moves"].as_array().unwrap().len(), 4);
    assert_eq!(result["truncation_reasons"], json!(["diagnostic_count"]));
    for decision in decisions {
        assert!(
            !decision["anchors"][0]["expected_text"]
                .as_str()
                .unwrap()
                .is_empty()
        );
        assert_eq!(decision["evidence"], json!([]));
    }
    let groups = &result["plan"]["decision_groups"];
    assert_eq!(groups.as_array().unwrap().len(), 1);
    assert_eq!(groups[0]["count"], 360);
    assert_eq!(
        groups[0]["decision_ids"],
        json!([{"first_id":"d/0","count":360}])
    );
    assert_eq!(groups[0]["blocks_applicability"], true);
    assert_eq!(groups[0]["reason"], "member_or_constructor_unproved");
    assert_eq!(groups[0]["route"], "unsupported_in_engine");
    assert_eq!(
        groups[0]["actions"][0]["construct"],
        "member_or_constructor_unproved"
    );
    assert!(groups[0]["actions"][0]["instruction"].is_string());

    let mut full = args.clone();
    full["limits"] = json!({"diagnostic_count":512});
    let full = run(&repo, full);
    assert_eq!(full["plan"]["decisions"].as_array().unwrap().len(), 360);
    assert!(full["counts"]["omissions"]["decisions"].is_null());
    assert_eq!(full["plan"]["decision_groups"], *groups);
    for (compact, expanded) in decisions
        .iter()
        .zip(full["plan"]["decisions"].as_array().unwrap())
    {
        assert_eq!(compact["anchors"], expanded["anchors"]);
        assert!(compact.get("next_action").is_none());
        assert!(compact["action"].get("instruction").is_none());
        assert_eq!(groups[0]["actions"][0], expanded["action"]);
        assert_eq!(
            groups[0]["unresolved_consequence"],
            expanded["unresolved_consequence"]
        );
    }
    for count in [4, 64, 100_000] {
        let mut explicit = args.clone();
        explicit["limits"] = json!({"diagnostic_count":count});
        let explicit = run(&repo, explicit);
        assert_eq!(
            explicit["plan"]["decisions"],
            json!(full["plan"]["decisions"].as_array().unwrap()[..count.min(360)])
        );
    }
    let mut zero = args;
    zero["limits"] = json!({"diagnostic_count":0});
    let zero = run(&repo, zero);
    assert_eq!(zero["plan"]["decisions"], json!([]));
    assert_eq!(zero["counts"]["omissions"]["decisions"], 360);
    assert_eq!(zero["plan"]["decision_groups"], *groups);
}

#[test]
fn blocked_rewrite_previews_are_counted_capped_and_explicitly_expandable() {
    let (repo, args) = rewrite_batch(true);
    let result = run(&repo, args.clone());
    assert_eq!(result["status"], "complete");
    assert_eq!(result["plan"]["state"], "blocked");
    assert_eq!(result["plan"]["applicable"], false);
    assert_eq!(result["plan"]["integrity"]["semantic"], "not_performed");
    let rewrites = result["plan"]["rewrites"].as_array().unwrap();
    assert_eq!(rewrites.len(), 4);
    assert_eq!(result["plan"]["decisions"].as_array().unwrap().len(), 1);
    let total = result["counts"]["rewrites"].as_u64().unwrap() as usize;
    assert!(total > 64);
    assert_eq!(result["counts"]["omissions"]["rewrites"], total - 4);
    assert_eq!(result["truncation_reasons"], json!(["diagnostic_count"]));
    for artifact in ["edits", "created_files", "patch"] {
        assert!(result["plan"][artifact].is_null());
    }

    let mut expanded = args.clone();
    expanded["limits"] = json!({"diagnostic_count":512});
    let expanded = run(&repo, expanded);
    let full = expanded["plan"]["rewrites"].as_array().unwrap();
    assert_eq!(full.len(), total);
    assert_eq!(&full[..4], rewrites);
    assert_eq!(expanded["counts"]["rewrites"], total);
    assert!(expanded["counts"]["omissions"]["rewrites"].is_null());
    assert_eq!(expanded["truncation_reasons"], json!([]));
    assert_eq!(
        expanded["plan"]["decision_groups"],
        result["plan"]["decision_groups"]
    );
    for field in ["moves", "origins"] {
        let all = expanded["plan"][field].as_array().unwrap();
        assert_eq!(result["plan"][field], json!(all[..4]));
        assert_eq!(result["counts"]["omissions"][field], all.len() - 4);
    }
    for count in [0, 4, 5, 8, 63, 64, 65, 512, 100_000] {
        let mut limited = args.clone();
        limited["limits"] = json!({"diagnostic_count":count});
        let limited = run(&repo, limited);
        let kept = count.min(total);
        assert_eq!(limited["plan"]["rewrites"], json!(full[..kept]));
        assert_eq!(
            limited["counts"]["omissions"]["rewrites"]
                .as_u64()
                .unwrap_or(0),
            (total - kept) as u64
        );
        assert_eq!(
            limited["plan"]["decision_groups"],
            result["plan"]["decision_groups"]
        );
    }
    let mut implicit = args.clone();
    implicit["limits"] = json!({"text_bytes":0});
    let implicit = run(&repo, implicit);
    assert_eq!(implicit["plan"]["rewrites"].as_array().unwrap().len(), 4);
    assert_eq!(implicit["plan"]["decisions"].as_array().unwrap().len(), 1);
    let mut zero = args;
    zero["limits"] = json!({"diagnostic_count":0});
    let zero = run(&repo, zero);
    assert_eq!(zero["plan"]["rewrites"], json!([]));
    assert_eq!(zero["counts"]["rewrites"], total);
    assert_eq!(zero["counts"]["omissions"]["rewrites"], total);
    assert_eq!(
        zero["plan"]["decision_groups"],
        result["plan"]["decision_groups"]
    );
}

#[test]
fn failed_assembly_rewrite_previews_report_observed_counts_and_omissions() {
    let (repo, mut args) = rewrite_batch(false);
    args["rewrite_overrides"] = json!([{"target":{"kind":"synthesis","path":"cases/layout/destination.rs","slot":"import","items":[args["moves"][0]["item"]],"binding":"unpublished"},"action":"accept_default"}]);
    let result = run(&repo, args.clone());
    assert_eq!(result["status"], "failed");
    assert_eq!(result["error"]["code"], "INVALID_REWRITE_OVERRIDE");
    assert_eq!(result["plan"]["applicable"], false);
    assert_eq!(result["plan"]["rewrites"].as_array().unwrap().len(), 4);
    let total = result["counts"]["rewrites"].as_u64().unwrap() as usize;
    assert!(total > 64);
    assert_eq!(result["counts"]["omissions"]["rewrites"], total - 4);
    assert_eq!(result["truncation_reasons"], json!(["diagnostic_count"]));
    args["limits"] = json!({"diagnostic_count":512});
    let expanded = run(&repo, args);
    assert_eq!(expanded["status"], "failed");
    assert_eq!(expanded["counts"]["rewrites"], total);
    assert_eq!(
        expanded["plan"]["rewrites"].as_array().unwrap().len(),
        total
    );
    assert!(expanded["counts"]["omissions"]["rewrites"].is_null());
    assert_eq!(
        &expanded["plan"]["rewrites"].as_array().unwrap()[..4],
        result["plan"]["rewrites"].as_array().unwrap()
    );
}

#[test]
fn applicable_rewrite_audit_above_default_cap_is_byte_complete_at_zero_limits() {
    let (repo, mut args) = rewrite_batch(false);
    let baseline = run(&repo, args.clone());
    assert_eq!(baseline["plan"]["applicable"], true);
    let total = baseline["counts"]["rewrites"].as_u64().unwrap() as usize;
    assert!(total > 64);
    assert_eq!(
        baseline["plan"]["rewrites"].as_array().unwrap().len(),
        total
    );
    args["limits"] = json!({"diagnostic_count":0,"text_bytes":0});
    let limited = run(&repo, args);
    assert_eq!(limited["plan"]["applicable"], true);
    assert_eq!(limited["plan"]["integrity"]["semantic"], "not_performed");
    for field in ["rewrites", "edits", "created_files", "patch"] {
        assert_eq!(limited["plan"][field], baseline["plan"][field]);
    }
    assert!(limited["counts"]["omissions"]["rewrites"].is_null());
    assert_eq!(limited["truncation_reasons"], json!([]));
    let copy = apply(&repo, &limited);
    let source = std::fs::read_to_string(copy.0.join("cases/layout/source.rs")).unwrap();
    assert!(source.contains("crate::destination::selected_69(0)"));
}

#[test]
fn blocked_trivia_is_selected_or_gap_scoped_unless_explicitly_requested() {
    let text = "fn selected(x: u8) { x.foo(); /* selected internal */ }";
    let repo = fixture(&format!(
        "#[allow(dead_code)]\nfn before() {{}}\n// --- immediate gap ---\n\n#[inline]\n{text}\n#[allow(unused)]\nfn adjacent_unselected() {{ /* unrelated internal */ }}\n// --- distant gap ---\n\n#[allow(dead_code)]\nfn distant() {{}}\n"
    ));
    let args = request(&repo, &[text]);
    let result = run(&repo, args.clone());
    let trivia = result["plan"]["trivia_decisions"].as_array().unwrap();
    let snippets: Vec<_> = trivia
        .iter()
        .map(|t| t["span"]["text"].as_str().unwrap())
        .collect();
    assert!(snippets.contains(&"#[inline]"));
    assert!(snippets.iter().any(|t| t.contains("selected internal")));
    assert!(snippets.iter().any(|t| t.contains("immediate gap")));
    assert!(
        !snippets
            .iter()
            .any(|t| t.contains("allow") || t.contains("unrelated") || t.contains("distant"))
    );
    let mut full = args.clone();
    full["include_unselected_trivia"] = json!(true);
    let full = run(&repo, full);
    let all = full["plan"]["trivia_decisions"].as_array().unwrap();
    assert!(all.iter().any(|t| t["span"]["text"] == "#[allow(unused)]"));
    assert!(
        all.iter()
            .any(|t| t["span"]["text"].as_str().unwrap().contains("distant gap"))
    );
    assert_eq!(
        result["counts"]["omissions"]["trivia_decisions"],
        all.len() - trivia.len()
    );
    assert!(
        result["truncation_reasons"]
            .as_array()
            .unwrap()
            .contains(&json!("trivia_scope"))
    );
    assert_eq!(
        result["plan"]["decision_groups"],
        full["plan"]["decision_groups"]
    );
    assert!(full["counts"]["omissions"]["trivia_decisions"].is_null());

    // Explicit overrides stay visible even when they are protected and refused.
    let mut override_request = args;
    override_request["trivia_overrides"] = json!([{"trivia":anchor(&repo,"cases/layout/source.rs","#[allow(dead_code)]"),"disposition":"keep_in_place"}]);
    let overridden = run(&repo, override_request);
    assert!(
        overridden["plan"]["trivia_decisions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t["span"]["text"] == "#[allow(dead_code)]")
    );
}

#[test]
fn applicable_artifacts_and_full_trivia_survive_zero_display_limits() {
    let text = "fn selected() { /* keep exact é bytes */ }";
    let repo = fixture(&format!(
        "#[allow(dead_code)]\nfn before() {{}}\n#[inline]\n{text}\n#[allow(unused)]\nfn after() {{}}\n"
    ));
    let mut args = request(&repo, &[text]);
    args["moves"][0]["destination"] = json!({"kind":"new_sibling","path":"cases/layout/new_file.rs","parent_path":"cases/layout/lib.rs"});
    let baseline = run(&repo, args.clone());
    assert_eq!(baseline["plan"]["applicable"], true);
    args["limits"] = json!({"diagnostic_count":0,"text_bytes":0});
    let limited = run(&repo, args);
    assert_eq!(limited["plan"]["applicable"], true);
    for artifact in ["edits", "created_files", "patch", "rewrites"] {
        assert_eq!(limited["plan"][artifact], baseline["plan"][artifact]);
    }
    assert_eq!(
        limited["plan"]["trivia_decisions"]
            .as_array()
            .unwrap()
            .len(),
        baseline["plan"]["trivia_decisions"]
            .as_array()
            .unwrap()
            .len()
    );
    assert!(limited["counts"]["omissions"]["trivia_decisions"].is_null());
    let copy = apply(&repo, &limited);
    let content = std::fs::read_to_string(copy.0.join("cases/layout/new_file.rs")).unwrap();
    assert!(content.contains(text) && content.contains("#[inline]"));
}

#[test]
fn advice_and_execution_share_the_exact_attribute_allowlist() {
    for (attribute, blocked) in [
        ("#[derive(Clone)]", true),
        ("#[derive(custom::Derive)]", true),
        ("#[repr(C)]", false),
        ("#[allow(cfg)]", false),
        ("#[inline]", false),
        ("#[inline(always)]", false),
        ("#[inline_custom]", true),
        ("#[cfg(any())]", true),
    ] {
        let selected = "struct Selected;";
        let repo = fixture(&format!("{attribute}\n{selected}\nfn retained() {{}}\n"));
        let before = observe(&repo.0);
        let engine = Engine::new(repo.0.clone()).unwrap();
        let advice = engine.suggest_split(
            serde_json::from_value(json!({
                "repo_path":repo.0,"crate_root":"cases/layout/lib.rs",
                "source_path":"cases/layout/source.rs","paths":["cases/layout"],
                "limits":{"text_bytes":0}
            }))
            .unwrap(),
            &AtomicBool::new(false),
        );
        let advice = serde_json::to_value(advice).unwrap();
        let advice_risk = advice["decisions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["reason"] == "conditional_or_inherited_context");
        let moved = run(&repo, request(&repo, &[selected]));
        let execution_risk = moved["plan"]["decisions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["reason"] == "conditional_or_inherited_context");
        assert_eq!(advice_risk, blocked, "advice: {attribute}");
        assert_eq!(execution_risk, blocked, "execution: {attribute}");
        assert_eq!(moved["plan"]["applicable"], !blocked, "{attribute}");
        assert_eq!(observe(&repo.0), before);
    }
}

#[test]
fn stringified_object_and_null_errors_offer_an_omission_hint_not_coercion() {
    let repo = fixture("fn selected() {}\n");
    let mut client = stdio_client::Client::new();
    let valid = client.call("move_item", request(&repo, &["fn selected() {}"]));
    assert_eq!(valid["plan"]["applicable"], true);
    for (field, value) in [
        ("limits", json!("{\"text_bytes\":0}")),
        ("limits", json!("null")),
        ("context", json!("{}")),
        ("draft_provenance", json!("null")),
    ] {
        let mut args = request(&repo, &["fn selected() {}"]);
        args[field] = value;
        let result = call_error(&mut client, "move_item", args);
        assert_eq!(result["error"]["code"], "INVALID_PARAMS");
        let message = result["error"]["message"].as_str().unwrap();
        assert!(
            message.contains("a client may have stringified an object parameter"),
            "{message}"
        );
        assert!(message.contains("omit optional object parameters instead of passing null"));
        assert_eq!(result["plan"]["integrity"]["semantic"], "not_performed");
    }
    let shared = call_error(
        &mut client,
        "search",
        json!({"repo_path":repo.0,"pattern":"$x","limits":"{}"}),
    );
    assert!(
        shared["error"]["message"]
            .as_str()
            .unwrap()
            .contains("stringified")
    );
    let mut args = request(&repo, &["fn selected() {}"]);
    args["moves"][0]["destination"] = json!("{\"kind\":\"existing\"}");
    let nested = call_error(&mut client, "move_item", args);
    assert!(
        nested["error"]["message"]
            .as_str()
            .unwrap()
            .contains("stringified")
    );
    let wrong_scalar = call_error(
        &mut client,
        "move_item",
        json!({"repo_path":repo.0,"crate_root":"cases/layout/lib.rs","moves":[],"max_moves":"500"}),
    );
    assert!(
        !wrong_scalar["error"]["message"]
            .as_str()
            .unwrap()
            .contains("stringified")
    );
    let unknown = call_error(
        &mut client,
        "move_item",
        json!({"repo_path":repo.0,"crate_root":"cases/layout/lib.rs","moves":[],"unknown":"null"}),
    );
    assert!(
        !unknown["error"]["message"]
            .as_str()
            .unwrap()
            .contains("stringified")
    );
}
