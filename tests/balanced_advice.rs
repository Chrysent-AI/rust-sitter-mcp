#[allow(dead_code)]
#[path = "support/advice_flow.rs"]
mod advice_flow;
#[allow(dead_code)]
#[path = "support/fixture_gen.rs"]
mod fixture_gen;
#[allow(dead_code)]
#[path = "support/move_artifacts.rs"]
mod move_artifacts;
#[path = "support/stdio_client.rs"]
mod stdio_client;
use fixture_gen::{Fixture, observe};
use rust_sitter_mcp::split::SuggestSplitRequest;
use serde_json::{Value, json};
use std::collections::BTreeSet;
use stdio_client::Client;

fn request(repo: &Fixture, flag: Option<bool>) -> Value {
    let mut args = json!({"repo_path":repo.0,"crate_root":"src/lib.rs","source_path":"src/rich.rs","paths":["src"],"limits":{"text_bytes":0,"diagnostic_count":100000}});
    if let Some(flag) = flag {
        args["include_balanced"] = json!(flag);
    }
    args
}

fn linked_evidence(result: &Value) {
    advice_flow::complete(result);
    let inventory: BTreeSet<_> = result["inventory"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["id"].as_str().unwrap())
        .collect();
    let candidates = result["ownership_candidates"].as_array().unwrap();
    let candidate_ids: BTreeSet<_> = candidates
        .iter()
        .map(|candidate| candidate["id"].as_str().unwrap())
        .collect();
    let mut companion_ids = BTreeSet::new();
    for candidate in candidates {
        for id in candidate["core_item_ids"].as_array().unwrap() {
            assert!(inventory.contains(id.as_str().unwrap()));
        }
        for companion in candidate["companions"].as_array().unwrap() {
            assert!(companion_ids.insert(companion["id"].as_str().unwrap()));
            assert!(inventory.contains(companion["item_id"].as_str().unwrap()));
        }
    }
    assert!(!companion_ids.is_empty());
    for observation in result["boundary_observations"]["records"]
        .as_array()
        .unwrap()
    {
        for id in observation["ownership_candidate_ids"].as_array().unwrap() {
            assert!(candidate_ids.contains(id.as_str().unwrap()));
        }
    }
    let summaries = candidates.iter().map(|c| &c["consequence_summary"]).chain(
        result["drafts"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|d| d["groups"].as_array().unwrap())
            .map(|g| &g["consequence_summary"]),
    );
    for summary in summaries {
        assert_eq!(
            summary["destination_and_batch_applicability"],
            "not_assessed"
        );
        for class in summary["classes"]
            .as_object()
            .unwrap()
            .values()
            .chain(std::iter::once(&summary["unmapped"]))
        {
            let mut linked = BTreeSet::new();
            for reference in class["decision_refs"].as_array().unwrap() {
                let collection = reference["collection"].as_str().unwrap();
                for run in reference["id_runs"].as_array().unwrap() {
                    let (prefix, start) =
                        run["first_id"].as_str().unwrap().rsplit_once('/').unwrap();
                    let start: usize = start.parse().unwrap();
                    for i in start..start + run["count"].as_u64().unwrap() as usize {
                        let id = format!("{prefix}/{i}");
                        assert!(linked.insert((collection, id.clone())));
                        assert!(
                            result[collection]
                                .as_array()
                                .unwrap()
                                .iter()
                                .any(|d| d["id"] == id)
                        );
                    }
                }
            }
            assert_eq!(class["count"], linked.len());
        }
    }
}

#[test]
fn real_stdio_balanced_flag_discovery_default_matrix_and_original_order() {
    let repo = Fixture::generate();
    let before = observe(&repo.0);
    let mut client = Client::new();
    let tools = client.rpc("tools/list", json!({}));
    let tools = tools["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 7);
    for tool in tools.iter().filter(|tool| tool["name"] != "suggest_split") {
        assert!(
            tool["inputSchema"]["properties"]
                .get("include_balanced")
                .is_none()
        );
    }
    let suggest = tools
        .iter()
        .find(|tool| tool["name"] == "suggest_split")
        .unwrap();
    let schema = &suggest["inputSchema"];
    assert_eq!(schema["additionalProperties"], false);
    assert!(
        !schema["required"]
            .as_array()
            .unwrap()
            .contains(&json!("include_balanced"))
    );
    assert!(
        schema["properties"]["include_balanced"]
            .to_string()
            .contains("boolean")
    );
    assert!(
        schema["properties"]["include_balanced"]["description"]
            .as_str()
            .unwrap()
            .contains("omitted/false")
    );
    assert!(
        suggest["description"]
            .as_str()
            .unwrap()
            .contains("since v0.5.0 omission/false")
    );
    for flag in [None, Some(false), Some(true)] {
        let decoded: SuggestSplitRequest = serde_json::from_value(request(&repo, flag)).unwrap();
        assert_eq!(decoded.include_balanced, flag);
    }
    let omitted = client.call("suggest_split", request(&repo, None));
    let disabled = client.call("suggest_split", request(&repo, Some(false)));
    assert_eq!(omitted, disabled);
    assert_eq!(omitted["drafts"].as_array().unwrap().len(), 1);
    assert_eq!(omitted["partition_outcome"], "credible_written_candidates");
    linked_evidence(&omitted);
    assert!(
        !omitted["drafts"][0]["rationale"]
            .as_str()
            .unwrap()
            .contains("balanced alternative")
    );
    let enabled = client.call("suggest_split", request(&repo, Some(true)));
    linked_evidence(&enabled);
    let drafts = enabled["drafts"].as_array().unwrap();
    assert_eq!(drafts.len(), 2);
    assert_eq!(enabled["partition_outcome"], "credible_written_candidates");
    for (plain, explicit) in omitted["drafts"][0]["groups"]
        .as_array()
        .unwrap()
        .iter()
        .zip(drafts[0]["groups"].as_array().unwrap())
    {
        assert_eq!(plain["item_ids"], explicit["item_ids"]);
    }
    assert!(drafts[1]["rationale"].as_str().unwrap().contains(
        "low-confidence original-order balanced alternative; no semantic boundary claimed"
    ));
    let groups = drafts[1]["groups"].as_array().unwrap();
    assert!(groups.iter().all(|g| g["confidence"]["level"] == "low"));
    assert!(groups.iter().skip(1).all(|g| {
        g["rationale"]
            .as_str()
            .unwrap()
            .contains("not a natural semantic boundary")
    }));
    let eligible: Vec<_> = enabled["inventory"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|item| {
            item["eligibility"] == "supported_unit"
                || (item.get("enclosing_impl").is_some()
                    && item["reasons"].as_array().unwrap().is_empty())
        })
        .map(|item| item["id"].clone())
        .collect();
    assert!(
        groups[0]["item_ids"]
            .as_array()
            .unwrap()
            .contains(&eligible[0])
    );
    let moved: Vec<_> = groups
        .iter()
        .skip(1)
        .flat_map(|g| g["item_ids"].as_array().unwrap().iter().cloned())
        .collect();
    assert_eq!(moved, eligible[1..]);
    for item in enabled["inventory"].as_array().unwrap() {
        if !eligible.contains(&item["id"]) {
            assert!(
                groups[0]["item_ids"]
                    .as_array()
                    .unwrap()
                    .contains(&item["id"])
            );
        }
    }
    assert_eq!(
        disabled,
        client.call("suggest_split", request(&repo, Some(false)))
    );
    assert_eq!(omitted, client.call("suggest_split", request(&repo, None)));
    let mut wrong = request(&repo, None);
    wrong["include_balanced"] = json!("true");
    let rejected = client.rpc(
        "tools/call",
        json!({"name":"suggest_split","arguments":wrong}),
    );
    assert_eq!(rejected["isError"], true);
    assert_eq!(
        rejected["structuredContent"]["error"]["code"],
        "INVALID_PARAMS"
    );
    assert_eq!(observe(&repo.0), before);
}

#[test]
fn explicit_balancing_deduplicates_and_never_replaces_distinct_negative_outcomes() {
    let repo = Fixture::generate();
    repo.write(
        "src/rich.rs",
        "fn first() { second(); }\nfn second() { first(); }\n",
    );
    let mut client = Client::new();
    let enabled = client.call("suggest_split", request(&repo, Some(true)));
    advice_flow::complete(&enabled);
    assert_eq!(enabled["drafts"].as_array().unwrap().len(), 1);
    assert_eq!(enabled, client.call("suggest_split", request(&repo, None)));
    repo.write(
        "src/rich.rs",
        "fn alpha_one() {}\nfn alpha_two() {}\nfn beta() {}\n",
    );
    let before = observe(&repo.0);
    for flag in [None, Some(false), Some(true)] {
        let negative = client.call("suggest_split", request(&repo, flag));
        advice_flow::complete(&negative);
        assert_eq!(
            negative["partition_outcome"],
            "no_credible_written_partition"
        );
        assert_eq!(negative["drafts"], json!([]));
        assert_eq!(negative["inventory"].as_array().unwrap().len(), 3);
        assert!(!negative["signals"].as_array().unwrap().is_empty());
        let mut capped = request(&repo, flag);
        capped["max_items"] = json!(1);
        let incomplete = client.call("suggest_split", capped);
        assert_eq!(incomplete["status"], "partial");
        assert_eq!(incomplete["partition_outcome"], "incomplete_analysis");
        assert_eq!(incomplete["drafts"], json!([]));
        let mut singleton = request(&repo, flag);
        singleton["source_path"] = json!("src/single.rs");
        let singleton = client.call("suggest_split", singleton);
        advice_flow::complete(&singleton);
        assert_eq!(singleton["partition_outcome"], "insufficient_input");
        assert_eq!(singleton["drafts"], json!([]));
    }
    assert_eq!(observe(&repo.0), before);
    repo.write(
        "src/rich.rs",
        "fn first() { second(); }\nfn second() { first(); }\n",
    );
    repo.write("src/lib.rs", "");
    let before = observe(&repo.0);
    for flag in [None, Some(false), Some(true)] {
        let unsupported = client.call("suggest_split", request(&repo, flag));
        advice_flow::complete(&unsupported);
        assert_eq!(unsupported["partition_outcome"], "unsupported_layout");
        assert_eq!(unsupported["drafts"], json!([]));
        assert!(
            !unsupported["ownership_candidates"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        assert!(
            !unsupported["chain_diagnostics"]
                .as_array()
                .unwrap()
                .is_empty()
        );
    }
    assert_eq!(observe(&repo.0), before);
}

#[test]
fn balanced_options_survive_full_compact_and_retained_historical_groups() {
    let repo = Fixture::generate();
    let before = observe(&repo.0);
    let mut client = Client::new();
    for flag in [None, Some(false), Some(true)] {
        let full = client.call("suggest_split", request(&repo, flag));
        for mode in ["full", "compact"] {
            let mut args = request(&repo, flag);
            args["retain_snapshot"] = json!(true);
            args["response_mode"] = json!(mode);
            let retained = client.call("suggest_split", args);
            assert_eq!(retained["retention"]["state"], "retained");
            assert_eq!(
                retained["retention"]["normalized_request"]["include_balanced"],
                flag.unwrap_or(false)
            );
            assert_eq!(
                retained["retention"]["provenance"]["server_version"],
                "0.5.0"
            );
            let mut detail = json!({"analysis_handle":retained["retention"]["analysis_handle"],"snapshot_id":retained["snapshot_id"],"selector":{"kind":"page","collection":"groups","page_size":100},"limits":{"response_bytes":2097152}});
            let historical = client.call("get_split_detail", detail.clone());
            assert_eq!(historical["historical"], true);
            assert_eq!(historical["live_freshness"], "not_checked");
            assert_eq!(historical["collection_exhausted"], true);
            let drafts = full["drafts"].as_array().unwrap();
            let records = historical["records"].as_array().unwrap();
            assert_eq!(
                records.len(),
                drafts
                    .iter()
                    .map(|d| d["groups"].as_array().unwrap().len())
                    .sum::<usize>()
            );
            for (record, group) in records
                .iter()
                .zip(drafts.iter().flat_map(|d| d["groups"].as_array().unwrap()))
            {
                assert_eq!(record["item_ids"], group["item_ids"]);
                assert_eq!(record["consequence_summary"], group["consequence_summary"]);
            }
            if mode == "compact" {
                assert_eq!(retained["analysis_status"], "complete");
                assert_eq!(retained["manifest_complete"], true);
                let memberships = retained["draft_memberships"].as_array().unwrap();
                assert_eq!(memberships.len(), drafts.len());
                for (membership, draft) in memberships.iter().zip(drafts) {
                    for (summary, group) in membership["groups"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .zip(draft["groups"].as_array().unwrap())
                    {
                        assert_eq!(summary["item_ids"], group["item_ids"]);
                        assert_eq!(summary["consequence_summary"], group["consequence_summary"]);
                    }
                }
            } else {
                assert_eq!(retained["drafts"], full["drafts"]);
            }
            detail["selector"] = json!({"kind":"release"});
            assert_eq!(client.call("get_split_detail", detail)["released"], true);
        }
    }
    assert_eq!(observe(&repo.0), before);
}
