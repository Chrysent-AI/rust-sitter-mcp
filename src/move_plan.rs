//! One simultaneous, read-only relocation plan with itemized written-binding repairs.
mod actions;
use crate::{
    edit::{self, Edit},
    items::{self, DecisionReason, Item, ModuleEvidence, ParsedFile},
    matching::Lines,
    patch,
    plan::{BaseFile, Blocker, Integrity, SourceAnchor},
    result::*,
    rewrites::{self, Repair},
    scope::{self, FileSnapshot, Scope},
    trivia::{self, MoveOrigin},
};
pub(crate) use actions::decision_groups;
use rmcp::schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    sync::atomic::AtomicBool,
    time::{Duration, Instant},
};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[schemars(crate = "rmcp::schemars")]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Destination {
    Existing {
        path: String,
        before_item: Option<SourceAnchor>,
    },
    NewSibling {
        path: String,
        parent_path: String,
    },
}
impl Destination {
    pub(crate) fn path(&self) -> &str {
        match self {
            Self::Existing { path, .. } | Self::NewSibling { path, .. } => path,
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(deny_unknown_fields)]
pub struct Move {
    pub item: SourceAnchor,
    pub destination: Destination,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(rename_all = "snake_case")]
pub enum MoveDisposition {
    KeepInPlace,
    CarryWithItem,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(deny_unknown_fields)]
pub struct MoveTriviaOverride {
    pub trivia: SourceAnchor,
    pub disposition: MoveDisposition,
    pub target_item: Option<SourceAnchor>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[schemars(crate = "rmcp::schemars")]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum RewriteTarget {
    Source {
        anchor: SourceAnchor,
    },
    Synthesis {
        path: String,
        slot: String,
        items: Vec<SourceAnchor>,
        boundary_role: Option<String>,
        parent_path: Option<String>,
        binding: Option<String>,
    },
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(rename_all = "snake_case")]
pub enum RewriteAction {
    AcceptDefault,
    Retain,
    Replace,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(deny_unknown_fields)]
pub struct RewriteOverride {
    pub target: RewriteTarget,
    pub action: RewriteAction,
    pub replacement_text: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(deny_unknown_fields)]
pub struct DraftProvenance {
    pub draft_id: String,
    pub source_snapshot_id: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(deny_unknown_fields)]
pub struct MoveRequest {
    pub repo_path: String,
    pub crate_root: String,
    pub moves: Vec<Move>,
    pub paths: Option<Vec<String>>,
    pub globs: Option<Vec<String>>,
    #[serde(default)]
    pub context: Context,
    #[serde(default)]
    pub limits: Limits,
    #[serde(default = "default_max_moves")]
    pub max_moves: usize,
    pub trivia_overrides: Option<Vec<MoveTriviaOverride>>,
    pub rewrite_overrides: Option<Vec<RewriteOverride>>,
    pub draft_provenance: Option<DraftProvenance>,
}
fn default_max_moves() -> usize {
    500
}
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct MoveRecord {
    pub id: String,
    pub item: Item,
    pub destination: Destination,
    pub old_module: Option<ModuleEvidence>,
    pub new_module: Option<ModuleEvidence>,
    pub carried_spans: Vec<CarriedSpan>,
    pub output_range: Option<ByteRange>,
    pub origin_ids: Vec<String>,
    pub rewrite_ids: Vec<String>,
    pub decision_ids: Vec<String>,
}
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct CarriedSpan {
    pub path: String,
    pub span: SourceSlice,
}
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct MoveTriviaDecision {
    pub id: String,
    pub path: String,
    pub span: SourceSlice,
    pub classification: String,
    pub reason: String,
    pub owner: Option<ByteRange>,
    pub preservable: bool,
    pub default_disposition: String,
    pub selected_disposition: String,
    pub suggested_dispositions: Vec<String>,
    pub item_ids: Vec<String>,
    pub target_item_id: Option<String>,
    pub origin_ids: Vec<String>,
}
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct Decision {
    pub reason: DecisionReason,
    pub action: DecisionAction,
    pub id: String,
    pub category: String,
    pub anchors: Vec<SourceAnchor>,
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(rename_all = "snake_case")]
pub enum DecisionPurpose {
    ResolveDecision,
    ReviewDefault,
    SubmitForAnalysis,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(rename_all = "snake_case")]
pub enum DecisionRoute {
    RequestField,
    SelectionChangeRequired,
    UnsupportedInEngine,
}
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(tag = "route", rename_all = "snake_case")]
pub enum DecisionAction {
    RequestField {
        tool: String,
        field: String,
        target: Option<Box<RewriteTarget>>,
        trivia: Option<SourceAnchor>,
        target_item: Option<SourceAnchor>,
        choices: Vec<String>,
        purpose: DecisionPurpose,
    },
    SelectionChangeRequired {
        fields: Vec<String>,
        instruction: String,
    },
    UnsupportedInEngine {
        construct: String,
        instruction: String,
    },
}
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct DecisionGroup {
    pub category: String,
    pub reason: DecisionReason,
    pub route: DecisionRoute,
    pub blocks_applicability: bool,
    pub decision_ids: Vec<String>,
    pub count: usize,
}
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct Confidence {
    pub basis: String,
    pub level: String,
    pub limitations: Vec<String>,
}
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ArtifactLink {
    Edit {
        index: usize,
        replacement_range: ByteRange,
    },
    CreatedFile {
        id: String,
        content_range: ByteRange,
    },
}
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct Rewrite {
    pub id: String,
    pub kind: String,
    pub item_ids: Vec<String>,
    pub decision_ids: Vec<String>,
    pub anchors: Vec<SourceAnchor>,
    pub target: RewriteTarget,
    pub before_text: String,
    pub after_text: String,
    pub origin: String,
    pub evidence: Vec<String>,
    pub rationale: String,
    pub confidence: Confidence,
    pub default_action: String,
    pub selected_action: String,
    pub artifact_links: Vec<ArtifactLink>,
}
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DeclarationLink {
    Synthesized { rewrite_id: String },
    Reused { path: String, span: SourceSlice },
}
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct CreatedFile {
    pub id: String,
    pub path: String,
    pub must_be_absent: bool,
    pub mode: String,
    pub content: String,
    pub parent_path: String,
    pub declaration_link: DeclarationLink,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub declaration_visibility_rewrite_id: Option<String>,
    pub item_ids: Vec<String>,
    pub trivia_ids: Vec<String>,
    pub rewrite_ids: Vec<String>,
    pub origins: Vec<MoveOrigin>,
}
#[derive(Debug, Clone, Default, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct MoveCounts {
    pub discovered_files: usize,
    pub scanned_files: usize,
    pub eligible_files: usize,
    pub inventory_items: usize,
    pub selected_items: usize,
    pub reference_candidates: usize,
    pub analysis_descriptor_bytes: usize,
    pub rewrites: usize,
    pub creations: usize,
    pub omissions: BTreeMap<String, usize>,
}
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct WorkLimits {
    pub inventory_descriptors: usize,
    pub reference_candidates: usize,
    pub analysis_descriptor_bytes: usize,
    pub max_moves: usize,
    pub query_state_limit_applicable: bool,
    pub reference_coverage: String,
}
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct MovePlan {
    pub state: String,
    pub applicable: bool,
    pub selected_count: Option<usize>,
    pub moves: Vec<MoveRecord>,
    pub trivia_decisions: Vec<MoveTriviaDecision>,
    pub decisions: Vec<Decision>,
    pub decision_groups: Vec<DecisionGroup>,
    pub chain_diagnostics: Vec<items::ChainDiagnostic>,
    pub rewrites: Vec<Rewrite>,
    pub base_files: Vec<BaseFile>,
    pub blockers: Vec<Blocker>,
    pub edits: Option<Vec<Edit>>,
    pub created_files: Option<Vec<CreatedFile>>,
    pub patch: Option<String>,
    pub origins: Vec<MoveOrigin>,
    pub integrity: Integrity,
}
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct MoveEnvelope {
    pub schema_version: u8,
    pub tool: String,
    pub root: Option<String>,
    pub snapshot_id: Option<String>,
    pub status: String,
    pub coverage: Coverage,
    pub counts: MoveCounts,
    pub limits: Limits,
    pub effective_work_limits: WorkLimits,
    pub truncation_reasons: Vec<String>,
    pub skipped: Skipped,
    pub diagnostics: Vec<Diagnostic>,
    pub diagnostics_omitted: usize,
    pub error: Option<DomainError>,
    pub draft_provenance: Option<DraftProvenance>,
    pub plan: MovePlan,
}
impl MoveEnvelope {
    pub fn empty(limits: Limits) -> Self {
        Self {
            schema_version: 1,
            tool: "move_item".into(),
            root: None,
            snapshot_id: None,
            status: "complete".into(),
            coverage: Coverage::default(),
            counts: MoveCounts::default(),
            limits,
            effective_work_limits: WorkLimits {
                inventory_descriptors: 100_000,
                reference_candidates: 100_000,
                analysis_descriptor_bytes: 128 * 1024 * 1024,
                max_moves: 500,
                query_state_limit_applicable: false,
                reference_coverage:
                    "admitted caller scope only; not exhaustive generated/build-target references"
                        .into(),
            },
            truncation_reasons: Vec::new(),
            skipped: skipped_map(),
            diagnostics: Vec::new(),
            diagnostics_omitted: 0,
            error: None,
            draft_provenance: None,
            plan: MovePlan {
                state: "blocked".into(),
                applicable: false,
                selected_count: None,
                moves: Vec::new(),
                trivia_decisions: Vec::new(),
                decisions: Vec::new(),
                decision_groups: Vec::new(),
                chain_diagnostics: Vec::new(),
                rewrites: Vec::new(),
                base_files: Vec::new(),
                blockers: Vec::new(),
                edits: None,
                created_files: None,
                patch: None,
                origins: Vec::new(),
                integrity: Integrity {
                    grammar: "tree-sitter-rust@0.24.2".into(),
                    syntax: "not_checked".into(),
                    semantic: "not_performed".into(),
                },
            },
        }
    }
    pub fn failed(limits: Limits, error: DomainError) -> Self {
        let mut result = Self::empty(limits);
        result.status = "failed".into();
        result.error = Some(error);
        result
    }
    fn account(&mut self, bytes: usize) -> Result<(), DomainError> {
        self.counts.analysis_descriptor_bytes += bytes;
        if self.counts.analysis_descriptor_bytes
            > self.effective_work_limits.analysis_descriptor_bytes
        {
            return Err(DomainError::new(
                "analysis_descriptor_bytes",
                "whole-call descriptor guard reached",
            ));
        }
        Ok(())
    }
    fn chain_decisions(
        &mut self,
        diagnostics: Vec<items::ChainDiagnostic>,
        fallback: &SourceAnchor,
        files: &BTreeMap<String, FileSnapshot>,
    ) -> Result<(), DomainError> {
        for mut diagnostic in diagnostics {
            diagnostic.id.clear();
            if let Some(prior) = self.plan.chain_diagnostics.iter().position(|d| {
                let mut prior = d.clone();
                prior.id.clear();
                prior == diagnostic
            }) {
                let id = &self.plan.chain_diagnostics[prior].id;
                let index = self
                    .plan
                    .decisions
                    .iter()
                    .position(|d| d.chain_diagnostic_ids.contains(id))
                    .expect("chain decision");
                if !self.plan.decisions[index].anchors.contains(fallback) {
                    self.account(descriptor_bytes(fallback)?)?;
                    self.plan.decisions[index].anchors.push(fallback.clone());
                }
                continue;
            }
            diagnostic.id = format!("chain/{}", self.plan.chain_diagnostics.len());
            let location = diagnostic.declaration.as_ref();
            let mut anchors = location
                .and_then(|d| {
                    files
                        .get(&d.path)
                        .map(|f| anchor(&d.path, &f.source, &d.range))
                })
                .map(|a| vec![a])
                .unwrap_or_else(|| vec![fallback.clone()]);
            if !anchors.contains(fallback) {
                anchors.push(fallback.clone());
            }
            let action = DecisionAction::chain(&diagnostic, "move_item");
            let decision = Decision {
                reason: DecisionReason::ModuleChainFailure,
                next_action: action.next_action(),
                action,
                id: format!("d/{}", self.plan.decisions.len()),
                category: "module_context".into(),
                anchors,
                item_ids: Vec::new(),
                evidence: Vec::new(),
                unresolved_consequence: format!(
                    "ordinary chain from supplied root {} cannot prove {}: {:?}",
                    diagnostic.crate_root, diagnostic.requested_path, diagnostic.reason
                ),
                resolution: "request_change_required".into(),
                supported_choices: Vec::new(),
                selected_choice: None,
                blocks_applicability: true,
                chain_diagnostic_ids: vec![diagnostic.id.clone()],
                lexical_uncertainty: None,
            };
            self.account(descriptor_bytes(&(&diagnostic, &decision))?)?;
            self.plan.chain_diagnostics.push(diagnostic);
            self.plan.decisions.push(decision);
        }
        Ok(())
    }
    fn structural_decision(
        &mut self,
        category: &str,
        reason: DecisionReason,
        anchors: Vec<SourceAnchor>,
        item_ids: Vec<String>,
        message: &str,
    ) -> Result<(), DomainError> {
        let action = DecisionAction::cause(reason);
        let decision = Decision {
            reason,
            next_action: action.next_action(),
            action,
            id: format!("d/{}", self.plan.decisions.len()),
            category: category.into(),
            anchors,
            item_ids,
            evidence: Vec::new(),
            unresolved_consequence: message.into(),
            resolution: "request_change_required".into(),
            supported_choices: Vec::new(),
            selected_choice: None,
            blocks_applicability: true,
            chain_diagnostic_ids: Vec::new(),
            lexical_uncertainty: None,
        };
        self.account(descriptor_bytes(&decision)?)?;
        self.plan.decisions.push(decision);
        Ok(())
    }
    fn finish_decision_groups(&mut self, groups: Result<Vec<DecisionGroup>, DomainError>) {
        match groups {
            Ok(groups) => self.plan.decision_groups = groups,
            Err(error) => {
                // Grouping stopped: publish neither ungrouped decisions nor dangling audit links.
                for (name, count) in [
                    ("decisions", self.plan.decisions.len()),
                    ("decision_groups", self.plan.decision_groups.len()),
                    (
                        "decision_group_references",
                        self.plan
                            .decision_groups
                            .iter()
                            .map(|g| g.decision_ids.len())
                            .sum(),
                    ),
                    (
                        "chain_diagnostic_references",
                        self.plan
                            .decisions
                            .iter()
                            .map(|d| d.chain_diagnostic_ids.len())
                            .sum(),
                    ),
                    (
                        "move_decision_references",
                        self.plan.moves.iter().map(|m| m.decision_ids.len()).sum(),
                    ),
                    (
                        "rewrite_decision_references",
                        self.plan
                            .rewrites
                            .iter()
                            .map(|r| r.decision_ids.len())
                            .sum(),
                    ),
                ] {
                    *self.counts.omissions.entry(name.into()).or_default() += count;
                }
                self.plan.decisions.clear();
                self.plan.decision_groups.clear();
                for record in &mut self.plan.moves {
                    record.decision_ids.clear();
                }
                for rewrite in &mut self.plan.rewrites {
                    rewrite.decision_ids.clear();
                }
                if error.code == "CANCELLED" {
                    self.status = "failed".into();
                    self.error = Some(error);
                    self.plan.state = "blocked".into();
                    self.withhold();
                } else {
                    self.incomplete(&error.code);
                }
            }
        }
    }
    fn withhold(&mut self) {
        self.plan.applicable = false;
        self.plan.edits = None;
        self.plan.created_files = None;
        self.plan.patch = None;
    }
    fn blocker(&mut self, code: &str, message: &str, path: Option<&str>, range: Option<ByteRange>) {
        self.withhold();
        // Category summaries coalesce; decisions retain every original anchor and consequence.
        if matches!(
            code,
            "BINDING_COLLISION"
                | "GLOB_DEPENDENCY"
                | "MACRO_DEPENDENCY"
                | "REEXPORT_DEPENDENCY"
                | "MODULE_CONTEXT"
                | "VISIBILITY_CONTEXT"
                | "SCOPE_DEPENDENCY"
                | "TRIVIA_OWNERSHIP"
                | "UNSUPPORTED_DEPENDENCY_FORM"
        ) && self.plan.blockers.iter().any(|b| b.code == code)
        {
            return;
        }
        if self.plan.blockers.len() < self.limits.diagnostic_count.max(1) {
            self.plan.blockers.push(Blocker {
                code: code.into(),
                message: message.into(),
                path: path.map(str::to_owned),
                range,
            });
        } else {
            *self.counts.omissions.entry("blockers".into()).or_default() += 1;
        }
    }
    fn incomplete(&mut self, code: &str) {
        self.status = "partial".into();
        self.plan.state = "incomplete".into();
        self.coverage.scope_exhaustive = false;
        self.truncation_reasons.push(code.into());
        self.blocker(
            code,
            "all artifacts withheld; narrow scope/request or raise limits",
            None,
            None,
        );
    }
    pub fn wire_bytes(&self) -> usize {
        let value = serde_json::to_value(self).expect("serializable move");
        serde_json::to_vec(&serde_json::json!({"content":[{"type":"text","text":value.to_string()}],"structuredContent":value,"isError":self.error.is_some()})).expect("wire JSON").len() + 4096
    }
    fn fit(&mut self) {
        if self.wire_bytes() <= self.limits.response_bytes {
            return;
        }
        self.incomplete("response_bytes");
        // No dangling audit links or reconstructable subset: bound the entire preview together.
        let arrays = [
            ("moves", self.plan.moves.len()),
            ("trivia_decisions", self.plan.trivia_decisions.len()),
            ("decisions", self.plan.decisions.len()),
            ("decision_groups", self.plan.decision_groups.len()),
            (
                "decision_group_references",
                self.plan
                    .decision_groups
                    .iter()
                    .map(|g| g.decision_ids.len())
                    .sum(),
            ),
            ("chain_diagnostics", self.plan.chain_diagnostics.len()),
            (
                "chain_diagnostic_references",
                self.plan
                    .decisions
                    .iter()
                    .map(|d| d.chain_diagnostic_ids.len())
                    .sum(),
            ),
            ("rewrites", self.plan.rewrites.len()),
            ("base_files", self.plan.base_files.len()),
            ("origins", self.plan.origins.len()),
        ];
        for (name, count) in arrays {
            *self.counts.omissions.entry(name.into()).or_default() += count;
        }
        self.plan.moves.clear();
        self.plan.trivia_decisions.clear();
        self.plan.decisions.clear();
        self.plan.decision_groups.clear();
        self.plan.chain_diagnostics.clear();
        self.plan.rewrites.clear();
        self.plan.base_files.clear();
        self.plan.origins.clear();
        self.diagnostics_omitted += self.diagnostics.len();
        self.diagnostics.clear();
        for skipped in self.skipped.values_mut() {
            skipped.examples_omitted += skipped.examples.len() as u64;
            skipped.examples.clear();
        }
        while self.wire_bytes() > self.limits.response_bytes && self.plan.blockers.len() > 1 {
            self.plan.blockers.pop();
            *self.counts.omissions.entry("blockers".into()).or_default() += 1;
        }
        if self.wire_bytes() > self.limits.response_bytes {
            self.root = None;
            self.draft_provenance = None;
        }
    }
}
// Count without allocating a second serialized analysis corpus.
fn descriptor_bytes(value: &impl Serialize) -> Result<usize, DomainError> {
    struct Counter(usize);
    impl std::io::Write for Counter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0 += bytes.len();
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
    serde_json::to_writer(&mut counter, value)
        .map_err(|_| DomainError::new("analysis_descriptor_bytes", "descriptor guard reached"))?;
    Ok(counter.0)
}
fn error(code: &str, message: &str, field: &str) -> DomainError {
    let mut error = DomainError::new(code, message);
    error.field = Some(field.into());
    error
}
fn range(start: usize, end: usize) -> ByteRange {
    ByteRange {
        start_byte: start,
        end_byte: end,
    }
}
fn anchor(path: &str, source: &str, span: &ByteRange) -> SourceAnchor {
    SourceAnchor {
        path: path.into(),
        range: span.clone(),
        expected_text: source[span.start_byte..span.end_byte].into(),
    }
}
pub fn run(launch: &Path, request: MoveRequest, cancelled: &AtomicBool) -> MoveEnvelope {
    run_with_recheck(launch, request, cancelled, || {})
}
fn run_with_recheck(
    launch: &Path,
    request: MoveRequest,
    cancelled: &AtomicBool,
    before_recheck: impl FnOnce(),
) -> MoveEnvelope {
    let started = Instant::now();
    let mut result = MoveEnvelope::empty(request.limits.clone());
    result.effective_work_limits.max_moves = request.max_moves;
    if let Err(error) = build(launch, &request, cancelled, before_recheck, &mut result) {
        if matches!(
            error.code.as_str(),
            "planning_deadline"
                | "scan_incomplete"
                | "inventory_work_limit"
                | "reference_work_limit"
                | "analysis_descriptor_bytes"
        ) {
            result.incomplete(&error.code);
        } else {
            result.status = "failed".into();
            result.error = Some(error);
            result.plan.state = "blocked".into();
            result.withhold();
        }
    }
    let links = items::finalize_chain(&mut result.plan.chain_diagnostics);
    for decision in &mut result.plan.decisions {
        for id in &mut decision.chain_diagnostic_ids {
            *id = links[id].clone();
        }
    }
    let groups = decision_groups(
        result.plan.decisions.iter().map(|d| {
            (
                d.category.as_str(),
                d.reason,
                &d.action,
                d.blocks_applicability,
                d.id.as_str(),
            )
        }),
        &mut result.counts.analysis_descriptor_bytes,
        (
            started + Duration::from_millis(request.limits.time_budget_ms.min(300_000)),
            cancelled,
        ),
    );
    result.finish_decision_groups(groups);
    result.fit();
    if result.plan.applicable
        && let Err(error) = items::check(
            started + Duration::from_millis(request.limits.time_budget_ms.min(300_000)),
            cancelled,
        )
    {
        if error.code == "planning_deadline" {
            result.incomplete("planning_deadline");
        } else {
            result.status = "failed".into();
            result.error = Some(error);
            result.plan.state = "blocked".into();
            result.withhold();
        }
        result.fit();
    }
    tracing::info!(tool="move_item", elapsed_ms=started.elapsed().as_millis(), status=%result.status, selected=?result.plan.selected_count, error=?result.error.as_ref().map(|e| &e.code), "move plan finished");
    result
}

#[derive(Clone)]
struct Run {
    path: String,
    range: ByteRange,
}
struct Selection {
    source: String,
    item: Item,
    destination: Destination,
    runs: Vec<Run>,
}
struct Insertion {
    path: String,
    at: usize,
    text: String,
    copies: Vec<MoveOrigin>,
    rewrites: Vec<(String, ByteRange)>,
    item_ids: Vec<String>,
}
struct Creation {
    parent: String,
    link: Option<DeclarationLink>,
    visibility_id: Option<String>,
    selections: Vec<usize>,
}
fn validate_anchor<'a>(
    value: &SourceAnchor,
    files: &'a BTreeMap<String, FileSnapshot>,
    parsed: &BTreeMap<String, ParsedFile>,
    code: &str,
    field: &str,
) -> Result<&'a FileSnapshot, DomainError> {
    let file = files.get(&value.path).ok_or_else(|| {
        error(
            code,
            "unknown/out-of-scope path; broaden scope or obtain fresh anchor",
            field,
        )
    })?;
    if value.range.start_byte >= value.range.end_byte
        || file
            .source
            .get(value.range.start_byte..value.range.end_byte)
            != Some(&value.expected_text)
    {
        return Err(error(
            code,
            "full original anchor bytes changed or range invalid",
            field,
        ));
    }
    if !parsed[&value.path]
        .items
        .iter()
        .any(|i| i.span.range == value.range)
    {
        return Err(error(
            "INVALID_ITEM_SELECTION",
            "anchor must identify one whole direct top-level item",
            field,
        ));
    }
    Ok(file)
}
fn build(
    launch: &Path,
    request: &MoveRequest,
    cancelled: &AtomicBool,
    before_recheck: impl FnOnce(),
    result: &mut MoveEnvelope,
) -> Result<(), DomainError> {
    request.limits.validate()?;
    if !(1..=5000).contains(&request.max_moves)
        || request.context.before_lines > 20
        || request.context.after_lines > 20
    {
        return Err(error(
            "INVALID_PARAMS",
            "max_moves is 1–5000; context is 0–20 lines",
            "max_moves",
        ));
    }
    if request
        .draft_provenance
        .as_ref()
        .is_some_and(|d| d.draft_id.len() > 256 || d.source_snapshot_id.len() > 256)
    {
        return Err(error(
            "INVALID_PARAMS",
            "provenance strings are at most 256 bytes",
            "draft_provenance",
        ));
    }
    result.draft_provenance = request.draft_provenance.clone();
    let deadline = Instant::now() + Duration::from_millis(request.limits.time_budget_ms);
    items::check(deadline, cancelled)?;
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
    let (files, snapshot) = scope::discover(&scope, &mut scan, deadline, cancelled)?;
    result.coverage = scan.coverage;
    result.skipped = scan.skipped;
    result.truncation_reasons = scan.truncation_reasons;
    result.counts.discovered_files = scan.counts.discovered_files;
    result.counts.scanned_files = scan.counts.scanned_files;
    result.counts.eligible_files = scan.counts.eligible_files;
    result.snapshot_id = snapshot.clone();
    if !result.coverage.scope_exhaustive || snapshot.is_none() {
        result.incomplete("scan_incomplete");
        return Ok(());
    }
    let files: BTreeMap<_, _> = files.into_iter().map(|f| (f.path.clone(), f)).collect();
    if !files.contains_key(&request.crate_root) {
        let mut diagnostic = items::ChainDiagnostic::boundary(
            &request.crate_root,
            &request.crate_root,
            items::ChainRole::Source,
            items::ChainReason::ChainFileUnadmitted,
        );
        diagnostic.id = "chain/0".into();
        diagnostic.candidate_paths.push(request.crate_root.clone());
        result.account(descriptor_bytes(&diagnostic)?)?;
        result.plan.chain_diagnostics.push(diagnostic);
        return Err(error(
            "STALE_SELECTION",
            "crate_root must be an admitted existing Rust file",
            "crate_root",
        ));
    }
    if request.moves.len() > request.max_moves {
        result.incomplete("max_moves");
        return Ok(());
    }
    result.plan.selected_count = Some(request.moves.len());
    result.counts.selected_items = request.moves.len();
    let mut parsed = BTreeMap::new();
    for (path, file) in &files {
        items::check(deadline, cancelled)?;
        let data = items::parse(
            file,
            request.limits.text_bytes,
            deadline,
            cancelled,
            &mut result.counts.inventory_items,
        )?;
        result.account(
            descriptor_bytes(&(&data.items, &data.trivia))?
                + data
                    .items
                    .iter()
                    .map(|i| i.visibility_key.len())
                    .sum::<usize>(),
        )?;
        if result.counts.inventory_items > 100_000 {
            return Err(DomainError::new(
                "inventory_work_limit",
                "top-level descriptor cap reached",
            ));
        }
        parsed.insert(path.clone(), data);
    }
    let module_analysis = items::modules(
        &scope,
        &request.crate_root,
        &files,
        &parsed,
        deadline,
        cancelled,
        &mut result.counts.analysis_descriptor_bytes,
    )?;
    let contexts = &module_analysis.contexts;
    result.account(descriptor_bytes(&contexts)?)?;
    if request.moves.is_empty()
        && (request
            .trivia_overrides
            .as_ref()
            .is_some_and(|v| !v.is_empty())
            || request
                .rewrite_overrides
                .as_ref()
                .is_some_and(|v| !v.is_empty()))
    {
        return Err(error(
            "INVALID_PARAMS",
            "empty moves forbids stray overrides",
            "moves",
        ));
    }
    let mut selected = Vec::new();
    let mut seen = BTreeSet::new();
    let mut creations: BTreeMap<String, Creation> = BTreeMap::new();
    for (index, entry) in request.moves.iter().enumerate() {
        items::check(deadline, cancelled)?;
        let field = format!("moves[{index}].item");
        validate_anchor(&entry.item, &files, &parsed, "STALE_SELECTION", &field)?;
        if !seen.insert((entry.item.path.clone(), entry.item.range.clone())) {
            return Err(error(
                "DUPLICATE_MOVE",
                "each item has exactly one destination",
                &field,
            ));
        }
        let item = parsed[&entry.item.path]
            .items
            .iter()
            .find(|i| i.span.range == entry.item.range)
            .expect("validated item")
            .clone();
        let destination = entry.destination.path();
        scope::normalized_path(destination).map_err(|mut e| {
            e.field = Some(format!("moves[{index}].destination.path"));
            e
        })?;
        if destination == entry.item.path {
            return Err(error(
                "INVALID_DESTINATION",
                "same-file reorder is not supported",
                &format!("moves[{index}].destination"),
            ));
        }
        match &entry.destination {
            Destination::Existing { path, before_item } => {
                if !files.contains_key(path) {
                    return Err(error(
                        "INVALID_DESTINATION",
                        "existing destination must be an admitted regular Rust file",
                        &format!("moves[{index}].destination.path"),
                    ));
                }
                if let Some(before) = before_item {
                    if before.path != *path {
                        return Err(error(
                            "STALE_DESTINATION",
                            "before_item must belong to destination",
                            &format!("moves[{index}].destination.before_item"),
                        ));
                    }
                    validate_anchor(
                        before,
                        &files,
                        &parsed,
                        "STALE_DESTINATION",
                        &format!("moves[{index}].destination.before_item"),
                    )?;
                    if request
                        .moves
                        .iter()
                        .any(|m| m.item.path == before.path && m.item.range == before.range)
                    {
                        return Err(error(
                            "INVALID_DESTINATION",
                            "before_item cannot be selected",
                            &format!("moves[{index}].destination.before_item"),
                        ));
                    }
                }
            }
            Destination::NewSibling { path, parent_path } => {
                let name = items::module_name(path).map_err(|mut e| {
                    e.field = Some(format!("moves[{index}].destination.path"));
                    e
                })?;
                if Path::new(path).parent() != Path::new(&entry.item.path).parent() {
                    return Err(error(
                        "INVALID_DESTINATION",
                        "new path must be a literal sibling of every assigned source",
                        &format!("moves[{index}].destination.path"),
                    ));
                }
                if !files.contains_key(parent_path)
                    || items::child_path(parent_path, &request.crate_root, name) != *path
                {
                    let mut diagnostic = items::ChainDiagnostic::boundary(
                        &request.crate_root,
                        parent_path,
                        items::ChainRole::DeclarationParent,
                        if files.contains_key(parent_path) {
                            items::ChainReason::OrdinaryLayoutMismatch
                        } else {
                            items::ChainReason::ChainFileUnadmitted
                        },
                    );
                    diagnostic.at_file_path = parent_path.clone();
                    diagnostic.candidate_paths = if files.contains_key(parent_path) {
                        vec![
                            items::child_path(parent_path, &request.crate_root, name),
                            path.clone(),
                        ]
                    } else {
                        vec![parent_path.clone()]
                    };
                    diagnostic.evidenced_prefix_paths = contexts
                        .get(parent_path)
                        .map(|e| e.filesystem_paths.clone())
                        .unwrap_or_default();
                    diagnostic.parent_candidates.push(parent_path.clone());
                    result.chain_decisions(vec![diagnostic], &entry.item, &files)?;
                    return Err(error(
                        "INVALID_DECLARATION_PARENT",
                        "ordinary 2018 child path does not equal requested sibling",
                        &format!("moves[{index}].destination.parent_path"),
                    ));
                }
                scope
                    .admit_new_path(path, deadline, cancelled)
                    .map_err(|mut e| {
                        e.field = Some(format!("moves[{index}].destination.path"));
                        e
                    })?;
                let competing = format!("{}/mod.rs", path.trim_end_matches(".rs"));
                if items::present(&scope, &competing)? {
                    let mut diagnostic = items::ChainDiagnostic::boundary(
                        &request.crate_root,
                        path,
                        items::ChainRole::DeclarationParent,
                        items::ChainReason::CompetingFileLayout,
                    );
                    diagnostic.at_file_path = parent_path.clone();
                    diagnostic.evidenced_prefix_paths = contexts
                        .get(parent_path)
                        .map(|e| e.filesystem_paths.clone())
                        .unwrap_or_default();
                    diagnostic.candidate_paths = vec![path.clone(), competing];
                    result.chain_decisions(vec![diagnostic], &entry.item, &files)?;
                    return Err(error(
                        "MODULE_DECLARATION_CONFLICT",
                        "competing name/mod.rs layout",
                        &format!("moves[{index}].destination.path"),
                    ));
                }
                if creations
                    .keys()
                    .any(|p| p != path && p.to_lowercase() == path.to_lowercase())
                {
                    let mut diagnostic = items::ChainDiagnostic::boundary(
                        &request.crate_root,
                        path,
                        items::ChainRole::DeclarationParent,
                        items::ChainReason::CompetingDeclarations,
                    );
                    diagnostic.at_file_path = parent_path.clone();
                    diagnostic.candidate_paths = creations
                        .keys()
                        .filter(|p| p.to_lowercase() == path.to_lowercase())
                        .cloned()
                        .chain(std::iter::once(path.clone()))
                        .collect();
                    result.chain_decisions(vec![diagnostic], &entry.item, &files)?;
                    return Err(error(
                        "MODULE_DECLARATION_CONFLICT",
                        "batch has case-folded creation aliases",
                        &format!("moves[{index}].destination.path"),
                    ));
                }
                let creation = creations.entry(path.clone()).or_insert(Creation {
                    parent: parent_path.clone(),
                    link: None,
                    visibility_id: None,
                    selections: Vec::new(),
                });
                if creation.parent != *parent_path {
                    let mut diagnostic = items::ChainDiagnostic::boundary(
                        &request.crate_root,
                        path,
                        items::ChainRole::DeclarationParent,
                        items::ChainReason::CompetingDeclarations,
                    );
                    diagnostic.at_file_path = parent_path.clone();
                    diagnostic.parent_candidates =
                        vec![creation.parent.clone(), parent_path.clone()];
                    result.chain_decisions(vec![diagnostic], &entry.item, &files)?;
                    return Err(error(
                        "MODULE_DECLARATION_CONFLICT",
                        "conflicting creation descriptors",
                        &format!("moves[{index}].destination"),
                    ));
                }
                creation.selections.push(index);
            }
        }
        selected.push(Selection {
            source: entry.item.path.clone(),
            item,
            destination: entry.destination.clone(),
            runs: Vec::new(),
        });
    }
    let mut new_contexts = contexts.clone();
    for (path, creation) in &mut creations {
        items::check(deadline, cancelled)?;
        let name = items::module_name(path)?;
        let parent = &parsed[&creation.parent];
        let matches: Vec<_> = parent
            .items
            .iter()
            .filter(|i| {
                i.name
                    .as_deref()
                    .is_some_and(|n| n.trim_start_matches("r#") == name)
                    && !selected
                        .iter()
                        .any(|s| s.source == creation.parent && s.item.span.range == i.span.range)
            })
            .collect();
        if selected.iter().any(|s| {
            s.destination.path() == creation.parent
                && s.item
                    .name
                    .as_deref()
                    .is_some_and(|n| n.trim_start_matches("r#") == name)
        }) {
            result.blocker(
                "BINDING_COLLISION",
                "incoming item collides with the proposed parent module declaration",
                Some(&creation.parent),
                None,
            );
            let arrivals: Vec<_> = selected
                .iter()
                .filter(|s| {
                    s.destination.path() == creation.parent
                        && s.item
                            .name
                            .as_deref()
                            .is_some_and(|n| n.trim_start_matches("r#") == name)
                })
                .collect();
            result.structural_decision(
                "binding_collision",
                DecisionReason::DestinationBindingConflict,
                arrivals
                    .iter()
                    .map(|s| anchor(&s.source, &files[&s.source].source, &s.item.span.range))
                    .collect(),
                arrivals.iter().map(|s| s.item.id.clone()).collect(),
                "incoming item collides with the proposed parent module declaration",
            )?;
        }
        if !matches.is_empty() {
            let declaration = matches[0];
            let node = parent
                .tree
                .root_node()
                .named_descendant_for_byte_range(
                    declaration.span.range.start_byte,
                    declaration.span.range.end_byte,
                )
                .expect("item");
            if matches.len() != 1
                || declaration.kind != "mod_item"
                || !declaration.attributes.is_empty()
                || node.child_by_field_name("body").is_some()
            {
                let mut reasons =
                    items::declaration_reasons(parent, &files[&creation.parent], declaration);
                if matches.len() != 1 || declaration.kind != "mod_item" {
                    reasons.push(items::ChainReason::CompetingDeclarations);
                }
                let diagnostics = reasons
                    .into_iter()
                    .map(|reason| {
                        let mut diagnostic = items::ChainDiagnostic::boundary(
                            &request.crate_root,
                            path,
                            items::ChainRole::DeclarationParent,
                            reason,
                        );
                        diagnostic.at_file_path = creation.parent.clone();
                        diagnostic.evidenced_prefix_paths = contexts
                            .get(&creation.parent)
                            .map(|e| e.filesystem_paths.clone())
                            .unwrap_or_default();
                        diagnostic.declaration = Some(items::ChainLocation {
                            path: creation.parent.clone(),
                            range: declaration.span.range.clone(),
                        });
                        diagnostic.candidate_paths = vec![
                            path.clone(),
                            format!("{}/mod.rs", path.trim_end_matches(".rs")),
                        ];
                        diagnostic
                    })
                    .collect();
                result.chain_decisions(
                    diagnostics,
                    &request.moves[creation.selections[0]].item,
                    &files,
                )?;
                return Err(error(
                    "MODULE_DECLARATION_CONFLICT",
                    "name conflicts with conditional/inline/competing declaration",
                    "moves.destination",
                ));
            }
            if declaration.visibility_key == "restricted"
                && items::absolute_visibility(
                    node,
                    &files[&creation.parent].source,
                    (deadline, cancelled),
                )
                .is_none()
            {
                result.blocker("VISIBILITY_CONTEXT", "reused restricted declaration needs verified absolute access context; no visibility repair is performed", Some(&creation.parent), Some(declaration.span.range.clone()));
                result.structural_decision("visibility_context", DecisionReason::VisibilityScopeUnproved,
                    vec![anchor(&creation.parent, &files[&creation.parent].source, &declaration.span.range)],
                    creation.selections.iter().map(|i| selected[*i].item.id.clone()).collect(), "reused restricted declaration needs verified absolute access context; no visibility repair is performed")?;
            }
            creation.link = Some(DeclarationLink::Reused {
                path: creation.parent.clone(),
                span: declaration.span.clone(),
            });
        }
        if let Some(parent_context) = contexts.get(&creation.parent) {
            let mut context = parent_context.clone();
            context.module_segments.push(name.into());
            context.filesystem_paths.push(path.clone());
            if let Some(DeclarationLink::Reused { span, .. }) = &creation.link {
                context.declaration_anchors.push(anchor(
                    &creation.parent,
                    &files[&creation.parent].source,
                    &span.range,
                ));
            }
            new_contexts.insert(path.clone(), context);
        }
    }
    result.account(descriptor_bytes(&new_contexts)?)?;
    for selection in &selected {
        items::check(deadline, cancelled)?;
        let old = contexts.get(&selection.source);
        let new = new_contexts.get(selection.destination.path());
        let new_exposure = match creations.get(selection.destination.path()) {
            Some(creation) => {
                matches!(&creation.link, Some(DeclarationLink::Reused { .. }))
                    && new.is_some_and(|e| items::public_chain(e, &parsed))
            }
            None => new.is_some_and(|e| items::public_chain(e, &parsed)),
        };
        if selection.item.visibility_key == "pub" && new_exposure {
            result.blocker(
                "REEXPORT_DEPENDENCY",
                "destination would expose a changed public path; explicit API decision required",
                Some(&selection.source),
                Some(selection.item.span.range.clone()),
            );
            result.structural_decision(
                "reexport_dependency",
                DecisionReason::PublicPathChange,
                vec![anchor(
                    &selection.source,
                    &files[&selection.source].source,
                    &selection.item.span.range,
                )],
                vec![selection.item.id.clone()],
                "destination would expose a changed public path; explicit API decision required",
            )?;
        }
        if old.is_none_or(|e| !e.unresolved.is_empty())
            || new.is_none_or(|e| !e.unresolved.is_empty())
        {
            result.blocker("CRATE_IDENTITY_UNCERTAIN", "required ordinary module chain is missing/ambiguous/recovered; admit the chain or change destination", Some(&selection.source), Some(selection.item.span.range.clone()));
            let fallback = anchor(
                &selection.source,
                &files[&selection.source].source,
                &selection.item.span.range,
            );
            let mut diagnostics = module_analysis.project(
                &request.crate_root,
                &selection.source,
                items::ChainRole::Source,
                (deadline, cancelled),
            )?;
            let (path, role) = match &selection.destination {
                Destination::Existing { path, .. } => (path, items::ChainRole::Destination),
                Destination::NewSibling { parent_path, .. } => {
                    (parent_path, items::ChainRole::DeclarationParent)
                }
            };
            diagnostics.extend(module_analysis.project(
                &request.crate_root,
                path,
                role,
                (deadline, cancelled),
            )?);
            result.chain_decisions(diagnostics, &fallback, &files)?;
        }
    }
    let tuples: Vec<_> = selected
        .iter()
        .map(|s| {
            (
                s.source.clone(),
                s.item.clone(),
                s.destination.path().into(),
            )
        })
        .collect();
    let (needs, candidates) = items::dependencies(
        &files,
        &parsed,
        &tuples,
        contexts,
        (deadline, cancelled),
        &mut result.counts.reference_candidates,
        &mut result.counts.analysis_descriptor_bytes,
    )?;
    result.counts.reference_candidates = candidates;
    let analysis = rewrites::analyze(
        request,
        &files,
        &parsed,
        &tuples,
        contexts,
        &new_contexts,
        needs,
        (deadline, cancelled),
        &mut result.counts.reference_candidates,
    )?;
    result.account(analysis.descriptor_bytes)?;
    for need in analysis.needs {
        items::check(deadline, cancelled)?;
        if result.plan.decisions.len() == 100_000 {
            return Err(DomainError::new(
                "analysis_descriptor_bytes",
                "decision cap reached",
            ));
        }
        let id = format!("d/{}", result.plan.decisions.len());
        result.blocker(
            &need.category.to_uppercase(),
            &need.message,
            Some(&need.path),
            Some(need.range.clone()),
        );
        let action = need
            .choice_target
            .map(DecisionAction::rewrite)
            .unwrap_or_else(|| DecisionAction::cause(need.reason));
        let supported_choices = match &action {
            DecisionAction::RequestField { choices, .. } => choices.clone(),
            _ => Vec::new(),
        };
        let decision = Decision {
            reason: need.reason,
            next_action: action.next_action(),
            action,
            id,
            category: need.category.into(),
            anchors: vec![anchor(&need.path, &files[&need.path].source, &need.range)],
            item_ids: need.item_ids,
            evidence: vec![Lines::new(&files[&need.path].source).slice(
                need.range.start_byte,
                need.range.end_byte,
                request.limits.text_bytes,
            )],
            unresolved_consequence: need.message,
            resolution: if supported_choices.is_empty() {
                "request_change_required"
            } else {
                "choice_available"
            }
            .into(),
            supported_choices,
            selected_choice: None,
            blocks_applicability: true,
            chain_diagnostic_ids: Vec::new(),
            lexical_uncertainty: need.lexical_uncertainty,
        };
        result.account(descriptor_bytes(&decision)?)?;
        result.plan.decisions.push(decision);
    }
    for choice in request.rewrite_overrides.as_deref().unwrap_or_default() {
        let anchors: &[SourceAnchor] = match &choice.target {
            RewriteTarget::Source { anchor } => std::slice::from_ref(anchor),
            RewriteTarget::Synthesis { items, .. } => items,
        };
        for value in anchors {
            items::check(deadline, cancelled)?;
            if files
                .get(&value.path)
                .and_then(|f| f.source.get(value.range.start_byte..value.range.end_byte))
                != Some(value.expected_text.as_str())
            {
                return Err(error(
                    "STALE_REWRITE_OVERRIDE",
                    "rewrite contributor/source anchor is changed or outside scope",
                    "rewrite_overrides.target",
                ));
            }
        }
    }
    collect_trivia(
        request,
        &files,
        &parsed,
        &mut selected,
        (deadline, cancelled),
        result,
    )?;
    for decision in &mut result.plan.decisions {
        if decision.reason == DecisionReason::ModuleChainFailure {
            for s in &selected {
                items::check(deadline, cancelled)?;
                if decision
                    .anchors
                    .iter()
                    .any(|a| a.path == s.source && a.range == s.item.span.range)
                    && !decision.item_ids.contains(&s.item.id)
                {
                    result.counts.analysis_descriptor_bytes += s.item.id.len() + 32;
                    if result.counts.analysis_descriptor_bytes
                        > result.effective_work_limits.analysis_descriptor_bytes
                    {
                        return Err(DomainError::new(
                            "analysis_descriptor_bytes",
                            "chain item-link descriptor guard reached",
                        ));
                    }
                    decision.item_ids.push(s.item.id.clone());
                }
            }
        }
    }
    result.account(0)?;
    for s in &selected {
        items::check(deadline, cancelled)?;
        result.plan.moves.push(MoveRecord {
            id: s.item.id.clone(),
            item: s.item.clone(),
            destination: s.destination.clone(),
            old_module: contexts.get(&s.source).cloned(),
            new_module: new_contexts.get(s.destination.path()).cloned(),
            carried_spans: s
                .runs
                .iter()
                .map(|r| CarriedSpan {
                    path: r.path.clone(),
                    span: Lines::new(&files[&r.path].source).slice(
                        r.range.start_byte,
                        r.range.end_byte,
                        request.limits.text_bytes,
                    ),
                })
                .collect(),
            output_range: None,
            origin_ids: Vec::new(),
            rewrite_ids: Vec::new(),
            decision_ids: result
                .plan
                .decisions
                .iter()
                .filter(|d| d.item_ids.contains(&s.item.id))
                .map(|d| d.id.clone())
                .collect(),
        });
    }
    result.account(descriptor_bytes(&result.plan.moves)?)?;
    assemble(
        request,
        &files,
        &parsed,
        &selected,
        &mut creations,
        &analysis.repairs,
        (deadline, cancelled),
        result,
    )?;
    result.account(descriptor_bytes(&(
        &result.plan.rewrites,
        &result.plan.origins,
        &result.plan.created_files,
    ))?)?;
    items::check(deadline, cancelled)?;
    if !result.plan.blockers.is_empty() {
        result.plan.integrity.syntax = "blocked".into();
        return Ok(());
    }
    before_recheck();
    // Check absence first so a newly discovered .rs file is explicitly a creation race.
    for path in creations.keys() {
        match scope.admit_new_path(path, deadline, cancelled) {
            Ok(()) => {}
            Err(e) if e.code == "DESTINATION_ALREADY_EXISTS" => {
                result.incomplete("CREATION_RACE");
                return Ok(());
            }
            Err(e)
                if matches!(
                    e.code.as_str(),
                    "CANCELLED" | "planning_deadline" | "scan_incomplete"
                ) =>
            {
                return Err(e);
            }
            Err(_) => {
                result.incomplete("SOURCE_CHANGED");
                return Ok(());
            }
        }
        if items::present(&scope, &format!("{}/mod.rs", path.trim_end_matches(".rs")))? {
            result.incomplete("CREATION_RACE");
            return Ok(());
        }
    }
    let mut recheck = SearchEnvelope::empty(request.limits.clone());
    let (_, fresh) = scope::discover(&scope, &mut recheck, deadline, cancelled)?;
    if fresh != snapshot || !recheck.coverage.scope_exhaustive {
        result.incomplete("SOURCE_CHANGED");
        return Ok(());
    }
    items::check(deadline, cancelled)?;
    result.plan.applicable = true;
    result.plan.state = "applicable".into();
    result.plan.integrity.syntax = if request.moves.is_empty() {
        "not_checked"
    } else {
        "checked"
    }
    .into();
    Ok(())
}

fn collect_trivia(
    request: &MoveRequest,
    files: &BTreeMap<String, FileSnapshot>,
    parsed: &BTreeMap<String, ParsedFile>,
    selected: &mut [Selection],
    controls: (Instant, &AtomicBool),
    result: &mut MoveEnvelope,
) -> Result<(), DomainError> {
    let (deadline, cancelled) = controls;
    let overrides = request.trivia_overrides.as_deref().unwrap_or_default();
    let mut used = BTreeSet::new();
    let mut extra: BTreeMap<usize, Vec<Run>> = BTreeMap::new();
    for s in selected.iter_mut() {
        items::check(deadline, cancelled)?;
        s.runs.push(Run {
            path: s.source.clone(),
            range: s.item.span.range.clone(),
        });
    }
    for (path, data) in parsed {
        items::check(deadline, cancelled)?;
        let source = &files[path].source;
        for t in &data.trivia {
            items::check(deadline, cancelled)?;
            let owned = selected.iter().position(|s| {
                s.source == *path
                    && ((s.item.span.range.start_byte <= t.range.start
                        && s.item.span.range.end_byte >= t.range.end)
                        || t.owned_by(s.item.span.range.start_byte..s.item.span.range.end_byte))
            });
            let previous = data
                .items
                .iter()
                .rev()
                .find(|i| i.span.range.end_byte <= t.range.start);
            let next = data
                .items
                .iter()
                .find(|i| i.span.range.start_byte >= t.range.end);
            let choice_relevant = owned.is_some()
                || selected.iter().any(|s| {
                    s.source == *path
                        && (previous.is_some_and(|i| i.id == s.item.id)
                            || next.is_some_and(|i| i.id == s.item.id))
                });
            let original_anchor = SourceAnchor {
                path: path.clone(),
                range: range(t.range.start, t.range.end),
                expected_text: source[t.range.clone()].into(),
            };
            let choices: Vec<_> = overrides
                .iter()
                .enumerate()
                .filter(|(_, o)| o.trivia == original_anchor)
                .collect();
            if choices.len() > 1 {
                return Err(error(
                    "INVALID_MOVE_TRIVIA_OVERRIDE",
                    "duplicate/conflicting trivia target",
                    "trivia_overrides",
                ));
            }
            let mut target = owned;
            if let Some((index, choice)) = choices.first() {
                used.insert(*index);
                if !choice_relevant
                    && !(t.protected() && selected.iter().any(|s| s.source == *path))
                {
                    return Err(error(
                        "INVALID_MOVE_TRIVIA_OVERRIDE",
                        "trivia is unrelated to a selected source",
                        "trivia_overrides",
                    ));
                }
                if matches!(choice.disposition, MoveDisposition::KeepInPlace)
                    && choice.target_item.is_some()
                {
                    return Err(error(
                        "INVALID_MOVE_TRIVIA_OVERRIDE",
                        "keep_in_place forbids target_item",
                        "trivia_overrides",
                    ));
                }
                if t.protected() || owned.is_some() {
                    result.blocker("UNSUPPORTED_TRIVIA_DISPOSITION", "protected/owned/internal trivia cannot be independently detached or retargeted", Some(path), Some(original_anchor.range.clone()));
                } else {
                    match choice.disposition {
                        MoveDisposition::KeepInPlace => {
                            if choice.target_item.is_some() {
                                return Err(error(
                                    "INVALID_MOVE_TRIVIA_OVERRIDE",
                                    "keep_in_place forbids target_item",
                                    "trivia_overrides",
                                ));
                            }
                        }
                        MoveDisposition::CarryWithItem => {
                            let target_anchor = choice.target_item.as_ref().ok_or_else(|| {
                                error(
                                    "INVALID_MOVE_TRIVIA_OVERRIDE",
                                    "carry requires a selected full target_item",
                                    "trivia_overrides",
                                )
                            })?;
                            target = request.moves.iter().position(|m| m.item == *target_anchor);
                            if target.is_none() {
                                return Err(error(
                                    "INVALID_MOVE_TRIVIA_OVERRIDE",
                                    "target is stale or unselected",
                                    "trivia_overrides.target_item",
                                ));
                            }
                            if t.classification != "ambiguous" {
                                result.blocker(
                                    "UNSUPPORTED_TRIVIA_DISPOSITION",
                                    "only ordinary ambiguous trivia can be independently carried",
                                    Some(path),
                                    Some(original_anchor.range.clone()),
                                );
                            }
                        }
                    }
                }
            }
            let relevant =
                owned.is_some() || choices.len() == 1 || selected.iter().any(|s| s.source == *path);
            if !relevant {
                continue;
            }
            if let Some(index) = target {
                let inside = selected[index].source == *path
                    && selected[index].item.span.range.start_byte <= t.range.start
                    && selected[index].item.span.range.end_byte >= t.range.end;
                if !inside {
                    let run = Run {
                        path: path.clone(),
                        range: original_anchor.range.clone(),
                    };
                    if owned.is_some() {
                        selected[index].runs.push(run);
                    } else {
                        extra.entry(index).or_default().push(run);
                    }
                }
            }
            let id = format!("t/{path}/{}/{}", t.range.start, t.range.end);
            let default = if owned.is_some() {
                "carry_with_item"
            } else {
                "keep_in_place"
            };
            result.plan.trivia_decisions.push(MoveTriviaDecision {
                id: id.clone(),
                path: path.clone(),
                span: Lines::new(source).slice(
                    t.range.start,
                    t.range.end,
                    request.limits.text_bytes,
                ),
                classification: t.classification.clone(),
                reason: t.reason.clone(),
                owner: t.owner_range().map(|r| range(r.start, r.end)),
                preservable: true,
                default_disposition: default.into(),
                selected_disposition: if target.is_some() {
                    "carry_with_item"
                } else {
                    "keep_in_place"
                }
                .into(),
                suggested_dispositions: if !t.protected() && owned.is_none() && choice_relevant {
                    vec!["keep_in_place".into(), "carry_with_item".into()]
                } else {
                    vec![default.into()]
                },
                item_ids: owned
                    .into_iter()
                    .map(|i| selected[i].item.id.clone())
                    .collect(),
                target_item_id: target.map(|i| selected[i].item.id.clone()),
                origin_ids: Vec::new(),
            });
            result.account(descriptor_bytes(&result.plan.trivia_decisions.last())?)?;
            if t.classification == "ambiguous" {
                let action = DecisionAction::RequestField {
                    tool: "move_item".into(),
                    field: "trivia_overrides[]".into(),
                    target: None,
                    trivia: Some(original_anchor.clone()),
                    target_item: target.map(|i| request.moves[i].item.clone()).or_else(|| {
                        request
                            .moves
                            .iter()
                            .find(|m| m.item.path == *path)
                            .map(|m| m.item.clone())
                    }),
                    choices: if choice_relevant {
                        vec!["keep_in_place".into(), "carry_with_item".into()]
                    } else {
                        vec!["keep_in_place".into()]
                    },
                    purpose: DecisionPurpose::ReviewDefault,
                };
                result.plan.decisions.push(Decision { reason: DecisionReason::OrdinaryTriviaChoice, next_action: action.next_action(), action, id: format!("d/{}", result.plan.decisions.len()), category: "trivia_ownership".into(), anchors: vec![original_anchor], item_ids: target.into_iter().map(|i| selected[i].item.id.clone()).collect(), evidence: Vec::new(), unresolved_consequence: "ordinary ambiguous trivia stays in its original gap unless explicitly carried".into(), resolution: "choice_available".into(), supported_choices: if choice_relevant { vec!["keep_in_place".into(), "carry_with_item".into()] } else { vec!["keep_in_place".into()] }, selected_choice: Some(if target.is_some() { "carry_with_item" } else { "keep_in_place" }.into()), blocks_applicability: false, chain_diagnostic_ids: Vec::new(), lexical_uncertainty: None });
                result.account(descriptor_bytes(&result.plan.decisions.last())?)?;
            }
        }
    }
    if used.len() != overrides.len() {
        return Err(error(
            "INVALID_MOVE_TRIVIA_OVERRIDE",
            "unknown/stale trivia anchor",
            "trivia_overrides",
        ));
    }
    for (index, s) in selected.iter_mut().enumerate() {
        items::check(deadline, cancelled)?;
        s.runs
            .sort_by(|a, b| (&a.path, &a.range).cmp(&(&b.path, &b.range)));
        let mut merged: Vec<Run> = Vec::new();
        for run in s.runs.drain(..) {
            items::check(deadline, cancelled)?;
            if let Some(last) = merged.last_mut()
                && last.path == run.path
                && last.range.end_byte <= run.range.start_byte
                && files[&run.path].source[last.range.end_byte..run.range.start_byte]
                    .trim()
                    .is_empty()
            {
                last.range.end_byte = run.range.end_byte;
            } else {
                merged.push(run);
            }
        }
        let mut prefix = extra.remove(&index).unwrap_or_default();
        prefix.sort_by(|a, b| (&a.path, &a.range).cmp(&(&b.path, &b.range)));
        prefix.extend(merged);
        s.runs = prefix;
    }
    let mut consumed: Vec<_> = selected.iter().flat_map(|s| s.runs.iter()).collect();
    consumed.sort_by(|a, b| (&a.path, &a.range).cmp(&(&b.path, &b.range)));
    if consumed.windows(2).any(|pair| {
        pair[0].path == pair[1].path && pair[0].range.end_byte > pair[1].range.start_byte
    }) {
        result.blocker(
            "OVERLAPPING_EDITS",
            "consumed item/trivia runs overlap",
            None,
            None,
        );
    }
    Ok(())
}
fn ending(source: &str, at: usize) -> &'static str {
    if let Some(i) = source[..at].rfind('\n') {
        return if i > 0 && source.as_bytes()[i - 1] == b'\r' {
            "\r\n"
        } else {
            "\n"
        };
    }
    if let Some(i) = source[at..].find('\n')
        && i > 0
        && source.as_bytes()[at + i - 1] == b'\r'
    {
        return "\r\n";
    }
    "\n"
}
#[allow(clippy::too_many_arguments)] // Original target, default bytes and validated choice stay explicit for audit.
fn rewrite(
    request: &MoveRequest,
    used: &mut BTreeSet<usize>,
    result: &mut MoveEnvelope,
    target: RewriteTarget,
    kind: &str,
    text: &str,
    ids: &[String],
    validated_choice: Option<&str>,
) -> Result<(String, String), DomainError> {
    let mut after = text.to_owned();
    let mut action = "accept_default";
    let mut origin = if matches!(target, RewriteTarget::Source { .. }) {
        "copied"
    } else {
        "synthesized"
    };
    for (index, choice) in request
        .rewrite_overrides
        .as_deref()
        .unwrap_or_default()
        .iter()
        .enumerate()
    {
        if choice.target != target {
            continue;
        }
        if !used.insert(index)
            || request
                .rewrite_overrides
                .as_ref()
                .expect("overrides")
                .iter()
                .take(index)
                .any(|o| o.target == target)
        {
            return Err(error(
                "INVALID_REWRITE_OVERRIDE",
                "duplicate/conflicting synthesis target",
                "rewrite_overrides",
            ));
        }
        match choice.action {
            RewriteAction::AcceptDefault | RewriteAction::Retain => {
                if choice.replacement_text.is_some() {
                    return Err(error(
                        "INVALID_REWRITE_OVERRIDE",
                        "only replace accepts replacement_text",
                        "rewrite_overrides",
                    ));
                }
                if matches!(choice.action, RewriteAction::Retain) {
                    action = "retain";
                    after = match &target {
                        RewriteTarget::Source { anchor } => anchor.expected_text.clone(),
                        _ => String::new(),
                    };
                }
            }
            RewriteAction::Replace => {
                after = choice.replacement_text.clone().ok_or_else(|| {
                    error(
                        "INVALID_REWRITE_OVERRIDE",
                        "replace requires replacement_text",
                        "rewrite_overrides",
                    )
                })?;
                let requested = after.clone();
                if let Some(validated) = validated_choice {
                    after = validated.into();
                }
                if requested.len() > 64 * 1024
                    || (kind == "separator"
                        && !after
                            .bytes()
                            .all(|b| matches!(b, b' ' | b'\t' | b'\r' | b'\n')))
                    || (kind != "separator" && validated_choice.is_none() && after != text)
                {
                    return Err(error(
                        "INVALID_REWRITE_OVERRIDE",
                        "supported alternatives are whitespace separators or the same private ordinary declaration; no code/header/API injection",
                        "rewrite_overrides",
                    ));
                }
                origin = "caller_override";
                action = "replace";
            }
        }
    }
    if kind == "separator" && !after.contains('\n') {
        result.blocker("UNSUPPORTED_TRIVIA_DISPOSITION", "retained/replaced required synthesis does not provide a safe separate-line boundary/declaration", None, None);
    }
    let mut decision_ids = Vec::new();
    if action == "retain" && kind != "separator" {
        let category = match kind {
            "visibility" => "visibility_context",
            "module_declaration" => "module_context",
            _ => "unsupported_dependency_form",
        };
        let anchors = match &target {
            RewriteTarget::Source { anchor } => vec![anchor.clone()],
            RewriteTarget::Synthesis { items, .. } => items.clone(),
        };
        let id = format!("d/{}", result.plan.decisions.len());
        result.blocker(
            &category.to_uppercase(),
            "retained syntax leaves a proven relocation dependency/access unresolved",
            anchors.first().map(|a| a.path.as_str()),
            anchors.first().map(|a| a.range.clone()),
        );
        let route = DecisionAction::rewrite(target.clone());
        let decision = Decision { reason: DecisionReason::RequiredRewriteRetained, next_action: route.next_action(), action: route, id:id.clone(), category:category.into(), anchors, item_ids:ids.to_vec(), evidence:Vec::new(), unresolved_consequence:"the required binding/path/declaration/access is absent after the selected retain choice; all artifacts withheld".into(), resolution:"choice_available".into(), supported_choices:vec!["accept_default".into(), "replace".into()], selected_choice:Some("retain".into()), blocks_applicability:true, chain_diagnostic_ids:Vec::new(), lexical_uncertainty:None };
        result.account(descriptor_bytes(&decision)?)?;
        result.plan.decisions.push(decision);
        decision_ids.push(id);
    }
    let id = format!("r/{}", result.plan.rewrites.len());
    result.plan.rewrites.push(Rewrite {
        id: id.clone(),
        kind: kind.into(),
        item_ids: ids.to_vec(),
        decision_ids,
        anchors: Vec::new(),
        target,
        before_text: String::new(),
        after_text: after.clone(),
        origin: origin.into(),
        evidence: vec![
            "literal ordinary declaration or minimum line boundary; no semantic evidence".into(),
        ],
        rationale: if kind == "module_declaration" {
            "link the admitted new sibling through one private ordinary declaration in its validated parent"
        } else {
            "prevent a line comment/token boundary from swallowing or attaching inserted payload"
        }.into(),
        confidence: Confidence {
            basis: "syntactic_heuristic".into(),
            level: "high".into(),
            limitations: vec!["syntax/byte attachment only".into()],
        },
        default_action: "accept_default".into(),
        selected_action: action.into(),
        artifact_links: Vec::new(),
    });
    Ok((id, after))
}
fn add_separator(
    request: &MoveRequest,
    used: &mut BTreeSet<usize>,
    result: &mut MoveEnvelope,
    insertion: &mut Insertion,
    contributors: &[usize],
    selected: &[Selection],
    boundary: (&str, &str, String),
) -> Result<(), DomainError> {
    let (eol, role, identity) = boundary;
    let ids: Vec<_> = contributors
        .iter()
        .map(|i| selected[*i].item.id.clone())
        .collect();
    let target = RewriteTarget::Synthesis {
        path: insertion.path.clone(),
        slot: "separator".into(),
        items: contributors
            .iter()
            .map(|i| request.moves[*i].item.clone())
            .collect(),
        boundary_role: Some(role.into()),
        parent_path: None,
        binding: Some(identity),
    };
    let (id, text) = rewrite(request, used, result, target, "separator", eol, &ids, None)?;
    let start = insertion.text.len();
    insertion.text.push_str(&text);
    insertion
        .rewrites
        .push((id, range(start, insertion.text.len())));
    Ok(())
}
fn copy_run(
    insertion: &mut Insertion,
    files: &BTreeMap<String, FileSnapshot>,
    path: &str,
    start: usize,
    end: usize,
) {
    if start == end {
        return;
    }
    let at = insertion.text.len();
    insertion.text.push_str(&files[path].source[start..end]);
    insertion.copies.push(MoveOrigin {
        id: String::new(),
        source_path: path.into(),
        source_range: range(start, end),
        output_path: insertion.path.clone(),
        output_range: range(at, insertion.text.len()),
        role: "item".into(),
    });
}
#[allow(clippy::too_many_arguments)] // The original-coordinate assembly has one immutable corpus.
fn assemble(
    request: &MoveRequest,
    files: &BTreeMap<String, FileSnapshot>,
    parsed: &BTreeMap<String, ParsedFile>,
    selected: &[Selection],
    creations: &mut BTreeMap<String, Creation>,
    repairs: &[Repair],
    controls: (Instant, &AtomicBool),
    result: &mut MoveEnvelope,
) -> Result<(), DomainError> {
    let (deadline, cancelled) = controls;
    let mut used = BTreeSet::new();
    let mut authored = Vec::new();
    for repair in repairs {
        items::check(deadline, cancelled)?;
        let (id, after) = rewrite(
            request,
            &mut used,
            result,
            repair.target.clone(),
            repair.kind,
            &repair.after,
            &repair.item_ids,
            repair.caller_override.then_some(repair.after.as_str()),
        )?;
        let audit = result.plan.rewrites.last_mut().expect("repair audit");
        audit.before_text = files
            .get(&repair.path)
            .map(|f| f.source[repair.range.start_byte..repair.range.end_byte].to_owned())
            .unwrap_or_default();
        audit.anchors = repair.anchors.clone();
        audit.rationale = repair.rationale.clone();
        audit.evidence = repair
            .anchors
            .iter()
            .map(|a| format!("{}:{}..{}", a.path, a.range.start_byte, a.range.end_byte))
            .collect();
        if repair.caller_override {
            audit.origin = "caller_override".into();
        }
        if let Some(path) = &repair.declaration_for {
            creations
                .get_mut(path)
                .expect("proposed declaration")
                .visibility_id = Some(id.clone());
        }
        authored.push((repair, id, after));
    }
    let mut groups: BTreeMap<(String, usize), Vec<usize>> = BTreeMap::new();
    for (index, s) in selected.iter().enumerate() {
        items::check(deadline, cancelled)?;
        let at = match &s.destination {
            Destination::Existing { path, before_item } => {
                if let Some(before) = before_item {
                    parsed[path]
                        .trivia
                        .iter()
                        .filter(|t| {
                            t.owned_by(before.range.start_byte..before.range.end_byte)
                                && t.range.end <= before.range.start_byte
                        })
                        .map(|t| t.range.start)
                        .min()
                        .unwrap_or(before.range.start_byte)
                } else {
                    files[path].source.len()
                }
            }
            Destination::NewSibling { .. } => 0,
        };
        groups
            .entry((s.destination.path().into(), at))
            .or_default()
            .push(index);
    }
    let mut declarations: BTreeMap<(String, usize), Vec<String>> = BTreeMap::new();
    for (path, creation) in creations.iter() {
        items::check(deadline, cancelled)?;
        if creation.link.is_none() {
            declarations
                .entry((
                    creation.parent.clone(),
                    files[&creation.parent].source.len(),
                ))
                .or_default()
                .push(path.clone());
        }
    }
    let mut imports: BTreeMap<(String, usize), Vec<usize>> = BTreeMap::new();
    for (index, (repair, _, _)) in authored.iter().enumerate() {
        if repair.kind == "import_insert"
            && !(repair.import_scope.is_some()
                && selected.iter().any(|s| {
                    s.runs.iter().any(|run| {
                        run.path == repair.path
                            && run.range.start_byte <= repair.range.start_byte
                            && run.range.end_byte >= repair.range.end_byte
                    })
                }))
        {
            imports
                .entry((repair.path.clone(), repair.range.start_byte))
                .or_default()
                .push(index);
        }
    }
    let keys: BTreeSet<_> = groups
        .keys()
        .chain(declarations.keys())
        .chain(imports.keys())
        .cloned()
        .collect();
    let mut insertions = Vec::new();
    for (path, at) in keys {
        items::check(deadline, cancelled)?;
        let key = (path.clone(), at);
        let mut indexes = groups.remove(&key).unwrap_or_default();
        let mut first_sources = BTreeMap::new();
        for (position, index) in indexes.iter().enumerate() {
            items::check(deadline, cancelled)?;
            first_sources
                .entry(selected[*index].source.clone())
                .or_insert(position);
        }
        indexes.sort_by_key(|i| {
            (
                first_sources[&selected[*i].source],
                selected[*i].item.span.range.start_byte,
            )
        });
        let original = files.get(&path).map(|f| f.source.as_str()).unwrap_or("");
        let eol = if files.contains_key(&path) {
            ending(original, at)
        } else {
            indexes
                .first()
                .map(|i| {
                    ending(
                        &files[&selected[*i].source].source,
                        selected[*i].item.span.range.start_byte,
                    )
                })
                .unwrap_or("\n")
        };
        let mut insertion = Insertion {
            path: path.clone(),
            at,
            text: String::new(),
            copies: Vec::new(),
            rewrites: Vec::new(),
            item_ids: Vec::new(),
        };
        let mut import_indexes = imports.remove(&key).unwrap_or_default();
        import_indexes.sort_by_key(|i| authored[*i].2.clone());
        for index in import_indexes {
            let (repair, id, text) = &authored[index];
            let contributors: Vec<_> = selected
                .iter()
                .enumerate()
                .filter(|(_, s)| repair.item_ids.contains(&s.item.id))
                .map(|(i, _)| i)
                .collect();
            if !text.is_empty()
                && ((!insertion.text.is_empty() && !insertion.text.ends_with('\n'))
                    || (insertion.text.is_empty() && at > 0 && !original[..at].ends_with('\n')))
            {
                add_separator(
                    request,
                    &mut used,
                    result,
                    &mut insertion,
                    &contributors,
                    selected,
                    (eol, "before_payload", format!("import:{}", repair.after)),
                )?;
            }
            let start = insertion.text.len();
            insertion.text.push_str(text);
            insertion
                .rewrites
                .push((id.clone(), range(start, insertion.text.len())));
            insertion.item_ids.extend(repair.item_ids.clone());
            if !text.is_empty() {
                add_separator(
                    request,
                    &mut used,
                    result,
                    &mut insertion,
                    &contributors,
                    selected,
                    (eol, "after_payload", format!("import:{}", repair.after)),
                )?;
            }
        }
        for created_path in declarations.remove(&key).unwrap_or_default() {
            items::check(deadline, cancelled)?;
            let creation = &creations[&created_path];
            if (!insertion.text.is_empty() && !insertion.text.ends_with('\n'))
                || (insertion.text.is_empty() && at > 0 && !original[..at].ends_with('\n'))
            {
                add_separator(
                    request,
                    &mut used,
                    result,
                    &mut insertion,
                    &creation.selections,
                    selected,
                    (eol, "before_payload", format!("declaration:{created_path}")),
                )?;
            }
            let ids: Vec<_> = creation
                .selections
                .iter()
                .map(|i| selected[*i].item.id.clone())
                .collect();
            let target = RewriteTarget::Synthesis {
                path: path.clone(),
                slot: "module_declaration".into(),
                items: creation
                    .selections
                    .iter()
                    .map(|i| request.moves[*i].item.clone())
                    .collect(),
                boundary_role: None,
                parent_path: Some(path.clone()),
                binding: None,
            };
            if let Some((_, id, text)) = authored
                .iter()
                .find(|(r, _, _)| r.declaration_for.as_deref() == Some(&created_path))
            {
                let start = insertion.text.len();
                insertion.text.push_str(text);
                insertion
                    .rewrites
                    .push((id.clone(), range(start, insertion.text.len())));
            }
            let name = items::module_name(&created_path)?;
            let (id, text) = rewrite(
                request,
                &mut used,
                result,
                target,
                "module_declaration",
                &format!("mod {name};"),
                &ids,
                None,
            )?;
            let start = insertion.text.len();
            insertion.text.push_str(&text);
            insertion
                .rewrites
                .push((id.clone(), range(start, insertion.text.len())));
            insertion.item_ids.extend(ids);
            creations.get_mut(&created_path).expect("creation").link =
                Some(DeclarationLink::Synthesized { rewrite_id: id });
        }
        for index in indexes.iter().copied() {
            items::check(deadline, cancelled)?;
            let s = &selected[index];
            insertion.item_ids.push(s.item.id.clone());
            for run in &s.runs {
                items::check(deadline, cancelled)?;
                if (!insertion.text.is_empty() && !insertion.text.ends_with('\n'))
                    || (insertion.text.is_empty() && at > 0 && !original[..at].ends_with('\n'))
                {
                    add_separator(
                        request,
                        &mut used,
                        result,
                        &mut insertion,
                        &[index],
                        selected,
                        (
                            eol,
                            "before_payload",
                            format!(
                                "run:{}:{}:{}",
                                run.path, run.range.start_byte, run.range.end_byte
                            ),
                        ),
                    )?;
                }
                let mut cursor = run.range.start_byte;
                let mut internal: Vec<_> = authored
                    .iter()
                    .filter(|(r, _, _)| {
                        (r.kind != "import_insert" || r.import_scope.is_some())
                            && r.path == run.path
                            && r.range.start_byte >= run.range.start_byte
                            && r.range.end_byte <= run.range.end_byte
                    })
                    .collect();
                internal.sort_by_key(|(r, _, _)| r.range.clone());
                for (repair, id, text) in internal {
                    items::check(deadline, cancelled)?;
                    copy_run(
                        &mut insertion,
                        files,
                        &run.path,
                        cursor,
                        repair.range.start_byte,
                    );
                    let local_eol =
                        if repair.kind == "import_insert" && insertion.text.contains('\n') {
                            ending(&insertion.text, insertion.text.len())
                        } else {
                            eol
                        };
                    if repair.kind == "import_insert"
                        && !text.is_empty()
                        && !insertion.text.ends_with('\n')
                    {
                        add_separator(
                            request,
                            &mut used,
                            result,
                            &mut insertion,
                            &[index],
                            selected,
                            (
                                local_eol,
                                "before_payload",
                                format!("local-import:{}:{}", repair.path, repair.range.start_byte),
                            ),
                        )?;
                    }
                    let start = insertion.text.len();
                    insertion.text.push_str(text);
                    insertion
                        .rewrites
                        .push((id.clone(), range(start, insertion.text.len())));
                    if repair.kind == "import_insert" && !text.is_empty() {
                        add_separator(
                            request,
                            &mut used,
                            result,
                            &mut insertion,
                            &[index],
                            selected,
                            (
                                local_eol,
                                "after_payload",
                                format!("local-import:{}:{}", repair.path, repair.range.start_byte),
                            ),
                        )?;
                    }
                    cursor = repair.range.end_byte;
                }
                copy_run(&mut insertion, files, &run.path, cursor, run.range.end_byte);
            }
        }
        if at < original.len() && !insertion.text.is_empty() && !insertion.text.ends_with('\n') {
            add_separator(
                request,
                &mut used,
                result,
                &mut insertion,
                &indexes,
                selected,
                (eol, "after_payload", format!("insertion:{at}")),
            )?;
        }
        insertions.push(insertion);
    }
    let mut absorbed: BTreeSet<_> = authored
        .iter()
        .filter(|(r, _, _)| {
            r.declaration_for.as_ref().is_some_and(|p| {
                matches!(creations[p].link, Some(DeclarationLink::Synthesized { .. }))
            })
        })
        .map(|(_, id, _)| id.clone())
        .collect();
    for (repair, id, text) in &authored {
        if repair.kind != "import_insert"
            && !absorbed.contains(id)
            && repair.range.start_byte == repair.range.end_byte
            && !selected.iter().any(|s| {
                s.runs.iter().any(|run| {
                    run.path == repair.path
                        && run.range.start_byte <= repair.range.start_byte
                        && repair.range.start_byte < run.range.end_byte
                })
            })
            && let Some(insertion) = insertions
                .iter_mut()
                .find(|i| i.path == repair.path && i.at == repair.range.start_byte)
        {
            let start = insertion.text.len();
            insertion.text.push_str(text);
            insertion
                .rewrites
                .push((id.clone(), range(start, insertion.text.len())));
            absorbed.insert(id.clone());
        }
    }
    if used.len()
        != request
            .rewrite_overrides
            .as_deref()
            .unwrap_or_default()
            .len()
    {
        return Err(error(
            "INVALID_REWRITE_OVERRIDE",
            "unknown/stale synthesis/source target; replay a complete target from this request's audit",
            "rewrite_overrides",
        ));
    }
    let mut edits = Vec::new();
    for s in selected {
        for run in &s.runs {
            items::check(deadline, cancelled)?;
            let mut deletion = Edit::new(
                &run.path,
                &files[&run.path].source,
                run.range.start_byte,
                run.range.end_byte,
                String::new(),
                "",
            );
            deletion.match_ids.clear();
            deletion.item_ids.push(s.item.id.clone());
            deletion.trivia_ids = result
                .plan
                .trivia_decisions
                .iter()
                .filter(|t| {
                    t.path == run.path
                        && t.span.range.start_byte >= run.range.start_byte
                        && t.span.range.end_byte <= run.range.end_byte
                })
                .map(|t| t.id.clone())
                .collect();
            edits.push(deletion);
        }
    }
    for (repair, id, text) in &authored {
        if repair.kind == "import_insert"
            || absorbed.contains(id)
            || selected.iter().any(|s| {
                s.runs.iter().any(|run| {
                    run.path == repair.path
                        && repair.range.start_byte >= run.range.start_byte
                        && repair.range.end_byte <= run.range.end_byte
                })
            })
        {
            continue;
        }
        let mut edit = Edit::new(
            &repair.path,
            &files[&repair.path].source,
            repair.range.start_byte,
            repair.range.end_byte,
            text.clone(),
            "",
        );
        edit.match_ids.clear();
        edit.item_ids = repair.item_ids.clone();
        edit.rewrite_ids.push(id.clone());
        edits.push(edit);
    }
    // Sort deletions first; any consumed overlap already has a blocker. No arbitrary folding.
    if edit::sort_validate(&mut edits).is_err() {
        result.blocker("OVERLAPPING_EDITS", "consumed runs overlap", None, None);
        return Ok(());
    }
    let mut insertion_edits: BTreeMap<(String, usize), usize> = BTreeMap::new();
    for (index, insertion) in insertions.iter().enumerate() {
        items::check(deadline, cancelled)?;
        if !files.contains_key(&insertion.path) {
            continue;
        }
        let related = edits.iter().position(|e| {
            e.path == insertion.path
                && e.replacement_text.is_empty()
                && (e.range.start_byte == insertion.at || e.range.end_byte == insertion.at)
        });
        let edit_at;
        if let Some(deletion_index) = related {
            let deletion = &mut edits[deletion_index];
            edit_at = deletion.range.start_byte;
            deletion.replacement_text = insertion.text.clone();
            deletion.item_ids.extend(insertion.item_ids.clone());
        } else {
            edit_at = insertion.at;
            let mut edit = Edit::new(
                &insertion.path,
                &files[&insertion.path].source,
                insertion.at,
                insertion.at,
                insertion.text.clone(),
                "",
            );
            edit.match_ids.clear();
            edit.item_ids = insertion.item_ids.clone();
            edits.push(edit);
        }
        insertion_edits.insert((insertion.path.clone(), edit_at), index);
    }
    if let Err(e) = edit::sort_validate(&mut edits) {
        result.blocker(&e.code, &e.message, None, None);
        return Ok(());
    }
    let affected: BTreeSet<_> = edits.iter().map(|e| e.path.clone()).collect();
    let mut outputs: BTreeMap<String, (String, Vec<trivia::Trivia>)> = BTreeMap::new();
    let mut originals = BTreeMap::new();
    let mut sections = BTreeMap::new();
    for path in &affected {
        items::check(deadline, cancelled)?;
        let file = &files[path];
        if !trivia::move_clean(&parsed[path].tree, deadline, cancelled)? {
            result.blocker(
                "PREEXISTING_SYNTAX_ERROR",
                "whole modified file has ERROR/MISSING; repair source first",
                Some(path),
                None,
            );
        }
        let relevant: Vec<_> = edits.iter().filter(|e| e.path == *path).cloned().collect();
        let content = edit::reconstruct(&file.source, &relevant)?;
        let tree = trivia::parse(&content, deadline, cancelled)?
            .ok_or_else(|| DomainError::new("planning_deadline", "virtual parse stopped"))?;
        if !trivia::move_clean(&tree, deadline, cancelled)? {
            result.blocker(
                "NEW_SYNTAX_ERROR",
                "virtual file has ERROR/MISSING",
                Some(path),
                None,
            );
        }
        outputs.insert(
            path.clone(),
            (
                content.clone(),
                trivia::move_inventory(&tree, &content, deadline, cancelled)?,
            ),
        );
        originals.insert(
            path.clone(),
            (
                file.source.clone(),
                trivia::move_inventory(&parsed[path].tree, &file.source, deadline, cancelled)?,
            ),
        );
        let mut original_at = 0;
        let mut output_at = 0;
        for (edit_index, e) in edits
            .iter_mut()
            .enumerate()
            .filter(|(_, e)| e.path == *path)
        {
            items::check(deadline, cancelled)?;
            if original_at < e.range.start_byte {
                result.plan.origins.push(MoveOrigin {
                    id: String::new(),
                    source_path: path.clone(),
                    source_range: range(original_at, e.range.start_byte),
                    output_path: path.clone(),
                    output_range: range(output_at, output_at + e.range.start_byte - original_at),
                    role: "gap".into(),
                });
            }
            output_at += e.range.start_byte - original_at;
            if let Some(index) = insertion_edits.get(&(path.clone(), e.range.start_byte)) {
                for copy in &insertions[*index].copies {
                    items::check(deadline, cancelled)?;
                    let mut copy = copy.clone();
                    copy.output_range.start_byte += output_at;
                    copy.output_range.end_byte += output_at;
                    result.plan.origins.push(copy);
                }
                for (id, local_range) in &insertions[*index].rewrites {
                    items::check(deadline, cancelled)?;
                    e.rewrite_ids.push(id.clone());
                    result
                        .plan
                        .rewrites
                        .iter_mut()
                        .find(|r| r.id == *id)
                        .expect("rewrite")
                        .artifact_links
                        .push(ArtifactLink::Edit {
                            index: edit_index,
                            replacement_range: local_range.clone(),
                        });
                }
            }
            for id in &e.rewrite_ids {
                let audit = result
                    .plan
                    .rewrites
                    .iter_mut()
                    .find(|r| r.id == *id)
                    .expect("audit");
                if audit.artifact_links.is_empty() {
                    audit.artifact_links.push(ArtifactLink::Edit {
                        index: edit_index,
                        replacement_range: range(0, e.replacement_text.len()),
                    });
                }
            }
            output_at += e.replacement_text.len();
            original_at = e.range.end_byte;
        }
        if original_at < file.source.len() {
            result.plan.origins.push(MoveOrigin {
                id: String::new(),
                source_path: path.clone(),
                source_range: range(original_at, file.source.len()),
                output_path: path.clone(),
                output_range: range(output_at, output_at + file.source.len() - original_at),
                role: "gap".into(),
            });
        }
        result.plan.base_files.push(BaseFile {
            path: path.clone(),
            original_length: file.source.len(),
            mode: format!("{:o}", file.mode),
        });
        sections.insert(path.clone(), patch::section(path, &file.source, &content)?);
    }
    let mut created = Vec::new();
    for (path, creation) in creations.iter() {
        items::check(deadline, cancelled)?;
        let insertion = insertions
            .iter()
            .find(|i| i.path == *path)
            .expect("creation insertion");
        let content = &insertion.text;
        let tree = trivia::parse(content, deadline, cancelled)?
            .ok_or_else(|| DomainError::new("planning_deadline", "creation parse stopped"))?;
        if !trivia::move_clean(&tree, deadline, cancelled)? {
            result.blocker(
                "NEW_SYNTAX_ERROR",
                "created content has ERROR/MISSING",
                Some(path),
                None,
            );
        }
        outputs.insert(
            path.clone(),
            (
                content.clone(),
                trivia::move_inventory(&tree, content, deadline, cancelled)?,
            ),
        );
        result.plan.origins.extend(insertion.copies.clone());
        let id = format!("c/{path}");
        for (rewrite_id, local_range) in &insertion.rewrites {
            items::check(deadline, cancelled)?;
            result
                .plan
                .rewrites
                .iter_mut()
                .find(|r| r.id == *rewrite_id)
                .expect("rewrite")
                .artifact_links
                .push(ArtifactLink::CreatedFile {
                    id: id.clone(),
                    content_range: local_range.clone(),
                });
        }
        sections.insert(path.clone(), patch::creation_section(path, content)?);
        created.push(CreatedFile {
            id,
            path: path.clone(),
            must_be_absent: true,
            mode: "100644".into(),
            content: content.clone(),
            parent_path: creation.parent.clone(),
            declaration_link: creation.link.clone().expect("linked declaration"),
            declaration_visibility_rewrite_id: creation.visibility_id.clone(),
            item_ids: insertion.item_ids.clone(),
            trivia_ids: Vec::new(),
            rewrite_ids: insertion
                .rewrites
                .iter()
                .map(|(id, _)| id.clone())
                .collect(),
            origins: Vec::new(),
        });
    }
    items::check(deadline, cancelled)?;
    if !trivia::verify_move(
        &originals,
        &outputs,
        &result.plan.origins,
        deadline,
        cancelled,
    ) {
        result.blocker("ATTRIBUTE_ATTACHMENT_CHANGED", "path-qualified exact-once copied trivia/item bytes or protected owners did not survive reparsing", None, None);
    }
    items::check(deadline, cancelled)?;
    for (index, origin) in result.plan.origins.iter_mut().enumerate() {
        items::check(deadline, cancelled)?;
        origin.id = format!("o/{index}");
    }
    for record in &mut result.plan.moves {
        for origin in &result.plan.origins {
            items::check(deadline, cancelled)?;
            if let Some((path, r)) = origin.mapped(
                &record.item.path,
                &(record.item.span.range.start_byte..record.item.span.range.end_byte),
            ) && path == record.destination.path()
            {
                record.output_range = Some(range(r.start, r.end));
            }
            if record
                .carried_spans
                .iter()
                .any(|s| s.path == origin.source_path && s.span.range == origin.source_range)
            {
                record.origin_ids.push(origin.id.clone());
            }
        }
        record.decision_ids = result
            .plan
            .decisions
            .iter()
            .filter(|d| d.item_ids.contains(&record.id))
            .map(|d| d.id.clone())
            .collect();
        record.rewrite_ids = result
            .plan
            .rewrites
            .iter()
            .filter(|r| r.item_ids.contains(&record.id))
            .map(|r| r.id.clone())
            .collect();
    }
    for decision in &mut result.plan.trivia_decisions {
        items::check(deadline, cancelled)?;
        decision.origin_ids = result
            .plan
            .origins
            .iter()
            .filter(|o| {
                o.mapped(
                    &decision.path,
                    &(decision.span.range.start_byte..decision.span.range.end_byte),
                )
                .is_some()
            })
            .map(|o| o.id.clone())
            .collect();
    }
    for file in &mut created {
        items::check(deadline, cancelled)?;
        file.origins = result
            .plan
            .origins
            .iter()
            .filter(|o| o.output_path == file.path)
            .cloned()
            .collect();
        file.trivia_ids = result
            .plan
            .trivia_decisions
            .iter()
            .filter(|t| {
                t.origin_ids
                    .iter()
                    .any(|id| file.origins.iter().any(|o| &o.id == id))
            })
            .map(|t| t.id.clone())
            .collect();
    }
    // IDs were request-local while assembling; sort final audits deterministically and remap all links.
    items::check(deadline, cancelled)?;
    sort_rewrites(result, &mut edits, &mut created);
    items::check(deadline, cancelled)?;
    result.counts.creations = created.len();
    result.counts.rewrites = result.plan.rewrites.len();
    if result.plan.blockers.is_empty() {
        result.plan.edits = Some(edits);
        result.plan.created_files = Some(created);
        result.plan.patch = Some(sections.into_values().collect());
    }
    Ok(())
}
fn sort_rewrites(result: &mut MoveEnvelope, edits: &mut [Edit], created: &mut [CreatedFile]) {
    for rewrite in &mut result.plan.rewrites {
        rewrite.item_ids.sort();
        rewrite.item_ids.dedup();
    }
    result.plan.rewrites.sort_by_key(|r| {
        let destination = match r.artifact_links.first() {
            Some(ArtifactLink::Edit { index, .. }) => edits[*index].path.clone(),
            Some(ArtifactLink::CreatedFile { id, .. }) => created
                .iter()
                .find(|f| &f.id == id)
                .expect("created artifact")
                .path
                .clone(),
            None => match &r.target {
                RewriteTarget::Source { anchor } => anchor.path.clone(),
                RewriteTarget::Synthesis { path, .. } => path.clone(),
            },
        };
        let original = match &r.target {
            RewriteTarget::Source { anchor } => (false, anchor.path.clone(), anchor.range.clone()),
            RewriteTarget::Synthesis { .. } => (true, String::new(), range(0, 0)),
        };
        (
            destination,
            r.kind.clone(),
            original,
            serde_json::to_string(&r.target).expect("target JSON"),
            r.item_ids.clone(),
        )
    });
    let ids: BTreeMap<_, _> = result
        .plan
        .rewrites
        .iter()
        .enumerate()
        .map(|(i, r)| (r.id.clone(), format!("r/{i}")))
        .collect();
    for r in &mut result.plan.rewrites {
        r.id = ids[&r.id].clone();
    }
    for e in edits {
        for id in &mut e.rewrite_ids {
            *id = ids[id].clone();
        }
    }
    for record in &mut result.plan.moves {
        for id in &mut record.rewrite_ids {
            *id = ids[id].clone();
        }
    }
    for file in created {
        for id in &mut file.rewrite_ids {
            *id = ids[id].clone();
        }
        if let Some(id) = &mut file.declaration_visibility_rewrite_id {
            *id = ids[id].clone();
        }
        if let DeclarationLink::Synthesized { rewrite_id } = &mut file.declaration_link {
            *rewrite_id = ids[rewrite_id].clone();
        }
    }
}

#[cfg(test)]
#[path = "move_plan_tests.rs"]
mod tests;
