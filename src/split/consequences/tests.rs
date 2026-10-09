use super::*;

#[test]
fn unknown_reasons_are_visible_even_alongside_known_reasons() {
    let mut result = SuggestSplitEnvelope::empty(Limits::default());
    result.test_observations.limitations.push(TestLimitation {
        reason: "future_test_route_reason".into(),
        anchor: None,
        paths: Vec::new(),
        note: "unknown future evidence".into(),
    });
    let candidate = OwnershipCandidate {
        id: "candidate/0".into(),
        core_item_ids: vec!["unit".into()],
        consequence_summary: ConsequenceSummary::default(),
        structural_signal_ids: Vec::new(),
        companions: Vec::new(),
        alternatives: Vec::new(),
        ranking: OwnershipRanking::default(),
        observation_scope: String::new(),
        selection_policy: String::new(),
    };
    result.ownership_candidates.push(candidate);
    attach(
        &mut result,
        Controls {
            deadline: Instant::now() + Duration::from_secs(30),
            cancelled: &AtomicBool::new(false),
        },
    )
    .unwrap();
    let summary = &result.ownership_candidates[0].consequence_summary;
    assert_eq!(summary.unmapped.membership.count, 1);
    assert_eq!(summary.unmapped.reasons, ["future_test_route_reason"]);
    let links = serde_json::to_value(&summary.unmapped.membership.decision_refs).unwrap();
    assert_eq!(
        links,
        serde_json::json!([{"collection":"advice_decisions","id_runs":[{"first_id":"ad/0","count":1}]}])
    );
    assert!(summary.classes.values().all(|m| m.count == 0));
    assert_eq!(summary.destination_and_batch_applicability, "not_assessed");
    assert!(classes("future_local_reason").is_empty());
}

#[test]
fn exact_runs_do_not_bridge_interleaved_causes_or_collections() {
    let records: Vec<_> = [
        ("decisions", "d/0"),
        ("decisions", "d/1"),
        ("decisions", "d/2"),
        ("advice_decisions", "ad/0"),
        ("advice_decisions", "ad/1"),
    ]
    .into_iter()
    .map(|(collection, id)| Record {
        collection,
        id,
        classes: BTreeSet::new(),
        unmapped: BTreeSet::new(),
    })
    .collect();
    let value = membership(&records, &BTreeSet::from([0, 2, 3, 4]));
    assert_eq!(value.count, 4);
    assert_eq!(
        serde_json::to_value(value.decision_refs).unwrap(),
        serde_json::json!([
            {"collection":"advice_decisions","id_runs":[{"first_id":"ad/0","count":2}]},
            {"collection":"decisions","id_runs":[{"first_id":"d/0","count":1},{"first_id":"d/2","count":1}]}
        ])
    );
    assert_eq!(
        classes("glob_consumer_unrepaired"),
        [
            "written_context_unprovable",
            "consumer_outside_supported_repair"
        ]
    );
    assert!(!classes("boundary_identity_unproved").contains(&"consumer_outside_supported_repair"));
    assert_eq!(classes("public_path_change"), ["public_path_change"]);
    assert_eq!(
        classes("module_chain_failure"),
        ["written_context_unprovable", "missing_evidence"]
    );
}
