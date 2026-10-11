use super::*;
use crate::{
    move_plan::{Decision, DecisionAction},
    result::Limits,
};
use serde_json::json;

fn decision(reason: items::DecisionReason, classes: &[&str], blocking: bool) -> Decision {
    let anchor = crate::plan::SourceAnchor {
        path: "source.rs".into(),
        range: crate::result::ByteRange {
            start_byte: 0,
            end_byte: 9,
        },
        expected_text: "fn f() {}".into(),
    };
    let action = DecisionAction::cause(reason);
    Decision {
        reason,
        next_action: action.next_action(),
        action,
        id: "d/0".into(),
        category: "scope_dependency".into(),
        anchors: vec![anchor.clone()],
        item_ids: Vec::new(),
        evidence: Vec::new(),
        unresolved_consequence: "uncertain context".into(),
        resolution: "request_change_required".into(),
        supported_choices: Vec::new(),
        selected_choice: None,
        blocks_applicability: blocking,
        chain_diagnostic_ids: Vec::new(),
        lexical_uncertainty: None,
        removal_gap: None,
        refusal_basis: classes
            .iter()
            .enumerate()
            .map(|(index, class)| {
                items::RefusalBasis::new(
                    class,
                    "source.rs",
                    Some(crate::result::ByteRange {
                        start_byte: index,
                        end_byte: index + 1,
                    }),
                )
            })
            .collect(),
    }
}

#[test]
fn every_class_is_counted_once_per_blocking_decision_without_reclassification() {
    use items::DecisionReason::{ConditionalOrInheritedContext, LexicalContextUnproved};
    let mut result = MoveEnvelope::empty(Limits::default());
    result.plan.decisions = vec![
        decision(
            ConditionalOrInheritedContext,
            &["derive_veto", "shadow", "derive_veto"],
            true,
        ),
        decision(ConditionalOrInheritedContext, &["derive_veto"], true),
        decision(LexicalContextUnproved, &[], true),
        decision(ConditionalOrInheritedContext, &["ignored"], false),
    ];
    let original = serde_json::to_value(&result.plan.decisions).unwrap();
    result.summarize_blockers();
    let entries = serde_json::to_value(&result.plan.blocked_summary).unwrap();
    assert_eq!(result.counts.blocked_decisions, 3);
    assert_eq!(result.counts.blocked_summary, 3);
    let derive = entries
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["class"] == "derive_veto")
        .unwrap();
    assert_eq!(derive["count"], 2);
    assert_eq!(derive["reason"], "conditional_or_inherited_context");
    assert_eq!(
        derive["top_anchor"],
        json!({"path":"source.rs","range":{"start_byte":0,"end_byte":1}})
    );
    assert!(
        entries
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["class"].is_null() && e["count"] == 1)
    );
    assert!(
        entries
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["class"] == "shadow" && e["count"] == 1)
    );
    assert_eq!(
        serde_json::to_value(&result.plan.decisions).unwrap(),
        original
    );
}

#[test]
fn summary_pressure_omits_only_summary_and_discloses_memberships() {
    let mut result = MoveEnvelope::empty(Limits::default());
    result.plan.decisions.push(decision(
        items::DecisionReason::ConditionalOrInheritedContext,
        &["derive_veto"],
        true,
    ));
    result.summarize_blockers();
    // Make only the optional summary oversized; original individual evidence fits.
    result.plan.blocked_summary[0].class = Some("x".repeat(20_000));
    result.limits.response_bytes = result.planning_wire_bytes() + 1024;
    let original = serde_json::to_value(&result.plan.decisions).unwrap();
    result.fit();
    assert!(result.wire_bytes() <= result.limits.response_bytes);
    assert!(result.plan.blocked_summary.is_empty());
    assert_eq!(result.counts.blocked_summary, 1);
    assert_eq!(result.counts.blocked_decisions, 1);
    assert_eq!(result.counts.omissions["blocked_summary"], 1);
    assert_eq!(result.counts.omissions["blocked_summary_decisions"], 1);
    assert_eq!(
        result.truncation_reasons,
        ["blocked_summary_response_bytes"]
    );
    assert_eq!(result.status, "complete");
    assert_eq!(
        serde_json::to_value(&result.plan.decisions).unwrap(),
        original
    );
    assert!(!result.counts.omissions.contains_key("decisions"));
}

#[test]
fn optional_summary_cannot_withhold_otherwise_fitting_artifacts() {
    let mut result = MoveEnvelope::empty(Limits::default());
    result.plan.applicable = true;
    result.plan.state = "applicable".into();
    result.plan.edits = Some(Vec::new());
    result.plan.created_files = Some(Vec::new());
    result.plan.patch = Some(String::new());
    result.summarize_blockers();
    result.limits.response_bytes = result.planning_wire_bytes();
    result.fit();
    assert!(result.plan.applicable);
    assert_eq!(result.status, "complete");
    assert!(
        result.plan.edits.is_some()
            && result.plan.created_files.is_some()
            && result.plan.patch.is_some()
    );
    assert!(result.plan.blocked_summary.is_empty());
}

#[test]
fn generated_schema_describes_summary_and_totals() {
    let schema = serde_json::to_value(rmcp::schemars::schema_for!(MoveEnvelope)).unwrap();
    let text = schema.to_string();
    for field in [
        "blocked_summary",
        "blocked_decisions",
        "top_anchor",
        "BlockedSummary",
        "RefusalAnchor",
    ] {
        assert!(text.contains(field), "missing {field}");
    }
}
