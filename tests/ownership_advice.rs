#[allow(dead_code)]
#[path = "support/fixture_gen.rs"]
mod fixture_gen;
use fixture_gen::{Fixture, observe};
use rust_sitter_mcp::engine::Engine;
use serde_json::{Value, json};
use std::{collections::BTreeSet, sync::atomic::AtomicBool};

fn fixture(source: &str) -> Fixture {
    let repo = Fixture::generate();
    repo.write(
        "cases/ownership/lib.rs",
        "mod core; mod left; mod right; mod outside;\n",
    );
    repo.write("cases/ownership/core.rs", source);
    repo.write("cases/ownership/left.rs", "");
    repo.write("cases/ownership/right.rs", "");
    repo.write("cases/ownership/outside.rs", "");
    repo
}
fn advice(repo: &Fixture, extra: Value) -> Value {
    let mut request = json!({"repo_path":repo.0,"crate_root":"cases/ownership/lib.rs", "source_path":"cases/ownership/core.rs", "paths":["cases/ownership"], "limits":{"text_bytes":8192,"diagnostic_count":100000}});
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
fn id<'a>(response: &'a Value, name: &str) -> &'a Value {
    &response["inventory"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["name"] == name)
        .unwrap()["id"]
}
fn core_names(response: &Value, candidate: &Value) -> BTreeSet<String> {
    candidate["core_item_ids"]
        .as_array()
        .unwrap()
        .iter()
        .map(|id| {
            response["inventory"]
                .as_array()
                .unwrap()
                .iter()
                .find(|i| i["id"] == *id)
                .unwrap()["name"]
                .as_str()
                .unwrap()
                .to_owned()
        })
        .collect()
}
fn companion<'a>(response: &Value, candidate: &'a Value, name: &str) -> &'a Value {
    candidate["companions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["item_id"] == *id(response, name))
        .unwrap()
}
fn complete(response: &Value) {
    assert_eq!(response["status"], "complete", "{response}");
    assert_eq!(response["integrity"]["semantic"], "not_performed");
    assert_eq!(
        response["boundary_observations"]["coverage"]["completed"],
        true
    );
    let inventory: BTreeSet<_> = response["inventory"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["id"].as_str().unwrap())
        .collect();
    for draft in response["drafts"].as_array().unwrap() {
        let ids: Vec<_> = draft["groups"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|g| g["item_ids"].as_array().unwrap())
            .map(|i| i.as_str().unwrap())
            .collect();
        assert_eq!(ids.len(), inventory.len());
        assert_eq!(ids.into_iter().collect::<BTreeSet<_>>(), inventory);
    }
}

#[test]
fn collect_families_have_distinct_consumers_without_prefix_mergers() {
    let repo = fixture(
        "fn collect_files() { collect_paths(); }\nfn collect_paths() { let _ = collect_files; shared(); }\nfn collect_events() { collect_records(); }\nfn collect_records() { let _ = collect_events; shared(); }\nfn shared() {}\nfn orchestrate() { collect_files(); collect_events(); }\n",
    );
    repo.write(
        "cases/ownership/left.rs",
        "use crate::core::collect_files as gather; fn run() { gather(); }\n",
    );
    repo.write(
        "cases/ownership/right.rs",
        "fn run() { crate::core::collect_events(); }\n",
    );
    let before = observe(&repo.0);
    let response = advice(&repo, json!({}));
    complete(&response);
    let candidates = response["ownership_candidates"].as_array().unwrap();
    assert_eq!(candidates.len(), 2);
    assert_eq!(
        core_names(&response, &candidates[0]),
        BTreeSet::from(["collect_files".into(), "collect_paths".into()])
    );
    assert_eq!(
        core_names(&response, &candidates[1]),
        BTreeSet::from(["collect_events".into(), "collect_records".into()])
    );
    for candidate in candidates {
        assert_eq!(
            companion(&response, candidate, "shared")["classification"],
            "observed_shared"
        );
        assert!(
            !candidate["core_item_ids"]
                .as_array()
                .unwrap()
                .contains(id(&response, "orchestrate"))
        );
        assert_eq!(candidate["ranking"]["distinct_internal_relationships"], 2);
    }
    let observations = response["boundary_observations"]["records"]
        .as_array()
        .unwrap();
    for name in ["collect_files", "collect_events"] {
        assert!(
            observations.iter().any(|r| r["direction"] == "incoming"
                && r["certainty"] == "written_route"
                && r["item_ids"]
                    .as_array()
                    .unwrap()
                    .contains(id(&response, name))),
            "{observations:?}"
        );
    }
    assert_eq!(response, advice(&repo, json!({})));
    assert_eq!(observe(&repo.0), before);
    let original = std::fs::read_to_string(repo.0.join("cases/ownership/core.rs")).unwrap();
    repo.write(
        "cases/ownership/core.rs",
        &original.replace("collect_", "inspect_"),
    );
    let renamed = advice(&repo, json!({}));
    assert_eq!(renamed["ownership_candidates"].as_array().unwrap().len(), 2);
    assert_eq!(
        renamed["ownership_candidates"][0]["ranking"]["structural_support"],
        candidates[0]["ranking"]["structural_support"]
    );
}

#[test]
fn state_impl_payload_validators_and_constants_are_inspection_not_closure() {
    let repo = fixture(
        "struct State;\nstruct Payload;\nconst LIMIT: u8 = 4;\nimpl State { fn accept(&self, p: Payload) { validate(p); let _ = LIMIT; } fn reject(&self, p: Payload) { validate(p); } }\nfn validate(p: Payload) { let _ = p; let _ = LIMIT; }\nfn unrelated() { let _ = LIMIT; }\n",
    );
    let response = advice(&repo, json!({}));
    complete(&response);
    let candidate = response["ownership_candidates"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["core_item_ids"] == json!([id(&response, "State")]))
        .unwrap();
    assert_eq!(
        companion(&response, candidate, "Payload")["classification"],
        "observed_exclusive"
    );
    assert_eq!(
        companion(&response, candidate, "validate")["classification"],
        "observed_exclusive"
    );
    assert_eq!(
        companion(&response, candidate, "LIMIT")["classification"],
        "observed_shared"
    );
    assert_eq!(candidate["core_item_ids"], json!([id(&response, "State")]));
    assert!(
        candidate["alternatives"]
            .as_array()
            .unwrap()
            .iter()
            .any(|a| a["kind"] == "whole_impl")
    );
    assert!(
        candidate["alternatives"]
            .as_array()
            .unwrap()
            .iter()
            .any(|a| a["kind"] == "members" && a["item_ids"].as_array().unwrap().len() == 2)
    );
}

#[test]
fn infrastructural_verb_does_not_join_unrelated_members_or_owner_hubs() {
    let repo = fixture(
        "struct Manager;\nstruct Request;\nimpl Manager { fn persist_begin(&self, p: Request) { self.persist_finish(p); } fn persist_finish(&self, p: Request) { let _ = p; } fn persist_clock(&self) {} fn persist_flush(&self) {} }\n",
    );
    let response = advice(&repo, json!({}));
    complete(&response);
    let top = &response["ownership_candidates"][0];
    assert_eq!(
        core_names(&response, top),
        BTreeSet::from(["persist_begin".into(), "persist_finish".into()])
    );
    assert!(
        !top["core_item_ids"]
            .as_array()
            .unwrap()
            .contains(id(&response, "Manager"))
    );
    assert_eq!(
        companion(&response, top, "Request")["classification"],
        "observed_exclusive"
    );
    assert_eq!(top["alternatives"][0]["kind"], "whole_impl");
    let negative = fixture("fn persist_clock() {}\nfn persist_flush() {}\nfn persist_probe() {}\n");
    let no_partition = advice(&negative, json!({}));
    complete(&no_partition);
    assert_eq!(
        no_partition["partition_outcome"],
        "no_credible_written_partition"
    );
    assert_eq!(no_partition["drafts"], json!([]));
    assert_eq!(no_partition["ownership_candidates"], json!([]));
    assert!(
        no_partition["signals"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["kind"] == "name_prefix")
    );
    let incomplete = advice(&negative, json!({"max_items":1}));
    assert_eq!(incomplete["partition_outcome"], "incomplete_analysis");
    assert_eq!(
        incomplete["boundary_observations"]["coverage"]["completed"],
        false
    );
}

#[test]
fn signature_payload_seeds_an_acyclic_subset_without_self_hubs() {
    let repo = fixture(
        "struct Owner;\nstruct Input;\nimpl Owner { fn begin(&self, p: Input) { let _ = p; } fn finish(&self, p: Input) { let _ = p; } fn clock(&self) {} }\n",
    );
    let response = advice(&repo, json!({}));
    complete(&response);
    assert_eq!(
        core_names(&response, &response["ownership_candidates"][0]),
        BTreeSet::from(["begin".into(), "finish".into()])
    );
}

#[test]
fn same_owner_orchestrator_is_not_an_acyclic_joining_hub() {
    let repo = fixture(
        "struct Owner;\nimpl Owner { fn left(&self) { self.left_tail(); } fn left_tail(&self) {} fn right(&self) { self.right_tail(); } fn right_tail(&self) {} fn orchestrate(&self) { self.left(); self.right(); } }\n",
    );
    let response = advice(&repo, json!({}));
    complete(&response);
    for c in response["ownership_candidates"].as_array().unwrap() {
        assert!(
            !c["core_item_ids"]
                .as_array()
                .unwrap()
                .contains(id(&response, "orchestrate"))
        );
        let names = core_names(&response, c);
        assert!(!(names.contains("left") && names.contains("right")));
    }
    assert_eq!(
        core_names(&response, &response["ownership_candidates"][0]),
        BTreeSet::from(["left".into(), "left_tail".into()])
    );
}

#[test]
fn outgoing_admitted_routes_keep_anchors_and_do_not_follow_shared_imports() {
    let repo = fixture("fn a() { b(); crate::outside::dependency(); }\nfn b() { a(); }\n");
    repo.write("cases/ownership/outside.rs", "pub fn dependency() {}\n");
    let response = advice(&repo, json!({}));
    complete(&response);
    let outgoing = response["boundary_observations"]["records"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["direction"] == "outgoing")
        .unwrap();
    assert_eq!(outgoing["certainty"], "written_route");
    assert_eq!(
        outgoing["anchor"]["span"]["text"],
        "crate::outside::dependency"
    );
    assert_eq!(outgoing["item_ids"], json!([id(&response, "a")]));
    assert_eq!(
        outgoing["counterpart"]["path"],
        "cases/ownership/outside.rs"
    );
}

#[test]
fn trait_wrapper_discloses_excluded_impl_and_concrete_component_without_semantic_claim() {
    let repo = fixture(
        "trait Storage { fn put(&self); }\nstruct Sink;\nstruct Wrapper { sink: Sink }\nimpl Storage for Wrapper { fn put(&self) {} }\nimpl<T> Storage for T { fn put(&self) {} }\nfn platform() {}\n",
    );
    let response = advice(&repo, json!({}));
    complete(&response);
    let candidate = response["ownership_candidates"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["core_item_ids"] == json!([id(&response, "Wrapper")]))
        .unwrap();
    assert_eq!(
        companion(&response, candidate, "Sink")["role"],
        "payload_or_type"
    );
    assert!(
        candidate["companions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["role"] == "implementation" && c["classification"] == "undetermined")
    );
    assert!(
        candidate["alternatives"]
            .as_array()
            .unwrap()
            .iter()
            .any(|a| a["kind"] == "whole_impl"
                && !a["excluded_item_ids"].as_array().unwrap().is_empty())
    );
    assert!(response["inventory"].as_array().unwrap().iter().any(|i| {
        i["reasons"]
            .as_array()
            .unwrap()
            .contains(&json!("trait_impl"))
    }));
}

#[test]
fn boundary_coverage_and_glob_candidates_do_not_redefine_local_forecasts() {
    let repo = fixture("fn a() { b(); }\nfn b() { a(); }\n");
    repo.write(
        "cases/ownership/outside.rs",
        "use crate::core::*; fn call() { a(); }\n",
    );
    let response = advice(&repo, json!({}));
    complete(&response);
    assert!(response["boundary_observations"]["records"].as_array().unwrap().iter().any(|r| r["certainty"] == "candidate" && r["basis"].as_str().unwrap().contains("glob")));
    for group in response["drafts"][0]["groups"].as_array().unwrap() {
        assert_eq!(group["assessment_scope"]["assessed"], "local_only");
        assert!(
            group["assessment_scope"]["not_assessed"]
                .as_array()
                .unwrap()
                .contains(&json!("other_consumers"))
        );
    }
    let narrowed = advice(
        &repo,
        json!({"paths":["cases/ownership/lib.rs","cases/ownership/core.rs"]}),
    );
    assert!(
        !narrowed["boundary_observations"]["coverage"]["admitted_paths"]
            .as_array()
            .unwrap()
            .contains(&json!("cases/ownership/outside.rs"))
    );
    assert!(
        narrowed["boundary_observations"]["coverage"]["not_assessed"]
            .as_array()
            .unwrap()
            .contains(&json!("unadmitted_files"))
    );
    assert!(
        !narrowed["ownership_candidates"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}
