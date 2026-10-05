//! Request-local advisory inventories and partitions; never an execution plan.
mod drafts;
mod signals;
#[cfg(test)]
mod tests;

use crate::{
    items::{self, DecisionReason, Item, ParsedFile},
    matching::Lines,
    move_plan::{Confidence, DecisionAction, DecisionGroup, decision_groups},
    plan::Integrity,
    result::*,
    scope::{self, FileSnapshot, Scope},
    trivia,
};
use rmcp::schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
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
    #[serde(default)]
    pub context: Context,
    #[serde(default)]
    pub limits: Limits,
    /// Bounds displayed membership, not an execution selection. Default 500; 1–5000.
    #[serde(default = "default_max_items")]
    pub max_items: usize,
}
fn default_max_items() -> usize {
    500
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
    pub evidence: Vec<SourceSlice>,
    /// Present only for directed reference candidates.
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
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct Group {
    pub kind: String,
    pub destination: Option<ProposedDestination>,
    pub item_ids: Vec<String>,
    pub rationale: String,
    pub confidence: Confidence,
    pub sizes: Sizes,
    pub signal_ids: Vec<String>,
    pub facts: BTreeMap<String, usize>,
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
    pub drafts: usize,
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
    pub item_contexts: Vec<ItemContext>,
    pub impl_contexts: Vec<ImplContext>,
    pub signals: Vec<Signal>,
    pub decisions: Vec<AdviceDecision>,
    pub decision_groups: Vec<DecisionGroup>,
    pub drafts: Vec<Draft>,
    pub draft_eligibility: DraftEligibility,
    pub integrity: Integrity,
}
impl SuggestSplitEnvelope {
    pub fn empty(limits: Limits) -> Self {
        Self {
            schema_version: 1,
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
            item_contexts: Vec::new(),
            impl_contexts: Vec::new(),
            signals: Vec::new(),
            decisions: Vec::new(),
            decision_groups: Vec::new(),
            drafts: Vec::new(),
            draft_eligibility: DraftEligibility {
                state: "no_draft".into(),
                reasons: Vec::new(),
                membership_complete: false,
                evidence_complete: false,
            },
            integrity: Integrity {
                grammar: "tree-sitter-rust@0.24.2".into(),
                syntax: "not_checked".into(),
                semantic: "not_performed".into(),
            },
        }
    }
    pub fn failed(limits: Limits, error: DomainError) -> Self {
        let mut result = Self::empty(limits);
        result.status = "failed".into();
        result.error = Some(error);
        result.draft_eligibility.reasons.push("failed_call".into());
        result
    }
    fn omit(&mut self, key: &str, count: usize) {
        if count != 0 {
            *self.counts.omissions.entry(key.into()).or_default() += count;
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
        self.status = "partial".into();
        self.coverage.scope_exhaustive = false;
        self.draft_eligibility.state = "incomplete".into();
        self.draft_eligibility.evidence_complete = false;
        if !self.truncation_reasons.iter().any(|r| r == reason) {
            self.truncation_reasons.push(reason.into());
            self.draft_eligibility.reasons.push(reason.into());
        }
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
        self.incomplete("response_bytes");
        self.omit("signals", self.signals.len());
        self.omit("decisions", self.decisions.len());
        self.omit("decision_groups", self.decision_groups.len());
        self.omit(
            "decision_group_references",
            self.decision_groups
                .iter()
                .map(|g| g.decision_ids.len())
                .sum(),
        );
        self.decision_groups.clear();
        self.omit("chain_diagnostics", self.chain_diagnostics.len());
        self.omit(
            "chain_diagnostic_references",
            self.decisions
                .iter()
                .map(|d| d.chain_diagnostic_ids.len())
                .sum(),
        );
        self.chain_diagnostics.clear();
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
        if self.wire_bytes() > self.limits.response_bytes {
            self.source = None;
            self.omit("source", 1);
            self.root = None;
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

pub fn run(
    launch: &Path,
    request: SuggestSplitRequest,
    cancelled: &AtomicBool,
) -> SuggestSplitEnvelope {
    run_with_recheck(launch, request, cancelled, || {})
}
fn run_with_recheck(
    launch: &Path,
    request: SuggestSplitRequest,
    cancelled: &AtomicBool,
    before_recheck: impl FnOnce(),
) -> SuggestSplitEnvelope {
    let started = Instant::now();
    let controls = Controls {
        deadline: started + Duration::from_millis(request.limits.time_budget_ms.min(300_000)),
        cancelled,
    };
    let mut result = SuggestSplitEnvelope::empty(request.limits.clone());
    result.effective_work_limits.max_items = request.max_items;
    let outcome = build(launch, &request, controls, before_recheck, &mut result)
        .and_then(|()| drafts::finalize(&mut result, controls))
        .and_then(|()| result.fit(controls))
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
            result
                .decision_groups
                .iter()
                .map(|g| g.decision_ids.len())
                .sum(),
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
            result = SuggestSplitEnvelope::empty(request.limits.clone());
            result.counts = counts;
            result.error = error;
            result.incomplete("response_fit_stopped");
        }
    }
    result.counts.returned_items = result.inventory.len();
    tracing::info!(tool="suggest_split", elapsed_ms=started.elapsed().as_millis(), status=%result.status, items=result.counts.inventory_items, candidates=result.counts.reference_candidates, drafts=result.drafts.len(), error=?result.error.as_ref().map(|e| &e.code), "split advice finished");
    result
}
fn build(
    launch: &Path,
    request: &SuggestSplitRequest,
    controls: Controls<'_>,
    before_recheck: impl FnOnce(),
    result: &mut SuggestSplitEnvelope,
) -> Result<(), DomainError> {
    request.limits.validate()?;
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
        limits: request.limits.clone(),
        page_size: 100,
        cursor: None,
    };
    let scope = Scope::new(root, &scan_request)?;
    let mut scan = SearchEnvelope::empty(request.limits.clone());
    let (files, snapshot) =
        scope::discover(&scope, &mut scan, controls.deadline, controls.cancelled)?;
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
    let source_data = items::parse(
        source,
        request.limits.text_bytes,
        controls.deadline,
        controls.cancelled,
        &mut result.counts.inventory_descriptors,
    )?;
    result.account(descriptor_bytes(&(
        &source_data.items,
        &source_data.trivia,
    ))?)?;
    let clean = trivia::move_clean(&source_data.tree, controls.deadline, controls.cancelled)?;
    result.integrity.syntax = if clean {
        "input_checked"
    } else {
        "input_recovered"
    }
    .into();
    result.counts.inventory_items = source_data.items.len();
    result.counts.eligible_items = source_data
        .items
        .iter()
        .filter(|i| i.eligibility == "supported_unit")
        .count();
    result.inventory = source_data
        .items
        .iter()
        .take(request.max_items)
        .cloned()
        .collect();
    result.draft_eligibility.membership_complete =
        result.inventory.len() == source_data.items.len();
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
            source_data.items.len() - result.inventory.len(),
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
        controls.deadline,
        controls.cancelled,
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
    signals::collect(
        source,
        &parsed[&source.path],
        contexts.get(&source.path),
        &lines,
        controls,
        result,
    )?;
    signals::risks(source, &parsed[&source.path], &lines, controls, result)?;
    result.draft_eligibility.evidence_complete = true;
    if !clean {
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
    let mut final_scan = SearchEnvelope::empty(request.limits.clone());
    let (_, latest) = scope::discover(
        &scope,
        &mut final_scan,
        controls.deadline,
        controls.cancelled,
    )?;
    if !final_scan.coverage.scope_exhaustive || latest != snapshot {
        return Err(DomainError::new(
            "SOURCE_CHANGED",
            "source corpus changed or final scan incomplete; obtain fresh advice",
        ));
    }
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
        &result.source,
    ))?)?;
    Ok(())
}
