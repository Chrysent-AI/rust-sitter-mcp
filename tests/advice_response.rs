#[allow(dead_code)]
#[path = "support/fixture_gen.rs"]
mod fixture_gen;
#[path = "support/stdio_client.rs"]
mod stdio_client;
use fixture_gen::{Fixture, observe};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use stdio_client::Client;

fn fixture(source: &str) -> Fixture {
    let repo = Fixture::generate();
    repo.write("cases/advice/lib.rs", source);
    repo
}
fn request(repo: &Fixture) -> Value {
    // Deliberately omit both optional objects: these are the actual default budgets.
    json!({"repo_path":repo.0,"crate_root":"cases/advice/lib.rs",
        "source_path":"cases/advice/lib.rs","paths":["cases/advice"]})
}
fn ids(group: &Value) -> Vec<String> {
    group["decision_ids"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|run| {
            let first = run["first_id"]
                .as_str()
                .unwrap()
                .strip_prefix("d/")
                .unwrap()
                .parse::<usize>()
                .unwrap();
            let count = run["count"].as_u64().unwrap() as usize;
            assert!(count > 0);
            (first..first + count).map(|index| format!("d/{index}"))
        })
        .collect()
}
fn assert_groups(compact: &Value, full: &Value) {
    assert_eq!(compact["schema_version"], 2);
    assert_eq!(compact["decision_groups"], full["decision_groups"]);
    let decisions: BTreeMap<_, _> = full["decisions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|decision| (decision["id"].as_str().unwrap(), decision))
        .collect();
    let mut seen = BTreeSet::new();
    for group in compact["decision_groups"].as_array().unwrap() {
        let members = ids(group);
        assert_eq!(group["count"], members.len());
        assert_eq!(group["actions"].as_array().unwrap().len(), 1);
        let summary = &group["actions"][0];
        assert_eq!(summary["route"], group["route"]);
        for id in members {
            assert!(seen.insert(id.clone()));
            let decision = decisions[id.as_str()];
            for field in [
                "category",
                "reason",
                "blocks_applicability",
                "unresolved_consequence",
            ] {
                assert_eq!(group[field], decision[field]);
            }
            let mut action = decision["action"].clone();
            if action["route"] == "request_field" {
                for field in ["target", "trivia", "target_item"] {
                    action[field] = Value::Null;
                }
            }
            assert_eq!(*summary, action);
        }
    }
    assert_eq!(seen.len(), decisions.len());
    assert_eq!(compact["counts"]["decisions"], seen.len());
    for decision in compact["decisions"].as_array().unwrap() {
        // Surviving exemplars are the original full records, including all anchor bytes.
        assert_eq!(decision, decisions[decision["id"].as_str().unwrap()]);
    }
    assert_eq!(
        compact["decisions"].as_array().unwrap().len()
            + compact["counts"]["omissions"]["decisions"]
                .as_u64()
                .unwrap_or(0) as usize,
        decisions.len()
    );
}

#[test]
fn default_counts_first_advice_keeps_full_drafts_and_exact_risk_links() {
    let source = format!(
        "fn retained() {{ risky_0(0); }}\n{}",
        (0..140)
            .map(|i| format!(
                "fn risky_{i}(value: u8) {{ {} {} }}\n",
                if i == 0 { "let _ = retained;" } else { "" },
                "value.method();".repeat(12)
            ))
            .collect::<String>()
    );
    let repo = fixture(&source);
    let before = observe(&repo.0);
    let mut client = Client::new();
    let args = request(&repo);
    let compact = client.call("suggest_split", args.clone());
    let mut expanded = args.clone();
    expanded["limits"] = json!({"diagnostic_count":100000,"response_bytes":16777216});
    let full = client.call("suggest_split", expanded);
    assert_groups(&compact, &full);
    assert_eq!(compact["status"], "complete");
    assert_eq!(compact["truncation_reasons"], json!(["diagnostic_count"]));
    assert_eq!(compact["integrity"]["semantic"], "not_performed");
    assert!(compact["root"].is_string() && compact["snapshot_id"].is_string());
    assert!(!compact["drafts"].as_array().unwrap().is_empty());
    assert_eq!(compact["drafts"], full["drafts"]);
    assert_eq!(compact["inventory"], full["inventory"]);
    assert_eq!(compact["signals"], full["signals"]);
    assert_eq!(compact["counts"]["omissions"].as_object().unwrap().len(), 1);
    assert_eq!(full["decisions"].as_array().unwrap().len(), 1680);
    assert_eq!(compact["decisions"].as_array().unwrap().len(), 1);
    let covered: BTreeSet<_> = compact["decision_groups"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(ids)
        .collect();
    for draft in compact["drafts"].as_array().unwrap() {
        let mut member_count = 0;
        let mut risks = 0;
        for group in draft["groups"].as_array().unwrap() {
            member_count += group["item_ids"].as_array().unwrap().len();
            assert_eq!(group["assessment_scope"]["assessed"], "local_only");
            assert_eq!(group["expected_to_block"]["lower_bound"], true);
            let count: u64 = group["expected_to_block"]["counts"]
                .as_object()
                .unwrap()
                .values()
                .map(|v| v.as_u64().unwrap())
                .sum();
            let links = group["expected_to_block"]["decision_ids"]
                .as_array()
                .unwrap();
            assert_eq!(count as usize, links.len());
            risks += count;
            for id in links {
                assert!(covered.contains(id.as_str().unwrap()));
            }
        }
        assert_eq!(member_count, 141);
        assert_eq!(risks, 1680);
    }
    for decision in full["decisions"].as_array().unwrap() {
        for anchor in decision["anchors"].as_array().unwrap() {
            let span = &anchor["span"];
            let start = span["range"]["start_byte"].as_u64().unwrap() as usize;
            let end = span["range"]["end_byte"].as_u64().unwrap() as usize;
            assert_eq!(span["text"], &source[start..end]);
            assert_eq!(span["text_omitted"], false);
        }
    }
    let wire = json!({"content":[{"type":"text","text":compact.to_string()}],
        "structuredContent":compact,"isError":false})
    .to_string()
    .len()
        + 4096;
    assert!(wire <= 2097152);
    // Unrelated explicit limits must not accidentally select the expanded profile.
    let mut unrelated = args;
    unrelated["limits"] = json!({"response_bytes":16777216});
    let unrelated = client.call("suggest_split", unrelated);
    assert_eq!(compact["decisions"], unrelated["decisions"]);
    assert_eq!(compact["decision_groups"], unrelated["decision_groups"]);
    assert_eq!(observe(&repo.0), before);
}

#[test]
fn explicit_advice_caps_expand_original_detail_and_count_every_omission() {
    let repo = fixture(&format!(
        "fn retained() {{}}\nfn risky(value: u8) {{ {} }}\n",
        "value.method();".repeat(100)
    ));
    let mut client = Client::new();
    let args = request(&repo);
    let compact = client.call("suggest_split", args.clone());
    let mut expanded = args.clone();
    expanded["limits"] = json!({"diagnostic_count":100000});
    let full = client.call("suggest_split", expanded);
    assert_groups(&compact, &full);
    for count in [0, 1, 64, 100, 257] {
        let mut limited = args.clone();
        limited["limits"] = json!({"diagnostic_count":count});
        let limited = client.call("suggest_split", limited);
        assert_groups(&limited, &full);
        assert_eq!(limited["status"], "complete");
        // Discovery snapshots include limit-dependent skip evidence, not just source bytes.
        let mut drafts = limited["drafts"].clone();
        for draft in drafts.as_array_mut().unwrap() {
            assert_eq!(draft["source_snapshot_id"], limited["snapshot_id"]);
            draft["source_snapshot_id"] = full["snapshot_id"].clone();
        }
        assert_eq!(drafts, full["drafts"]);
        assert_eq!(
            limited["decisions"].as_array().unwrap(),
            &full["decisions"].as_array().unwrap()[..count.min(100)]
        );
    }
    let mut invalid = args;
    invalid["limits"] = json!({"diagnostic_count":100001});
    let error = client.rpc(
        "tools/call",
        json!({"name":"suggest_split","arguments":invalid}),
    );
    assert_eq!(error["isError"], true);
    assert_eq!(error["structuredContent"]["schema_version"], 2);
    assert_eq!(
        error["structuredContent"]["error"]["code"],
        "INVALID_PARAMS"
    );
    assert_eq!(
        error["structuredContent"]["error"]["field"],
        "limits.diagnostic_count"
    );
}

#[test]
fn counts_only_advice_keeps_chain_routes_and_accounts_hidden_links() {
    let repo = fixture("fn retained() {}\nfn moved() {}\n");
    repo.write("cases/advice/main.rs", "fn main() {}\n");
    let mut args = request(&repo);
    args["crate_root"] = json!("cases/advice/main.rs");
    args["limits"] = json!({"diagnostic_count":100000});
    let mut client = Client::new();
    let full = client.call("suggest_split", args.clone());
    args["limits"]["diagnostic_count"] = json!(0);
    let compact = client.call("suggest_split", args);
    assert_groups(&compact, &full);
    assert_eq!(compact["decisions"], json!([]));
    assert_eq!(compact["status"], "complete");
    assert_eq!(compact["chain_diagnostics"], full["chain_diagnostics"]);
    let links: usize = full["decisions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["chain_diagnostic_ids"].as_array().unwrap().len())
        .sum();
    assert!(links > 0);
    assert_eq!(
        compact["counts"]["omissions"]["chain_diagnostic_references"],
        links
    );
    let group = compact["decision_groups"]
        .as_array()
        .unwrap()
        .iter()
        .find(|g| g["reason"] == "module_chain_failure")
        .unwrap();
    assert_eq!(group["actions"][0]["tool"], "suggest_split");
    assert_eq!(group["actions"][0]["field"], "crate_root");
    assert_eq!(group["actions"][0]["purpose"], "submit_for_analysis");
}

#[test]
fn stdio_advice_exposes_containment_without_changing_descriptor_sizes() {
    let source = "struct Recorder;\nimpl Recorder {\nfn record_start() {}\nfn record_stop() {}\nfn other() {}\n}\n";
    let repo = fixture(source);
    let before = observe(&repo.0);
    let mut client = Client::new();
    // The explicit balanced alternative places retained impls across member groups.
    let mut args = request(&repo);
    args["include_balanced"] = json!(true);
    let result = client.call("suggest_split", args);
    assert_eq!(result["status"], "complete");
    assert_eq!(result["schema_version"], 2);
    assert_eq!(result["integrity"]["semantic"], "not_performed");
    assert_eq!(result["inventory"].as_array().unwrap().len(), 5);
    let inventory: BTreeMap<_, _> = result["inventory"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| (i["id"].as_str().unwrap(), i))
        .collect();
    let overlaps = result["overlaps"].as_array().unwrap();
    assert_eq!(overlaps.len(), 3);
    for overlap in overlaps {
        let owner = inventory[overlap["item_ids"][0].as_str().unwrap()];
        let member = inventory[overlap["item_ids"][1].as_str().unwrap()];
        assert_eq!(member["enclosing_impl_id"], owner["id"]);
        assert_eq!(member["size_interpretation"]["additivity"], "non_additive");
        assert_eq!(owner["size_interpretation"]["additivity"], "non_additive");
    }
    let mut crossing = false;
    for draft in result["drafts"].as_array().unwrap() {
        let mut seen = BTreeSet::new();
        for group in draft["groups"].as_array().unwrap() {
            let mut bytes = 0;
            let mut lines = 0;
            for id in group["item_ids"].as_array().unwrap() {
                assert!(seen.insert(id.as_str().unwrap()));
                let item = inventory[id.as_str().unwrap()];
                bytes += item["bytes"].as_u64().unwrap();
                lines += item["lines"].as_u64().unwrap();
            }
            assert_eq!(group["sizes"]["bytes"], bytes);
            assert_eq!(group["sizes"]["lines"], lines);
            if !group["overlap_ids"].as_array().unwrap().is_empty() {
                assert_eq!(group["size_interpretation"]["additivity"], "non_additive");
            }
        }
        assert_eq!(seen.len(), inventory.len());
        for overlap in overlaps {
            let link = overlap["draft_groups"]
                .as_array()
                .unwrap()
                .iter()
                .find(|link| link["draft_id"] == draft["id"])
                .unwrap();
            crossing |= link["impl_group_index"] != link["member_group_index"];
        }
    }
    assert!(crossing);
    assert_eq!(observe(&repo.0), before);
}

#[test]
fn full_stdio_duplicate_displays_reconstruct_all_records_without_retention() {
    let source = (0..400)
        .map(|i| {
            let family = if i < 200 { "alpha" } else { "beta" };
            let next = i ^ 1;
            format!(
                "fn {family}_{i:03}() {{ {family}_{next:03}(); let _payload = \"{}\"; }}\n",
                "x".repeat(1800)
            )
        })
        .collect::<String>();
    let repo = fixture("mod worker;\n");
    repo.write("cases/advice/worker.rs", &source);
    let before = observe(&repo.0);
    let mut args = request(&repo);
    args["source_path"] = json!("cases/advice/worker.rs");
    // Keep both distinct drafts to exercise duplicate fitting at the default wire cap.
    args["include_balanced"] = json!(true);
    args["limits"] = json!({"text_bytes":0});
    let mut client = Client::new();
    let fitted = client.call("suggest_split", args.clone());
    assert_eq!(fitted["status"], "complete");
    assert_eq!(fitted["draft_eligibility"]["evidence_complete"], true);
    assert_eq!(fitted["coverage"]["scope_exhaustive"], true);
    assert!(fitted.get("retention").is_none());
    assert_eq!(fitted["inventory"].as_array().unwrap().len(), 400);
    assert_eq!(fitted["signals"].as_array().unwrap().len(), 802);
    assert_eq!(
        fitted["ownership_candidates"].as_array().unwrap().len(),
        200
    );
    assert_eq!(fitted["drafts"].as_array().unwrap().len(), 2);
    let mut summaries = fitted["ownership_candidates"].as_array().unwrap().len();
    let mut memberships = 0;
    for draft in fitted["drafts"].as_array().unwrap() {
        let mut seen = BTreeSet::new();
        for group in draft["groups"].as_array().unwrap() {
            summaries += 1;
            assert!(group["consequence_summary"].is_object());
            for id in group["item_ids"].as_array().unwrap() {
                assert!(seen.insert(id.as_str().unwrap()));
                memberships += 1;
            }
        }
        assert_eq!(seen.len(), 400);
    }
    assert_eq!((memberships, summaries), (800, 206));
    let occurrences: usize = fitted["signals"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|s| s["kind"] == "reference_candidate")
        .map(|s| s["evidence"].as_array().unwrap().len())
        .sum();
    assert_eq!(occurrences, 400);

    // A larger display budget is a control, not a substitute for the default-budget call.
    args["limits"]["response_bytes"] = json!(16777216);
    let mut direct = client.call("suggest_split", args);
    assert_eq!(direct["status"], "complete");
    assert!(
        direct["counts"]["omissions"]
            .get("duplicate_declaration_display_spans")
            .is_none()
    );
    let spans: BTreeMap<_, _> = fitted["inventory"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| (i["id"].as_str().unwrap(), i["span"].clone()))
        .collect();
    let mut reconstructed = fitted.clone();
    let mut copies = 0;
    for signal in reconstructed["signals"].as_array_mut().unwrap() {
        if matches!(
            signal["kind"].as_str().unwrap(),
            "item_size" | "name_prefix"
        ) {
            assert_eq!(signal["evidence"], json!([]));
            let evidence: Vec<_> = signal["item_ids"]
                .as_array()
                .unwrap()
                .iter()
                .map(|id| spans[id.as_str().unwrap()].clone())
                .collect();
            copies += evidence.len();
            signal["evidence"] = json!(evidence);
        }
    }
    assert_eq!(copies, 800);
    assert_eq!(
        reconstructed["counts"]["omissions"]
            .as_object_mut()
            .unwrap()
            .remove("duplicate_declaration_display_spans"),
        Some(json!(copies))
    );
    // All remaining differences are the pre-existing neighboring-context display tier
    // and the explicitly different budget. Compare the whole response, not just totals.
    assert_eq!(reconstructed["item_contexts"], json!([]));
    let contexts = direct["item_contexts"].as_array().unwrap().len();
    assert_eq!(contexts, 400);
    direct["item_contexts"] = json!([]);
    direct["counts"]["omissions"]["item_contexts"] = json!(contexts);
    direct["limits"]["response_bytes"] = reconstructed["limits"]["response_bytes"].clone();
    assert_eq!(reconstructed, direct);
    let wire = json!({"content":[{"type":"text","text":fitted.to_string()}],
        "structuredContent":fitted,"isError":false})
    .to_string()
    .len()
        + 4096;
    assert!(wire <= 2097152);
    assert_eq!(observe(&repo.0), before);
}

#[test]
fn exact_file_scope_still_honestly_reports_unadmitted_sibling_destinations() {
    let repo = fixture("fn retained() { moved(); }\nfn moved() { retained(); }\n");
    let mut args = request(&repo);
    args["paths"] = json!(["cases/advice/lib.rs"]);
    let mut client = Client::new();
    let result = client.call("suggest_split", args);
    assert_eq!(result["status"], "complete");
    assert_eq!(result["drafts"], json!([]));
    assert_eq!(result["draft_eligibility"]["state"], "no_draft");
    assert_eq!(
        result["draft_eligibility"]["reasons"],
        json!([
            "destination_scope_rejected",
            "no_credible_concept_name",
            "no_admitted_nonconflicting_name_in_base_or_suffix_2_through_99"
        ])
    );
    assert_eq!(result["error"], Value::Null);
    assert_eq!(result["draft_eligibility"]["hints"], json!([SCOPE_HINT]));
    assert_eq!(result["draft_eligibility"].as_object().unwrap().len(), 5);
    let wire = json!({"content":[{"type":"text","text":result.to_string()}],
        "structuredContent":result,"isError":false})
    .to_string()
    .len()
        + 4096;
    assert!(wire <= 2097152);
    // Do not mislabel a scope restriction as an insufficient response budget.
    assert_eq!(result["truncation_reasons"], json!([]));
    assert_naming_is_not_no_work(&result);
}

fn assert_naming_is_not_no_work(result: &Value) {
    let text = result.to_string().to_lowercase();
    assert!(!text.contains("already modularized"));
    assert!(!text.contains("nothing to do"));
    assert_eq!(result["partition_outcome"], "credible_written_candidates");
    assert!(
        !result["ownership_candidates"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

const SCOPE_HINT: &str = "destination naming search was blocked by paths/globs; admit an existing parent directory (such as the source's parent) in paths and ensure globs include proposed sibling files";
const NAMED_CORE: &str = "fn retained() {}\nfn packet_read() { packet_write(); }\nfn packet_write() { packet_read(); }\n";

fn assert_no_scope_hint(result: &Value) {
    assert!(result["draft_eligibility"].get("hints").is_none());
    assert_eq!(result["draft_eligibility"].as_object().unwrap().len(), 4);
}

#[test]
fn naming_prefix_and_collision_suffix_keep_success_and_disclose_real_collision() {
    let repo = fixture(NAMED_CORE);
    let mut client = Client::new();
    let plain = client.call("suggest_split", request(&repo));
    assert_eq!(
        plain["drafts"][0]["groups"][1]["destination"]["path"],
        "cases/advice/packet.rs"
    );
    assert_eq!(plain["drafts"][0]["groups"][1]["warnings"], json!([]));
    assert_no_scope_hint(&plain);
    assert_naming_is_not_no_work(&plain);
    repo.write("cases/advice/packet.rs", "// occupied\n");
    let collision = client.call("suggest_split", request(&repo));
    assert_eq!(
        collision["drafts"][0]["groups"][1]["destination"]["path"],
        "cases/advice/packet_2.rs"
    );
    let warnings = collision["drafts"][0]["groups"][1]["warnings"].to_string();
    assert!(warnings.contains("destination_name_collision: 1"));
    assert!(!warnings.contains("destination_scope_rejected"));
    assert!(!warnings.contains("no_credible_concept_name"));
    assert_no_scope_hint(&collision);
    assert_naming_is_not_no_work(&collision);
}

#[test]
fn naming_exhaustion_distinguishes_collision_scope_and_mixed_causes() {
    let repo = fixture(NAMED_CORE);
    let mut client = Client::new();
    let mut args = request(&repo);
    args["paths"] = json!(["cases/advice/lib.rs"]);
    let rejected = client.call("suggest_split", args.clone());
    assert_eq!(
        rejected["draft_eligibility"]["reasons"],
        json!([
            "destination_scope_rejected",
            "no_admitted_nonconflicting_name_in_base_or_suffix_2_through_99"
        ])
    );
    assert_eq!(rejected["draft_eligibility"]["hints"], json!([SCOPE_HINT]));
    assert_naming_is_not_no_work(&rejected);
    repo.write("cases/advice/packet.rs", "// occupied\n");
    // Only this candidate is in scope, and it really exists; the rest are excluded.
    args["paths"] = json!(["cases/advice/lib.rs", "cases/advice/packet.rs"]);
    let mixed = client.call("suggest_split", args);
    assert_eq!(
        mixed["draft_eligibility"]["reasons"],
        json!([
            "destination_scope_rejected",
            "destination_name_collision",
            "no_admitted_nonconflicting_name_in_base_or_suffix_2_through_99"
        ])
    );
    assert_no_scope_hint(&mixed);
    assert_naming_is_not_no_work(&mixed);
    for suffix in 2..=99 {
        repo.write(&format!("cases/advice/packet_{suffix}.rs"), "// occupied\n");
    }
    let exhausted = client.call("suggest_split", request(&repo));
    assert_eq!(
        exhausted["draft_eligibility"]["reasons"],
        json!([
            "destination_name_collision",
            "no_admitted_nonconflicting_name_in_base_or_suffix_2_through_99"
        ])
    );
    assert_no_scope_hint(&exhausted);
    assert_naming_is_not_no_work(&exhausted);
}

#[test]
fn glob_scope_exclusion_hint_fits_the_small_wire_budget_without_echoing_filters() {
    let repo = fixture(NAMED_CORE);
    let mut client = Client::new();
    let mut args = request(&repo);
    args["globs"] = json!(["cases/advice/lib.rs"]);
    args["limits"] = json!({"response_bytes":65536,"text_bytes":0});
    let result = client.call("suggest_split", args);
    assert_eq!(result["status"], "complete");
    assert_eq!(result["draft_eligibility"]["hints"], json!([SCOPE_HINT]));
    assert_eq!(result["drafts"], json!([]));
    assert_eq!(result["schema_version"], 2);
    assert_eq!(result["truncation_reasons"], json!([]));
    let wire = json!({"content":[{"type":"text","text":result.to_string()}],
        "structuredContent":result,"isError":false})
    .to_string()
    .len()
        + 4096;
    assert!(wire <= 65536);
}

#[test]
fn ignore_and_mixed_ignore_scope_exhaustion_do_not_offer_the_parent_remedy() {
    let repo = fixture(NAMED_CORE);
    repo.write(".gitignore", "cases/advice/packet*.rs\n");
    let before = observe(&repo.0);
    let mut client = Client::new();
    let ignored = client.call("suggest_split", request(&repo));
    assert_eq!(ignored["status"], "complete");
    assert_eq!(ignored["drafts"], json!([]));
    assert_eq!(
        ignored["draft_eligibility"]["reasons"],
        json!([
            "destination_scope_rejected",
            "no_admitted_nonconflicting_name_in_base_or_suffix_2_through_99"
        ])
    );
    assert_no_scope_hint(&ignored);
    let mut args = request(&repo);
    // The base is admitted by the glob but ignored; all 98 suffixes fail the glob.
    args["globs"] = json!(["cases/advice/lib.rs", "cases/advice/packet.rs"]);
    let mixed = client.call("suggest_split", args);
    assert_eq!(mixed["status"], "complete");
    assert_eq!(mixed["drafts"], json!([]));
    assert_eq!(mixed["draft_eligibility"], ignored["draft_eligibility"]);
    assert_no_scope_hint(&mixed);
    assert_naming_is_not_no_work(&ignored);
    assert_naming_is_not_no_work(&mixed);
    assert_eq!(observe(&repo.0), before);
}

#[test]
fn earlier_group_ignore_or_collision_skips_suppress_a_later_scope_remedy() {
    for ignored in [true, false] {
        let repo = fixture(
            "fn retained() {}\nfn alpha_read() { alpha_write(); }\nfn alpha_write() { alpha_read(); }\nfn beta_read() { beta_write(); }\nfn beta_write() { beta_read(); }\n",
        );
        if ignored {
            repo.write(".gitignore", "cases/advice/alpha.rs\n");
        } else {
            repo.write("cases/advice/alpha.rs", "// occupied\n");
        }
        let mut args = request(&repo);
        args["globs"] = json!([
            "cases/advice/lib.rs",
            "cases/advice/alpha.rs",
            "cases/advice/alpha_2.rs"
        ]);
        let mut client = Client::new();
        let result = client.call("suggest_split", args);
        assert_eq!(result["status"], "complete");
        assert_eq!(result["drafts"], json!([]));
        assert_eq!(
            result["draft_eligibility"]["reasons"],
            json!([
                "destination_scope_rejected",
                "no_admitted_nonconflicting_name_in_base_or_suffix_2_through_99"
            ])
        );
        // The second group alone is all-prefix exclusion; the whole search is not.
        assert_no_scope_hint(&result);
    }
}

#[test]
fn drafted_primary_has_no_scope_hint_when_balanced_names_are_excluded() {
    let repo = fixture(NAMED_CORE);
    let mut args = request(&repo);
    args["globs"] = json!(["cases/advice/lib.rs", "cases/advice/packet.rs"]);
    let mut client = Client::new();
    let primary = client.call("suggest_split", args.clone());
    args["include_balanced"] = json!(true);
    let result = client.call("suggest_split", args);
    assert_eq!(result["status"], "complete");
    assert_eq!(result["draft_eligibility"]["state"], "drafted");
    assert_eq!(result["drafts"], primary["drafts"]);
    assert!(
        result["draft_eligibility"]["reasons"]
            .as_array()
            .unwrap()
            .contains(&json!("destination_scope_rejected"))
    );
    assert_no_scope_hint(&result);
}

#[test]
fn naming_without_supported_concept_labels_file_fallback_as_placeholder() {
    let repo = fixture("fn retained() {}\nfn first() { second(); }\nfn second() { first(); }\n");
    let mut client = Client::new();
    let plain = client.call("suggest_split", request(&repo));
    assert_eq!(
        plain["drafts"][0]["groups"][1]["destination"]["path"],
        "cases/advice/lib_part.rs"
    );
    let warnings = plain["drafts"][0]["groups"][1]["warnings"].to_string();
    assert!(warnings.contains("no_credible_concept_name:"));
    assert!(warnings.contains("placeholder, not a concept name"));
    assert!(warnings.contains("caller-chosen destination"));
    assert!(!warnings.contains("destination_name_collision"));
    assert!(!warnings.contains("destination_scope_rejected"));
    assert_no_scope_hint(&plain);
    assert_naming_is_not_no_work(&plain);
    repo.write("cases/advice/lib_part.rs", "// occupied\n");
    let suffixed = client.call("suggest_split", request(&repo));
    assert_eq!(
        suffixed["drafts"][0]["groups"][1]["destination"]["path"],
        "cases/advice/lib_part_2.rs"
    );
    let warnings = suffixed["drafts"][0]["groups"][1]["warnings"].to_string();
    assert!(warnings.contains("no_credible_concept_name:"));
    assert!(warnings.contains("destination_name_collision: 1"));
    assert_no_scope_hint(&suffixed);
    assert_naming_is_not_no_work(&suffixed);
}
