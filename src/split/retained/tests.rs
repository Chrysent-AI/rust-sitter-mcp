use super::*;
use crate::engine::Engine;
use std::{fs, path::PathBuf, process::Command};
#[path = "budget_tests.rs"]
mod budget_tests;

static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Fixture(PathBuf);
impl Fixture {
    fn new(source: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "rust-sitter-retained-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        assert!(
            Command::new("git")
                .args(["init", "--quiet", "--template="])
                .arg(&root)
                .status()
                .unwrap()
                .success()
        );
        fs::create_dir(root.join("src")).unwrap();
        fs::write(root.join("src/lib.rs"), "mod worker;\nmod outside;\n").unwrap();
        fs::write(root.join("src/worker.rs"), source).unwrap();
        fs::write(
            root.join("src/outside.rs"),
            "fn inspect() { crate::worker::alpha(); }\n",
        )
        .unwrap();
        Self(root)
    }
    fn request(&self) -> SuggestSplitRequest {
        serde_json::from_value(json!({"repo_path":self.0,"crate_root":"src/lib.rs","source_path":"src/worker.rs","paths":["src"],"retain_snapshot":true,"limits":{"diagnostic_count":0,"text_bytes":0,"response_bytes":16777216}})).unwrap()
    }
    fn analyze(&self, request: &SuggestSplitRequest) -> (SuggestSplitEnvelope, Evidence) {
        let (full, evidence) =
            analyze_with_recheck(&self.0, request, &AtomicBool::new(false), || {});
        assert_eq!(
            full.status,
            "complete",
            "{}",
            serde_json::to_string(&full).unwrap()
        );
        assert!(full.error.is_none());
        (full, evidence.unwrap())
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}
const SOURCE: &str = "struct Worker;\nimpl Worker {\n fn first() { Self::second(); }\n fn second() { Self::first(); }\n}\nfn alpha() { beta(); }\nfn beta() { alpha(); }\nfn uncertain(x: u8) { x.one(); x.two(); }\n#[cfg(test)] mod tests { use super::*; #[test] fn check() { alpha(); assert!(true); } }\n";
fn controls(cancelled: &AtomicBool) -> Controls<'_> {
    Controls {
        deadline: Instant::now() + Duration::from_secs(60),
        cancelled,
    }
}
fn identity(retention: &Retention, selector: Selector) -> DetailRequest {
    DetailRequest {
        analysis_handle: retention.analysis_handle.clone().unwrap(),
        snapshot_id: retention.snapshot_id.clone().unwrap(),
        analysis_id: retention.analysis_id.clone(),
        scope_input_digest: retention.scope_input_digest.clone(),
        selector,
        limits: DetailLimits::default(),
    }
}
fn page(collection: Collection, count: usize) -> Selector {
    Selector::Page {
        collection,
        filter: DetailFilter::default(),
        page_size: count,
        page_token: None,
    }
}
fn retain(store: &Store, repo: &Fixture, request: &SuggestSplitRequest, now: Instant) -> Retention {
    let (full, evidence) = repo.analyze(request);
    let record = store.prepare(&full, evidence, request, now).unwrap();
    store
        .publish(record, controls(&AtomicBool::new(false)), now)
        .unwrap()
}
fn error(result: DetailEnvelope, code: &str) {
    assert_eq!(result.error.unwrap().code, code);
    assert!(result.records.is_empty());
}

#[test]
fn every_page_union_is_uncapped_canonical_evidence_and_retries_are_identical() {
    let repo = Fixture::new(SOURCE);
    let store = Store::default();
    let request = repo.request();
    let (full, evidence) = repo.analyze(&request);
    assert!(!full.decisions.is_empty() && !full.advice_decisions.is_empty());
    let record = store
        .prepare(&full, evidence, &request, Instant::now())
        .unwrap();
    let canonical = record.canonical.clone();
    let retained = store
        .publish(record, controls(&AtomicBool::new(false)), Instant::now())
        .unwrap();
    let record = store
        .obtain(
            &identity(&retained, page(Collection::Inventory, 1)),
            Instant::now(),
        )
        .unwrap();
    assert_eq!(record.canonical, canonical);
    for collection in [
        Collection::Inventory,
        Collection::Groups,
        Collection::Decisions,
        Collection::AdviceDecisions,
        Collection::Companions,
        Collection::BoundaryObservations,
        Collection::TestCoupling,
        Collection::Overlaps,
        Collection::ChainDiagnostics,
    ] {
        let expected: Vec<_> =
            collection_records(&record, collection, controls(&AtomicBool::new(false)))
                .unwrap()
                .iter()
                .map(|r| {
                    detail_record(
                        &record,
                        r,
                        controls(&AtomicBool::new(false)),
                        16 * 1024 * 1024,
                    )
                    .unwrap()
                })
                .collect();
        let mut request = identity(&retained, page(collection, 1));
        let mut actual = Vec::new();
        loop {
            let first = store.detail(request.clone(), &AtomicBool::new(false));
            assert!(first.error.is_none(), "{:?}", first.error);
            let second = store.detail(request.clone(), &AtomicBool::new(false));
            assert_eq!(
                serde_json::to_vec(&first).unwrap(),
                serde_json::to_vec(&second).unwrap()
            );
            assert!(first.historical && first.returned_page_complete);
            assert_eq!(first.live_freshness, "not_checked");
            assert_eq!(first.integrity.semantic, "not_performed");
            assert_eq!(first.total, expected.len());
            assert!(wire_bytes(&first) <= request.limits.response_bytes);
            actual.extend(first.records);
            let Some(token) = first.next_page_token else {
                assert!(first.collection_exhausted);
                break;
            };
            assert!(!first.collection_exhausted);
            if let Selector::Page { page_token, .. } = &mut request.selector {
                *page_token = Some(token);
            }
        }
        assert_eq!(actual, expected, "collection {}", collection.name());
    }
    assert_eq!(store.accounted_allocation().0, 1);
    assert_eq!(store.accounted_allocation().1, retained.accounted_bytes);
    assert!(retained.accounted_bytes > SOURCE.len());
}

#[test]
fn full_unit_and_header_anchors_are_exact_and_history_never_reads_live_files() {
    let repo = Fixture::new(SOURCE);
    let engine = Engine::new(repo.0.clone()).unwrap();
    let mut request = repo.request();
    request.limits.response_bytes = 65536;
    let response =
        serde_json::to_value(engine.suggest_split(request, &AtomicBool::new(false))).unwrap();
    let retention: &Value = &response["retention"];
    assert_eq!(retention["state"], "retained");
    let id = response["inventory"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["name"] == "first")
        .unwrap()["id"]
        .as_str()
        .unwrap();
    let request: DetailRequest = serde_json::from_value(json!({"analysis_handle":retention["analysis_handle"],"snapshot_id":retention["snapshot_id"],"selector":{"kind":"units","item_ids":[id]}})).unwrap();
    let before = engine.get_split_detail(request.clone(), &AtomicBool::new(false));
    assert!(before.error.is_none());
    assert_eq!(
        before.records[0]["item"]["expected_text"],
        "fn first() { Self::second(); }"
    );
    assert_eq!(
        before.records[0]["enclosing_impl"]["expected_text"],
        "impl Worker "
    );
    assert_eq!(
        before.records[0]["unit_ref"]["analysis_id"],
        retention["analysis_id"]
    );
    fs::write(repo.0.join("src/worker.rs"), "fn replaced() {}\n").unwrap();
    fs::write(repo.0.join(".gitignore"), "*.rs\n").unwrap();
    fs::remove_file(repo.0.join("src/outside.rs")).unwrap();
    let after = engine.get_split_detail(request, &AtomicBool::new(false));
    assert_eq!(
        serde_json::to_vec(&before).unwrap(),
        serde_json::to_vec(&after).unwrap()
    );
    assert_eq!(
        fs::read_to_string(repo.0.join("src/worker.rs")).unwrap(),
        "fn replaced() {}\n"
    );
}

#[test]
fn fixed_expiry_starts_at_publication_and_does_not_slide_after_navigation() {
    let repo = Fixture::new(SOURCE);
    let store = Store::new(RetentionLimits {
        ttl_seconds: 10,
        ..RetentionLimits::default()
    });
    let initial = store.origin;
    let (full, evidence) = repo.analyze(&repo.request());
    let record = store
        .prepare(&full, evidence, &repo.request(), initial)
        .unwrap();
    let published = initial + Duration::from_secs(2);
    let retention = store
        .publish(record, controls(&AtomicBool::new(false)), published)
        .unwrap();
    let request = identity(&retention, page(Collection::Inventory, 1));
    assert!(
        store
            .detail_at(
                request.clone(),
                &AtomicBool::new(false),
                initial + Duration::from_secs(11)
            )
            .error
            .is_none()
    );
    error(
        store.detail_at(
            request.clone(),
            &AtomicBool::new(false),
            initial + Duration::from_secs(12),
        ),
        "ADVICE_SNAPSHOT_EXPIRED",
    );
    assert_eq!(store.accounted_allocation(), (0, 0));
    error(
        store.detail_at(
            request,
            &AtomicBool::new(false),
            initial + Duration::from_secs(13),
        ),
        "ADVICE_SNAPSHOT_EXPIRED",
    );
}

#[test]
fn capacity_release_restart_and_inflight_accounting_do_not_silently_evict() {
    let repo = Fixture::new(SOURCE);
    let store = Store::default();
    let request = repo.request();
    let retention = retain(&store, &repo, &request, Instant::now());
    let detail = identity(&retention, page(Collection::Inventory, 2));
    let inflight = store.obtain(&detail, Instant::now()).unwrap();
    let (full, evidence) = repo.analyze(&request);
    let extra = store
        .prepare(&full, evidence, &request, Instant::now())
        .unwrap();
    assert_eq!(
        store
            .publish(extra, controls(&AtomicBool::new(false)), Instant::now())
            .unwrap_err(),
        "capacity"
    );
    assert!(
        store
            .detail(detail.clone(), &AtomicBool::new(false))
            .error
            .is_none()
    );
    error(
        Store::default().detail(detail.clone(), &AtomicBool::new(false)),
        "ADVICE_SNAPSHOT_UNKNOWN",
    );
    let release = identity(&retention, Selector::Release {});
    assert_eq!(
        store.detail(release, &AtomicBool::new(false)).released,
        Some(true)
    );
    assert_eq!(store.accounted_allocation(), (1, retention.accounted_bytes));
    error(
        store.detail(detail, &AtomicBool::new(false)),
        "ADVICE_SNAPSHOT_UNKNOWN",
    );
    drop(inflight);
    assert_eq!(store.accounted_allocation(), (0, 0));
    let next = retain(&store, &repo, &request, Instant::now());
    assert_ne!(next.analysis_handle, retention.analysis_handle);
}

#[test]
fn independent_analyses_on_equal_corpus_never_alias() {
    let repo = Fixture::new(SOURCE);
    let engine = Engine::with_retention_limits(
        repo.0.clone(),
        RetentionLimits {
            records: 3,
            ..RetentionLimits::default()
        },
    )
    .unwrap();
    let a = serde_json::to_value(engine.suggest_split(repo.request(), &AtomicBool::new(false)))
        .unwrap();
    let mut request = repo.request();
    request.context.before_lines = 1;
    let b = serde_json::to_value(engine.suggest_split(request, &AtomicBool::new(false))).unwrap();
    let c = serde_json::to_value(engine.suggest_split(repo.request(), &AtomicBool::new(false)))
        .unwrap();
    assert_eq!(a["snapshot_id"], b["snapshot_id"]);
    assert_eq!(a["snapshot_id"], c["snapshot_id"]);
    assert_ne!(
        a["retention"]["analysis_handle"],
        b["retention"]["analysis_handle"]
    );
    assert_ne!(
        a["retention"]["analysis_handle"],
        c["retention"]["analysis_handle"]
    );
    assert_ne!(a["retention"]["analysis_id"], b["retention"]["analysis_id"]);
    assert_eq!(engine.retained_allocation().0, 3);
}

#[test]
fn cancelled_preparation_publication_and_detail_leave_no_partial_record_or_anchors() {
    let repo = Fixture::new(SOURCE);
    let store = Store::default();
    let request = repo.request();
    let (full, evidence) = repo.analyze(&request);
    let record = store
        .prepare(&full, evidence, &request, Instant::now())
        .unwrap();
    let handle = record.retention.clone();
    assert_eq!(
        store
            .publish(record, controls(&AtomicBool::new(true)), Instant::now())
            .unwrap_err(),
        "cancelled_before_publication"
    );
    assert_eq!(store.accounted_allocation(), (0, 0));
    error(
        store.detail(
            identity(&handle, page(Collection::Inventory, 1)),
            &AtomicBool::new(false),
        ),
        "ADVICE_SNAPSHOT_UNKNOWN",
    );
    let engine = Engine::new(repo.0.clone()).unwrap();
    let response =
        serde_json::to_value(engine.suggest_split(request, &AtomicBool::new(true))).unwrap();
    assert_eq!(response["error"]["code"], "CANCELLED");
    assert!(response["retention"]["analysis_handle"].is_null());
    assert_eq!(engine.retained_allocation(), (0, 0));
    let retention = retain(&store, &repo, &repo.request(), Instant::now());
    error(
        store.detail(
            identity(&retention, page(Collection::Inventory, 1)),
            &AtomicBool::new(true),
        ),
        "CANCELLED",
    );
    assert_eq!(store.accounted_allocation().0, 1);
    error(
        store.detail(
            identity(&retention, Selector::Release {}),
            &AtomicBool::new(true),
        ),
        "CANCELLED",
    );
    assert_eq!(store.accounted_allocation().0, 1);
}

#[test]
fn identity_filter_ids_and_page_options_are_validated_and_bound() {
    let repo = Fixture::new(SOURCE);
    let store = Store::new(RetentionLimits {
        records: 2,
        ..RetentionLimits::default()
    });
    let retained = retain(&store, &repo, &repo.request(), Instant::now());
    let other = retain(&store, &repo, &repo.request(), Instant::now());
    let request = identity(&retained, page(Collection::Inventory, 1));
    let first = store.detail(request.clone(), &AtomicBool::new(false));
    let token = first.next_page_token.unwrap();
    let mut resumed = request.clone();
    if let Selector::Page { page_token, .. } = &mut resumed.selector {
        *page_token = Some(token.clone());
    }
    for field in ["snapshot_id", "analysis_id", "scope_input_digest"] {
        let mut wrong = serde_json::to_value(&request).unwrap();
        wrong[field] = json!("wrong");
        let response = store.detail(
            serde_json::from_value(wrong).unwrap(),
            &AtomicBool::new(false),
        );
        assert_eq!(
            response.error.as_ref().unwrap().field.as_deref(),
            Some(field)
        );
        error(response, "ADVICE_SNAPSHOT_MISMATCH");
    }
    for mutation in [
        json!({"collection":"decisions"}),
        json!({"page_size":2}),
        json!({"filter":{"candidate_id":"candidate/0"}}),
        json!({"page_token":"p1/not-a-token"}),
    ] {
        let mut wrong = serde_json::to_value(&resumed).unwrap();
        for (key, value) in mutation.as_object().unwrap() {
            wrong["selector"][key] = value.clone();
        }
        error(
            store.detail(
                serde_json::from_value(wrong).unwrap(),
                &AtomicBool::new(false),
            ),
            "INVALID_ADVICE_PAGE",
        );
    }
    let mut wrong = resumed.clone();
    wrong.limits.response_bytes = 65536;
    error(
        store.detail(wrong, &AtomicBool::new(false)),
        "INVALID_ADVICE_PAGE",
    );
    let mut wrong = identity(&other, resumed.selector.clone());
    error(
        store.detail(wrong.clone(), &AtomicBool::new(false)),
        "INVALID_ADVICE_PAGE",
    );
    wrong.analysis_handle = "forged".into();
    error(
        store.detail(wrong, &AtomicBool::new(false)),
        "ADVICE_SNAPSHOT_UNKNOWN",
    );
    for selector in [
        Selector::Records {
            collection: Collection::Inventory,
            ids: vec!["d/0".into()],
        },
        Selector::Units {
            item_ids: vec!["unknown".into()],
        },
        Selector::Page {
            collection: Collection::Inventory,
            filter: DetailFilter {
                id: Some("unknown".into()),
                ..DetailFilter::default()
            },
            page_size: 1,
            page_token: None,
        },
        Selector::Page {
            collection: Collection::Inventory,
            filter: DetailFilter {
                candidate_id: Some("candidate/unknown".into()),
                ..DetailFilter::default()
            },
            page_size: 1,
            page_token: None,
        },
    ] {
        error(
            store.detail(identity(&retained, selector), &AtomicBool::new(false)),
            "UNKNOWN_ADVICE_ID",
        );
    }
    error(
        store.detail(
            identity(
                &retained,
                Selector::Page {
                    collection: Collection::Inventory,
                    filter: DetailFilter {
                        reason: Some("made_up".into()),
                        ..DetailFilter::default()
                    },
                    page_size: 1,
                    page_token: None,
                },
            ),
            &AtomicBool::new(false),
        ),
        "INVALID_PARAMS",
    );
    for selector in [
        json!({"kind":"page","collection":"inventory","filter":{"unknown":"value"}}),
        json!({"kind":"page","collection":"unknown"}),
        json!({"kind":"release","text_bytes":1}),
    ] {
        let mut invalid = serde_json::to_value(&request).unwrap();
        invalid["selector"] = selector;
        assert!(serde_json::from_value::<DetailRequest>(invalid).is_err());
    }
}

#[test]
fn compact_preserves_canonical_membership_consequences_counts_and_availability() {
    let repo = Fixture::new(SOURCE);
    let engine = Engine::with_retention_limits(
        repo.0.clone(),
        RetentionLimits {
            records: 2,
            ..RetentionLimits::default()
        },
    )
    .unwrap();
    let mut request = repo.request();
    request.limits.diagnostic_count = 100000;
    let full = serde_json::to_value(engine.suggest_split(request.clone(), &AtomicBool::new(false)))
        .unwrap();
    request.response_mode = ResponseMode::Compact;
    request.limits.diagnostic_count = 0;
    request.max_items = 1;
    let compact =
        serde_json::to_value(engine.suggest_split(request, &AtomicBool::new(false))).unwrap();
    assert_eq!(compact["schema_version"], 1);
    assert_eq!(compact["envelope_kind"], "split_manifest");
    assert_eq!(compact["analysis_status"], "complete");
    assert_eq!(compact["manifest_complete"], true);
    assert!(compact.get("inventory").is_none());
    assert_eq!(compact["retention"]["state"], "retained");
    assert_eq!(
        compact["totals"]["inventory"],
        full["counts"]["inventory_items"]
    );
    assert_eq!(compact["totals"]["decisions"], full["counts"]["decisions"]);
    assert_eq!(compact["candidate_summaries"], full["ownership_candidates"]);
    assert_eq!(compact["decision_groups"], full["decision_groups"]);
    for (summary, draft) in compact["draft_memberships"]
        .as_array()
        .unwrap()
        .iter()
        .zip(full["drafts"].as_array().unwrap())
    {
        for (group, original) in summary["groups"]
            .as_array()
            .unwrap()
            .iter()
            .zip(draft["groups"].as_array().unwrap())
        {
            assert_eq!(group["item_ids"], original["item_ids"]);
            assert_eq!(
                group["consequence_summary"],
                original["consequence_summary"]
            );
        }
    }
    for availability in compact["detail_availability"].as_array().unwrap() {
        assert_eq!(availability["retrievable"], true);
        assert_eq!(availability["permanently_unavailable"], 0);
    }
}

#[test]
fn record_byte_capacity_and_manifest_overflow_preserve_honest_analysis_status() {
    let repo = Fixture::new(SOURCE);
    let engine = Engine::with_retention_limits(
        repo.0.clone(),
        RetentionLimits {
            aggregate_bytes: 1,
            ..RetentionLimits::default()
        },
    )
    .unwrap();
    let mut request = repo.request();
    request.response_mode = ResponseMode::Compact;
    let response =
        serde_json::to_value(engine.suggest_split(request, &AtomicBool::new(false))).unwrap();
    assert_eq!(response["analysis_status"], "complete");
    assert_eq!(response["retention"]["reason"], "record_too_large");
    assert_eq!(engine.retained_allocation(), (0, 0));
    for a in response["detail_availability"].as_array().unwrap() {
        assert_eq!(a["retrievable"], false);
        assert_eq!(a["permanently_unavailable"], a["total"]);
    }
    let repo = Fixture::new(
        &(0..600)
            .map(|i| format!("fn item_{i}() {{ item_{}(); }}\n", i ^ 1))
            .collect::<String>(),
    );
    let engine = Engine::new(repo.0.clone()).unwrap();
    let mut request = repo.request();
    request.response_mode = ResponseMode::Compact;
    request.limits.response_bytes = 65536;
    let response = engine.suggest_split(request, &AtomicBool::new(false));
    assert!(wire_bytes(&response) <= 65536);
    let response = serde_json::to_value(response).unwrap();
    assert_eq!(response["error"]["code"], "ADVICE_MANIFEST_TOO_LARGE");
    assert_eq!(response["analysis_status"], "complete");
    assert_eq!(response["manifest_complete"], false);
    assert!(response["retention"]["analysis_handle"].is_null());
    assert_eq!(engine.retained_allocation(), (0, 0));
    for a in response["detail_availability"].as_array().unwrap() {
        assert_eq!(a["retrievable"], false);
    }
}

#[test]
fn oversized_unit_and_record_refuse_without_complete_looking_snippets() {
    let text = format!(
        "fn enormous() {{ /*{}*/ }}\nfn small() {{}}\n",
        "λ\\\"".repeat(20000)
    );
    let repo = Fixture::new(&text);
    let store = Store::default();
    let retained = retain(&store, &repo, &repo.request(), Instant::now());
    let record = store
        .obtain(
            &identity(&retained, page(Collection::Inventory, 1)),
            Instant::now(),
        )
        .unwrap();
    let id = record.canonical["inventory"][0]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    for selector in [
        Selector::Units {
            item_ids: vec![id.clone()],
        },
        Selector::Records {
            collection: Collection::Inventory,
            ids: vec![id],
        },
        page(Collection::Inventory, 1),
    ] {
        let mut request = identity(&retained, selector);
        request.limits.response_bytes = 65536;
        error(
            store.detail(request, &AtomicBool::new(false)),
            "DETAIL_TOO_LARGE",
        );
    }
}

#[test]
fn legacy_off_is_identical_and_compact_does_not_implicitly_enable_retention() {
    let repo = Fixture::new(SOURCE);
    let engine = Engine::new(repo.0.clone()).unwrap();
    let mut request = repo.request();
    request.retain_snapshot = false;
    let expected = run_with_recheck(&repo.0, request.clone(), &AtomicBool::new(false), || {});
    let actual = engine.suggest_split(request.clone(), &AtomicBool::new(false));
    assert_eq!(
        serde_json::to_value(&actual).unwrap(),
        serde_json::to_value(expected).unwrap()
    );
    assert_eq!(engine.retained_allocation(), (0, 0));
    assert!(
        serde_json::to_value(actual)
            .unwrap()
            .get("retention")
            .is_none()
    );
    request.response_mode = ResponseMode::Compact;
    let compact =
        serde_json::to_value(engine.suggest_split(request, &AtomicBool::new(false))).unwrap();
    assert_eq!(compact["retention"]["state"], "not_requested");
    assert_eq!(engine.retained_allocation(), (0, 0));
    assert_eq!(compact["analysis_status"], "complete");
    for a in compact["detail_availability"].as_array().unwrap() {
        assert_eq!(a["retrievable"], false);
    }
}

#[test]
fn ignore_presence_bytes_and_removal_change_input_digest_not_corpus_snapshot() {
    let repo = Fixture::new(SOURCE);
    let request = repo.request();
    let (base, evidence) = repo.analyze(&request);
    let snapshot = base.snapshot_id;
    let initial_digest = evidence.scope_input_digest;
    assert!(
        evidence
            .inputs
            .ignores
            .iter()
            .any(|i| i.path == "src/.gitignore" && i.bytes.is_none())
    );
    let mut digests = BTreeSet::from([initial_digest.clone()]);
    for text in [
        "# same admitted files\n",
        "# changed comment only\r\n",
        "*.never_present\n",
    ] {
        fs::write(repo.0.join("src/.gitignore"), text).unwrap();
        let (full, evidence) = repo.analyze(&request);
        assert_eq!(snapshot, full.snapshot_id);
        assert!(digests.insert(evidence.scope_input_digest));
        assert_eq!(
            evidence
                .inputs
                .ignores
                .iter()
                .find(|i| i.path == "src/.gitignore")
                .unwrap()
                .bytes
                .as_deref(),
            Some(text.as_bytes())
        );
    }
    fs::remove_file(repo.0.join("src/.gitignore")).unwrap();
    let (_, evidence) = repo.analyze(&request);
    assert_eq!(evidence.scope_input_digest, initial_digest);
    let (full, _) = analyze_with_recheck(&repo.0, &request, &AtomicBool::new(false), || {
        fs::write(repo.0.join(".gitignore"), "# changed during observation\n").unwrap();
    });
    assert_ne!(full.status, "complete");
    assert!(
        full.truncation_reasons
            .iter()
            .any(|r| r == "SOURCE_CHANGED")
    );
}

#[test]
fn unsafe_policy_inputs_fail_closed_and_hard_boundaries_are_not_observed() {
    let repo = Fixture::new(SOURCE);
    fs::create_dir(repo.0.join("target")).unwrap();
    fs::create_dir(repo.0.join("target/.gitignore")).unwrap();
    fs::create_dir(repo.0.join("nested")).unwrap();
    fs::create_dir(repo.0.join("nested/.git")).unwrap();
    fs::create_dir(repo.0.join("nested/.gitignore")).unwrap();
    let (_, evidence) = repo.analyze(&repo.request());
    assert!(
        !evidence
            .inputs
            .ignores
            .iter()
            .any(|i| i.path.starts_with("target/") || i.path.starts_with("nested/"))
    );
    std::os::unix::fs::symlink(repo.0.join("src/lib.rs"), repo.0.join("src/.gitignore")).unwrap();
    let (full, evidence) =
        analyze_with_recheck(&repo.0, &repo.request(), &AtomicBool::new(false), || {});
    assert!(evidence.is_none());
    assert_ne!(full.status, "complete");
    assert!(
        full.truncation_reasons
            .iter()
            .any(|r| r == "scan_incomplete")
    );
}
