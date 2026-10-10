#[allow(dead_code)]
#[path = "support/fixture_gen.rs"]
mod fixture_gen;
use fixture_gen::Fixture;
use rust_sitter_mcp::engine::Engine;
use serde_json::{Value, json};
use std::{collections::BTreeSet, sync::atomic::AtomicBool};

fn advice(source: &str, extra: Value) -> Value {
    let repo = Fixture::generate();
    repo.write("cases/presentation/lib.rs", "mod core;\n");
    repo.write("cases/presentation/core.rs", source);
    let mut request = json!({"repo_path":repo.0,"crate_root":"cases/presentation/lib.rs", "source_path":"cases/presentation/core.rs", "paths":["cases/presentation"], "limits":{"text_bytes":0,"diagnostic_count":100000}});
    request
        .as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    serde_json::to_value(Engine::new(repo.0.clone()).unwrap().suggest_split(
        serde_json::from_value(request).unwrap(),
        &AtomicBool::new(false),
    ))
    .unwrap()
}
fn item_id<'a>(response: &'a Value, name: &str) -> &'a Value {
    &response["inventory"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["name"] == name)
        .unwrap()["id"]
}
fn type_candidate<'a>(response: &'a Value, name: &str) -> &'a Value {
    response["ownership_candidates"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| {
            c["core_item_ids"]
                .as_array()
                .unwrap()
                .contains(item_id(response, name))
        })
        .unwrap()
}
fn bundle(candidate: &Value) -> &Value {
    candidate["alternatives"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| {
            matches!(
                a["kind"].as_str(),
                Some("complete_supported_bundle" | "supported_bundle_subset")
            )
        })
        .unwrap()
}
fn assert_nonoverlapping(response: &Value, alternative: &Value) {
    let selected: BTreeSet<_> = alternative["item_ids"]
        .as_array()
        .unwrap()
        .iter()
        .map(|id| id.as_str().unwrap())
        .collect();
    for overlap in response["overlaps"].as_array().unwrap() {
        let ids = overlap["item_ids"].as_array().unwrap();
        assert!(
            !(selected.contains(ids[0].as_str().unwrap())
                && selected.contains(ids[1].as_str().unwrap()))
        );
    }
}

#[test]
fn type_only_groups_withhold_destinations_and_offer_explicit_complete_members() {
    let response = advice(
        "struct State;\nimpl State { fn read(&self) {} fn write(&self) {} }\nfn unrelated() {}\n",
        json!({}),
    );
    assert_eq!(response["status"], "complete");
    assert_eq!(response["schema_version"], 2);
    let candidate = type_candidate(&response, "State");
    assert_eq!(
        candidate["core_item_ids"],
        json!([item_id(&response, "State")])
    );
    let alternative = bundle(candidate);
    assert_eq!(alternative["kind"], "complete_supported_bundle");
    let included = alternative["item_ids"].as_array().unwrap();
    for name in ["State", "read", "write"] {
        assert!(included.contains(item_id(&response, name)));
    }
    assert!(
        !alternative["excluded_item_ids"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_nonoverlapping(&response, alternative);
    assert_eq!(response["drafts"], json!([]));
    let group = response["inspection_partitions"][0]["groups"]
        .as_array()
        .unwrap()
        .iter()
        .find(|g| {
            g["kind"] == "extraction_core"
                && g["item_ids"]
                    .as_array()
                    .unwrap()
                    .contains(item_id(&response, "State"))
        })
        .unwrap();
    assert_eq!(group["selection_completeness"]["state"], "incomplete");
    assert_eq!(
        group["selection_completeness"]["applicability"],
        "not_assessed"
    );
    assert!(
        !group["selection_completeness"]["missing_item_ids"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(group.get("destination").is_none());
    assert_eq!(group["item_ids"], candidate["core_item_ids"]);
    assert_eq!(response["integrity"]["semantic"], "not_performed");
}

#[test]
fn trait_generic_and_excluded_members_stay_visible_in_reviewed_subsets() {
    for source in [
        "trait Access { fn act(&self); }\nstruct State;\nimpl State { fn read(&self) {} }\nimpl Access for State { fn act(&self) {} }\nfn unrelated() {}\n",
        "struct State<T>(T);\nimpl<T> State<T> { fn read(&self) {} }\nfn unrelated() {}\n",
        "struct State;\nimpl State { fn read(&self) {} fn generic<T>(&self, _: T) {} }\nfn unrelated() {}\n",
    ] {
        let response = advice(source, json!({}));
        assert_eq!(response["status"], "complete", "{response}");
        let alternative = bundle(type_candidate(&response, "State"));
        assert_eq!(alternative["kind"], "supported_bundle_subset", "{response}");
        assert!(
            !alternative["excluded_item_ids"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        assert_nonoverlapping(&response, alternative);
        let groups = response["inspection_partitions"][0]["groups"]
            .as_array()
            .unwrap();
        let group = groups
            .iter()
            .find(|g| {
                g["kind"] == "extraction_core"
                    && g["item_ids"]
                        .as_array()
                        .unwrap()
                        .contains(item_id(&response, "State"))
            })
            .unwrap();
        assert_eq!(group["selection_completeness"]["state"], "incomplete");
        assert!(group.get("destination").is_none());
    }
}

#[test]
fn dominance_keeps_a_real_small_core_and_withholds_uninformative_names() {
    let giant = format!("fn giant() {{ {} }}\n", "let _ = 0;\n".repeat(400));
    let response = advice(
        &format!("{giant}fn first() {{ second(); }}\nfn second() {{ first(); }}\n"),
        json!({"include_balanced":true}),
    );
    assert_eq!(response["partition_outcome"], "credible_written_candidates");
    let payoff = &response["extraction_payoff"];
    assert_eq!(payoff["completed"], true);
    assert_eq!(
        payoff["dominant_item_ids"],
        json!([item_id(&response, "giant")])
    );
    assert_eq!(payoff["expected_useful_extraction"]["items"], 2);
    assert!(
        payoff["expected_useful_extraction"]["bytes"]
            .as_u64()
            .unwrap()
            < 100
    );
    assert_eq!(
        payoff["largest_items"][0]["item_id"],
        *item_id(&response, "giant")
    );
    assert_eq!(response["drafts"], json!([]));
    for partition in response["inspection_partitions"].as_array().unwrap() {
        for group in partition["groups"].as_array().unwrap() {
            assert!(group.get("destination").is_none());
        }
    }
    assert_eq!(
        response["inspection_partitions"][0]["groups"][1]["item_ids"],
        json!([item_id(&response, "first"), item_id(&response, "second")])
    );
    let negative = advice(&format!("{giant}fn small() {{}}\n"), json!({}));
    assert_eq!(
        negative["partition_outcome"],
        "no_credible_written_partition"
    );
    assert_eq!(negative["extraction_payoff"]["completed"], true);
    assert_eq!(
        negative["extraction_payoff"]["expected_useful_extraction"]["bytes"],
        0
    );
    assert!(
        !negative["extraction_payoff"]["dominant_item_ids"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(negative["drafts"], json!([]));
}

#[test]
fn giant_members_and_excluded_impls_disclose_dominance_without_double_counting() {
    let response = advice(
        &format!(
            "struct State;\nimpl State {{ fn giant(&self) {{ {} }} fn tiny(&self) {{}} }}\n",
            "let _ = 0;\n".repeat(400)
        ),
        json!({}),
    );
    assert_eq!(
        response["extraction_payoff"]["dominant_item_ids"],
        json!([item_id(&response, "giant")])
    );
    assert_eq!(
        response["extraction_payoff"]["largest_items"][0]["supported_member_partition"],
        true
    );
    let excluded = advice(
        &format!(
            "trait Access {{ fn giant(&self); }}\nstruct State;\nimpl Access for State {{ fn giant(&self) {{ {} }} }}\n",
            "let _ = 0;\n".repeat(400)
        ),
        json!({}),
    );
    let dominant = excluded["extraction_payoff"]["dominant_item_ids"]
        .as_array()
        .unwrap();
    assert_eq!(dominant.len(), 1);
    let unit = excluded["inventory"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["id"] == dominant[0])
        .unwrap();
    assert_eq!(unit["kind"], "impl_item");
    assert_eq!(
        excluded["extraction_payoff"]["largest_items"][0]["supported_member_partition"],
        false
    );
}

#[test]
fn compact_membership_preserves_completeness_and_payoff_without_implicit_selection() {
    let source = "struct State;\nimpl State { fn read(&self) {} }\nfn unrelated() {}\n";
    let full = advice(source, json!({}));
    let compact = advice(
        source,
        json!({"response_mode":"compact","retain_snapshot":true}),
    );
    assert_eq!(compact["schema_version"], 1);
    assert_eq!(compact["manifest_complete"], true);
    assert_eq!(compact["extraction_payoff"], full["extraction_payoff"]);
    assert_eq!(
        compact["inspection_partitions"][0]["groups"],
        full["inspection_partitions"][0]["groups"]
    );
    assert_eq!(
        compact["candidate_summaries"][0]["alternatives"],
        full["ownership_candidates"][0]["alternatives"]
    );
    let incomplete = advice(source, json!({"max_items":1}));
    assert_eq!(incomplete["partition_outcome"], "incomplete_analysis");
    assert_eq!(incomplete["extraction_payoff"]["completed"], false);
    assert_eq!(incomplete["drafts"], json!([]));
}
