use super::*;

#[test]
fn stage_charges_include_revisited_phases_and_direct_counter_writes() {
    let mut result = SuggestSplitEnvelope::empty(Limits::default());
    result.account(7).unwrap();
    result.analysis_stage(AnalysisStage::Inventory);
    result.account(11).unwrap();
    result.analysis_stage(AnalysisStage::ModuleChain);
    // Module and decision-group collectors also write the shared counter directly.
    result.counts.analysis_descriptor_bytes += 13;
    result.analysis_stage(AnalysisStage::Inventory);
    result.account(17).unwrap();
    result.analysis_stage(AnalysisStage::TestObservations);
    result.effective_work_limits.analysis_descriptor_bytes = 50;
    let error = result.account(19).unwrap_err();
    result.disclose_analysis_budget(&error);
    let budget = serde_json::to_value(result.analysis_budget.as_ref().unwrap()).unwrap();
    assert_eq!(budget["stopped_by"], "aggregate_descriptor_bytes");
    assert_eq!(budget["exhausted_stage"], "test_observations");
    assert_eq!(budget["aggregate_limit_bytes"], 50);
    assert_eq!(budget["accounted_bytes"], 67);
    assert_eq!(
        budget["stage_bytes"],
        serde_json::json!({
            "scope_inputs":7,"inventory":28,"module_chain":13,"test_observations":19
        })
    );
}

#[test]
fn local_guard_does_not_claim_aggregate_limit_exhaustion() {
    let mut result = SuggestSplitEnvelope::empty(Limits::default());
    result.analysis_stage(AnalysisStage::Decisions);
    result.account(23).unwrap();
    result.disclose_analysis_budget(&DomainError::new(
        "analysis_descriptor_bytes",
        "advice decision guard reached",
    ));
    let budget = serde_json::to_value(&result.analysis_budget).unwrap();
    assert_eq!(budget["stopped_by"], "stage_local_guard");
    assert_eq!(budget["exhausted_stage"], "decisions");
    assert_eq!(budget["accounted_bytes"], 23);
    assert_eq!(budget["aggregate_limit_bytes"], 134217728);
}

#[test]
fn absent_disclosure_and_internal_accounting_preserve_serialized_bytes() {
    let mut result = SuggestSplitEnvelope::empty(Limits::default());
    let bytes = serde_json::to_vec(&result).unwrap();
    result.analysis_stage(AnalysisStage::Signals);
    result.analysis_stage(AnalysisStage::Decisions);
    result.disclose_analysis_budget(&DomainError::new("planning_deadline", "expired"));
    assert_eq!(serde_json::to_vec(&result).unwrap(), bytes);
    let value = serde_json::to_value(&result).unwrap();
    assert!(value.get("analysis_budget").is_none());
    assert!(value.get("accounting").is_none());
}

#[test]
fn terminal_wire_fit_keeps_budget_and_incomplete_labels() {
    let mut result = SuggestSplitEnvelope::empty(Limits::default());
    result.limits.response_bytes = 64 * 1024;
    result.analysis_stage(AnalysisStage::TestObservations);
    let error = result.account(134217729).unwrap_err();
    result.disclose_analysis_budget(&error);
    result.incomplete(&error.code);
    result.inventory = vec![];
    result
        .fit(Controls {
            deadline: Instant::now() + Duration::from_secs(5),
            cancelled: &AtomicBool::new(false),
        })
        .unwrap();
    assert_eq!(result.status, "partial");
    assert_eq!(result.partition_outcome, "incomplete_analysis");
    assert!(!result.draft_eligibility.evidence_complete);
    assert!(result.analysis_budget.is_some());
    assert!(result.wire_bytes() <= result.limits.response_bytes);
}
