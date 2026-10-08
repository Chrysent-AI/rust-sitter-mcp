#[path = "support/fixture_gen.rs"]
mod fixture_gen;
#[path = "support/move_artifacts.rs"]
mod move_artifacts;
use fixture_gen::{Fixture, observe};
use move_artifacts::{anchor, apply};
use rust_sitter_mcp::{engine::Engine, move_plan::MoveRequest};
use serde_json::{Value, json};
use std::{fs, sync::atomic::AtomicBool};

fn run(repo: &Fixture, request: Value) -> Value {
    let before = observe(&repo.0);
    let result = Engine::new(repo.0.clone()).unwrap().move_item(
        serde_json::from_value::<MoveRequest>(request).unwrap(),
        &AtomicBool::new(false),
    );
    assert_eq!(observe(&repo.0), before, "planning must be read-only");
    serde_json::to_value(result).unwrap()
}
fn gaps(result: &Value) -> Vec<&Value> {
    result["plan"]["decisions"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|d| d["category"] == "removal_gap")
        .collect()
}
fn source(repo: &Fixture) -> String {
    fs::read_to_string(repo.0.join("cases/layout/source.rs")).unwrap()
}

#[test]
fn nineteen_method_batch_collapses_only_created_gaps_with_audited_keep_replay() {
    for eol in ["\n", "\r\n", "\n\r\n"] {
        let repo = Fixture::generate();
        repo.write("cases/layout/lib.rs", "mod source; pub struct Record;\n");
        let methods: Vec<_> = (0..19).map(|i| format!("fn selected_{i}() {{}}")).collect();
        // A retained middle method splits the removals into two independent gaps.
        let first = methods[..9].join(&format!("{eol}{eol}    "));
        let last = methods[9..].join(&format!("{eol}{eol}    "));
        let original = format!(
            "use crate::Record;{eol}impl Record {{{eol}    fn before() {{}}{eol}{eol}    {first}{eol}{eol}    fn middle() {{}}{eol}{eol}    {last}{eol}{eol}    fn after() {{}}{eol}{eol}{eol}    fn unrelated() {{}}{eol}}}{eol}"
        );
        repo.write("cases/layout/source.rs", &original);
        let request = json!({"repo_path":repo.0,"crate_root":"cases/layout/lib.rs","paths":["cases/layout"],"moves": methods.iter().map(|method| json!({
            "item":anchor(&repo,"cases/layout/source.rs",method),
            "enclosing_impl":anchor(&repo,"cases/layout/source.rs","impl Record "),
            "destination":{"kind":"new_sibling","path":"cases/layout/target.rs","parent_path":"cases/layout/lib.rs"}
        })).collect::<Vec<_>>()});
        let result = run(&repo, request.clone());
        assert_eq!(run(&repo, request.clone())["plan"], result["plan"]);
        let choices = gaps(&result);
        assert_eq!(choices.len(), 2);
        for choice in &choices {
            assert_eq!(choice["blocks_applicability"], false);
            assert_eq!(choice["removal_gap"]["default_disposition"], "collapse");
            assert_eq!(choice["removal_gap"]["selected_disposition"], "collapse");
            let anchor = &choice["action"]["target"]["anchor"];
            let start = anchor["range"]["start_byte"].as_u64().unwrap() as usize;
            let end = anchor["range"]["end_byte"].as_u64().unwrap() as usize;
            assert_eq!(anchor["expected_text"], original[start..end]);
            let audit = result["plan"]["rewrites"]
                .as_array()
                .unwrap()
                .iter()
                .find(|r| r["kind"] == "removal_gap" && r["decision_ids"] == json!([choice["id"]]))
                .unwrap();
            assert_eq!(audit["origin"], "synthesized");
            assert_eq!(audit["default_action"], "accept_default");
            assert_eq!(audit["selected_action"], "accept_default");
            assert_eq!(audit["before_text"], anchor["expected_text"]);
            assert_eq!(audit["after_text"], choice["removal_gap"]["after_text"]);
        }
        let output = source(&apply(&repo, &result));
        // Preserve the first two actual terminators, including mixed line endings.
        let collapsed = if eol == "\n\r\n" {
            eol.to_owned()
        } else {
            eol.repeat(2)
        };
        assert!(output.contains(&format!("fn before() {{}}{collapsed}    fn middle()")));
        assert!(output.contains(&format!("fn middle() {{}}{collapsed}    fn after()")));
        assert!(output.contains(&format!("fn after() {{}}{eol}{eol}{eol}    fn unrelated()")));
        let mut kept = request.clone();
        kept["rewrite_overrides"] = json!(
            choices
                .iter()
                .map(|d| json!({"target":d["action"]["target"],"action":"retain"}))
                .collect::<Vec<_>>()
        );
        let kept = run(&repo, kept);
        assert!(
            gaps(&kept)
                .iter()
                .all(|d| d["removal_gap"]["selected_disposition"] == "keep_in_place")
        );
        let mut expected = original;
        for choice in &choices {
            expected = expected.replace(
                choice["action"]["target"]["anchor"]["expected_text"]
                    .as_str()
                    .unwrap(),
                choice["removal_gap"]["before_text"].as_str().unwrap(),
            );
        }
        assert_eq!(source(&apply(&repo, &kept)), expected);
        // Explicit replacement remains available and is honestly caller-authored.
        let mut explicit = request;
        explicit["rewrite_overrides"] = json!(choices.iter().map(|d| json!({"target":d["action"]["target"],"action":"replace","replacement_text":d["removal_gap"]["after_text"]})).collect::<Vec<_>>());
        let explicit = run(&repo, explicit);
        assert_eq!(source(&apply(&repo, &explicit)), output);
        assert!(
            explicit["plan"]["rewrites"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|r| r["kind"] == "removal_gap")
                .all(|r| r["origin"] == "caller_override")
        );
    }
}

#[test]
fn boundary_batches_keep_oversized_gaps_and_offer_explicit_collapse() {
    for eol in ["\n", "\r\n"] {
        for original in [
            format!("fn first() {{}}{eol}{eol}fn second() {{}}{eol}{eol}{eol}fn keep() {{}}"),
            format!(
                "{eol}fn first() {{}}{eol}{eol}fn second() {{}}{eol}{eol}{eol}fn keep() {{}}{eol}"
            ),
            format!(
                "fn keep() {{}}{eol}{eol}fn first() {{}}{eol}{eol}fn second() {{}}{eol}{eol}{eol}"
            ),
        ] {
            let repo = Fixture::generate();
            repo.write("cases/layout/lib.rs", "mod source;\n");
            repo.write("cases/layout/source.rs", &original);
            let request = json!({"repo_path":repo.0,"crate_root":"cases/layout/lib.rs","paths":["cases/layout"],"moves": (["fn first() {}","fn second() {}"]).iter().map(|item| json!({
                "item":anchor(&repo,"cases/layout/source.rs",item),
                "destination":{"kind":"new_sibling","path":"cases/layout/target.rs","parent_path":"cases/layout/lib.rs"}
            })).collect::<Vec<_>>()});
            let result = run(&repo, request.clone());
            let choices = gaps(&result);
            assert_eq!(choices.len(), 1);
            let choice = choices[0];
            let boundary = &choice["action"]["target"]["anchor"]["range"];
            assert!(boundary["start_byte"] == 0 || boundary["end_byte"] == original.len());
            assert_eq!(
                choice["removal_gap"]["default_disposition"],
                "keep_in_place"
            );
            assert_eq!(
                choice["removal_gap"]["selected_disposition"],
                "keep_in_place"
            );
            assert!(
                !result["plan"]["rewrites"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|r| r["kind"] == "removal_gap")
            );
            let expected = original
                .replace("fn first() {}", "")
                .replace("fn second() {}", "");
            assert_eq!(source(&apply(&repo, &result)), expected);
            for action in ["accept_default", "retain", "replace"] {
                let mut replay = request.clone();
                let mut override_choice =
                    json!({"target":choice["action"]["target"],"action":action});
                if action == "replace" {
                    override_choice["replacement_text"] =
                        choice["removal_gap"]["after_text"].clone();
                }
                replay["rewrite_overrides"] = json!([override_choice]);
                let replayed = run(&repo, replay);
                let output = source(&apply(&repo, &replayed));
                if action == "replace" {
                    let anchor = &choice["action"]["target"]["anchor"];
                    assert_eq!(
                        output,
                        original.replace(
                            anchor["expected_text"].as_str().unwrap(),
                            choice["removal_gap"]["after_text"].as_str().unwrap()
                        )
                    );
                    let audit = replayed["plan"]["rewrites"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .find(|r| r["kind"] == "removal_gap")
                        .unwrap();
                    assert_eq!(audit["origin"], "caller_override");
                    assert_eq!(
                        audit["evidence"],
                        json!([
                            "exact selected removals and adjacent whitespace; explicit caller-selected collapse"
                        ])
                    );
                    assert_eq!(
                        audit["rationale"],
                        "explicitly collapse only whitespace left by these engine-owned removals; no retained syntax is changed"
                    );
                } else {
                    assert_eq!(output, expected);
                }
            }
        }
    }
}

#[test]
fn single_item_serialized_plans_match_legacy_default_and_replay_bytes() {
    let repo = Fixture::generate();
    repo.write("cases/layout/lib.rs", "mod source;\n");
    repo.write(
        "cases/layout/source.rs",
        "fn before() {}\n\nfn selected() {}\n\nfn after() {}\n",
    );
    let request = json!({"repo_path":repo.0,"crate_root":"cases/layout/lib.rs","paths":["cases/layout"],"moves":[{
        "item":anchor(&repo,"cases/layout/source.rs","fn selected() {}"),
        "destination":{"kind":"new_sibling","path":"cases/layout/target.rs","parent_path":"cases/layout/lib.rs"}
    }]});
    let result = run(&repo, request.clone());
    apply(&repo, &result);
    let choice = gaps(&result)[0];
    let mut plans = vec![result["plan"].clone()];
    for action in ["retain", "accept_default", "replace"] {
        let mut replay = request.clone();
        let mut override_choice = json!({"target":choice["action"]["target"],"action":action});
        if action == "replace" {
            override_choice["replacement_text"] = choice["removal_gap"]["after_text"].clone();
        }
        replay["rewrite_overrides"] = json!([override_choice]);
        let replayed = run(&repo, replay);
        apply(&repo, &replayed);
        plans.push(replayed["plan"].clone());
    }
    // Captured from the parent of the batch-collapse change (85f14c7), not rebuilt
    // from the current response: compare complete serialized plans, including audits.
    assert_eq!(
        serde_json::to_string(&plans).unwrap(),
        include_str!("fixtures/removal-gap-plans/single-item.json")
    );
}

#[test]
fn gap_endpoint_visibility_repair_remains_separate_and_applicable() {
    let repo = Fixture::generate();
    repo.write("cases/layout/lib.rs", "mod source;\n");
    repo.write(
        "cases/layout/source.rs",
        "fn keep() {}\n\nfn first() { helper(); }\n\nfn second() {}\n\nfn helper() {}\n",
    );
    let moves: Vec<_> = ["fn first() { helper(); }", "fn second() {}"].iter().map(|item| json!({"item":anchor(&repo,"cases/layout/source.rs",item),"destination":{"kind":"new_sibling","path":"cases/layout/target.rs","parent_path":"cases/layout/lib.rs"}})).collect();
    let result = run(
        &repo,
        json!({"repo_path":repo.0,"crate_root":"cases/layout/lib.rs","paths":["cases/layout"],"moves":moves}),
    );
    let choices = gaps(&result);
    assert_eq!(choices.len(), 1);
    let output = source(&apply(&repo, &result));
    assert_eq!(output, "fn keep() {}\n\npub(crate) fn helper() {}\n");
    let end = choices[0]["action"]["target"]["anchor"]["range"]["end_byte"]
        .as_u64()
        .unwrap();
    assert!(
        result["plan"]["edits"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["path"] == "cases/layout/source.rs"
                && e["range"]["start_byte"] == end
                && e["range"]["end_byte"] == end
                && e["replacement_text"] == "pub(crate) ")
    );
}

#[test]
fn batch_single_double_and_bof_double_newlines_remain_byte_identical() {
    for (left, right) in [("", "\n"), ("", "\n\n"), ("\n", "\n"), ("\r\n", "\r\n")] {
        let repo = Fixture::generate();
        repo.write("cases/layout/lib.rs", "mod source;\n");
        let original = format!(
            "{left}fn first() {{}}{right}fn keep() {{}}\nfn second() {{}}\nfn last() {{}}\n"
        );
        repo.write("cases/layout/source.rs", &original);
        let request = json!({"repo_path":repo.0,"crate_root":"cases/layout/lib.rs","paths":["cases/layout"],"moves":(["fn first() {}","fn second() {}"]).iter().map(|item| json!({"item":anchor(&repo,"cases/layout/source.rs",item),"destination":{"kind":"new_sibling","path":"cases/layout/target.rs","parent_path":"cases/layout/lib.rs"}})).collect::<Vec<_>>()});
        let result = run(&repo, request);
        assert!(
            gaps(&result)
                .iter()
                .all(|d| d["removal_gap"]["default_disposition"] == "keep_in_place")
        );
        assert!(
            !result["plan"]["rewrites"]
                .as_array()
                .unwrap()
                .iter()
                .any(|r| r["kind"] == "removal_gap")
        );
        assert_eq!(
            source(&apply(&repo, &result)),
            original
                .replace("fn first() {}", "")
                .replace("fn second() {}", "")
        );
    }
}
