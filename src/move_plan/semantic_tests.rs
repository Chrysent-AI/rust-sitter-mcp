use super::*;
use std::sync::atomic::Ordering;

#[test]
fn controller_cancels_both_live_databases_and_joins() {
    let old = RootDatabase::default();
    let new = RootDatabase::default();
    let cancelled = AtomicBool::new(false);
    let (ready, started) = mpsc::channel();
    std::thread::scope(|threads| {
        let flag = &cancelled;
        let stop = threads.spawn(move || {
            started.recv_timeout(Duration::from_secs(2)).unwrap();
            flag.store(true, Ordering::Relaxed);
        });
        let outcome = controlled(
            &old,
            &new,
            (Instant::now() + Duration::from_secs(10), &cancelled),
            || {
                ready.send(()).unwrap();
                // Real Salsa cancellation unwinding, not an adapter preflight check.
                loop {
                    old.unwind_if_revision_cancelled();
                    new.unwind_if_revision_cancelled();
                    std::thread::yield_now();
                }
            },
        );
        stop.join().unwrap();
        assert_eq!(outcome.unwrap_err().code, "CANCELLED");
        assert!(old.cancellation_token().is_cancelled());
        assert!(new.cancellation_token().is_cancelled());
    });
}

#[test]
fn controller_deadline_is_reported_as_incomplete_planning_not_success() {
    let old = RootDatabase::default();
    let new = RootDatabase::default();
    let cancelled = AtomicBool::new(false);
    let outcome = controlled(
        &old,
        &new,
        (Instant::now() + Duration::from_millis(20), &cancelled),
        || {
            loop {
                new.unwind_if_revision_cancelled();
                std::thread::yield_now();
            }
        },
    );
    assert_eq!(outcome.unwrap_err().code, "planning_deadline");
}

#[test]
fn non_cancellation_panic_still_stops_and_joins_the_controller() {
    let old = RootDatabase::default();
    let new = RootDatabase::default();
    let cancelled = AtomicBool::new(false);
    let outcome = std::panic::catch_unwind(|| {
        controlled(
            &old,
            &new,
            (Instant::now() + Duration::from_secs(10), &cancelled),
            || {
                panic!("test programmer error");
            },
        )
    });
    assert!(outcome.is_err());
    assert!(!old.cancellation_token().is_cancelled());
    assert!(!new.cancellation_token().is_cancelled());
}

#[test]
fn failed_semantic_stages_disclose_only_the_stage_actually_reached() {
    let flag = AtomicBool::new(false);
    let controls = (Instant::now() + Duration::from_secs(30), &flag);
    let source = "fn called() {} fn selected() { called(); }";
    let config: Configuration = serde_json::from_value(serde_json::json!({"crates":[{
        "name":"probe", "root_file":"lib.rs", "edition":"2024", "features":[], "cfg":[], "dependencies":[]
    }]})).unwrap();
    let texts = |text: &str| BTreeMap::from([("lib.rs".into(), text.into())]);
    let old = Inputs::new(texts(source), &config, controls).unwrap();
    let coverage = ResolutionCoverage {
        decisions: 0,
        configuration: config.clone(),
        snapshot_id: String::new(),
        semantic_input_digest: String::new(),
        final_overlay_digest: String::new(),
        analyzer: ANALYZER.into(),
        statement: String::new(),
        omissions: Vec::new(),
    };
    let start = source.rfind("called").unwrap();
    let need = Need {
        attribute_range: None,
        reason: DecisionReason::ExternalOrMissingBinding,
        choice_target: None,
        lexical_uncertainty: None,
        refusal_basis: Vec::new(),
        category: "test",
        path: "lib.rs".into(),
        range: range(start, start + 6),
        message: "unproved".into(),
        item_ids: Vec::new(),
    };
    for (final_text, class) in [
        (
            "fn called() {} fn selected() { other_(); }",
            "semantic_mapping_unproved",
        ),
        (
            "fn absent() {} fn selected() { called(); }",
            "semantic_final_fact_unproved",
        ),
        (
            "fn absent() {} fn selected() { called(); } fn called() {}",
            "semantic_identity_unproved",
        ),
    ] {
        let new = Inputs::new(texts(final_text), &config, controls).unwrap();
        let mut needs = vec![need.clone()];
        assert!(
            evaluate(&old, &new, &mut needs, &[], &coverage, controls)
                .unwrap()
                .is_empty()
        );
        assert_eq!(needs.len(), 1);
        assert_eq!(needs[0].refusal_basis[0].class, class);
        assert_eq!(
            needs[0].refusal_basis[0].anchor.range,
            Some(need.range.clone())
        );
    }
    let absent = Inputs::new(
        texts("fn absent() {} fn selected() { called(); }"),
        &config,
        controls,
    )
    .unwrap();
    let mut needs = vec![need.clone()];
    assert!(
        evaluate(&absent, &old, &mut needs, &[], &coverage, controls)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        needs[0].refusal_basis[0].class,
        "semantic_source_fact_unproved"
    );
    let mut needs = vec![need.clone()];
    configuration_refusal(&mut needs);
    assert_eq!(
        needs[0].refusal_basis[0].class,
        "semantic_configuration_unproved"
    );
    let mut need = need;
    need.refuse_at_occurrence("semantic_overlay_unavailable");
    assert_eq!(need.refusal_basis[0].class, "semantic_overlay_unavailable");
    assert_eq!(need.refusal_basis[0].anchor.range, Some(need.range.clone()));
}
