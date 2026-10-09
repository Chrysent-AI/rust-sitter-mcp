#[path = "support/fixture_gen.rs"]
#[allow(dead_code)]
mod fixture_gen;
#[path = "support/stdio_client.rs"]
mod stdio_client;
use fixture_gen::{Fixture, observe};
use rust_sitter_mcp::engine::Engine;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::atomic::AtomicBool,
};

fn request(repo: &Fixture) -> Value {
    json!({"repo_path":repo.0,"crate_root":"src/lib.rs","source_path":"src/worker.rs","paths":["src"],"limits":{"response_bytes":16777216,"text_bytes":0}})
}
fn fixture() -> Fixture {
    let repo = Fixture::generate();
    repo.write("src/lib.rs", "mod worker; mod observer;\n");
    repo.write("src/worker.rs", "pub struct Recorder;\nimpl Recorder { fn record(&self) { self.finish(); } fn finish(&self) {} }\nfn helper() { other(); missing(); }\nfn other() { helper(); }\n#[cfg(test)] mod tests;\n");
    repo.write(
        "src/observer.rs",
        "use crate::worker::*; fn observe() { let _ = Recorder; }\n",
    );
    repo.write("src/worker/tests.rs", "use super::*; #[test] fn check() { let recorder = Recorder; recorder.record(); assert!(recorder.finish()); }\n");
    repo
}
fn summaries(result: &Value) -> Vec<Value> {
    result["ownership_candidates"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["consequence_summary"].clone())
        .chain(
            result["drafts"]
                .as_array()
                .unwrap()
                .iter()
                .flat_map(|d| d["groups"].as_array().unwrap())
                .map(|g| g["consequence_summary"].clone()),
        )
        .collect()
}
fn expanded(member: &Value) -> BTreeSet<(String, String)> {
    let mut ids = BTreeSet::new();
    for link in member["decision_refs"].as_array().unwrap() {
        let collection = link["collection"].as_str().unwrap();
        for run in link["id_runs"].as_array().unwrap() {
            let (prefix, start) = run["first_id"].as_str().unwrap().rsplit_once('/').unwrap();
            let start: usize = start.parse().unwrap();
            for i in start..start + run["count"].as_u64().unwrap() as usize {
                assert!(ids.insert((collection.to_owned(), format!("{prefix}/{i}"))));
            }
        }
    }
    assert_eq!(member["count"], ids.len());
    ids
}
fn advice_links(summaries: &[Value]) -> BTreeSet<(String, String)> {
    summaries
        .iter()
        .flat_map(|summary| {
            summary["classes"]
                .as_object()
                .unwrap()
                .values()
                .chain(std::iter::once(&summary["unmapped"]))
                .flat_map(expanded)
        })
        .filter(|(collection, _)| collection == "advice_decisions")
        .collect()
}
#[test]
fn uncapped_summaries_conserve_links_at_zero_default_and_full_expansion() {
    let repo = fixture();
    let before = observe(&repo.0);
    let engine = Engine::new(repo.0.clone()).unwrap();
    let mut responses = Vec::new();
    for cap in [Some(0), None, Some(100000)] {
        let mut args = request(&repo);
        if let Some(cap) = cap {
            args["limits"]["diagnostic_count"] = json!(cap);
        }
        responses.push(
            serde_json::to_value(engine.suggest_split(
                serde_json::from_value(args).unwrap(),
                &AtomicBool::new(false),
            ))
            .unwrap(),
        );
    }
    let full = &responses[2];
    assert_eq!(full["status"], "complete", "{full}");
    assert!(!full["ownership_candidates"].as_array().unwrap().is_empty());
    assert!(!full["drafts"].as_array().unwrap().is_empty());
    let expected = summaries(full);
    assert!(!expected.is_empty());
    let expected_advice_links = advice_links(&expected);
    for collection in ["boundary_observations", "test_observations"] {
        let records = full[collection]["records"].as_array().unwrap();
        assert!(!records.is_empty(), "fixture must exercise {collection}");
        for record in records {
            let advice: Vec<_> = full["advice_decisions"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|decision| {
                    decision["evidence_refs"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|e| e["collection"] == collection && e["id"] == record["id"])
                })
                .collect();
            assert!(
                !advice.is_empty(),
                "missing advice projection for {collection} record {record}"
            );
            for decision in advice {
                let link = (
                    "advice_decisions".to_owned(),
                    decision["id"].as_str().unwrap().to_owned(),
                );
                assert!(
                    expected_advice_links.contains(&link),
                    "missing qualified summary link for {collection} record {record}: {link:?}"
                );
            }
        }
    }
    let canonical: BTreeSet<_> = ["decisions", "advice_decisions"]
        .into_iter()
        .flat_map(|collection| {
            full[collection]
                .as_array()
                .unwrap()
                .iter()
                .map(move |d| (collection.to_owned(), d["id"].as_str().unwrap().to_owned()))
        })
        .collect();
    let mut multi_class = false;
    for summary in &expected {
        assert_eq!(summary["basis"], "uncapped_written_evidence");
        assert_eq!(summary["count_unit"], "distinct_decision_records_per_class");
        assert_eq!(summary["non_additive"], true);
        assert_eq!(
            summary["destination_and_batch_applicability"],
            "not_assessed"
        );
        assert_eq!(summary["classes"].as_object().unwrap().len(), 6);
        let mut memberships: BTreeMap<_, usize> = BTreeMap::new();
        for (class, member) in summary["classes"].as_object().unwrap() {
            for id in expanded(member) {
                assert!(canonical.contains(&id));
                if id.0 == "advice_decisions" {
                    assert_ne!(class, "consumer_outside_supported_repair");
                }
                *memberships.entry(id).or_default() += 1;
            }
        }
        multi_class |= memberships.values().any(|n| *n > 1);
        for id in expanded(&summary["unmapped"]) {
            assert!(canonical.contains(&id));
        }
    }
    assert!(
        multi_class,
        "a decision must visibly contribute to multiple classes"
    );
    for result in &responses {
        assert_eq!(summaries(result), expected);
        assert_eq!(advice_links(&summaries(result)), expected_advice_links);
        for (draft, canonical_draft) in result["drafts"]
            .as_array()
            .unwrap()
            .iter()
            .zip(full["drafts"].as_array().unwrap())
        {
            for (group, canonical_group) in draft["groups"]
                .as_array()
                .unwrap()
                .iter()
                .zip(canonical_draft["groups"].as_array().unwrap())
            {
                assert_eq!(
                    group["expected_to_block"],
                    canonical_group["expected_to_block"]
                );
                assert_eq!(group["expected_to_block"]["lower_bound"], true);
                assert_eq!(
                    group["assessment_scope"],
                    canonical_group["assessment_scope"]
                );
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
            }
        }
    }
    assert_eq!(responses[0]["decisions"], json!([]));
    assert_eq!(responses[0]["advice_decisions"], json!([]));
    assert_eq!(observe(&repo.0), before);
    // The real stdio contract exposes the same mandatory summaries at zero detail.
    let mut args = request(&repo);
    args["limits"]["diagnostic_count"] = json!(0);
    let wire = stdio_client::Client::new().call("suggest_split", args);
    assert_eq!(summaries(&wire), expected);
    assert_eq!(advice_links(&summaries(&wire)), expected_advice_links);
}
