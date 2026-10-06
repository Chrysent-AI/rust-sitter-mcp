//! Shared assertions and the caller-owned edited-draft execution demonstration.
use super::{fixture_gen::Fixture, move_artifacts};
use serde_json::{Value, json};
use std::{collections::BTreeSet, fs};

pub fn complete(result: &Value) {
    assert_eq!(result["status"], "complete", "{result}");
    assert_eq!(result["advisory"], true);
    assert_eq!(result["integrity"]["semantic"], "not_performed");
    assert!(matches!(
        result["integrity"]["syntax"].as_str(),
        Some("input_checked" | "input_recovered")
    ));
    for field in [
        "patch",
        "edits",
        "created_files",
        "plan",
        "execution_handle",
        "next_cursor",
    ] {
        assert!(result.get(field).is_none(), "advice leaked {field}");
    }
    let inventory = result["inventory"].as_array().unwrap();
    assert_eq!(result["counts"]["inventory_items"], inventory.len());
    let ids: BTreeSet<_> = inventory
        .iter()
        .map(|i| i["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids.len(), inventory.len());
    let signals: BTreeSet<_> = result["signals"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["id"].as_str().unwrap())
        .collect();
    let decisions: BTreeSet<_> = result["decisions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["id"].as_str().unwrap())
        .collect();
    for item in inventory {
        for id in item["signal_ids"].as_array().unwrap() {
            assert!(signals.contains(id.as_str().unwrap()));
        }
    }
    for signal in result["signals"].as_array().unwrap() {
        for id in signal["item_ids"].as_array().unwrap() {
            assert!(ids.contains(id.as_str().unwrap()));
        }
    }
    for draft in result["drafts"].as_array().unwrap() {
        assert_eq!(draft["advisory"], true);
        let groups = draft["groups"].as_array().unwrap();
        assert!((2..=3).contains(&groups.len()));
        assert_eq!(groups[0]["kind"], "retain");
        let mut accounted = BTreeSet::new();
        for group in groups {
            assert!(!group["item_ids"].as_array().unwrap().is_empty());
            assert!(!group["rationale"].as_str().unwrap().is_empty());
            assert_eq!(group["assessment_scope"]["assessed"], "local_only");
            assert_eq!(
                group["assessment_scope"]["not_assessed"],
                json!([
                    "other_consumers",
                    "destination",
                    "batch",
                    "module_chain",
                    "trivia_ownership"
                ])
            );
            assert_eq!(group["expected_to_block"]["lower_bound"], true);
            let count: u64 = group["expected_to_block"]["counts"]
                .as_object()
                .unwrap()
                .values()
                .map(|n| n.as_u64().unwrap())
                .sum();
            assert_eq!(
                count as usize,
                group["expected_to_block"]["decision_ids"]
                    .as_array()
                    .unwrap()
                    .len()
            );
            for id in group["expected_to_block"]["decision_ids"]
                .as_array()
                .unwrap()
            {
                assert!(
                    decisions.contains(id.as_str().unwrap()),
                    "dangling risk decision"
                );
            }
            for id in group["item_ids"].as_array().unwrap() {
                assert!(accounted.insert(id.as_str().unwrap()), "duplicate member");
            }
            for id in group["signal_ids"].as_array().unwrap() {
                assert!(signals.contains(id.as_str().unwrap()));
            }
        }
        assert_eq!(accounted, ids);
        for id in draft["unresolved_decision_ids"].as_array().unwrap() {
            assert!(
                decisions.contains(id.as_str().unwrap()),
                "dangling decision"
            );
        }
        for id in draft["cross_group_signal_ids"].as_array().unwrap() {
            assert!(signals.contains(id.as_str().unwrap()));
        }
    }
}

pub fn edited_batch(repo: &Fixture, advice: &Value) -> Value {
    complete(advice);
    let draft = &advice["drafts"][0];
    assert_eq!(draft["groups"].as_array().unwrap().len(), 3);
    assert!(
        !draft["cross_group_signal_ids"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let inventory = advice["inventory"].as_array().unwrap();
    let source = fs::read_to_string(repo.0.join("src/rich.rs")).unwrap();
    // The caller changes membership AND filenames. No server draft handle is executed.
    // beta_write joins alpha; Marker and both impl units are explicitly retained.
    let chosen = [
        ("alpha_read", "src/edited_a.rs"),
        ("alpha_parse", "src/edited_a.rs"),
        ("beta_write", "src/edited_a.rs"),
        ("beta_flush", "src/edited_b.rs"),
    ];
    let moves: Vec<_> = chosen.into_iter().map(|(name, path)| {
        let item = inventory.iter().find(|i| i["name"] == name).unwrap();
        let range = &item["span"]["range"];
        let start = range["start_byte"].as_u64().unwrap() as usize;
        let end = range["end_byte"].as_u64().unwrap() as usize;
        json!({"item":{"path":"src/rich.rs","range":range,"expected_text":&source[start..end]},"destination":{"kind":"new_sibling","path":path,"parent_path":"src/lib.rs"}})
    }).collect();
    // Cross-check full-byte anchors independently of displayed inventory text.
    assert_eq!(
        moves[0]["item"],
        move_artifacts::anchor(
            repo,
            "src/rich.rs",
            "fn alpha_read() -> u8 { alpha_parse() }"
        )
    );
    json!({"repo_path":repo.0,"crate_root":"src/lib.rs","paths":["src"],"moves":moves,"draft_provenance":{"draft_id":draft["id"],"source_snapshot_id":advice["snapshot_id"]}})
}
