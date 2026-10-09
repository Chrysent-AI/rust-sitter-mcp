use super::*;

#[test]
fn dense_diagnostics_are_retrievable_after_zero_detail_compact_at_fixed_budget() {
    let source = format!(
        "fn alpha() {{ beta(); }}\nfn beta() {{ alpha(); }}\n{}",
        (0..50)
            .map(|i| format!("fn risk_{i}(x: u8) {{ {} }}\n", "x.unknown(); ".repeat(20)))
            .collect::<String>()
    );
    let repo = Fixture::new(&source);
    let store = Store::default();
    let mut request = repo.request();
    request.response_mode = ResponseMode::Compact;
    request.limits.response_bytes = 512 * 1024;
    let (full, evidence) = repo.analyze(&request);
    assert!(full.decisions.len() >= 1000);
    let expected_count = full.decisions.len();
    let response = present(
        full,
        Some(evidence),
        &request,
        &store,
        controls(&AtomicBool::new(false)),
    );
    assert!(wire_bytes(&response) <= request.limits.response_bytes);
    let SplitResponse::Compact(manifest) = response else {
        panic!("expected compact");
    };
    assert!(manifest.manifest_complete && manifest.error.is_none());
    assert_eq!(manifest.analysis_status, "complete");
    assert_eq!(manifest.retention.state, "retained");
    let record = store
        .obtain(
            &identity(&manifest.retention, page(Collection::Decisions, 100)),
            Instant::now(),
        )
        .unwrap();
    let expected: Vec<_> = array(&record.canonical["decisions"])
        .iter()
        .map(|r| detail_record(&record, r, controls(&AtomicBool::new(false)), 65536).unwrap())
        .collect();
    let mut detail = identity(&manifest.retention, page(Collection::Decisions, 100));
    detail.limits.response_bytes = 65536;
    let mut actual = Vec::new();
    loop {
        let result = store.detail(detail.clone(), &AtomicBool::new(false));
        assert!(result.error.is_none(), "{:?}", result.error);
        assert!(wire_bytes(&result) <= 65536);
        assert_eq!(result.total, expected_count);
        actual.extend(result.records);
        let Some(token) = result.next_page_token else {
            break;
        };
        if let Selector::Page { page_token, .. } = &mut detail.selector {
            *page_token = Some(token);
        }
    }
    assert_eq!(actual, expected);
}

#[test]
fn full_display_caps_do_not_cap_the_retained_canonical_inventory_or_decisions() {
    let repo = Fixture::new(SOURCE);
    let engine = Engine::new(repo.0.clone()).unwrap();
    let mut request = repo.request();
    request.max_items = 1;
    let response =
        serde_json::to_value(engine.suggest_split(request, &AtomicBool::new(false))).unwrap();
    assert_eq!(response["status"], "partial");
    assert_eq!(response["inventory"].as_array().unwrap().len(), 1);
    assert_eq!(response["retention"]["state"], "retained");
    let detail: DetailRequest = serde_json::from_value(json!({"analysis_handle":response["retention"]["analysis_handle"],"snapshot_id":response["snapshot_id"],"selector":{"kind":"page","collection":"inventory","page_size":1000}})).unwrap();
    let result = engine.get_split_detail(detail, &AtomicBool::new(false));
    assert!(result.error.is_none());
    assert_eq!(
        result.records.len() as u64,
        response["counts"]["inventory_items"].as_u64().unwrap()
    );
    let decision_detail: DetailRequest = serde_json::from_value(json!({"analysis_handle":response["retention"]["analysis_handle"],"snapshot_id":response["snapshot_id"],"selector":{"kind":"page","collection":"decisions","page_size":1000}})).unwrap();
    let result = engine.get_split_detail(decision_detail, &AtomicBool::new(false));
    assert!(result.error.is_none());
    assert_eq!(
        result.records.len() as u64,
        response["counts"]["decisions"].as_u64().unwrap()
    );
    assert!(response["decisions"].as_array().unwrap().is_empty());
}

#[test]
fn duplicate_display_fitting_preserves_canonical_records_and_historical_pages() {
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
    let repo = Fixture::new(&source);
    let mut request = repo.request();
    request.limits.response_bytes = 2097152;
    request.limits.diagnostic_count = 100000;
    let (full, evidence) = repo.analyze(&request);
    let mut canonical = serde_json::to_value(&full).unwrap();
    // Preserve the existing canonical storage projection: original bytes live once
    // in immutable buffers; fitting must not introduce any additional change.
    strip_source_text(&mut canonical);
    let store = Store::default();
    let response = present(
        full.clone(),
        Some(evidence),
        &request,
        &store,
        controls(&AtomicBool::new(false)),
    );
    assert!(wire_bytes(&response) <= request.limits.response_bytes);
    let SplitResponse::Full(fitted) = response else {
        panic!("expected full");
    };
    assert_eq!(fitted.status, "complete");
    assert_eq!(
        fitted.counts.omissions["duplicate_declaration_display_spans"],
        800
    );
    let retention = fitted.retention.as_ref().unwrap();
    assert_eq!(retention.state, "retained");
    let record = store
        .obtain(
            &identity(retention, page(Collection::Inventory, 1000)),
            Instant::now(),
        )
        .unwrap();
    // Every canonical record, including direct signal evidence, is independent of fitting.
    assert_eq!(record.canonical, canonical);
    assert_eq!(array(&record.canonical["signals"]).len(), 802);
    assert!(
        array(&record.canonical["signals"])
            .iter()
            .filter(|s| matches!(s["kind"].as_str(), Some("item_size" | "name_prefix")))
            .all(|s| !array(&s["evidence"]).is_empty())
    );

    // Compact presentation is constructed from uncapped analysis, never from fitted full.
    let compact = Manifest::from_full(&full, retention.clone());
    assert!(compact.manifest_complete);
    assert!(
        !compact
            .omissions
            .contains_key("duplicate_declaration_display_spans")
    );
    assert_eq!(
        compact.candidate_summaries,
        array(&canonical["ownership_candidates"])
    );
    for (draft, membership) in full.drafts.iter().zip(&compact.draft_memberships) {
        for (group, summary) in draft.groups.iter().zip(&membership.groups) {
            assert_eq!(group.item_ids, summary.item_ids);
            assert_eq!(
                serde_json::to_value(&group.consequence_summary).unwrap(),
                serde_json::to_value(&summary.consequence_summary).unwrap()
            );
        }
    }

    let mut pages = Vec::new();
    for collection in [
        Collection::Inventory,
        Collection::Groups,
        Collection::Decisions,
        Collection::AdviceDecisions,
        Collection::Companions,
    ] {
        let mut detail = identity(retention, page(collection, 1000));
        detail.limits.response_bytes = 16777216;
        let first = store.detail(detail.clone(), &AtomicBool::new(false));
        assert!(
            first.error.is_none() && first.returned_page_complete && first.collection_exhausted
        );
        let expected: Vec<_> =
            collection_records(&record, collection, controls(&AtomicBool::new(false)))
                .unwrap()
                .iter()
                .map(|r| {
                    detail_record(&record, r, controls(&AtomicBool::new(false)), 16777216).unwrap()
                })
                .collect();
        assert_eq!(first.records, expected);
        pages.push((detail, serde_json::to_value(first).unwrap()));
    }
    let unit = identity(
        retention,
        Selector::Units {
            item_ids: vec![full.inventory[0].id.clone()],
        },
    );
    let original = store.detail(unit.clone(), &AtomicBool::new(false));
    assert!(original.error.is_none());
    let range = &full.inventory[0].span.range;
    assert_eq!(
        original.records[0]["item"]["expected_text"],
        &source[range.start_byte..range.end_byte]
    );
    let export = store.export(serde_json::from_value(json!({"analysis_handle":retention.analysis_handle,"snapshot_id":retention.snapshot_id,
        "selection":[{"unit_ref":{"analysis_id":retention.analysis_id,"item_id":full.inventory[0].id},
        "destination":{"kind":"new_sibling","parent_path":"src/lib.rs","path":"src/helpers.rs"}}]})).unwrap(), &AtomicBool::new(false));
    assert!(export.error.is_none(), "{:?}", export.error);
    assert_eq!(
        export.request.unwrap().moves[0].item.expected_text,
        source[range.start_byte..range.end_byte]
    );
    fs::write(repo.0.join("src/worker.rs"), "fn changed() {}\n").unwrap();
    for (detail, before) in pages {
        assert_eq!(
            serde_json::to_value(store.detail(detail, &AtomicBool::new(false))).unwrap(),
            before
        );
    }
    assert_eq!(
        serde_json::to_value(store.detail(unit, &AtomicBool::new(false))).unwrap(),
        serde_json::to_value(original).unwrap()
    );
    assert_eq!(record.canonical, canonical);
}

#[test]
fn aggregate_allocation_capacity_is_independent_of_record_count() {
    let repo = Fixture::new(SOURCE);
    let (full, evidence) = repo.analyze(&repo.request());
    let reserved = preparation_allocation(
        &full,
        &evidence,
        &repo.request(),
        usize::MAX,
        controls(&AtomicBool::new(false)),
    )
    .unwrap();
    let store = Store::new(RetentionLimits {
        records: 2,
        aggregate_bytes: reserved,
        ..RetentionLimits::default()
    });
    let retention = retain(&store, &repo, &repo.request(), Instant::now());
    let (full, evidence) = repo.analyze(&repo.request());
    assert_eq!(
        store
            .prepare(
                &full,
                evidence,
                &repo.request(),
                Instant::now(),
                controls(&AtomicBool::new(false))
            )
            .err(),
        Some("capacity")
    );
    assert_eq!(store.materializations.load(Ordering::Relaxed), 1);
    assert_eq!(store.accounted_allocation(), (1, retention.accounted_bytes));
    assert!(
        store
            .detail(
                identity(&retention, page(Collection::Inventory, 1)),
                &AtomicBool::new(false)
            )
            .error
            .is_none()
    );
}

#[test]
fn explicit_subsets_are_ordered_and_unknown_or_duplicate_ids_withhold_everything() {
    let repo = Fixture::new(SOURCE);
    let store = Store::default();
    let retention = retain(&store, &repo, &repo.request(), Instant::now());
    let items = store.detail(
        identity(&retention, page(Collection::Inventory, 100)),
        &AtomicBool::new(false),
    );
    let ids: Vec<_> = items
        .records
        .iter()
        .take(2)
        .rev()
        .map(|i| i["id"].as_str().unwrap().to_owned())
        .collect();
    let response = store.detail(
        identity(
            &retention,
            Selector::Records {
                collection: Collection::Inventory,
                ids: ids.clone(),
            },
        ),
        &AtomicBool::new(false),
    );
    assert_eq!(
        response
            .records
            .iter()
            .map(|r| r["id"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ids.iter().map(String::as_str).collect::<Vec<_>>()
    );
    error(
        store.detail(
            identity(
                &retention,
                Selector::Records {
                    collection: Collection::Inventory,
                    ids: vec![ids[0].clone(), "unknown".into()],
                },
            ),
            &AtomicBool::new(false),
        ),
        "UNKNOWN_ADVICE_ID",
    );
    error(
        store.detail(
            identity(
                &retention,
                Selector::Units {
                    item_ids: vec![ids[0].clone(), ids[0].clone()],
                },
            ),
            &AtomicBool::new(false),
        ),
        "INVALID_PARAMS",
    );
}

#[test]
fn scope_is_normalized_and_source_modes_participate_in_provenance() {
    use std::os::unix::fs::PermissionsExt;
    let repo = Fixture::new(SOURCE);
    let mut request = repo.request();
    request.paths = Some(vec!["src/./".into(), "src".into()]);
    let (full, evidence) = repo.analyze(&request);
    let store = Store::default();
    let prepared = store
        .prepare(
            &full,
            evidence,
            &request,
            Instant::now(),
            controls(&AtomicBool::new(false)),
        )
        .unwrap();
    assert_eq!(
        prepared.retention.normalized_request.as_ref().unwrap()["paths"],
        json!(["src"])
    );
    assert!(
        prepared.evidence.source_manifest["files"]
            .as_array()
            .unwrap()
            .iter()
            .all(|f| f["content_digest"].as_str().unwrap().starts_with("sha1:"))
    );
    let snapshot = full.snapshot_id;
    let digest = prepared.evidence.scope_input_digest;
    fs::set_permissions(
        repo.0.join("src/worker.rs"),
        fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    let (full, evidence) = repo.analyze(&request);
    assert_ne!(snapshot, full.snapshot_id);
    assert_ne!(digest, evidence.scope_input_digest);
}
