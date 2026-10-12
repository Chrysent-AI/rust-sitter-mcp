//! Disclosure of existing work accounting; neither retention nor a memory estimate.
use super::*;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(rename_all = "snake_case")]
pub enum AnalysisStage {
    #[default]
    ScopeInputs,
    Inventory,
    Overlaps,
    Contexts,
    ScopeTrivia,
    ModuleChain,
    Signals,
    Decisions,
    BoundaryObservations,
    Ownership,
    ExtractionPayoff,
    TestObservations,
    Drafts,
    FinalProjection,
    DecisionGroups,
    Consequences,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(rename_all = "snake_case")]
pub enum AnalysisBudgetStop {
    AggregateDescriptorBytes,
    /// A local collector guard stopped before its rejected work was charged.
    StageLocalGuard,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct AnalysisBudget {
    pub stopped_by: AnalysisBudgetStop,
    pub exhausted_stage: AnalysisStage,
    pub aggregate_limit_bytes: usize,
    pub accounted_bytes: usize,
    /// Additive shares of counts.analysis_descriptor_bytes, not unique data sizes.
    pub stage_bytes: BTreeMap<AnalysisStage, usize>,
}

#[derive(Debug, Clone, Default)]
pub(super) struct Accounting {
    stage: AnalysisStage,
    baseline: usize,
    bytes: BTreeMap<AnalysisStage, usize>,
}
impl Accounting {
    fn settle(&mut self, total: usize) {
        let delta = total.saturating_sub(self.baseline);
        let bytes = self.bytes.entry(self.stage).or_default();
        *bytes = bytes.saturating_add(delta);
        self.baseline = total;
    }
}
impl SuggestSplitEnvelope {
    pub(super) fn analysis_stage(&mut self, stage: AnalysisStage) {
        self.accounting
            .settle(self.counts.analysis_descriptor_bytes);
        self.accounting.stage = stage;
    }

    pub(super) fn disclose_analysis_budget(&mut self, error: &DomainError) {
        if error.code != "analysis_descriptor_bytes" {
            return;
        }
        self.accounting
            .settle(self.counts.analysis_descriptor_bytes);
        self.analysis_budget = Some(AnalysisBudget {
            stopped_by: if self.counts.analysis_descriptor_bytes
                > self.effective_work_limits.analysis_descriptor_bytes
            {
                AnalysisBudgetStop::AggregateDescriptorBytes
            } else {
                AnalysisBudgetStop::StageLocalGuard
            },
            exhausted_stage: self.accounting.stage,
            aggregate_limit_bytes: self.effective_work_limits.analysis_descriptor_bytes,
            accounted_bytes: self.counts.analysis_descriptor_bytes,
            stage_bytes: self.accounting.bytes.clone(),
        });
    }
}

#[cfg(test)]
mod tests;
