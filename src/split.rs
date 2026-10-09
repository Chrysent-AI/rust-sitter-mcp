//! Request-local advisory inventories and partitions; never an execution plan.
mod boundary;
mod consequences;
mod drafts;
pub use consequences::{
    ConsequenceAdviceDecision, ConsequenceDecisionRef, ConsequenceEvidenceRef,
    ConsequenceMembership, ConsequenceSummary, UnmappedConsequences,
};
mod ownership;
pub mod retained;
pub use retained::{DetailEnvelope, DetailRequest, ExportEnvelope, ExportRequest, SplitResponse};
mod signals;
mod test_observations;
pub use boundary::{BoundaryCoverage, BoundaryObservation, BoundaryObservations};
pub use ownership::{
    InspectionCompanion, OwnershipAlternative, OwnershipCandidate, OwnershipRanking,
};
pub use test_observations::{
    TestCandidateCoupling, TestCoverage, TestLimitation, TestObservation, TestObservations,
    TestRoute,
};
#[cfg(test)]
mod tests;

use crate::{
    items::{self, DecisionReason, Item, ParsedFile, SizeInterpretation},
    matching::Lines,
    move_plan::{
        Confidence, DecisionAction, DecisionGroup, DecisionIdRun, decision_groups, id_runs,
    },
    plan::Integrity,
    result::*,
    scope::{self, FileSnapshot, Scope},
    trivia,
};
use rmcp::schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    sync::atomic::AtomicBool,
    time::{Duration, Instant},
};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(deny_unknown_fields)]
pub struct SuggestSplitRequest {
    pub repo_path: String,
    /// Caller-selected translation-unit root, not an inferred Cargo target.
    pub crate_root: String,
    pub source_path: String,
    pub paths: Option<Vec<String>>,
    pub globs: Option<Vec<String>>,
    /// Since v0.5.0, omitted/false excludes byte balancing; true requests a distinct
    /// low-confidence original-order alternative only when structural advice is valid.
    pub include_balanced: Option<bool>,
    #[serde(default)]
    pub retain_snapshot: bool,
    #[serde(default)]
    pub response_mode: retained::ResponseMode,
    #[serde(default)]
    pub context: Context,
    #[serde(default)]
    pub limits: AdviceLimits,
    /// Bounds displayed membership, not an execution selection. Default 500; 1–5000.
    #[serde(default = "default_max_items")]
    pub max_items: usize,
}
/// Shared scan/display limits; an explicit diagnostic count expands advice detail.
#[derive(Debug, Clone, Default, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(transparent)]
pub struct AdviceLimits {
    inner: Limits,
    #[serde(skip)]
    diagnostic_count_explicit: bool,
}
impl<'de> Deserialize<'de> for AdviceLimits {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(deserializer)?;
        let diagnostic_count_explicit = value.get("diagnostic_count").is_some();
        let inner = serde_json::from_value(value).map_err(serde::de::Error::custom)?;
        Ok(Self {
            inner,
            diagnostic_count_explicit,
        })
    }
}
impl std::ops::Deref for AdviceLimits {
    type Target = Limits;
    fn deref(&self) -> &Limits {
        &self.inner
    }
}
impl std::ops::DerefMut for AdviceLimits {
    fn deref_mut(&mut self) -> &mut Limits {
        &mut self.inner
    }
}
impl From<AdviceLimits> for Limits {
    fn from(limits: AdviceLimits) -> Self {
        limits.inner
    }
}
impl AdviceLimits {
    fn expanded(&self) -> bool {
        self.diagnostic_count_explicit
            || self.diagnostic_count != Limits::default().diagnostic_count
    }
}
fn default_max_items() -> usize {
    500
}
/// Associated units still require their header anchor; admission here is advice only.
fn draftable(item: &Item) -> bool {
    item.eligibility == "supported_unit"
        || (item.enclosing_impl.is_some() && item.reasons.is_empty())
}
fn partitioned_impl(item: &Item) -> bool {
    item.kind == "impl_item"
        && item
            .reasons
            .iter()
            .any(|reason| reason == "partition_associated_units_instead")
}
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct AdviceAnchor {
    pub path: String,
    pub span: SourceSlice,
}
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct SourceDescription {
    pub path: String,
    pub bytes: usize,
    pub lines: usize,
    pub context: ContextLines,
    pub module: Option<AdviceModule>,
}
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct AdviceModule {
    pub crate_root: String,
    pub module_segments: Vec<String>,
    pub declarations: Vec<AdviceAnchor>,
    pub filesystem_paths: Vec<String>,
    pub assumptions: Vec<String>,
    pub unresolved: Vec<String>,
}
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct ImplContext {
    pub item_id: String,
    pub written_type: Option<SourceSlice>,
    pub written_trait: Option<SourceSlice>,
}
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct ItemContext {
    pub item_id: String,
    pub context: ContextLines,
}
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct InventoryOverlap {
    pub id: String,
    pub relation: &'static str,
    /// Enclosing impl first, contained member second; ranges follow the same order.
    pub item_ids: [String; 2],
    pub path: String,
    pub ranges: [ByteRange; 2],
    pub draft_groups: Vec<OverlapDraftGroups>,
}
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct OverlapDraftGroups {
    pub draft_id: String,
    pub impl_group_index: usize,
    pub member_group_index: usize,
}
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct ScopeTrivia {
    pub id: String,
    pub span: SourceSlice,
    pub classification: String,
    pub reason: String,
    pub protected: bool,
    pub default_disposition: String,
    /// Adjacency only; these links never assign ownership.
    pub adjacent_item_ids: Vec<String>,
}
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct Signal {
    pub id: String,
    pub kind: String,
    pub basis: String,
    pub item_ids: Vec<String>,
    /// Under response pressure, item_size/name_prefix duplicate declaration displays
    /// may be empty: resolve item_ids in order to same-response inventory spans.
    /// counts.omissions.duplicate_declaration_display_spans counts these copies;
    /// unique occurrence evidence is never shared this way.
    pub evidence: Vec<SourceSlice>,
    /// Present for directed reference candidates and observed test consumers.
    pub from_item_id: Option<String>,
    pub to_item_id: Option<String>,
    pub count: usize,
    pub label: Option<String>,
    pub facts: BTreeMap<String, usize>,
    pub limitations: Vec<String>,
}
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct AdviceDecision {
    pub reason: DecisionReason,
    pub action: DecisionAction,
    pub id: String,
    pub category: String,
    /// Display descriptors, not shortened execution freshness anchors.
    pub anchors: Vec<AdviceAnchor>,
    pub item_ids: Vec<String>,
    pub evidence: Vec<SourceSlice>,
    pub unresolved_consequence: String,
    pub next_action: String,
    pub resolution: String,
    pub supported_choices: Vec<String>,
    pub selected_choice: Option<String>,
    pub blocks_applicability: bool,
    pub chain_diagnostic_ids: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lexical_uncertainty: Option<items::LexicalUncertainty>,
}
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct AdviceDecisionGroup {
    #[serde(flatten)]
    pub summary: DecisionGroup<Vec<DecisionIdRun>>,
    pub unresolved_consequence: String,
    /// Complete route guidance without per-occurrence execution targets.
    pub actions: Vec<DecisionAction>,
}
impl std::ops::Deref for AdviceDecisionGroup {
    type Target = DecisionGroup<Vec<DecisionIdRun>>;
    fn deref(&self) -> &Self::Target {
        &self.summary
    }
}
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct ProposedDestination {
    pub kind: String,
    pub path: String,
    pub parent_path: String,
    pub parent_module: AdviceModule,
    pub existing_declaration: Option<AdviceAnchor>,
}
#[derive(Debug, Clone, Default, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct Sizes {
    pub items: usize,
    pub bytes: usize,
    pub lines: usize,
}
/// Observed advice decisions, not a prediction of the final move plan's size.
#[derive(Debug, Clone, Default, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct ExpectedBlockCounts {
    pub member_call: usize,
    pub macro_context: usize,
    pub external_binding: usize,
    pub conditional_or_derive: usize,
    pub cfg_test_consumer: usize,
    pub other_local: usize,
}
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct ExpectedBlocks {
    pub lower_bound: bool,
    pub counts: ExpectedBlockCounts,
    pub decision_ids: Vec<String>,
    pub note: String,
}
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct AssessmentScope {
    pub assessed: String,
    pub not_assessed: Vec<String>,
    pub note: String,
}
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct Group {
    pub kind: String,
    pub destination: Option<ProposedDestination>,
    pub item_ids: Vec<String>,
    pub rationale: String,
    pub confidence: Confidence,
    pub sizes: Sizes,
    pub overlap_ids: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size_interpretation: Option<SizeInterpretation>,
    pub signal_ids: Vec<String>,
    pub facts: BTreeMap<String, usize>,
    pub expected_to_block: ExpectedBlocks,
    pub consequence_summary: ConsequenceSummary,
    pub test_coupled: bool,
    pub assessment_scope: AssessmentScope,
    pub warnings: Vec<String>,
}
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct Draft {
    pub id: String,
    pub advisory: bool,
    pub source_snapshot_id: String,
    pub groups: Vec<Group>,
    pub rationale: String,
    pub cross_group_signal_ids: Vec<String>,
    pub unresolved_decision_ids: Vec<String>,
}
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct DraftMembership {
    pub id: String,
    pub source_snapshot_id: String,
    pub groups: Vec<GroupMembership>,
    pub unresolved_decision_ids: Vec<String>,
}
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct GroupMembership {
    pub consequence_summary: ConsequenceSummary,
    pub kind: String,
    pub destination_path: Option<String>,
    pub item_ids: Vec<String>,
    pub overlap_ids: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size_interpretation: Option<SizeInterpretation>,
}
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct DraftEligibility {
    pub state: String,
    pub reasons: Vec<String>,
    pub membership_complete: bool,
    pub evidence_complete: bool,
}
#[derive(Debug, Clone, Default, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct AdviceCounts {
    pub discovered_files: usize,
    pub scanned_files: usize,
    pub eligible_files: usize,
    pub inventory_descriptors: usize,
    pub inventory_items: usize,
    pub returned_items: usize,
    pub eligible_items: usize,
    pub reference_candidates: usize,
    pub signals: usize,
    pub decisions: usize,
    pub advice_decisions: usize,
    pub drafts: usize,
    pub ownership_candidates: usize,
    pub boundary_observations: usize,
    pub test_observations: usize,
    pub analysis_descriptor_bytes: usize,
    pub omissions: BTreeMap<String, usize>,
}
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct AdviceWorkLimits {
    pub inventory_descriptors: usize,
    pub reference_candidates: usize,
    pub analysis_descriptor_bytes: usize,
    pub max_items: usize,
    pub query_state_limit_applicable: bool,
    pub reference_coverage: String,
}
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct SuggestSplitEnvelope {
    pub schema_version: u8,
    pub tool: String,
    pub advisory: bool,
    pub root: Option<String>,
    pub snapshot_id: Option<String>,
    pub status: String,
    pub coverage: Coverage,
    pub counts: AdviceCounts,
    pub limits: Limits,
    pub effective_work_limits: AdviceWorkLimits,
    pub truncation_reasons: Vec<String>,
    pub skipped: Skipped,
    pub diagnostics: Vec<Diagnostic>,
    pub diagnostics_omitted: usize,
    pub chain_diagnostics: Vec<items::ChainDiagnostic>,
    pub error: Option<DomainError>,
    pub source: Option<SourceDescription>,
    pub scope_trivia: Vec<ScopeTrivia>,
    pub inventory: Vec<Item>,
    pub overlaps: Vec<InventoryOverlap>,
    pub item_contexts: Vec<ItemContext>,
    pub impl_contexts: Vec<ImplContext>,
    pub signals: Vec<Signal>,
    pub decisions: Vec<AdviceDecision>,
    pub advice_decisions: Vec<ConsequenceAdviceDecision>,
    pub decision_groups: Vec<AdviceDecisionGroup>,
    pub drafts: Vec<Draft>,
    pub ownership_candidates: Vec<OwnershipCandidate>,
    pub boundary_observations: BoundaryObservations,
    pub test_observations: TestObservations,
    pub partition_outcome: String,
    /// Non-executable membership summaries when output fitting withholds full drafts.
    pub draft_summaries: Vec<DraftMembership>,
    pub draft_eligibility: DraftEligibility,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retention: Option<retained::Retention>,
    pub integrity: Integrity,
}
impl SuggestSplitEnvelope {
    pub fn empty(limits: Limits) -> Self {
        Self {
            schema_version: 2,
            tool: "suggest_split".into(),
            advisory: true,
            root: None,
            snapshot_id: None,
            status: "complete".into(),
            coverage: Coverage::default(),
            counts: AdviceCounts::default(),
            limits,
            effective_work_limits: AdviceWorkLimits {
                inventory_descriptors: 100_000,
                reference_candidates: 100_000,
                analysis_descriptor_bytes: 128 * 1024 * 1024,
                max_items: 500,
                query_state_limit_applicable: false,
                reference_coverage: "intra-file written candidates only; admitted-scope module evidence; no generated/build-target reference proof".into(),
            },
            truncation_reasons: Vec::new(),
            skipped: skipped_map(),
            diagnostics: Vec::new(),
            diagnostics_omitted: 0,
            chain_diagnostics: Vec::new(),
            error: None,
            source: None,
            scope_trivia: Vec::new(),
            inventory: Vec::new(),
            overlaps: Vec::new(),
            item_contexts: Vec::new(),
            impl_contexts: Vec::new(),
            signals: Vec::new(),
            decisions: Vec::new(),
            advice_decisions: Vec::new(),
            decision_groups: Vec::new(),
            drafts: Vec::new(),
            ownership_candidates: Vec::new(),
            boundary_observations: BoundaryObservations::default(),
            test_observations: TestObservations::default(),
            partition_outcome: "incomplete_analysis".into(),
            draft_summaries: Vec::new(),
            draft_eligibility: DraftEligibility {
                state: "no_draft".into(),
                reasons: Vec::new(),
                membership_complete: false,
                evidence_complete: false,
            },
            retention: None,
            integrity: Integrity {
                grammar: "tree-sitter-rust@0.24.2".into(),
                syntax: "not_checked".into(),
                semantic: "not_performed".into(),
            },
        }
    }
    fn link_overlaps(&mut self, controls: Controls<'_>) -> Result<(), DomainError> {
        let index: BTreeMap<_, _> = self
            .inventory
            .iter()
            .enumerate()
            .map(|(i, item)| (item.id.clone(), i))
            .collect();
        for member in 0..self.inventory.len() {
            controls.check()?;
            let Some(parent) = self.inventory[member]
                .enclosing_impl_id
                .as_ref()
                .and_then(|id| index.get(id))
                .copied()
            else {
                continue;
            };
            let implementation = &self.inventory[parent];
            let unit = &self.inventory[member];
            let overlap = InventoryOverlap {
                id: format!("overlap/{}", self.overlaps.len()),
                relation: "member_contained_in_impl",
                item_ids: [implementation.id.clone(), unit.id.clone()],
                path: unit.path.clone(),
                ranges: [implementation.span.range.clone(), unit.span.range.clone()],
                draft_groups: Vec::new(),
            };
            self.account(descriptor_bytes(&overlap)?)?;
            for i in [parent, member] {
                self.inventory[i].overlap_ids.push(overlap.id.clone());
                self.inventory[i].size_interpretation = Some(SizeInterpretation::non_additive());
            }
            self.overlaps.push(overlap);
        }
        Ok(())
    }
    pub fn failed(limits: Limits, error: DomainError) -> Self {
        let mut result = Self::empty(limits);
        result.status = "failed".into();
        result.partition_outcome = "failed_analysis".into();
        result.error = Some(error);
        result.draft_eligibility.reasons.push("failed_call".into());
        result
    }
    fn omit(&mut self, key: &str, count: usize) {
        if count != 0 {
            *self.counts.omissions.entry(key.into()).or_default() += count;
        }
    }
    fn shape_decisions(&mut self, limits: &AdviceLimits) {
        let advice_before = self.advice_decisions.len();
        self.advice_decisions.truncate(limits.diagnostic_count);
        self.omit(
            "advice_decisions",
            advice_before - self.advice_decisions.len(),
        );
        if advice_before != self.advice_decisions.len() {
            self.truncation_reasons.push("diagnostic_count".into());
        }
        let before = self.decisions.len();
        let chain_links: usize = self
            .decisions
            .iter()
            .map(|d| d.chain_diagnostic_ids.len())
            .sum();
        if limits.expanded() {
            self.decisions.truncate(limits.diagnostic_count);
        } else {
            // One full exemplar per cause/route/consequence, in original ID order.
            let exemplars: BTreeSet<_> = self
                .decision_groups
                .iter()
                .take(limits.diagnostic_count)
                .map(|g| g.decision_ids[0].first_id.as_str())
                .collect();
            self.decisions.retain(|d| exemplars.contains(d.id.as_str()));
        }
        self.omit("decisions", before - self.decisions.len());
        self.omit(
            "chain_diagnostic_references",
            chain_links
                - self
                    .decisions
                    .iter()
                    .map(|d| d.chain_diagnostic_ids.len())
                    .sum::<usize>(),
        );
        if before != self.decisions.len() {
            self.truncation_reasons.push("diagnostic_count".into());
        }
    }
    fn withhold(&mut self) {
        self.omit("drafts", self.drafts.len());
        self.omit(
            "draft_membership_references",
            self.drafts
                .iter()
                .flat_map(|d| &d.groups)
                .map(|g| g.item_ids.len())
                .sum(),
        );
        self.omit(
            "draft_decision_references",
            self.drafts
                .iter()
                .map(|d| d.unresolved_decision_ids.len())
                .sum(),
        );
        self.drafts.clear();
    }
    fn incomplete(&mut self, reason: &str) {
        self.withhold();
        self.partition_outcome = "incomplete_analysis".into();
        self.boundary_observations.coverage.completed = false;
        self.test_observations.coverage.completed = false;
        for candidate in &mut self.ownership_candidates {
            for companion in &mut candidate.companions {
                companion.classification = "undetermined".into();
                companion.review_obligation = "association_unproved".into();
            }
        }
        self.status = "partial".into();
        self.coverage.scope_exhaustive = false;
        self.draft_eligibility.state = "incomplete".into();
        self.draft_eligibility.evidence_complete = false;
        if !self.truncation_reasons.iter().any(|r| r == reason) {
            self.truncation_reasons.push(reason.into());
            self.draft_eligibility.reasons.push(reason.into());
        }
    }
    fn omit_test_observations(&mut self) {
        self.counts.test_observations = self
            .counts
            .test_observations
            .max(self.test_observations.records.len());
        self.omit("test_observations", self.test_observations.records.len());
        self.omit("test_routes", self.test_observations.routes.len());
        self.omit("test_limitations", self.test_observations.limitations.len());
        self.test_observations = TestObservations::default();
    }
    fn account(&mut self, bytes: usize) -> Result<(), DomainError> {
        self.counts.analysis_descriptor_bytes =
            self.counts.analysis_descriptor_bytes.saturating_add(bytes);
        if self.counts.analysis_descriptor_bytes
            > self.effective_work_limits.analysis_descriptor_bytes
        {
            Err(DomainError::new(
                "analysis_descriptor_bytes",
                "advice descriptor guard reached; narrow scope/source",
            ))
        } else {
            Ok(())
        }
    }
    // Only declaration displays may share inventory spans. Occurrence evidence stays direct.
    // Preflight the whole projection so a mismatch cannot leave a partly shared response.
    fn fit_duplicate_declaration_displays(
        &mut self,
        controls: Controls<'_>,
    ) -> Result<bool, DomainError> {
        controls.check()?;
        if self.wire_bytes() <= self.limits.response_bytes {
            return Ok(true);
        }
        if self.status != "complete"
            || self.error.is_some()
            || !self.coverage.scan_exhausted
            || !self.coverage.eligible_scan_complete
            || !self.coverage.scope_exhaustive
            || !self.draft_eligibility.membership_complete
            || !self.draft_eligibility.evidence_complete
            || self.inventory.len() != self.counts.inventory_items
            || self.counts.returned_items != self.inventory.len()
            || self
                .counts
                .omissions
                .get("inventory_items")
                .copied()
                .unwrap_or(0)
                != 0
        {
            return Ok(false);
        }
        let inventory: BTreeMap<_, _> = self
            .inventory
            .iter()
            .map(|item| (item.id.as_str(), &item.span))
            .collect();
        if inventory.len() != self.inventory.len() {
            return Ok(false);
        }
        let mut indices = Vec::new();
        let mut count = 0;
        for (index, signal) in self.signals.iter().enumerate() {
            controls.check()?;
            if !matches!(signal.kind.as_str(), "item_size" | "name_prefix") {
                continue;
            }
            if signal.item_ids.is_empty() || signal.evidence.len() != signal.item_ids.len() {
                return Ok(false);
            }
            for (id, span) in signal.item_ids.iter().zip(&signal.evidence) {
                controls.check()?;
                // Full descriptor equality includes positions, text bytes and omission flags,
                // not just geometry. Iterating the IDs preserves evidence order exactly.
                if inventory.get(id.as_str()).copied() != Some(span) {
                    return Ok(false);
                }
            }
            count += signal.evidence.len();
            indices.push(index);
        }
        if indices.is_empty() {
            return Ok(false);
        }
        controls.check()?;
        let removed: Vec<_> = indices
            .into_iter()
            .map(|index| (index, std::mem::take(&mut self.signals[index].evidence)))
            .collect();
        let key = "duplicate_declaration_display_spans";
        let previous = self.counts.omissions.get(key).copied();
        self.omit(key, count);
        // Use the duplicated encoder, including escaping, omission metadata and reserve.
        // Inventory is unchanged after the preflight: every removed span is recoverable
        // from this response in item_ids order, without a retained handle.
        let fits = self.wire_bytes() <= self.limits.response_bytes;
        let checked = controls.check();
        if fits && checked.is_ok() {
            return Ok(true);
        }
        // Failed fitting (or cancellation) must leave the existing fail-closed path intact.
        for (index, spans) in removed {
            self.signals[index].evidence = spans;
        }
        if let Some(previous) = previous {
            self.counts.omissions.insert(key.into(), previous);
        } else {
            self.counts.omissions.remove(key);
        }
        checked?;
        Ok(false)
    }
    pub fn wire_bytes(&self) -> usize {
        let value = serde_json::to_value(self).expect("serializable advice");
        serde_json::to_vec(&serde_json::json!({"content":[{"type":"text","text":value.to_string()}],"structuredContent":value,"isError":self.error.is_some()})).expect("wire JSON").len() + 4096
    }
    fn fit(&mut self, controls: Controls<'_>) -> Result<(), DomainError> {
        self.counts.returned_items = self.inventory.len();
        controls.check()?;
        if self.wire_bytes() <= self.limits.response_bytes {
            return Ok(());
        }
        // Text is display only: dropping it preserves complete evidence ranges and membership.
        let mut omitted = 0;
        for item in &mut self.inventory {
            controls.check()?;
            omitted += omit_text(&mut item.span);
            for span in item.attributes.iter_mut().chain(&mut item.trivia) {
                controls.check()?;
                omitted += omit_text(span);
            }
        }
        for record in &mut self.impl_contexts {
            controls.check()?;
            for span in record
                .written_type
                .iter_mut()
                .chain(&mut record.written_trait)
            {
                omitted += omit_text(span);
            }
        }
        for record in &mut self.item_contexts {
            controls.check()?;
            for span in record
                .context
                .before
                .iter_mut()
                .chain(&mut record.context.after)
            {
                controls.check()?;
                omitted += omit_text(span);
            }
        }
        for record in &mut self.scope_trivia {
            controls.check()?;
            omitted += omit_text(&mut record.span);
        }
        for signal in &mut self.signals {
            controls.check()?;
            for span in &mut signal.evidence {
                controls.check()?;
                omitted += omit_text(span);
            }
        }
        for decision in &mut self.decisions {
            controls.check()?;
            for anchor in &mut decision.anchors {
                omitted += omit_text(&mut anchor.span);
            }
            for span in &mut decision.evidence {
                controls.check()?;
                omitted += omit_text(span);
            }
        }
        if let Some(source) = &mut self.source {
            for span in source
                .context
                .before
                .iter_mut()
                .chain(&mut source.context.after)
            {
                omitted += omit_text(span);
            }
            if let Some(module) = &mut source.module {
                omitted += module.omit_text();
            }
        }
        for draft in &mut self.drafts {
            controls.check()?;
            for group in &mut draft.groups {
                if let Some(destination) = &mut group.destination {
                    omitted += destination.parent_module.omit_text();
                    if let Some(anchor) = &mut destination.existing_declaration {
                        omitted += omit_text(&mut anchor.span);
                    }
                }
            }
        }
        for record in &mut self.boundary_observations.records {
            controls.check()?;
            omitted += omit_text(&mut record.anchor.span);
            for anchor in record
                .enclosing_unit
                .iter_mut()
                .chain(&mut record.counterpart)
                .chain(&mut record.route_evidence)
            {
                omitted += omit_text(&mut anchor.span);
            }
        }
        for anchor in &mut self
            .boundary_observations
            .coverage
            .unsupported_context_examples
        {
            omitted += omit_text(&mut anchor.span);
        }
        controls.check()?;
        omitted += self.test_observations.omit_text();
        self.omit("display_text_fields", omitted);
        controls.check()?;
        if self.wire_bytes() <= self.limits.response_bytes {
            return Ok(());
        }
        // Neighboring context is display only too. Record field-level array omissions.
        for record in &mut self.item_contexts {
            controls.check()?;
            record.context.before_omitted += record.context.before.len();
            record.context.after_omitted += record.context.after.len();
            record.context.before.clear();
            record.context.after.clear();
        }
        if self.wire_bytes() <= self.limits.response_bytes {
            return Ok(());
        }
        self.omit("item_contexts", self.item_contexts.len());
        self.item_contexts.clear();
        if self.wire_bytes() <= self.limits.response_bytes {
            return Ok(());
        }
        if self.fit_duplicate_declaration_displays(controls)? {
            return Ok(());
        }
        if !self.drafts.is_empty() {
            self.draft_summaries = self
                .drafts
                .iter()
                .map(|draft| DraftMembership {
                    id: draft.id.clone(),
                    source_snapshot_id: draft.source_snapshot_id.clone(),
                    groups: draft
                        .groups
                        .iter()
                        .map(|group| GroupMembership {
                            consequence_summary: group.consequence_summary.clone(),
                            kind: group.kind.clone(),
                            destination_path: group.destination.as_ref().map(|d| d.path.clone()),
                            item_ids: group.item_ids.clone(),
                            overlap_ids: group.overlap_ids.clone(),
                            size_interpretation: group.size_interpretation.clone(),
                        })
                        .collect(),
                    unresolved_decision_ids: draft.unresolved_decision_ids.clone(),
                })
                .collect();
        }
        self.incomplete("response_bytes");
        self.omit("ownership_candidates", self.ownership_candidates.len());
        self.ownership_candidates.clear();
        self.omit(
            "boundary_observations",
            self.boundary_observations.records.len(),
        );
        self.boundary_observations.records.clear();
        self.omit_test_observations();
        self.omit("signals", self.signals.len());
        self.omit("decisions", self.decisions.len());
        self.omit("advice_decisions", self.advice_decisions.len());
        self.advice_decisions.clear();
        self.omit("chain_diagnostics", self.chain_diagnostics.len());
        self.omit(
            "chain_diagnostic_references",
            self.decisions
                .iter()
                .map(|d| d.chain_diagnostic_ids.len())
                .sum(),
        );
        self.chain_diagnostics.clear();
        self.omit("overlaps", self.overlaps.len());
        self.omit(
            "overlap_draft_group_links",
            self.overlaps.iter().map(|o| o.draft_groups.len()).sum(),
        );
        self.overlaps.clear();
        self.omit("scope_trivia", self.scope_trivia.len());
        self.omit("impl_contexts", self.impl_contexts.len());
        self.impl_contexts.clear();
        self.signals.clear();
        self.decisions.clear();
        self.scope_trivia.clear();
        let links = self.inventory.iter().map(|i| i.signal_ids.len()).sum();
        self.omit("item_signal_references", links);
        for item in &mut self.inventory {
            controls.check()?;
            item.signal_ids.clear();
        }
        self.diagnostics_omitted += self.diagnostics.len();
        self.diagnostics.clear();
        for skipped in self.skipped.values_mut() {
            skipped.examples_omitted += skipped.examples.len() as u64;
            skipped.examples.clear();
        }
        // A logarithmic number of projections rather than quadratic pop/serialize fitting.
        if self.wire_bytes() > self.limits.response_bytes {
            let all = std::mem::take(&mut self.inventory);
            let mut low = 0;
            let mut high = all.len();
            while low < high {
                controls.check()?;
                let mid = low + (high - low).div_ceil(2);
                self.inventory = all[..mid].to_vec();
                self.counts.returned_items = self.inventory.len();
                if self.wire_bytes() <= self.limits.response_bytes {
                    low = mid;
                } else {
                    high = mid - 1;
                }
            }
            self.inventory = all[..low].to_vec();
            self.counts.returned_items = self.inventory.len();
            self.omit("inventory_items", all.len() - low);
            self.draft_eligibility.membership_complete = low == all.len();
        }
        // Omission metadata itself can cross the cap at an exact boundary.
        while self.wire_bytes() > self.limits.response_bytes && !self.inventory.is_empty() {
            controls.check()?;
            self.inventory.pop();
            self.counts.returned_items = self.inventory.len();
            self.omit("inventory_items", 1);
            self.draft_eligibility.membership_complete = false;
        }
        if self.wire_bytes() > self.limits.response_bytes && self.source.take().is_some() {
            self.omit("source", 1);
        }
        // Complete membership outlives display inventory and source descriptions.
        // Discard it only at the final tier, with exact reference omissions.
        if self.wire_bytes() > self.limits.response_bytes {
            self.omit("draft_summaries", self.draft_summaries.len());
            self.omit(
                "draft_summary_membership_references",
                self.draft_summaries
                    .iter()
                    .flat_map(|d| &d.groups)
                    .map(|g| g.item_ids.len())
                    .sum(),
            );
            self.omit(
                "draft_summary_decision_references",
                self.draft_summaries
                    .iter()
                    .map(|d| d.unresolved_decision_ids.len())
                    .sum(),
            );
            self.draft_summaries.clear();
        }
        if self.wire_bytes() > self.limits.response_bytes {
            self.omit("decision_groups", self.decision_groups.len());
            self.omit(
                "decision_group_references",
                self.decision_groups.iter().map(|g| g.count).sum(),
            );
            self.decision_groups.clear();
        }
        Ok(())
    }
}
fn omit_text(span: &mut SourceSlice) -> usize {
    let omitted = usize::from(span.text.take().is_some());
    span.text_omitted = span.text_bytes != 0;
    omitted
}
impl AdviceModule {
    fn omit_text(&mut self) -> usize {
        self.declarations
            .iter_mut()
            .map(|a| omit_text(&mut a.span))
            .sum()
    }
}
#[derive(Clone, Copy)]
struct Controls<'a> {
    deadline: Instant,
    cancelled: &'a AtomicBool,
}
impl Controls<'_> {
    fn check(self) -> Result<(), DomainError> {
        items::check(self.deadline, self.cancelled)
    }
}
/// Stream accounting without allocating another serialized analysis corpus.
fn descriptor_bytes(value: &impl Serialize) -> Result<usize, DomainError> {
    struct Counter(usize);
    impl std::io::Write for Counter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0 = self.0.saturating_add(bytes.len());
            if self.0 > 128 * 1024 * 1024 {
                return Err(std::io::ErrorKind::OutOfMemory.into());
            }
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut counter = Counter(0);
    serde_json::to_writer(&mut counter, value).map_err(|_| {
        DomainError::new(
            "analysis_descriptor_bytes",
            "serialized advice descriptor guard reached",
        )
    })?;
    Ok(counter.0)
}
fn field_error(code: &str, message: &str, field: &str) -> DomainError {
    let mut error = DomainError::new(code, message);
    error.field = Some(field.into());
    error
}
fn module_description(
    evidence: &items::ModuleEvidence,
    files: &BTreeMap<String, FileSnapshot>,
    text_bytes: usize,
) -> AdviceModule {
    AdviceModule {
        crate_root: evidence.crate_root.clone(),
        module_segments: evidence.module_segments.clone(),
        declarations: evidence
            .declaration_anchors
            .iter()
            .map(|a| AdviceAnchor {
                path: a.path.clone(),
                span: Lines::new(&files[&a.path].source).slice(
                    a.range.start_byte,
                    a.range.end_byte,
                    text_bytes,
                ),
            })
            .collect(),
        filesystem_paths: evidence.filesystem_paths.clone(),
        assumptions: evidence.assumptions.clone(),
        unresolved: evidence.unresolved.clone(),
    }
}

pub fn run_retained(
    launch: &Path,
    request: SuggestSplitRequest,
    cancelled: &AtomicBool,
    store: &retained::Store,
) -> SplitResponse {
    let controls = Controls {
        deadline: Instant::now()
            + Duration::from_millis(request.limits.time_budget_ms.min(300_000)),
        cancelled,
    };
    let (result, evidence) = analyze_with_recheck(launch, &request, cancelled, || {});
    retained::present(result, evidence, &request, store, controls)
}
#[cfg(test)]
fn run(
    launch: &Path,
    request: SuggestSplitRequest,
    cancelled: &AtomicBool,
) -> SuggestSplitEnvelope {
    run_with_recheck(launch, request, cancelled, || {})
}
#[cfg(test)]
fn run_with_recheck(
    launch: &Path,
    request: SuggestSplitRequest,
    cancelled: &AtomicBool,
    before_recheck: impl FnOnce(),
) -> SuggestSplitEnvelope {
    let (mut result, _) = analyze_with_recheck(launch, &request, cancelled, before_recheck);
    result.shape_decisions(&request.limits);
    let _ = result.fit(Controls {
        deadline: Instant::now() + Duration::from_secs(5),
        cancelled: &AtomicBool::new(false),
    });
    result
}
fn analyze_with_recheck(
    launch: &Path,
    request: &SuggestSplitRequest,
    cancelled: &AtomicBool,
    before_recheck: impl FnOnce(),
) -> (SuggestSplitEnvelope, Option<retained::Evidence>) {
    let started = Instant::now();
    let controls = Controls {
        deadline: started + Duration::from_millis(request.limits.time_budget_ms.min(300_000)),
        cancelled,
    };
    let mut result = SuggestSplitEnvelope::empty(request.limits.clone().into());
    result.effective_work_limits.max_items = request.max_items;
    let mut evidence = None;
    let outcome = build(
        launch,
        request,
        controls,
        before_recheck,
        &mut result,
        &mut evidence,
    )
    .and_then(|()| drafts::finalize(&mut result, controls))
    .and_then(|()| controls.check());
    if let Err(error) = outcome {
        // A stopped collector may still have provisional IDs. Never publish partial linked
        // evidence as finalized; keep its observed counts and explicit omission accounting.
        result.counts.signals = result.counts.signals.max(result.signals.len());
        result.counts.decisions = result.counts.decisions.max(result.decisions.len());
        result.counts.drafts = result.counts.drafts.max(result.drafts.len());
        result.omit("signals", result.signals.len());
        result.omit("decisions", result.decisions.len());
        result.omit("decision_groups", result.decision_groups.len());
        result.omit(
            "decision_group_references",
            result.decision_groups.iter().map(|g| g.count).sum(),
        );
        result.decision_groups.clear();
        result.omit(
            "item_signal_references",
            result.inventory.iter().map(|i| i.signal_ids.len()).sum(),
        );
        result.signals.clear();
        result.omit(
            "chain_diagnostic_references",
            result
                .decisions
                .iter()
                .map(|d| d.chain_diagnostic_ids.len())
                .sum(),
        );
        result.decisions.clear();
        result.omit("advice_decisions", result.advice_decisions.len());
        result.advice_decisions.clear();
        result.omit("ownership_candidates", result.ownership_candidates.len());
        result.ownership_candidates.clear();
        result.omit(
            "boundary_observations",
            result.boundary_observations.records.len(),
        );
        result.boundary_observations.records.clear();
        result.boundary_observations.coverage.completed = false;
        result.omit_test_observations();
        if error.code != "STALE_SELECTION" {
            result.omit("chain_diagnostics", result.chain_diagnostics.len());
            result.chain_diagnostics.clear();
        }
        for item in &mut result.inventory {
            item.signal_ids.clear();
        }
        if matches!(
            error.code.as_str(),
            "planning_deadline"
                | "scan_incomplete"
                | "inventory_work_limit"
                | "reference_work_limit"
                | "analysis_descriptor_bytes"
                | "SOURCE_CHANGED"
        ) {
            result.incomplete(&error.code);
        } else {
            result.withhold();
            result.status = "failed".into();
            result.partition_outcome = "failed_analysis".into();
            result.coverage.scope_exhaustive = false;
            result.draft_eligibility.state = "no_draft".into();
            result.draft_eligibility.evidence_complete = false;
            result.draft_eligibility.reasons.push("failed_call".into());
            result.error = Some(error);
        }
        // Cancellation/deadline must not prevent bounding the terminal response.
        let terminal_flag = AtomicBool::new(false);
        let terminal = Controls {
            deadline: Instant::now() + Duration::from_secs(5),
            cancelled: &terminal_flag,
        };
        if result.fit(terminal).is_err() {
            let error = result.error.take();
            let counts = std::mem::take(&mut result.counts);
            let root = result.root.take();
            let snapshot = result.snapshot_id.take();
            let coverage = std::mem::take(&mut result.coverage);
            result = SuggestSplitEnvelope::empty(request.limits.clone().into());
            result.root = root;
            result.snapshot_id = snapshot;
            result.coverage = coverage;
            result.counts = counts;
            result.error = error;
            result.incomplete("response_fit_stopped");
        }
    }
    result.counts.returned_items = result.inventory.len();
    tracing::info!(tool="suggest_split", elapsed_ms=started.elapsed().as_millis(), status=%result.status, items=result.counts.inventory_items, candidates=result.counts.reference_candidates, drafts=result.drafts.len(), error=?result.error.as_ref().map(|e| &e.code), "split advice finished");
    (result, evidence)
}
fn build(
    launch: &Path,
    request: &SuggestSplitRequest,
    controls: Controls<'_>,
    before_recheck: impl FnOnce(),
    result: &mut SuggestSplitEnvelope,
    evidence: &mut Option<retained::Evidence>,
) -> Result<(), DomainError> {
    let mut scan_limits: Limits = request.limits.clone().into();
    // The scan's generic diagnostic limit remains bounded independently of advice detail.
    scan_limits.diagnostic_count = scan_limits.diagnostic_count.min(256);
    scan_limits.validate()?;
    if request.limits.diagnostic_count > 100_000 {
        return Err(field_error(
            "INVALID_PARAMS",
            "advice diagnostic_count is 0–100000",
            "limits.diagnostic_count",
        ));
    }
    if !(1..=5000).contains(&request.max_items)
        || request.context.before_lines > 20
        || request.context.after_lines > 20
    {
        return Err(field_error(
            "INVALID_PARAMS",
            "max_items is 1–5000; context is 0–20 lines",
            "max_items",
        ));
    }
    for (field, path) in [
        ("source_path", &request.source_path),
        ("crate_root", &request.crate_root),
    ] {
        scope::normalized_path(path).map_err(|_| {
            field_error(
                "INVALID_PARAMS",
                "require a normalized root-relative .rs path",
                field,
            )
        })?;
    }
    controls.check()?;
    let root = scope::resolve(&request.repo_path, launch)?;
    result.root = Some(root.to_str().expect("UTF-8 root").into());
    let scan_request = SearchRequest {
        repo_path: request.repo_path.clone(),
        query: String::new(),
        paths: request.paths.clone(),
        globs: request.globs.clone(),
        context: request.context.clone(),
        limits: scan_limits,
        page_size: 100,
        cursor: None,
    };
    let scope = Scope::new(root, &scan_request)?;
    let mut scan = SearchEnvelope::empty(scan_request.limits.clone());
    let mut inputs = scope::ScopeInputManifest::default();
    let (files, snapshot) = scope::discover_observed(
        &scope,
        &mut scan,
        controls.deadline,
        controls.cancelled,
        request.retain_snapshot.then_some(&mut inputs),
    )?;
    let input_digest = if request.retain_snapshot {
        snapshot
            .as_ref()
            .map(|s| inputs.digest(&scope, s, (controls.deadline, controls.cancelled)))
            .transpose()?
    } else {
        None
    };
    result.account(inputs.accounted_allocation())?;
    result.coverage = scan.coverage;
    result.skipped = scan.skipped;
    result.truncation_reasons = scan.truncation_reasons;
    result.counts.discovered_files = scan.counts.discovered_files;
    result.counts.scanned_files = scan.counts.scanned_files;
    result.counts.eligible_files = scan.counts.eligible_files;
    result.snapshot_id = snapshot.clone();
    let files: BTreeMap<_, _> = files.into_iter().map(|f| (f.path.clone(), f)).collect();
    let Some(source) = files.get(&request.source_path) else {
        if !result.coverage.scope_exhaustive {
            result.incomplete("scan_incomplete");
            return Ok(());
        }
        return Err(field_error(
            "STALE_SELECTION",
            "source_path must be an admitted existing regular Rust file",
            "source_path",
        ));
    };
    let mut source_data = items::parse(
        source,
        request.limits.text_bytes,
        controls.deadline,
        controls.cancelled,
        &mut result.counts.inventory_descriptors,
    )?;
    source_data.associated_items = items::associated::inventory(
        source,
        &source_data.tree,
        &source_data.trivia,
        request.limits.text_bytes,
        (controls.deadline, controls.cancelled),
        &mut result.counts.inventory_descriptors,
    )?;
    result.account(descriptor_bytes(&(
        &source_data.items,
        &source_data.associated_items,
        &source_data.trivia,
    ))?)?;
    // Keep overlapping enclosing impls in the retain set while partitioning their members.
    for item in &mut source_data.items {
        if item.kind == "impl_item"
            && source_data.associated_items.iter().any(|m| {
                m.enclosing_impl
                    .as_ref()
                    .is_some_and(|p| p.range == item.span.range)
            })
        {
            item.eligibility = "context_sensitive".into();
            item.reasons
                .push("partition_associated_units_instead".into());
        }
    }
    let mut advice_units: Vec<_> = source_data
        .items
        .iter()
        .chain(&source_data.associated_items)
        .cloned()
        .collect();
    advice_units.sort_by_key(|i| (i.span.range.start_byte, i.span.range.end_byte));
    let clean = trivia::move_clean(&source_data.tree, controls.deadline, controls.cancelled)?;
    result.integrity.syntax = if clean {
        "input_checked"
    } else {
        "input_recovered"
    }
    .into();
    result.counts.inventory_items = advice_units.len();
    result.counts.eligible_items = advice_units.iter().filter(|i| draftable(i)).count();
    let canonical_requested =
        request.retain_snapshot || request.response_mode == retained::ResponseMode::Compact;
    result.inventory = advice_units
        .iter()
        .take(if canonical_requested {
            usize::MAX
        } else {
            request.max_items
        })
        .cloned()
        .collect();
    result.draft_eligibility.membership_complete = result.inventory.len() == advice_units.len();
    result.link_overlaps(controls)?;
    let lines = Lines::new(&source.source);
    let physical_lines = source.source.bytes().filter(|b| *b == b'\n').count()
        + usize::from(!source.source.is_empty() && !source.source.ends_with('\n'));
    // Context surrounds the entire requested source: there are no invented neighboring lines.
    result.source = Some(SourceDescription {
        path: source.path.clone(),
        bytes: source.source.len(),
        lines: physical_lines,
        context: ContextLines {
            before: Vec::new(),
            after: Vec::new(),
            before_clipped: request.context.before_lines,
            after_clipped: request.context.after_lines,
            before_omitted: 0,
            after_omitted: 0,
        },
        module: None,
    });
    for item in &result.inventory {
        controls.check()?;
        if item.kind == "impl_item" {
            let node = source_data
                .tree
                .root_node()
                .named_descendant_for_byte_range(
                    item.span.range.start_byte,
                    item.span.range.end_byte,
                )
                .expect("impl unit");
            result.impl_contexts.push(ImplContext {
                item_id: item.id.clone(),
                written_type: node
                    .child_by_field_name("type")
                    .map(|n| lines.slice(n.start_byte(), n.end_byte(), request.limits.text_bytes)),
                written_trait: node
                    .child_by_field_name("trait")
                    .map(|n| lines.slice(n.start_byte(), n.end_byte(), request.limits.text_bytes)),
            });
        }
        result.item_contexts.push(ItemContext {
            item_id: item.id.clone(),
            context: lines.context(
                item.span.range.start_byte,
                item.span.range.end_byte,
                &request.context,
                request.limits.text_bytes,
            ),
        });
    }
    result.account(descriptor_bytes(&(
        &result.item_contexts,
        &result.impl_contexts,
    ))?)?;
    signals::scope_trivia(&source_data, &lines, controls, result)?;
    if !result.draft_eligibility.membership_complete {
        result.omit(
            "inventory_items",
            advice_units.len() - result.inventory.len(),
        );
        result.incomplete("max_items");
        return Ok(());
    }
    if !result.coverage.scope_exhaustive || snapshot.is_none() {
        result.incomplete("scan_incomplete");
        return Ok(());
    }
    if !files.contains_key(&request.crate_root) {
        let mut diagnostic = items::ChainDiagnostic::boundary(
            &request.crate_root,
            &request.source_path,
            items::ChainRole::Source,
            items::ChainReason::ChainFileUnadmitted,
        );
        diagnostic.id = "chain/0".into();
        diagnostic.candidate_paths.push(request.crate_root.clone());
        result.account(descriptor_bytes(&diagnostic)?)?;
        result.chain_diagnostics.push(diagnostic);
        return Err(field_error(
            "STALE_SELECTION",
            "crate_root must be an admitted existing Rust file",
            "crate_root",
        ));
    }
    let mut parsed = BTreeMap::new();
    parsed.insert(source.path.clone(), source_data);
    for (path, file) in &files {
        controls.check()?;
        if path == &source.path {
            continue;
        }
        let data = items::parse(
            file,
            0,
            controls.deadline,
            controls.cancelled,
            &mut result.counts.inventory_descriptors,
        )?;
        result.account(descriptor_bytes(&(&data.items, &data.trivia))?)?;
        parsed.insert(path.clone(), data);
    }
    let analysis = items::modules(
        &scope,
        &request.crate_root,
        &files,
        &parsed,
        (controls.deadline, controls.cancelled),
        None,
        &mut result.counts.analysis_descriptor_bytes,
    )?;
    let contexts = &analysis.contexts;
    result.account(descriptor_bytes(&contexts)?)?;
    if let Some(evidence) = contexts.get(&source.path) {
        result.source.as_mut().expect("source").module = Some(module_description(
            evidence,
            &files,
            request.limits.text_bytes,
        ));
    }
    let glob_routes = items::GlobRoutes::new(
        &files,
        &parsed,
        &request.crate_root,
        (controls.deadline, controls.cancelled),
    )?;
    signals::collect(
        source,
        &parsed[&source.path],
        contexts.get(&source.path),
        &glob_routes,
        &lines,
        controls,
        result,
    )?;
    signals::risks(source, &parsed[&source.path], &lines, controls, result)?;
    boundary::collect(
        request,
        &files,
        &parsed,
        contexts,
        &glob_routes,
        controls,
        result,
    )?;
    ownership::build(result, controls)?;
    boundary::link(result, controls)?;
    test_observations::collect(
        request,
        &scope,
        &files,
        &parsed,
        contexts,
        &glob_routes,
        controls,
        result,
    )?;
    result.draft_eligibility.evidence_complete = true;
    if !clean {
        result.partition_outcome = "unsupported_input".into();
        result
            .draft_eligibility
            .reasons
            .push("input_recovered".into());
    } else if result.counts.eligible_items < 2 {
        result.draft_eligibility.reasons.push(
            if result.inventory.is_empty() {
                "empty_inventory"
            } else {
                "fewer_than_two_eligible_items"
            }
            .into(),
        );
    } else {
        drafts::build(
            &scope, request, &files, &parsed, &analysis, controls, result,
        )?;
    }
    result.counts.signals = result.signals.len();
    result.counts.decisions = result.decisions.len();
    result.counts.drafts = result.drafts.len();
    // Verify observed bytes/modes and prospective admission again, including no-draft calls.
    controls.check()?;
    before_recheck();
    let mut final_scan = SearchEnvelope::empty(scan_request.limits.clone());
    let mut final_inputs = scope::ScopeInputManifest::default();
    let (_, latest) = scope::discover_observed(
        &scope,
        &mut final_scan,
        controls.deadline,
        controls.cancelled,
        request.retain_snapshot.then_some(&mut final_inputs),
    )?;
    if let (Some(expected), Some(latest)) = (&input_digest, &latest)
        && final_inputs.digest(&scope, latest, (controls.deadline, controls.cancelled))?
            != *expected
    {
        return Err(DomainError::new(
            "SOURCE_CHANGED",
            "effective ignore inputs changed during analysis; obtain fresh advice",
        ));
    }
    if !final_scan.coverage.scope_exhaustive || latest != snapshot {
        return Err(DomainError::new(
            "SOURCE_CHANGED",
            "source corpus changed or final scan incomplete; obtain fresh advice",
        ));
    }
    test_observations::recheck(&scope, result, controls)?;
    for draft in &result.drafts {
        controls.check()?;
        for group in &draft.groups {
            controls.check()?;
            if let Some(destination) = &group.destination {
                scope.admit_new_path(&destination.path, controls.deadline, controls.cancelled)?;
                if items::present(
                    &scope,
                    &format!("{}/mod.rs", destination.path.trim_end_matches(".rs")),
                )? {
                    return Err(DomainError::new(
                        "SOURCE_CHANGED",
                        "competing sibling layout appeared; obtain fresh advice",
                    ));
                }
            }
        }
    }
    result.account(descriptor_bytes(&(
        &result.inventory,
        &result.signals,
        &result.decisions,
        &result.drafts,
        &result.scope_trivia,
        &result.item_contexts,
        &result.impl_contexts,
        &result.ownership_candidates,
        &result.boundary_observations,
        &result.test_observations,
        &result.source,
    ))?)?;
    if let Some(scope_input_digest) = input_digest {
        let canonical = serde_json::to_value(&*result).expect("canonical evidence JSON");
        let mut wanted = BTreeSet::from([request.source_path.clone(), request.crate_root.clone()]);
        retained::referenced_paths(&canonical, &mut wanted);
        let mut manifest_files = Vec::with_capacity(files.len());
        for file in files.values() {
            controls.check()?;
            let content_digest = scope::hash_serialized(
                &scope.root,
                &file.source,
                (controls.deadline, controls.cancelled),
            )?;
            manifest_files.push(serde_json::json!({"path":file.path,"mode":file.mode,"bytes":file.source.len(),"content_digest":content_digest}));
        }
        let source_manifest = serde_json::json!({"snapshot_id":snapshot,"files":manifest_files});
        let buffers = files
            .into_iter()
            .filter(|(p, _)| wanted.contains(p))
            .map(|(p, f)| (p, f.source))
            .collect();
        *evidence = Some(retained::Evidence {
            buffers,
            inputs,
            scope_input_digest,
            normalized_scope: serde_json::json!({"paths":scope.paths,"globs":scope.globs}),
            source_manifest,
        });
    }
    Ok(())
}
