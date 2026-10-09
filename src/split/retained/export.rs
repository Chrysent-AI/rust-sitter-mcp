//! Explicit frozen selectors become an inactive ordinary request, never an executed plan.
use super::*;
use crate::{
    move_plan::{Destination, Move, MoveLimits, MoveRequest},
    plan::SourceAnchor,
};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(deny_unknown_fields)]
pub struct UnitRef {
    pub analysis_id: String,
    pub item_id: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(deny_unknown_fields)]
pub struct ExportSelection {
    pub unit_ref: UnitRef,
    pub destination: Destination,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(deny_unknown_fields)]
pub struct MoveOptions {
    pub context: Option<Context>,
    pub limits: Option<MoveLimits>,
    pub max_moves: Option<usize>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(deny_unknown_fields)]
pub struct ExportRequest {
    pub analysis_handle: String,
    pub snapshot_id: String,
    pub analysis_id: Option<String>,
    pub scope_input_digest: Option<String>,
    pub selection: Vec<ExportSelection>,
    #[serde(default)]
    pub move_options: MoveOptions,
    #[serde(default)]
    pub limits: DetailLimits,
}
#[derive(Debug, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct ExportEnvelope {
    pub schema_version: u8,
    pub envelope_kind: String,
    pub tool: String,
    pub scaffold: bool,
    pub submitted: bool,
    pub applicability: String,
    pub source_freshness: String,
    #[serde(serialize_with = "serialize_request")]
    pub request: Option<MoveRequest>,
    pub provenance: Value,
    pub review: Value,
    pub counts: ExportCounts,
    pub integrity: Integrity,
    pub error: Option<DomainError>,
}
#[derive(Debug, Default, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct ExportCounts {
    pub selected_units: usize,
    pub request_bytes: usize,
    pub response_bytes: usize,
}
impl ExportEnvelope {
    pub fn failed(error: DomainError) -> Self {
        Self {
            schema_version: 1,
            envelope_kind: "move_request_scaffold".into(),
            tool: "export_move_request".into(),
            scaffold: true,
            submitted: false,
            applicability: "not_assessed".into(),
            source_freshness: "not_checked".into(),
            request: None,
            provenance: Value::Null,
            review: Value::Null,
            counts: ExportCounts::default(),
            integrity: Integrity {
                syntax: "not_performed".into(),
                semantic: "not_performed".into(),
                grammar: "tree-sitter-rust@0.24.2".into(),
            },
            error: Some(error),
        }
    }
}
fn ordinary_json(request: &MoveRequest) -> Value {
    let mut value = serde_json::to_value(request).expect("ordinary request JSON");
    for field in [
        "trivia_overrides",
        "rewrite_overrides",
        "draft_provenance",
        "include_unselected_trivia",
        "assume_standard_prelude",
        "assume_declared_helpers",
        "acknowledge_test_consumers",
        "resolve_semantic",
        "semantic_configuration",
    ] {
        value.as_object_mut().expect("request object").remove(field);
    }
    // Ordinary move previews distinguish omission from explicitly requested expansion.
    if !request.limits.diagnostic_count_expanded() {
        value["limits"]
            .as_object_mut()
            .expect("move limits")
            .remove("diagnostic_count");
    }
    value
}
fn serialize_request<S: serde::Serializer>(
    request: &Option<MoveRequest>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    request.as_ref().map(ordinary_json).serialize(serializer)
}
fn refusal(code: &str, field: &str, message: &str) -> DomainError {
    field_error(code, message, field)
}
fn unsupported() -> DomainError {
    refusal(
        "UNSUPPORTED_SCAFFOLD_SELECTION",
        "selection",
        "unsupported unit/context or missing complete original byte/header provenance",
    )
}
fn oversized() -> DomainError {
    DomainError::new(
        "SCAFFOLD_TOO_LARGE",
        "complete request exceeds decoded arguments or duplicated response budget; no partial request, snippets or automatic batch splitting",
    )
}
impl ExportRequest {
    fn identity(&self) -> DetailRequest {
        DetailRequest {
            analysis_handle: self.analysis_handle.clone(),
            snapshot_id: self.snapshot_id.clone(),
            analysis_id: self.analysis_id.clone(),
            scope_input_digest: self.scope_input_digest.clone(),
            selector: Selector::Release {},
            limits: self.limits.clone(),
        }
    }
    fn validate(&self) -> Result<(), DomainError> {
        self.identity().validate()?;
        let max = self.move_options.max_moves.unwrap_or(500);
        if !(1..=5000).contains(&max) || self.selection.is_empty() || self.selection.len() > max {
            return Err(refusal(
                "INVALID_SCAFFOLD_SELECTION",
                "selection",
                "selection must be nonempty and fit explicit max_moves (default 500, range 1–5000)",
            ));
        }
        if self
            .move_options
            .context
            .as_ref()
            .is_some_and(|c| c.before_lines > 20 || c.after_lines > 20)
        {
            return Err(refusal(
                "INVALID_PARAMS",
                "move_options.context",
                "context is 0–20 lines",
            ));
        }
        if let Some(limits) = &self.move_options.limits {
            let mut display = limits.clone();
            display.diagnostic_count = display.diagnostic_count.min(256);
            display.validate()?;
            if limits.diagnostic_count > 100_000 {
                return Err(refusal(
                    "INVALID_PARAMS",
                    "move_options.limits.diagnostic_count",
                    "move diagnostic_count is 0–100000",
                ));
            }
        }
        let mut ids = BTreeSet::new();
        for entry in &self.selection {
            let r = &entry.unit_ref;
            if r.analysis_id.is_empty()
                || r.analysis_id.len() > 200
                || r.item_id.is_empty()
                || r.item_id.len() > 512
                || !ids.insert(&r.item_id)
            {
                return Err(refusal(
                    "INVALID_SCAFFOLD_SELECTION",
                    "selection.unit_ref",
                    "invalid or duplicate explicit unit reference",
                ));
            }
        }
        Ok(())
    }
}
impl Store {
    pub fn export(&self, request: ExportRequest, cancelled: &AtomicBool) -> ExportEnvelope {
        self.export_at(request, cancelled, Instant::now())
    }
    pub(super) fn export_at(
        &self,
        request: ExportRequest,
        cancelled: &AtomicBool,
        now: Instant,
    ) -> ExportEnvelope {
        let mut result =
            ExportEnvelope::failed(DomainError::new("INTERNAL", "export not completed"));
        result.counts.selected_units = request.selection.len();
        let controls = Controls {
            deadline: now + Duration::from_millis(request.limits.time_budget_ms.min(300_000)),
            cancelled,
        };
        let outcome = (|| {
            request.validate()?;
            controls.check()?;
            let record = self.obtain(&request.identity(), now)?;
            let normalized = record
                .retention
                .normalized_request
                .as_ref()
                .ok_or_else(unsupported)?;
            let mut moves = Vec::with_capacity(request.selection.len());
            let index: BTreeMap<_, _> = array(&record.canonical["inventory"])
                .iter()
                .filter_map(|i| i["id"].as_str().map(|id| (id, i)))
                .collect();
            let mut spans = Vec::with_capacity(request.selection.len());
            let mut anchor_bytes = 0usize;
            for entry in &request.selection {
                controls.check()?;
                if Some(entry.unit_ref.analysis_id.as_str())
                    != record.retention.analysis_id.as_deref()
                {
                    return Err(refusal(
                        "INVALID_SCAFFOLD_SELECTION",
                        "selection.unit_ref.analysis_id",
                        "cross-analysis unit reference",
                    ));
                }
                let item = index.get(entry.unit_ref.item_id.as_str()).ok_or_else(|| {
                    refusal(
                        "UNKNOWN_ADVICE_ID",
                        "selection.unit_ref.item_id",
                        "unknown inventory unit ID",
                    )
                })?;
                // Partitioning is an advisory alternative, not a whole-impl move exclusion.
                let reasons = array(&item["reasons"]);
                let partitioned = item["kind"] == "impl_item"
                    && reasons
                        .iter()
                        .all(|r| r == "partition_associated_units_instead");
                let member = item["enclosing_impl"].is_object();
                if !(item["eligibility"] == "supported_unit"
                    || partitioned
                    || (member && reasons.is_empty()))
                    || item["syntax"]["subtree_has_recovery"] == true
                    || item["syntax"]["enclosing_has_recovery"] == true
                {
                    return Err(unsupported());
                }
                let path = item["path"].as_str().ok_or_else(unsupported)?;
                let range: ByteRange = serde_json::from_value(item["span"]["range"].clone())
                    .map_err(|_| unsupported())?;
                if range.start_byte >= range.end_byte {
                    return Err(unsupported());
                }
                anchor_bytes = anchor_bytes.saturating_add(range.end_byte - range.start_byte);
                let header = member.then(|| &item["enclosing_impl"]["anchor"]);
                if let Some(h) = header {
                    let r: ByteRange =
                        serde_json::from_value(h["range"].clone()).map_err(|_| unsupported())?;
                    anchor_bytes =
                        anchor_bytes.saturating_add(r.end_byte.saturating_sub(r.start_byte));
                }
                // Refuse before copying a large payload; exact escaped budgets are checked below.
                if anchor_bytes > 8 * 1024 * 1024
                    || anchor_bytes > request.limits.response_bytes / 2
                {
                    return Err(oversized());
                }
                let anchor = |p: &str, r: &Value| -> Result<SourceAnchor, DomainError> {
                    serde_json::from_value(
                        exact_anchor(&record.evidence, p, r).map_err(|_| unsupported())?,
                    )
                    .map_err(|_| unsupported())
                };
                let item_anchor = anchor(path, &item["span"]["range"])?;
                let enclosing_impl = header
                    .map(|h| {
                        let p = h["path"].as_str().ok_or_else(unsupported)?;
                        let a = anchor(p, &h["range"])?;
                        if p != path
                            || a.range.start_byte >= a.range.end_byte
                            || a.range.end_byte > range.start_byte
                        {
                            return Err(unsupported());
                        }
                        Ok(a)
                    })
                    .transpose()?;
                validate_destination(&entry.destination, &item_anchor, member, normalized)?;
                spans.push((path.to_owned(), range));
                moves.push(Move {
                    item: item_anchor,
                    enclosing_impl,
                    destination: entry.destination.clone(),
                });
            }
            spans.sort();
            for pair in spans.windows(2) {
                if pair[0].0 == pair[1].0 && pair[0].1.end_byte > pair[1].1.start_byte {
                    return Err(refusal(
                        "INVALID_SCAFFOLD_SELECTION",
                        "selection",
                        "overlapping units, including whole-impl/member alternatives, cannot be exported together",
                    ));
                }
            }
            controls.check()?;
            reobserve(&record, controls)?;
            // Only the documented presentation/work knobs may enter the strict ordinary request.
            let core = json!({"repo_path":normalized["repo_path"], "crate_root":normalized["crate_root"],
                "paths":record.evidence.normalized_scope["paths"], "globs":record.evidence.normalized_scope["globs"], "moves":moves});
            let mut strict: MoveRequest =
                serde_json::from_value(core).map_err(|_| unsupported())?;
            strict.context = request.move_options.context.clone().unwrap_or_default();
            strict.limits = request.move_options.limits.clone().unwrap_or_default();
            strict.max_moves = request.move_options.max_moves.unwrap_or(500);
            result.counts.request_bytes = serde_json::to_vec(&ordinary_json(&strict))
                .expect("ordinary request JSON")
                .len();
            if result.counts.request_bytes > 8 * 1024 * 1024 {
                return Err(oversized());
            }
            result.provenance = json!({"analysis_handle":record.retention.analysis_handle, "analysis_id":record.retention.analysis_id,
                "snapshot_id":record.retention.snapshot_id, "scope_input_digest":record.retention.scope_input_digest,
                "normalized_request":normalized, "provenance":record.retention.provenance});
            result.review = review(&record, &request.selection, controls)?;
            result.source_freshness = "checked_at_export".into();
            result.request = Some(strict);
            result.error = None;
            // Count fields themselves are included; usize::MAX reserves their maximum spelling.
            result.counts.response_bytes = usize::MAX;
            let bytes = wire_bytes(&result);
            result.counts.response_bytes = bytes;
            if bytes > request.limits.response_bytes {
                return Err(oversized());
            }
            controls.check()?;
            Ok(())
        })();
        if let Err(error) = outcome {
            result.request = None;
            result.review = Value::Null;
            result.provenance = Value::Null;
            result.source_freshness = "not_checked".into();
            result.error = Some(error);
        }
        result
    }
}
fn validate_destination(
    destination: &Destination,
    source: &SourceAnchor,
    member: bool,
    normalized: &Value,
) -> Result<(), DomainError> {
    let invalid = || {
        refusal(
            "INVALID_DESTINATION",
            "selection.destination",
            "destination syntax/path geometry or required caller anchor is invalid",
        )
    };
    let path = destination.path();
    scope::normalized_path(path).map_err(|_| invalid())?;
    if path == source.path || !path.ends_with(".rs") {
        return Err(invalid());
    }
    let check = |anchor: &SourceAnchor| -> Result<(), DomainError> {
        scope::normalized_path(&anchor.path).map_err(|_| invalid())?;
        if anchor.path != path
            || anchor.range.start_byte >= anchor.range.end_byte
            || anchor.range.end_byte - anchor.range.start_byte != anchor.expected_text.len()
        {
            return Err(invalid());
        }
        Ok(())
    };
    match destination {
        Destination::Existing { before_item, .. } => {
            if let Some(a) = before_item {
                check(a)?;
            }
        }
        Destination::ExistingImpl {
            implementation,
            before_item,
            ..
        } => {
            if !member {
                return Err(invalid());
            }
            check(implementation)?;
            if let Some(a) = before_item {
                check(a)?;
            }
        }
        Destination::NewSibling { parent_path, .. } => {
            scope::normalized_path(parent_path).map_err(|_| invalid())?;
            if !parent_path.ends_with(".rs") {
                return Err(invalid());
            }
            let name = items::module_name(path).map_err(|_| invalid())?;
            let root = normalized["crate_root"].as_str().ok_or_else(invalid)?;
            if Path::new(path).parent() != Path::new(&source.path).parent()
                || items::child_path(parent_path, root, name) != path
            {
                return Err(invalid());
            }
        }
    }
    Ok(())
}
fn reobserve(record: &Record, controls: Controls<'_>) -> Result<(), DomainError> {
    let stale = || {
        DomainError::new(
            "SOURCE_CHANGED",
            "captured corpus, scope, filesystem identity/modes or effective ignore inputs changed or cannot be completely reobserved; obtain fresh retained advice",
        )
    };
    let normalized = record
        .retention
        .normalized_request
        .as_ref()
        .ok_or_else(stale)?;
    let request: SuggestSplitRequest = {
        let mut r = normalized.clone();
        r.as_object_mut()
            .ok_or_else(stale)?
            .remove("diagnostic_count_explicit");
        serde_json::from_value(r).map_err(|_| stale())?
    };
    let root =
        scope::resolve(&request.repo_path, Path::new(&request.repo_path)).map_err(|_| stale())?;
    if root.to_str() != Some(request.repo_path.as_str()) {
        return Err(stale());
    }
    let scan_request = SearchRequest {
        repo_path: request.repo_path,
        query: String::new(),
        paths: request.paths,
        globs: request.globs,
        context: Context::default(),
        limits: request.limits.into(),
        page_size: 100,
        cursor: None,
    };
    let scope = Scope::new(root, &scan_request).map_err(|_| stale())?;
    let mut scan = SearchEnvelope::empty(scan_request.limits);
    let mut inputs = scope::ScopeInputManifest::default();
    let (_, snapshot) = scope::discover_observed(
        &scope,
        &mut scan,
        controls.deadline,
        controls.cancelled,
        Some(&mut inputs),
    )
    .map_err(|e| if e.code == "CANCELLED" { e } else { stale() })?;
    controls.check()?;
    if !scan.coverage.scope_exhaustive || snapshot != record.retention.snapshot_id {
        return Err(stale());
    }
    let digest = inputs.digest(
        &scope,
        snapshot.as_deref().ok_or_else(stale)?,
        (controls.deadline, controls.cancelled),
    )?;
    if digest != record.evidence.scope_input_digest {
        return Err(stale());
    }
    Ok(())
}
fn review(
    record: &Record,
    selection: &[ExportSelection],
    controls: Controls<'_>,
) -> Result<Value, DomainError> {
    let selected: BTreeSet<_> = selection
        .iter()
        .map(|s| s.unit_ref.item_id.as_str())
        .collect();
    let intersects = |ids: &Value| {
        array(ids)
            .iter()
            .any(|i| i.as_str().is_some_and(|i| selected.contains(i)))
    };
    let mut companions = BTreeMap::new();
    for candidate in array(&record.canonical["ownership_candidates"]) {
        controls.check()?;
        if intersects(&candidate["core_item_ids"])
            || array(&candidate["alternatives"])
                .iter()
                .any(|a| intersects(&a["item_ids"]))
            || array(&candidate["companions"])
                .iter()
                .any(|c| c["item_id"].as_str().is_some_and(|i| selected.contains(i)))
        {
            for companion in array(&candidate["companions"]) {
                controls.check()?;
                if !companion["item_id"]
                    .as_str()
                    .is_some_and(|i| selected.contains(i))
                    && let Some(id) = companion["id"].as_str()
                {
                    companions.insert(id, companion);
                }
            }
        }
    }
    let mut decisions = Vec::new();
    for collection in ["decisions", "advice_decisions"] {
        for decision in array(&record.canonical[collection]) {
            controls.check()?;
            if (intersects(&decision["item_ids"]) || array(&decision["item_ids"]).is_empty())
                && decision["resolution"] != "resolved"
            {
                decisions.push(json!({"collection":collection,"id":decision["id"]}));
            }
        }
    }
    Ok(
        json!({"selected_item_ids":selection.iter().map(|s| &s.unit_ref.item_id).collect::<Vec<_>>(),
        "companions_not_selected":companions.values().collect::<Vec<_>>(), "unresolved_decision_refs":decisions,
        "destination_and_batch_applicability":"not_assessed"}),
    )
}
