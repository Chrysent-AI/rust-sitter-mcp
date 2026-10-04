//! Complete read-only replacement planning over the same source/matching pipeline.
use crate::{
    edit::{self, Edit},
    matching::{self, Candidate},
    patch,
    pattern::Pattern,
    result::*,
    scope::{self, Scope},
    template::Template,
};
use rmcp::schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, PartialOrd, Ord)]
#[schemars(crate = "rmcp::schemars")]
#[serde(deny_unknown_fields)]
pub struct SourceAnchor {
    pub path: String,
    pub range: ByteRange,
    pub expected_text: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(rename_all = "snake_case")]
pub enum Disposition {
    KeepInPlace,
    BeforeMatch,
    AfterMatch,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(deny_unknown_fields)]
pub struct TriviaOverride {
    pub trivia: SourceAnchor,
    pub disposition: Disposition,
    pub target_match: Option<SourceAnchor>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(deny_unknown_fields)]
pub struct ReplaceRequest {
    pub repo_path: String,
    /// One supported sugar expression. Written syntax only, no semantics or expanded macros.
    pub pattern: String,
    /// Exact expression template: bound $name copies original bytes. Dollars in literals/comments are literal; no $$, ${name}, sequences, formatting or automatic parentheses.
    pub replacement: String,
    pub paths: Option<Vec<String>>,
    pub globs: Option<Vec<String>>,
    #[serde(default)]
    pub context: Context,
    #[serde(default)]
    pub limits: Limits,
    /// Omitted selects all scope matches; [] selects none. Unique original-byte anchors and exact expected_text required.
    pub selection: Option<Vec<SourceAnchor>>,
    pub trivia_overrides: Option<Vec<TriviaOverride>>,
    /// Counts accepted scope matches BEFORE selection. Default 500, maximum 5000. Exceeding it withholds all artifacts even for a tiny explicit selection.
    #[serde(default = "default_max_matches")]
    pub max_matches: usize,
}
fn default_max_matches() -> usize {
    500
}
impl ReplaceRequest {
    fn scan_request(&self) -> SearchRequest {
        SearchRequest {
            repo_path: self.repo_path.clone(),
            query: self.pattern.clone(),
            paths: self.paths.clone(),
            globs: self.globs.clone(),
            context: self.context.clone(),
            limits: self.limits.clone(),
            page_size: 100,
            cursor: None,
        }
    }
}
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct Blocker {
    pub code: String,
    pub message: String,
    pub path: Option<String>,
    pub range: Option<ByteRange>,
}
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct Integrity {
    pub grammar: String,
    pub syntax: String,
    pub semantic: String,
}
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct BaseFile {
    pub path: String,
    pub original_length: usize,
    pub mode: String,
}
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct Preview {
    pub applicable: bool,
    pub path: String,
    pub range: ByteRange,
    pub original_text: Option<String>,
    pub replacement_text: Option<String>,
    pub original_text_omitted: bool,
    pub replacement_text_omitted: bool,
}
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct Plan {
    pub state: String,
    pub applicable: bool,
    pub selected_count: Option<usize>,
    pub matches: Vec<MatchRecord>,
    pub matches_omitted: usize,
    pub trivia_decisions: Vec<TriviaDecision>,
    pub trivia_decisions_omitted: usize,
    pub blockers: Vec<Blocker>,
    pub blockers_omitted: usize,
    pub edits: Option<Vec<Edit>>,
    pub patch: Option<String>,
    pub base_files: Vec<BaseFile>,
    pub proposed_edits: Vec<Preview>,
    pub proposed_edits_omitted: usize,
    pub integrity: Integrity,
}
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct PlanEnvelope {
    pub schema_version: u8,
    pub tool: String,
    pub root: Option<String>,
    pub snapshot_id: Option<String>,
    pub status: String,
    pub coverage: Coverage,
    pub counts: Counts,
    pub limits: Limits,
    pub truncation_reasons: Vec<String>,
    pub skipped: Skipped,
    pub diagnostics: Vec<Diagnostic>,
    pub diagnostics_omitted: usize,
    pub error: Option<DomainError>,
    pub plan: Plan,
}
impl PlanEnvelope {
    pub fn empty(limits: Limits) -> Self {
        Self {
            schema_version: 1,
            tool: "replace".into(),
            root: None,
            snapshot_id: None,
            status: "complete".into(),
            coverage: Coverage::default(),
            counts: Counts::default(),
            limits,
            truncation_reasons: Vec::new(),
            skipped: skipped_map(),
            diagnostics: Vec::new(),
            diagnostics_omitted: 0,
            error: None,
            plan: Plan {
                state: "blocked".into(),
                applicable: false,
                selected_count: None,
                matches: Vec::new(),
                matches_omitted: 0,
                trivia_decisions: Vec::new(),
                trivia_decisions_omitted: 0,
                blockers: Vec::new(),
                blockers_omitted: 0,
                edits: None,
                patch: None,
                base_files: Vec::new(),
                proposed_edits: Vec::new(),
                proposed_edits_omitted: 0,
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
    pub fn blocker(
        &mut self,
        code: &str,
        message: &str,
        path: Option<&str>,
        range: Option<ByteRange>,
    ) {
        if self.plan.blockers.len() < self.limits.diagnostic_count.max(1) {
            self.plan.blockers.push(Blocker {
                code: code.into(),
                message: message.into(),
                path: path.map(str::to_owned),
                range,
            });
        } else {
            self.plan.blockers_omitted += 1;
        }
    }
    fn incomplete(&mut self, code: &str) {
        self.status = "partial".into();
        self.coverage.scope_exhaustive = false;
        self.plan.state = "incomplete".into();
        self.plan.applicable = false;
        self.plan.edits = None;
        self.plan.patch = None;
        self.truncation_reasons.push(code.into());
        self.blocker(
            code,
            "complete artifacts withheld; narrow scope/selection or increase limits",
            None,
            None,
        );
    }
    pub fn wire_bytes(&self) -> usize {
        let value = serde_json::to_value(self).expect("serializable plan");
        serde_json::to_vec(&serde_json::json!({"content":[{"type":"text","text":value.to_string()}],"structuredContent":value,"isError":self.error.is_some()})).expect("serializable wire").len() + 4096
    }
    fn preview(&mut self, edits: &[Edit]) {
        for edit in edits {
            let original_omitted = edit.original_text.len() > self.limits.text_bytes;
            let replacement_omitted = edit.replacement_text.len() > self.limits.text_bytes;
            self.plan.proposed_edits.push(Preview {
                applicable: false,
                path: edit.path.clone(),
                range: edit.range.clone(),
                original_text: (!original_omitted).then(|| edit.original_text.clone()),
                replacement_text: (!replacement_omitted).then(|| edit.replacement_text.clone()),
                original_text_omitted: original_omitted,
                replacement_text_omitted: replacement_omitted,
            });
        }
    }
    fn fit(&mut self) {
        if self.wire_bytes() > self.limits.response_bytes && self.plan.applicable {
            let edits = self.plan.edits.take().unwrap_or_default();
            self.incomplete("response_bytes");
            self.preview(&edits);
        }
        while self.wire_bytes() > self.limits.response_bytes && !self.plan.matches.is_empty() {
            self.plan.matches.pop();
            self.plan.matches_omitted += 1;
        }
        while self.wire_bytes() > self.limits.response_bytes && !self.plan.proposed_edits.is_empty()
        {
            self.plan.proposed_edits.pop();
            self.plan.proposed_edits_omitted += 1;
        }
        while self.wire_bytes() > self.limits.response_bytes
            && !self.plan.trivia_decisions.is_empty()
        {
            self.plan.trivia_decisions.pop();
            self.plan.trivia_decisions_omitted += 1;
        }
        while self.wire_bytes() > self.limits.response_bytes && !self.diagnostics.is_empty() {
            self.diagnostics.pop();
            self.diagnostics_omitted += 1;
        }
        if self.wire_bytes() > self.limits.response_bytes {
            self.plan.base_files.clear();
            for reason in self.skipped.values_mut() {
                reason.examples_omitted += reason.examples.len() as u64;
                reason.examples.clear();
            }
        }
        while self.wire_bytes() > self.limits.response_bytes && self.plan.blockers.len() > 1 {
            self.plan.blockers.pop();
            self.plan.blockers_omitted += 1;
        }
        self.counts.returned_matches = self.plan.matches.len();
    }
}

pub fn run(launch: &Path, request: ReplaceRequest, cancelled: &AtomicBool) -> PlanEnvelope {
    run_with_recheck(launch, request, cancelled, || {})
}
fn run_with_recheck(
    launch: &Path,
    request: ReplaceRequest,
    cancelled: &AtomicBool,
    before_recheck: impl FnOnce(),
) -> PlanEnvelope {
    let mut result = PlanEnvelope::empty(request.limits.clone());
    let started = Instant::now();
    if let Err(error) = build(launch, &request, cancelled, before_recheck, &mut result) {
        result.status = "failed".into();
        result.error = Some(error);
        result.plan.applicable = false;
        result.plan.state = "blocked".into();
        result.plan.edits = None;
        result.plan.patch = None;
    }
    result.fit();
    tracing::info!(tool="replace", root=?result.root, elapsed_ms=started.elapsed().as_millis(), status=%result.status, state=%result.plan.state, selected=?result.plan.selected_count, "replacement plan finished");
    result
}
fn build(
    launch: &Path,
    request: &ReplaceRequest,
    cancelled: &AtomicBool,
    before_recheck: impl FnOnce(),
    result: &mut PlanEnvelope,
) -> Result<(), DomainError> {
    request.limits.validate()?;
    if !(1..=5000).contains(&request.max_matches)
        || request.context.before_lines > 20
        || request.context.after_lines > 20
    {
        return Err(DomainError::new(
            "INVALID_PARAMS",
            "max_matches is 1–5000; context is 0–20 lines",
        ));
    }
    let compiled = Pattern::compile(&request.pattern)?;
    let template = Template::new(&request.replacement, &compiled)?;
    let mut anchors = BTreeSet::new();
    if let Some(selection) = &request.selection {
        for anchor in selection {
            if !anchors.insert(anchor.clone()) {
                return Err(DomainError::new(
                    "STALE_SELECTION",
                    "selection anchors must be unique",
                ));
            }
        }
    }
    let deadline = Instant::now() + Duration::from_millis(request.limits.time_budget_ms);
    let root = scope::resolve(&request.repo_path, launch)?;
    result.root = Some(root.to_str().expect("UTF-8 root").into());
    let scope = Scope::new(root, &request.scan_request())?;
    let mut scan = SearchEnvelope::empty(request.limits.clone());
    let (files, snapshot) = scope::discover(&scope, &mut scan, deadline, cancelled)?;
    result.snapshot_id = snapshot.clone();
    result.coverage = scan.coverage;
    result.counts = scan.counts;
    result.skipped = scan.skipped;
    result.truncation_reasons = scan.truncation_reasons;
    if !result.coverage.scope_exhaustive || snapshot.is_none() {
        result.incomplete("scan_incomplete");
        return Ok(());
    }
    let mut selected = Vec::new();
    for (ordinal, file) in files.iter().enumerate() {
        let data = matching::execute(file, &compiled, &request.limits, deadline, cancelled)?;
        let count = data.diagnostics.len().min(
            request
                .limits
                .diagnostic_count
                .saturating_sub(result.diagnostics.len()),
        );
        result
            .diagnostics
            .extend_from_slice(&data.diagnostics[..count]);
        result.diagnostics_omitted += data.diagnostics_count - count;
        if let Some(reason) = &data.stopped {
            result.incomplete(reason);
            return Ok(());
        }
        result.counts.matched_files += 1;
        result.counts.observed_matches += data.matches.len();
        if result.counts.observed_matches > request.max_matches {
            result.incomplete("max_matches");
            return Ok(());
        }
        let lines = matching::Lines::new(&file.source);
        let mut chosen = Vec::new();
        for (index, candidate) in data.matches.iter().enumerate() {
            let anchor = SourceAnchor {
                path: file.path.clone(),
                range: ByteRange {
                    start_byte: candidate.start,
                    end_byte: candidate.end,
                },
                expected_text: file.source[candidate.start..candidate.end].into(),
            };
            if request.selection.is_none() || anchors.remove(&anchor) {
                result.plan.matches.push(matching::render(
                    file,
                    &data,
                    candidate,
                    (ordinal, index),
                    &request.context,
                    request.limits.text_bytes,
                    &lines,
                ));
                chosen.push(index);
            }
        }
        selected.push((ordinal, data, chosen));
    }
    result.counts.total_matches = Some(result.counts.observed_matches);
    result.counts.total_is_exact = true;
    if !anchors.is_empty() {
        return Err(DomainError::new(
            "STALE_SELECTION",
            "unknown, changed or out-of-scope match anchor; search again",
        ));
    }
    result.plan.selected_count = Some(result.plan.matches.len());
    if request
        .trivia_overrides
        .as_ref()
        .is_some_and(|o| !o.is_empty())
    {
        result.blocker(
            "UNSUPPORTED_TRIVIA_DISPOSITION",
            "override preservation is not yet supported by this build",
            None,
            None,
        );
    }
    let mut edits = Vec::new();
    for (ordinal, data, chosen) in &selected {
        let file = &files[*ordinal];
        for index in chosen {
            let candidate = &data.matches[*index];
            if data.recovery {
                result.blocker(
                    "PREEXISTING_SYNTAX_ERROR",
                    "selected file contains ERROR/MISSING; repair source first",
                    Some(&file.path),
                    None,
                );
            }
            let expansion = template.expand(candidate, &file.source);
            let id = format!("m/{ordinal}/{index}");
            check_internal(&file.source, candidate, &expansion, result, &file.path)?;
            edits.push(Edit::new(
                &file.path,
                &file.source,
                candidate.start,
                candidate.end,
                expansion.text,
                &id,
            ));
        }
    }
    if let Err(error) = edit::sort_validate(&mut edits) {
        result.blocker(&error.code, &error.message, None, None);
    }
    if !result.plan.blockers.is_empty() || result.plan.blockers_omitted > 0 {
        result.plan.integrity.syntax = "blocked".into();
        result.preview(&edits);
        return Ok(());
    }
    edits.retain(|edit| edit.original_text != edit.replacement_text);
    let mut patch_text = String::new();
    for file in &files {
        let file_edits: Vec<_> = edits
            .iter()
            .filter(|edit| edit.path == file.path)
            .cloned()
            .collect();
        if file_edits.is_empty() {
            continue;
        }
        let proposed = edit::reconstruct(&file.source, &file_edits)?;
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_rust::LANGUAGE.into())
            .map_err(|_| DomainError::new("INTERNAL", "grammar ABI failed"))?;
        let tree = parser
            .parse(&proposed, None)
            .ok_or_else(|| DomainError::new("INTERNAL", "virtual parse failed"))?;
        if tree.root_node().has_error() {
            result.blocker(
                "NEW_SYNTAX_ERROR",
                "proposed bytes introduce ERROR/MISSING",
                Some(&file.path),
                None,
            );
        }
        match patch::section(&file.path, &file.source, &proposed) {
            Ok(section) => patch_text.push_str(&section),
            Err(error) => result.blocker(&error.code, &error.message, Some(&file.path), None),
        }
        result.plan.base_files.push(BaseFile {
            path: file.path.clone(),
            original_length: file.source.len(),
            mode: format!("{:o}", file.mode),
        });
    }
    if !result.plan.blockers.is_empty() {
        result.plan.integrity.syntax = "blocked".into();
        result.preview(&edits);
        return Ok(());
    }
    before_recheck();
    let mut recheck = SearchEnvelope::empty(request.limits.clone());
    let (_, fresh) = scope::discover(&scope, &mut recheck, deadline, cancelled)?;
    if fresh != snapshot || !recheck.coverage.scope_exhaustive {
        result.incomplete("SOURCE_CHANGED");
        result.preview(&edits);
        return Ok(());
    }
    if cancelled.load(Ordering::Relaxed) {
        return Err(DomainError::new("CANCELLED", "request cancelled"));
    }
    if Instant::now() >= deadline {
        result.incomplete("planning_deadline");
        return Ok(());
    }
    result.plan.state = "applicable".into();
    result.plan.applicable = true;
    result.plan.integrity.syntax = "checked".into();
    result.plan.edits = Some(edits);
    result.plan.patch = Some(patch_text);
    Ok(())
}
fn check_internal(
    source: &str,
    candidate: &Candidate,
    expansion: &crate::template::Expansion,
    result: &mut PlanEnvelope,
    path: &str,
) -> Result<(), DomainError> {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_rust::LANGUAGE.into())
        .map_err(|_| DomainError::new("INTERNAL", "grammar ABI failed"))?;
    let tree = parser
        .parse(source, None)
        .ok_or_else(|| DomainError::new("INTERNAL", "trivia parse failed"))?;
    let mut nodes = vec![tree.root_node()];
    while let Some(node) = nodes.pop() {
        if matches!(
            node.kind(),
            "line_comment"
                | "block_comment"
                | "attribute_item"
                | "inner_attribute_item"
                | "shebang"
        ) {
            let range = node.byte_range();
            if range.start >= candidate.start
                && range.end <= candidate.end
                && !expansion.copies.iter().any(|copy| {
                    copy.original.start <= range.start
                        && copy.original.end >= range.end
                        && expansion.text.get(copy.output.clone())
                            == source.get(copy.original.clone())
                })
            {
                result.blocker(
                    "UNRETAINED_TRIVIA",
                    "template consumes trivia without original-capture provenance",
                    Some(path),
                    Some(ByteRange {
                        start_byte: range.start,
                        end_byte: range.end,
                    }),
                );
            }
            continue;
        }
        for i in 0..node.child_count() {
            nodes.push(node.child(i).expect("child"));
        }
    }
    Ok(())
}
