//! Additive counts from the uncapped blocking decision set, never admission inputs.
#[cfg(test)]
#[path = "blocked_summary_tests.rs"]
mod tests;
use super::{MoveEnvelope, items};
use rmcp::schemars::JsonSchema;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct BlockedSummary {
    pub reason: items::DecisionReason,
    /// Additional refusal disclosure, not a classification of the typed reason.
    pub class: Option<String>,
    /// Distinct blocking decisions with this reason/class pair.
    pub count: usize,
    /// First original occurrence in analysis order; no copied source payload.
    pub top_anchor: Option<items::RefusalAnchor>,
}

impl MoveEnvelope {
    pub(super) fn summarize_blockers(&mut self) {
        let mut groups = BTreeMap::new();
        for decision in self
            .plan
            .decisions
            .iter()
            .filter(|d| d.blocks_applicability)
        {
            self.counts.blocked_decisions += 1;
            // A decision can disclose several independent causes. Count each distinct
            // class once, while retaining the original refusal records untouched.
            let mut classes = BTreeSet::new();
            let fallback = decision.anchors.first().map(|anchor| items::RefusalAnchor {
                path: anchor.path.clone(),
                range: Some(anchor.range.clone()),
            });
            let causes: Vec<_> = if decision.refusal_basis.is_empty() {
                vec![(None, fallback)]
            } else {
                decision
                    .refusal_basis
                    .iter()
                    .map(|basis| (Some(basis.class.clone()), Some(basis.anchor.clone())))
                    .collect()
            };
            for (class, top_anchor) in causes {
                if !classes.insert(class.clone()) {
                    continue;
                }
                let group = groups
                    .entry((decision.reason, class.clone()))
                    .or_insert_with(|| BlockedSummary {
                        reason: decision.reason,
                        class,
                        count: 0,
                        top_anchor,
                    });
                group.count += 1;
            }
        }
        self.plan.blocked_summary = groups.into_values().collect();
        self.counts.blocked_summary = self.plan.blocked_summary.len();
    }

    /// Existing fitting sees exactly the pre-summary response, so an optional
    /// disclosure cannot withhold artifacts or displace any existing diagnostic.
    pub(super) fn planning_wire_bytes(&self) -> usize {
        let mut value = serde_json::to_value(self).expect("serializable move");
        value["plan"]
            .as_object_mut()
            .expect("plan object")
            .remove("blocked_summary");
        let counts = value["counts"].as_object_mut().expect("counts object");
        counts.remove("blocked_summary");
        counts.remove("blocked_decisions");
        serde_json::to_vec(&serde_json::json!({"content":[{"type":"text","text":value.to_string()}],"structuredContent":value,"isError":self.error.is_some()})).expect("wire JSON").len() + 4096
    }

    pub(super) fn fit_blocked_summary(&mut self) {
        while self.wire_bytes() > self.limits.response_bytes
            && !self.plan.blocked_summary.is_empty()
        {
            let omitted = self.plan.blocked_summary.pop().expect("nonempty summary");
            *self
                .counts
                .omissions
                .entry("blocked_summary".into())
                .or_default() += 1;
            *self
                .counts
                .omissions
                .entry("blocked_summary_decisions".into())
                .or_default() += omitted.count;
            if !self
                .truncation_reasons
                .iter()
                .any(|r| r == "blocked_summary_response_bytes")
            {
                self.truncation_reasons
                    .push("blocked_summary_response_bytes".into());
            }
        }
    }
}
