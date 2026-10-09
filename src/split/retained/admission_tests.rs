use super::*;

#[test]
fn candidates_reserve_slots_before_materialization_and_drops_allow_retry() {
    let repo = Fixture::new(SOURCE);
    let request = repo.request();
    let store = Store::default();
    let (full, evidence) = repo.analyze(&request);
    let bound = preparation_allocation(
        &full,
        &evidence,
        &request,
        usize::MAX,
        controls(&AtomicBool::new(false)),
    )
    .unwrap();
    let candidate = store
        .prepare(
            &full,
            evidence,
            &request,
            store.origin,
            controls(&AtomicBool::new(false)),
        )
        .unwrap();
    let charge = candidate.retention.accounted_bytes;
    assert!(charge <= bound);
    assert_eq!(store.accounted_allocation(), (1, bound));
    let (full, evidence) = repo.analyze(&request);
    assert_eq!(
        store
            .prepare(
                &full,
                evidence,
                &request,
                store.origin,
                controls(&AtomicBool::new(false))
            )
            .err(),
        Some("capacity")
    );
    assert_eq!(store.materializations.load(Ordering::Relaxed), 1);
    drop(candidate);
    assert_eq!(store.accounted_allocation(), (0, 0));
    let published = retain(&store, &repo, &request, store.origin);
    assert_eq!(store.accounted_allocation(), (1, published.accounted_bytes));
    assert_eq!(store.materializations.load(Ordering::Relaxed), 2);
    for mode in [ResponseMode::Full, ResponseMode::Compact] {
        let (full, evidence) = repo.analyze(&request);
        let mut request = request.clone();
        request.response_mode = mode;
        let response = present(
            full,
            Some(evidence),
            &request,
            &store,
            controls(&AtomicBool::new(false)),
        );
        let value = serde_json::to_value(response).unwrap();
        assert_eq!(value["retention"]["reason"], "capacity");
        assert_eq!(
            value[if mode == ResponseMode::Full {
                "status"
            } else {
                "analysis_status"
            }],
            "complete"
        );
        assert_eq!(store.materializations.load(Ordering::Relaxed), 2);
        assert_eq!(store.accounted_allocation(), (1, published.accounted_bytes));
    }
}

#[test]
fn a_full_byte_reservation_blocks_another_candidate_without_materialization() {
    let repo = Fixture::new(SOURCE);
    let request = repo.request();
    let store = Store::new(RetentionLimits {
        records: 2,
        ..RetentionLimits::default()
    });
    let (_, mut reservation) = store
        .reserve(store.origin, controls(&AtomicBool::new(false)))
        .unwrap();
    store
        .reserve_bytes(&mut reservation, store.limits.aggregate_bytes)
        .unwrap();
    let (full, evidence) = repo.analyze(&request);
    assert_eq!(
        store
            .prepare(
                &full,
                evidence,
                &request,
                store.origin,
                controls(&AtomicBool::new(false))
            )
            .err(),
        Some("capacity")
    );
    assert_eq!(store.materializations.load(Ordering::Relaxed), 0);
    assert_eq!(
        store.accounted_allocation(),
        (1, store.limits.aggregate_bytes)
    );
    drop(reservation);
    assert_eq!(store.accounted_allocation(), (0, 0));
    retain(&store, &repo, &request, store.origin);
    assert_eq!(store.materializations.load(Ordering::Relaxed), 1);
}

#[test]
fn oversize_estimates_and_preparation_failures_refund_the_slot_before_retry() {
    let repo = Fixture::new(SOURCE);
    let request = repo.request();
    let (full, evidence) = repo.analyze(&request);
    let bound = preparation_allocation(
        &full,
        &evidence,
        &request,
        usize::MAX,
        controls(&AtomicBool::new(false)),
    )
    .unwrap();
    let mut store = Store::new(RetentionLimits {
        aggregate_bytes: bound,
        ..RetentionLimits::default()
    });
    let mut oversized = full.clone();
    oversized.root = Some("x".repeat(bound));
    assert_eq!(
        store
            .prepare(
                &oversized,
                evidence,
                &request,
                store.origin,
                controls(&AtomicBool::new(false))
            )
            .err(),
        Some("record_too_large")
    );
    assert_eq!(store.materializations.load(Ordering::Relaxed), 0);
    assert_eq!(store.accounted_allocation(), (0, 0));
    store.limits.ttl_seconds = u64::MAX;
    let (_, evidence) = repo.analyze(&request);
    assert_eq!(
        store
            .prepare(
                &full,
                evidence,
                &request,
                store.origin,
                controls(&AtomicBool::new(false))
            )
            .err(),
        Some("capacity")
    );
    assert_eq!(store.accounted_allocation(), (0, 0));
    store.limits.ttl_seconds = DEFAULT_TTL_SECONDS;
    retain(&store, &repo, &request, store.origin);
    assert_eq!(store.materializations.load(Ordering::Relaxed), 1);
}

#[test]
fn cancelled_sizing_refunds_an_inflight_reservation() {
    let store = Store::default();
    let cancelled = AtomicBool::new(false);
    let (_, reservation) = store.reserve(store.origin, controls(&cancelled)).unwrap();
    cancelled.store(true, Ordering::Relaxed);
    let repo = Fixture::new(SOURCE);
    let request = repo.request();
    let (full, evidence) = repo.analyze(&request);
    assert_eq!(
        preparation_allocation(&full, &evidence, &request, usize::MAX, controls(&cancelled)),
        Err("cancelled_before_publication")
    );
    drop(reservation);
    assert_eq!(store.accounted_allocation(), (0, 0));
    assert_eq!(store.materializations.load(Ordering::Relaxed), 0);
    assert_eq!(
        store
            .prepare(
                &full,
                evidence,
                &request,
                store.origin,
                controls(&cancelled)
            )
            .err(),
        Some("cancelled_before_publication")
    );
    assert_eq!(store.accounted_allocation(), (0, 0));
    assert_eq!(store.materializations.load(Ordering::Relaxed), 0);
    retain(&store, &repo, &request, store.origin);
}

#[test]
fn reconciliation_cannot_grow_a_reservation() {
    let store = Store::default();
    let (_, mut charge) = store
        .reserve(store.origin, controls(&AtomicBool::new(false)))
        .unwrap();
    store.reserve_bytes(&mut charge, 100).unwrap();
    assert_eq!(charge.reconcile(101), Err("record_too_large"));
    assert_eq!(store.accounted_allocation(), (1, 100));
    charge.reconcile(50).unwrap();
    assert_eq!(store.accounted_allocation(), (1, 50));
    drop(charge);
    assert_eq!(store.accounted_allocation(), (0, 0));
}

#[test]
fn expiry_reclamation_preserves_inflight_charges_and_retry_is_exact() {
    let repo = Fixture::new(SOURCE);
    let store = Store::new(RetentionLimits {
        ttl_seconds: 1,
        ..RetentionLimits::default()
    });
    let request = repo.request();
    let retained = retain(&store, &repo, &request, store.origin);
    let inflight = store
        .obtain(
            &identity(&retained, page(Collection::Inventory, 1)),
            store.origin,
        )
        .unwrap();
    let expired = store.origin + Duration::from_secs(1);
    let (full, evidence) = repo.analyze(&request);
    assert_eq!(
        store
            .prepare(
                &full,
                evidence,
                &request,
                expired,
                controls(&AtomicBool::new(false))
            )
            .err(),
        Some("capacity")
    );
    assert_eq!(store.materializations.load(Ordering::Relaxed), 1);
    assert_eq!(store.accounted_allocation(), (1, retained.accounted_bytes));
    assert!(store.state.lock().unwrap().records.is_empty());
    drop(inflight);
    assert_eq!(store.accounted_allocation(), (0, 0));
    let next = retain(&store, &repo, &request, expired);
    assert_eq!(store.accounted_allocation(), (1, next.accounted_bytes));
    let (full, evidence) = repo.analyze(&request);
    let candidate = store
        .prepare(
            &full,
            evidence,
            &request,
            expired + Duration::from_secs(1),
            controls(&AtomicBool::new(false)),
        )
        .unwrap();
    assert_eq!(store.accounted_allocation().0, 1);
    assert!(store.accounted_allocation().1 >= candidate.retention.accounted_bytes);
    drop(candidate);
    assert_eq!(store.accounted_allocation(), (0, 0));
}

#[test]
fn failed_full_and_compact_response_fits_refund_reservations_for_retry() {
    let repo = Fixture::new(SOURCE);
    let store = Store::default();
    let request = repo.request();
    // Exercise the presentation boundary with oversized mandatory normalized metadata.
    // The analysis/buffers already exist, as they do before ordinary capacity admission.
    for mode in [ResponseMode::Full, ResponseMode::Compact] {
        let (full, mut evidence) = repo.analyze(&request);
        evidence.normalized_scope["globs"] = json!([format!("**/{}", "x".repeat(40000))]);
        let mut limited = request.clone();
        limited.response_mode = mode;
        limited.limits.response_bytes = 65536;
        let response = present(
            full,
            Some(evidence),
            &limited,
            &store,
            controls(&AtomicBool::new(false)),
        );
        assert!(wire_bytes(&response) <= 65536);
        let value = serde_json::to_value(response).unwrap();
        assert_eq!(value["retention"]["reason"], "response_budget");
        assert!(value["retention"]["analysis_handle"].is_null());
        assert_eq!(store.accounted_allocation(), (0, 0));
    }
    assert_eq!(store.materializations.load(Ordering::Relaxed), 2);
    let retained = retain(&store, &repo, &request, store.origin);
    assert_eq!(store.accounted_allocation(), (1, retained.accounted_bytes));
}
