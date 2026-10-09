//! Immutable historical advice, separate from live search pagination and execution anchors.
use super::*;
use serde_json::{Value, json};
use std::{
    borrow::Cow,
    collections::hash_map::RandomState,
    hash::BuildHasher,
    io::{self, Write},
    mem::size_of,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

mod accounting;
use accounting::{evidence_allocation, preparation_allocation, retention_allocation};

/// Provisional measurement parameters, not a frozen product guarantee.
pub const PROVISIONAL_RECORDS: usize = 1;
pub const PROVISIONAL_BYTES: usize = 128 * 1024 * 1024;
pub const PROVISIONAL_TTL_SECONDS: u64 = 900;
#[derive(Debug, Clone, Copy, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct RetentionLimits {
    pub records: usize,
    pub aggregate_bytes: usize,
    pub ttl_seconds: u64,
}
impl Default for RetentionLimits {
    fn default() -> Self {
        Self {
            records: PROVISIONAL_RECORDS,
            aggregate_bytes: PROVISIONAL_BYTES,
            ttl_seconds: PROVISIONAL_TTL_SECONDS,
        }
    }
}
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct Retention {
    pub state: String,
    pub analysis_handle: Option<String>,
    pub analysis_id: Option<String>,
    pub snapshot_id: Option<String>,
    pub scope_input_digest: Option<String>,
    pub normalized_request: Option<Value>,
    pub provenance: Value,
    pub expires_at: Option<String>,
    pub limits: RetentionLimits,
    pub accounted_bytes: usize,
    pub reason: Option<String>,
}
impl Retention {
    fn unavailable(limits: RetentionLimits, reason: &str) -> Self {
        Self {
            state: "unavailable".into(),
            analysis_handle: None,
            analysis_id: None,
            snapshot_id: None,
            scope_input_digest: None,
            normalized_request: None,
            provenance: provenance(),
            expires_at: None,
            limits,
            accounted_bytes: 0,
            reason: Some(reason.into()),
        }
    }
}
fn provenance() -> Value {
    json!({"grammar":"tree-sitter-rust@0.24.2", "server_version":env!("CARGO_PKG_VERSION"),
        "build_revision":env!("GIT_SHA"), "grouping_version":"written-core-v1"})
}
#[derive(Debug, Default)]
pub(super) struct Evidence {
    pub buffers: BTreeMap<String, String>,
    pub inputs: scope::ScopeInputManifest,
    pub scope_input_digest: String,
    pub normalized_scope: Value,
    pub source_manifest: Value,
}
#[derive(Default)]
struct Usage {
    bytes: AtomicUsize,
    records: AtomicUsize,
}
struct Charge {
    usage: Arc<Usage>,
    bytes: usize,
}
impl Charge {
    fn reconcile(&mut self, bytes: usize) -> Result<(), &'static str> {
        // Never grow a candidate past the allocation reserved before materialization.
        let refund = self.bytes.checked_sub(bytes).ok_or("record_too_large")?;
        self.usage.bytes.fetch_sub(refund, Ordering::Relaxed);
        self.bytes = bytes;
        Ok(())
    }
}
impl Drop for Charge {
    fn drop(&mut self) {
        self.usage.bytes.fetch_sub(self.bytes, Ordering::Relaxed);
        self.usage.records.fetch_sub(1, Ordering::Relaxed);
    }
}
struct Record {
    canonical: Value,
    evidence: Evidence,
    retention: Retention,
    captured_at: Instant,
    published_at: Instant,
    expires: u64,
    charge: Option<Charge>,
}
struct State {
    next: u64,
    records: BTreeMap<String, Arc<Record>>,
}
pub struct Store {
    state: Mutex<State>,
    key: RandomState,
    origin: Instant,
    limits: RetentionLimits,
    usage: Arc<Usage>,
    #[cfg(test)]
    materializations: AtomicUsize,
}
impl Default for Store {
    fn default() -> Self {
        Self::new(RetentionLimits::default())
    }
}
impl Store {
    pub fn new(limits: RetentionLimits) -> Self {
        Self {
            state: Mutex::new(State {
                next: 0,
                records: BTreeMap::new(),
            }),
            key: RandomState::new(),
            origin: Instant::now(),
            limits,
            usage: Arc::new(Usage::default()),
            #[cfg(test)]
            materializations: AtomicUsize::new(0),
        }
    }
    /// Accounted live records and preparation reservations, not process RSS.
    pub fn accounted_allocation(&self) -> (usize, usize) {
        (
            self.usage.records.load(Ordering::Relaxed),
            self.usage.bytes.load(Ordering::Relaxed),
        )
    }
    fn tick(&self, now: Instant) -> u64 {
        now.saturating_duration_since(self.origin)
            .as_millis()
            .min(u64::MAX as u128) as u64
    }
    fn signed(&self, fields: &str) -> String {
        format!("{fields}/{:016x}", self.key.hash_one(fields))
    }
    fn expiry(&self, handle: &str) -> Result<u64, DomainError> {
        let unknown = || {
            field_error(
                "ADVICE_SNAPSHOT_UNKNOWN",
                "unknown, released or previous-process analysis handle",
                "analysis_handle",
            )
        };
        if handle.len() > 200 || !handle.is_ascii() {
            return Err(unknown());
        }
        let (fields, _) = handle.rsplit_once('/').ok_or_else(unknown)?;
        if self.signed(fields) != handle {
            return Err(unknown());
        }
        let parts: Vec<_> = fields.split('/').collect();
        if parts.len() != 3 || parts[0] != "a1" || parts[1].parse::<u64>().is_err() {
            return Err(unknown());
        }
        parts[2].parse().map_err(|_| unknown())
    }
    fn prune(&self, state: &mut State, now: Instant) -> Vec<Arc<Record>> {
        let tick = self.tick(now);
        let expired: Vec<_> = state
            .records
            .iter()
            .filter(|(_, r)| tick >= r.expires)
            .map(|(h, _)| h.clone())
            .collect();
        expired
            .into_iter()
            .filter_map(|h| state.records.remove(&h))
            .collect()
    }
    fn reserve(&self, now: Instant, controls: Controls<'_>) -> Result<(u64, Charge), &'static str> {
        // Removing registry entries does not refund readers' Arcs. Large drops stay unlocked.
        let expired = {
            let mut state = self.state.lock().expect("advice store lock");
            self.prune(&mut state, now)
        };
        drop(expired);
        let mut state = self.state.lock().expect("advice store lock");
        controls
            .check()
            .map_err(|_| "cancelled_before_publication")?;
        if self.usage.records.load(Ordering::Relaxed) >= self.limits.records {
            return Err("capacity");
        }
        state.next = state.next.checked_add(1).ok_or("capacity")?;
        self.usage.records.fetch_add(1, Ordering::Relaxed);
        Ok((
            state.next,
            Charge {
                usage: self.usage.clone(),
                bytes: 0,
            },
        ))
    }
    fn reserve_bytes(&self, charge: &mut Charge, bytes: usize) -> Result<(), &'static str> {
        if bytes > self.limits.aggregate_bytes {
            return Err("record_too_large");
        }
        let _state = self.state.lock().expect("advice store lock");
        if self
            .usage
            .bytes
            .load(Ordering::Relaxed)
            .saturating_add(bytes)
            > self.limits.aggregate_bytes
        {
            return Err("capacity");
        }
        self.usage.bytes.fetch_add(bytes, Ordering::Relaxed);
        charge.bytes = bytes;
        Ok(())
    }
    fn prepare(
        &self,
        canonical: &SuggestSplitEnvelope,
        evidence: Evidence,
        request: &SuggestSplitRequest,
        now: Instant,
        controls: Controls<'_>,
    ) -> Result<Record, &'static str> {
        // No canonical Value, normalized request or candidate record before both reservations.
        let (id, mut charge) = self.reserve(now, controls)?;
        let bytes = preparation_allocation(
            canonical,
            &evidence,
            request,
            self.limits.aggregate_bytes,
            controls,
        )?;
        self.reserve_bytes(&mut charge, bytes)?;
        controls
            .check()
            .map_err(|_| "cancelled_before_publication")?;
        let expires = self
            .tick(now)
            .checked_add(
                self.limits
                    .ttl_seconds
                    .checked_mul(1000)
                    .ok_or("capacity")?,
            )
            .ok_or("capacity")?;
        let handle = self.signed(&format!("a1/{id}/{expires:020}"));
        let mut normalized = serde_json::to_value(request).map_err(|_| "record_too_large")?;
        normalized["repo_path"] = json!(canonical.root);
        normalized["paths"] = evidence.normalized_scope["paths"].clone();
        normalized["globs"] = evidence.normalized_scope["globs"].clone();
        normalized["diagnostic_count_explicit"] = json!(request.limits.diagnostic_count_explicit);
        #[cfg(test)]
        self.materializations.fetch_add(1, Ordering::Relaxed);
        let mut record = Record {
            canonical: serde_json::to_value(canonical).map_err(|_| "record_too_large")?,
            retention: Retention {
                state: "retained".into(),
                analysis_handle: Some(handle.clone()),
                analysis_id: Some(self.signed(&format!("analysis/{id}"))),
                snapshot_id: canonical.snapshot_id.clone(),
                scope_input_digest: Some(evidence.scope_input_digest.clone()),
                normalized_request: Some(normalized),
                provenance: provenance(),
                expires_at: Some(wall_expiry(self.limits.ttl_seconds)),
                limits: self.limits,
                accounted_bytes: 0,
                reason: None,
            },
            evidence,
            captured_at: now,
            published_at: now,
            expires,
            charge: Some(charge),
        };
        // Source text lives once in frozen buffers; descriptors retain original coordinates.
        strip_source_text(&mut record.canonical);
        // Conservative owned-allocation accounting: capacities, not serialized JSON length.
        // BTree nodes/Arc/control blocks use a deliberately generous per-entry reserve.
        let bytes = size_of::<Record>()
            + allocation(&record.canonical)
            + retention_allocation(&record.retention)
            + evidence_allocation(&record.evidence)
            + handle.capacity()
            + 1024;
        if bytes > record.charge.as_ref().expect("reserved charge").bytes {
            return Err("record_too_large");
        }
        // Keep the preparation bound until publication finalizes identity/expiry allocation.
        record.retention.accounted_bytes = bytes;
        controls
            .check()
            .map_err(|_| "cancelled_before_publication")?;
        Ok(record)
    }
    fn publish(
        &self,
        mut record: Record,
        controls: Controls<'_>,
        now: Instant,
    ) -> Result<Retention, &'static str> {
        // Fixed expiry starts at publication, not at preparation or the last detail request.
        let metadata_bytes = retention_allocation(&record.retention);
        debug_assert!(now >= record.captured_at);
        record.published_at = now;
        record.expires = self
            .tick(record.published_at)
            .checked_add(
                self.limits
                    .ttl_seconds
                    .checked_mul(1000)
                    .ok_or("capacity")?,
            )
            .ok_or("capacity")?;
        let id = record
            .retention
            .analysis_handle
            .as_ref()
            .expect("prepared handle")
            .split('/')
            .nth(1)
            .expect("analysis sequence");
        record.retention.analysis_handle =
            Some(self.signed(&format!("a1/{id}/{:020}", record.expires)));
        record.retention.expires_at = Some(wall_expiry(self.limits.ttl_seconds));
        let bytes = record.retention.accounted_bytes - metadata_bytes
            + retention_allocation(&record.retention);
        record
            .charge
            .as_mut()
            .expect("reserved charge")
            .reconcile(bytes)?;
        record.retention.accounted_bytes = bytes;
        let published = record.retention.clone();
        // Reclaim large payloads outside the mutex.
        let expired = {
            let mut state = self.state.lock().expect("advice store lock");
            self.prune(&mut state, now)
        };
        drop(expired);
        let mut state = self.state.lock().expect("advice store lock");
        if controls.check().is_err() {
            return Err("cancelled_before_publication");
        }
        // The prepared record already owns its slot/byte charge. Publication transfers it.
        debug_assert_eq!(
            record.charge.as_ref().expect("reserved charge").bytes,
            record.retention.accounted_bytes
        );
        state.records.insert(
            record
                .retention
                .analysis_handle
                .clone()
                .expect("prepared handle"),
            Arc::new(record),
        );
        Ok(published)
    }
    fn obtain(&self, request: &DetailRequest, now: Instant) -> Result<Arc<Record>, DomainError> {
        let expiry = self.expiry(&request.analysis_handle)?;
        let (record, expired) = {
            let mut state = self.state.lock().expect("advice store lock");
            let expired = self.prune(&mut state, now);
            (
                state.records.get(&request.analysis_handle).cloned(),
                expired,
            )
        };
        drop(expired);
        if self.tick(now) >= expiry {
            return Err(field_error(
                "ADVICE_SNAPSHOT_EXPIRED",
                "fixed analysis lifetime expired",
                "analysis_handle",
            ));
        }
        let record = record.ok_or_else(|| {
            field_error(
                "ADVICE_SNAPSHOT_UNKNOWN",
                "unknown or released analysis handle",
                "analysis_handle",
            )
        })?;
        for (field, expected, supplied) in [
            (
                "snapshot_id",
                record.retention.snapshot_id.as_deref(),
                Some(request.snapshot_id.as_str()),
            ),
            (
                "analysis_id",
                record.retention.analysis_id.as_deref(),
                request.analysis_id.as_deref(),
            ),
            (
                "scope_input_digest",
                record.retention.scope_input_digest.as_deref(),
                request.scope_input_digest.as_deref(),
            ),
        ] {
            if supplied.is_some() && supplied != expected {
                return Err(field_error(
                    "ADVICE_SNAPSHOT_MISMATCH",
                    "supplied identity differs from frozen analysis",
                    field,
                ));
            }
        }
        Ok(record)
    }
    pub fn detail(&self, request: DetailRequest, cancelled: &AtomicBool) -> DetailEnvelope {
        self.detail_at(request, cancelled, Instant::now())
    }
    fn detail_at(
        &self,
        request: DetailRequest,
        cancelled: &AtomicBool,
        now: Instant,
    ) -> DetailEnvelope {
        let mut result = DetailEnvelope::empty();
        let controls = Controls {
            deadline: now + Duration::from_millis(request.limits.time_budget_ms.min(300_000)),
            cancelled,
        };
        let outcome = (|| {
            request.validate()?;
            controls.check()?;
            if let Selector::Page {
                page_token: Some(token),
                ..
            } = &request.selector
            {
                let (fields, _) = token.rsplit_once('/').ok_or_else(invalid_page)?;
                if !fields.starts_with("p1/") || self.signed(fields) != *token {
                    return Err(invalid_page());
                }
            }
            let record = self.obtain(&request, now)?;
            result.analysis_id = record.retention.analysis_id.clone();
            result.snapshot_id = record.retention.snapshot_id.clone();
            result.scope_input_digest = record.retention.scope_input_digest.clone();
            match &request.selector {
                Selector::Release {} => {
                    controls.check()?;
                    // No I/O or serialization while holding this lock. In-flight Arcs stay charged.
                    let removed = self
                        .state
                        .lock()
                        .expect("advice store lock")
                        .records
                        .remove(&request.analysis_handle);
                    drop(removed);
                    result.released = Some(true);
                }
                Selector::Units { item_ids } => {
                    result.collection = Some("units".into());
                    let index: BTreeMap<_, _> = array(&record.canonical["inventory"])
                        .iter()
                        .filter_map(|r| r["id"].as_str().map(|id| (id, r)))
                        .collect();
                    for id in item_ids {
                        controls.check()?;
                        let item = index.get(id.as_str()).ok_or_else(|| {
                            field_error(
                                "UNKNOWN_ADVICE_ID",
                                "unknown inventory item ID",
                                "selector.item_ids",
                            )
                        })?;
                        let range = &item["span"]["range"];
                        if range["end_byte"]
                            .as_u64()
                            .unwrap_or(u64::MAX)
                            .saturating_sub(range["start_byte"].as_u64().unwrap_or(0))
                            > (request.limits.response_bytes / 2) as u64
                        {
                            return Err(too_large());
                        }
                        let anchor = exact_anchor(
                            &record.evidence,
                            item["path"].as_str().expect("item path"),
                            range,
                        )?;
                        let implementation = if item["enclosing_impl"].is_object() {
                            let enclosing = &item["enclosing_impl"];
                            let range = &enclosing["anchor"]["range"];
                            if range["end_byte"]
                                .as_u64()
                                .unwrap_or(u64::MAX)
                                .saturating_sub(range["start_byte"].as_u64().unwrap_or(0))
                                > (request.limits.response_bytes / 2) as u64
                            {
                                return Err(too_large());
                            }
                            Some(exact_anchor(
                                &record.evidence,
                                enclosing["anchor"]["path"].as_str().expect("header path"),
                                &enclosing["anchor"]["range"],
                            )?)
                        } else {
                            None
                        };
                        result.records.push(json!({"item_id":id,"unit_ref":{"analysis_id":record.retention.analysis_id,"item_id":id},"item":anchor,"enclosing_impl":implementation,"eligibility":item["eligibility"],"exclusions":item["reasons"]}));
                        if wire_bytes(&result) > request.limits.response_bytes {
                            return Err(too_large());
                        }
                    }
                    result.total = result.records.len();
                    if wire_bytes(&result) > request.limits.response_bytes {
                        return Err(too_large());
                    }
                }
                Selector::Records { collection, ids } => {
                    result.collection = Some(collection.name().into());
                    let records = collection_records(&record, *collection, controls)?;
                    let index: BTreeMap<_, _> = records
                        .iter()
                        .filter_map(|r| r["id"].as_str().map(|id| (id, r.as_ref())))
                        .collect();
                    for id in ids {
                        controls.check()?;
                        let selected = index.get(id.as_str()).ok_or_else(|| {
                            field_error(
                                "UNKNOWN_ADVICE_ID",
                                "unknown or cross-collection ID",
                                "selector.ids",
                            )
                        })?;
                        result.records.push(detail_record(
                            &record,
                            selected,
                            controls,
                            request.limits.response_bytes,
                        )?);
                        if wire_bytes(&result) > request.limits.response_bytes {
                            return Err(too_large());
                        }
                    }
                    result.total = result.records.len();
                    if wire_bytes(&result) > request.limits.response_bytes {
                        return Err(too_large());
                    }
                }
                Selector::Page {
                    collection,
                    filter,
                    page_size,
                    page_token,
                } => {
                    result.collection = Some(collection.name().into());
                    result.filter = filter.clone();
                    let records = collection_records(&record, *collection, controls)?;
                    let selected = filter_records(&record, &records, filter, controls)?;
                    result.total = selected.len();
                    let binding = serde_json::to_string(&(
                        &request.analysis_handle,
                        &request.snapshot_id,
                        &record.retention.scope_input_digest,
                        collection,
                        filter,
                        page_size,
                        &request.limits,
                        "original-order-v1",
                    ))
                    .expect("page binding");
                    let token = |position| {
                        self.signed(&format!(
                            "p1/{position:020}/{:016x}",
                            self.key.hash_one(&binding)
                        ))
                    };
                    let start = if let Some(supplied) = page_token {
                        let position = supplied
                            .split('/')
                            .nth(1)
                            .and_then(|p| p.parse::<usize>().ok())
                            .ok_or_else(invalid_page)?;
                        if token(position) != *supplied || position >= selected.len() {
                            return Err(invalid_page());
                        }
                        position
                    } else {
                        0
                    };
                    let end = start.saturating_add(*page_size).min(selected.len());
                    // Incremental duplicated-wire accounting keeps temporary pages bounded and linear.
                    result.collection_exhausted = false;
                    result.next_page_token = Some(token(selected.len()));
                    let mut bytes = wire_bytes(&result);
                    let mut stop = start;
                    for r in &selected[start..end] {
                        controls.check()?;
                        let value = match detail_record(
                            &record,
                            r,
                            controls,
                            request.limits.response_bytes,
                        ) {
                            Err(e)
                                if e.code == "DETAIL_TOO_LARGE" && !result.records.is_empty() =>
                            {
                                break;
                            }
                            result => result?,
                        };
                        let json = value.to_string();
                        let cost =
                            json.len() + serde_json::to_vec(&json).expect("escaped record").len();
                        if bytes.saturating_add(cost) > request.limits.response_bytes {
                            break;
                        }
                        bytes += cost;
                        result.records.push(value);
                        stop += 1;
                    }
                    if stop == start && start < selected.len() {
                        return Err(too_large());
                    }
                    result.collection_exhausted = stop == selected.len();
                    result.next_page_token = (!result.collection_exhausted).then(|| token(stop));
                    if wire_bytes(&result) > request.limits.response_bytes {
                        return Err(too_large());
                    }
                }
            }
            controls.check()?;
            if wire_bytes(&result) > request.limits.response_bytes {
                return Err(too_large());
            }
            Ok(())
        })();
        if let Err(error) = outcome {
            return DetailEnvelope::failed(error);
        }
        result
    }
}

/// Counts owned strings/containers plus conservative BTree node storage. Not heap profiling/RSS.
fn allocation(value: &Value) -> usize {
    match value {
        Value::String(s) => s.capacity(),
        Value::Array(a) => {
            a.capacity() * size_of::<Value>() + a.iter().map(allocation).sum::<usize>()
        }
        Value::Object(o) => o
            .iter()
            .map(|(k, v)| 256 + k.capacity() + allocation(v))
            .sum(),
        _ => 0,
    }
}
fn wall_expiry(ttl: u64) -> String {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        .saturating_add(ttl);
    // Gregorian civil date from Unix days; timestamps are informative, expiry uses monotonic time.
    let z = (seconds / 86400) as i64 + 719468;
    let era = z / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let mut year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        seconds / 3600 % 24,
        seconds / 60 % 60,
        seconds % 60
    )
}
fn wire_bytes(value: &impl Serialize) -> usize {
    let value = serde_json::to_value(value).expect("response JSON");
    serde_json::to_vec(&json!({"content":[{"type":"text","text":value.to_string()}],"structuredContent":value,"isError":value["error"].is_object()})).expect("wire JSON").len() + 4096
}
pub(super) fn referenced_paths(value: &Value, paths: &mut BTreeSet<String>) {
    match value {
        Value::Array(a) => {
            for v in a {
                referenced_paths(v, paths);
            }
        }
        Value::Object(o) => {
            if let Some(Value::String(path)) = o.get("path") {
                paths.insert(path.clone());
            }
            for v in o.values() {
                referenced_paths(v, paths);
            }
        }
        _ => {}
    }
}
// Strip duplicated display/header text only; immutable buffers remain the source of truth.
fn strip_source_text(value: &mut Value) {
    match value {
        Value::Array(a) => {
            for v in a {
                strip_source_text(v);
            }
        }
        Value::Object(o) => {
            if o.contains_key("range") && o.contains_key("text_bytes") {
                o.insert("text".into(), Value::Null);
                o.insert("text_omitted".into(), json!(true));
            }
            if o.contains_key("range") && o.contains_key("expected_text") {
                o.insert("expected_text".into(), Value::Null);
            }
            if o.contains_key("anchor") && o.contains_key("header") {
                o.insert("header".into(), Value::Null);
            }
            for v in o.values_mut() {
                strip_source_text(v);
            }
        }
        _ => {}
    }
}
fn detail_record(
    record: &Record,
    value: &Value,
    controls: Controls<'_>,
    budget: usize,
) -> Result<Value, DomainError> {
    let mut out = value.clone();
    let source = record
        .retention
        .normalized_request
        .as_ref()
        .and_then(|r| r["source_path"].as_str())
        .expect("normalized source");
    restore_source_text(
        &mut out,
        source,
        &record.evidence,
        controls,
        &mut (budget / 2),
    )?;
    Ok(out)
}
fn restore_source_text(
    value: &mut Value,
    path: &str,
    evidence: &Evidence,
    controls: Controls<'_>,
    remaining: &mut usize,
) -> Result<(), DomainError> {
    controls.check()?;
    match value {
        Value::Array(a) => {
            for v in a {
                restore_source_text(v, path, evidence, controls, remaining)?;
            }
        }
        Value::Object(o) => {
            let path = o
                .get("path")
                .and_then(Value::as_str)
                .unwrap_or(path)
                .to_owned();
            if let Some(range) = o.get("range")
                && (o.contains_key("text_bytes") || o.contains_key("expected_text"))
            {
                let bytes = range["end_byte"]
                    .as_u64()
                    .unwrap_or(u64::MAX)
                    .saturating_sub(range["start_byte"].as_u64().unwrap_or(0));
                if bytes > *remaining as u64 {
                    return Err(too_large());
                }
                *remaining -= bytes as usize;
                let anchor = exact_anchor(evidence, &path, range)?;
                if o.contains_key("text_bytes") {
                    o.insert("text".into(), anchor["expected_text"].clone());
                    o.insert("text_omitted".into(), json!(false));
                } else {
                    o.insert("expected_text".into(), anchor["expected_text"].clone());
                }
            }
            for v in o.values_mut() {
                restore_source_text(v, &path, evidence, controls, remaining)?;
            }
            if o.contains_key("header")
                && let Some(text) = o.get("anchor").and_then(|a| a.get("expected_text"))
            {
                let bytes = text.as_str().map_or(0, str::len);
                if bytes > *remaining {
                    return Err(too_large());
                }
                *remaining -= bytes;
                let text = text.clone();
                o.insert("header".into(), text);
            }
        }
        _ => {}
    }
    Ok(())
}
fn array(value: &Value) -> &[Value] {
    value.as_array().map_or(&[], Vec::as_slice)
}
fn too_large() -> DomainError {
    DomainError::new(
        "DETAIL_TOO_LARGE",
        "complete indivisible evidence cannot fit the response budget; no snippets or partial anchors returned",
    )
}
fn invalid_page() -> DomainError {
    field_error(
        "INVALID_ADVICE_PAGE",
        "token does not bind this handle, digest, collection, filter, page size, limits or position",
        "selector.page_token",
    )
}
fn exact_anchor(evidence: &Evidence, path: &str, range: &Value) -> Result<Value, DomainError> {
    let start = range["start_byte"].as_u64().ok_or_else(too_large)? as usize;
    let end = range["end_byte"].as_u64().ok_or_else(too_large)? as usize;
    let text = evidence
        .buffers
        .get(path)
        .and_then(|b| b.get(start..end))
        .ok_or_else(|| {
            DomainError::new(
                "UNKNOWN_ADVICE_ID",
                "complete original byte provenance unavailable",
            )
        })?;
    Ok(json!({"path":path,"range":range,"expected_text":text}))
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(rename_all = "snake_case")]
pub enum Collection {
    Inventory,
    Groups,
    Decisions,
    AdviceDecisions,
    Companions,
    BoundaryObservations,
    TestCoupling,
    Overlaps,
    ChainDiagnostics,
}
impl Collection {
    fn name(self) -> &'static str {
        match self {
            Self::Inventory => "inventory",
            Self::Groups => "groups",
            Self::Decisions => "decisions",
            Self::AdviceDecisions => "advice_decisions",
            Self::Companions => "companions",
            Self::BoundaryObservations => "boundary_observations",
            Self::TestCoupling => "test_coupling",
            Self::Overlaps => "overlaps",
            Self::ChainDiagnostics => "chain_diagnostics",
        }
    }
}
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(deny_unknown_fields)]
pub struct DetailFilter {
    pub id: Option<String>,
    pub reason: Option<String>,
    pub candidate_id: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Selector {
    Page {
        collection: Collection,
        #[serde(default)]
        filter: DetailFilter,
        #[serde(default = "page_size")]
        page_size: usize,
        page_token: Option<String>,
    },
    Records {
        collection: Collection,
        ids: Vec<String>,
    },
    Units {
        item_ids: Vec<String>,
    },
    Release {},
}
fn page_size() -> usize {
    100
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(default, deny_unknown_fields)]
pub struct DetailLimits {
    pub response_bytes: usize,
    pub time_budget_ms: u64,
}
impl Default for DetailLimits {
    fn default() -> Self {
        Self {
            response_bytes: Limits::default().response_bytes,
            time_budget_ms: Limits::default().time_budget_ms,
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(deny_unknown_fields)]
pub struct DetailRequest {
    pub analysis_handle: String,
    pub snapshot_id: String,
    pub analysis_id: Option<String>,
    pub scope_input_digest: Option<String>,
    pub selector: Selector,
    #[serde(default)]
    pub limits: DetailLimits,
}
impl DetailRequest {
    fn validate(&self) -> Result<(), DomainError> {
        let invalid = || {
            DomainError::new(
                "INVALID_PARAMS",
                "detail limits/selector outside documented bounds",
            )
        };
        if !(64 * 1024..=16 * 1024 * 1024).contains(&self.limits.response_bytes)
            || !(1..=300_000).contains(&self.limits.time_budget_ms)
        {
            return Err(invalid());
        }
        for s in std::iter::once(&self.snapshot_id)
            .chain(self.analysis_id.iter())
            .chain(self.scope_input_digest.iter())
        {
            if s.len() > 200 || s.is_empty() {
                return Err(invalid());
            }
        }
        match &self.selector {
            Selector::Page {
                filter,
                page_size,
                page_token,
                ..
            } => {
                if !(1..=1000).contains(page_size)
                    || page_token
                        .as_ref()
                        .is_some_and(|s| s.len() > 200 || !s.is_ascii())
                {
                    return Err(invalid_page());
                }
                for s in filter
                    .id
                    .iter()
                    .chain(&filter.reason)
                    .chain(&filter.candidate_id)
                {
                    if s.is_empty() || s.len() > 512 {
                        return Err(invalid());
                    }
                }
            }
            Selector::Records { ids, .. } | Selector::Units { item_ids: ids } => {
                if ids.is_empty()
                    || ids.len() > 1000
                    || ids.iter().any(|s| s.is_empty() || s.len() > 512)
                    || ids.iter().collect::<BTreeSet<_>>().len() != ids.len()
                {
                    return Err(invalid());
                }
            }
            Selector::Release {} => {}
        }
        Ok(())
    }
}
#[derive(Debug, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct DetailEnvelope {
    pub schema_version: u8,
    pub envelope_kind: String,
    pub tool: String,
    pub analysis_id: Option<String>,
    pub snapshot_id: Option<String>,
    pub scope_input_digest: Option<String>,
    pub historical: bool,
    pub live_freshness: String,
    pub collection: Option<String>,
    pub filter: DetailFilter,
    pub total: usize,
    pub records: Vec<Value>,
    pub returned_page_complete: bool,
    pub collection_exhausted: bool,
    pub next_page_token: Option<String>,
    pub released: Option<bool>,
    pub error: Option<DomainError>,
    pub integrity: Integrity,
}
impl DetailEnvelope {
    fn empty() -> Self {
        Self {
            schema_version: 1,
            envelope_kind: "split_detail".into(),
            tool: "get_split_detail".into(),
            analysis_id: None,
            snapshot_id: None,
            scope_input_digest: None,
            historical: true,
            live_freshness: "not_checked".into(),
            collection: None,
            filter: DetailFilter::default(),
            total: 0,
            records: vec![],
            returned_page_complete: true,
            collection_exhausted: true,
            next_page_token: None,
            released: None,
            error: None,
            integrity: Integrity {
                grammar: "tree-sitter-rust@0.24.2".into(),
                syntax: "not_checked".into(),
                semantic: "not_performed".into(),
            },
        }
    }
    pub fn failed(error: DomainError) -> Self {
        let mut result = Self::empty();
        result.returned_page_complete = false;
        result.collection_exhausted = false;
        result.error = Some(error);
        result
    }
}
fn collection_records<'a>(
    record: &'a Record,
    collection: Collection,
    controls: Controls<'_>,
) -> Result<Vec<Cow<'a, Value>>, DomainError> {
    let c = &record.canonical;
    let mut records = Vec::new();
    match collection {
        Collection::Groups => {
            for d in array(&c["drafts"]) {
                for (i, g) in array(&d["groups"]).iter().enumerate() {
                    controls.check()?;
                    let mut g = g.clone();
                    g["id"] = json!(format!("{}/group/{i}", d["id"].as_str().expect("draft ID")));
                    records.push(Cow::Owned(g));
                }
            }
        }
        Collection::Companions => {
            for c in array(&c["ownership_candidates"]) {
                for companion in array(&c["companions"]) {
                    controls.check()?;
                    let mut companion = companion.clone();
                    companion["candidate_id"] = c["id"].clone();
                    records.push(Cow::Owned(companion));
                }
            }
        }
        other => {
            let source = match other {
                Collection::BoundaryObservations => &c["boundary_observations"]["records"],
                Collection::TestCoupling => &c["test_observations"]["records"],
                _ => &c[other.name()],
            };
            for r in array(source) {
                controls.check()?;
                records.push(Cow::Borrowed(r));
            }
        }
    }
    Ok(records)
}
fn filter_records<'a>(
    record: &Record,
    records: &'a [Cow<'_, Value>],
    filter: &DetailFilter,
    controls: Controls<'_>,
) -> Result<Vec<&'a Value>, DomainError> {
    let candidate = if let Some(id) = &filter.candidate_id {
        Some(
            array(&record.canonical["ownership_candidates"])
                .iter()
                .find(|c| c["id"] == *id)
                .ok_or_else(|| {
                    field_error(
                        "UNKNOWN_ADVICE_ID",
                        "unknown candidate filter",
                        "selector.filter.candidate_id",
                    )
                })?,
        )
    } else {
        None
    };
    if filter
        .id
        .as_ref()
        .is_some_and(|id| !records.iter().any(|r| r["id"] == *id))
    {
        return Err(field_error(
            "UNKNOWN_ADVICE_ID",
            "unknown ID filter in this collection",
            "selector.filter.id",
        ));
    }
    let reason_matches = |r: &Value, reason: &str| {
        r["reason"] == reason || array(&r["reasons"]).iter().any(|v| v == reason)
    };
    if filter
        .reason
        .as_ref()
        .is_some_and(|reason| !records.iter().any(|r| reason_matches(r, reason)))
    {
        return Err(field_error(
            "INVALID_PARAMS",
            "reason filter must name an observed typed reason in this collection",
            "selector.filter.reason",
        ));
    }
    let mut candidate_items = BTreeSet::new();
    if let Some(c) = candidate {
        for id in array(&c["core_item_ids"])
            .iter()
            .chain(array(&c["companions"]).iter().map(|r| &r["item_id"]))
            .chain(
                array(&c["alternatives"])
                    .iter()
                    .flat_map(|a| array(&a["item_ids"])),
            )
        {
            controls.check()?;
            if let Some(id) = id.as_str() {
                candidate_items.insert(id);
            }
        }
    }
    let mut selected = Vec::new();
    for r in records {
        controls.check()?;
        if filter.id.as_ref().is_some_and(|id| r["id"] != *id)
            || filter
                .reason
                .as_ref()
                .is_some_and(|reason| !reason_matches(r, reason))
        {
            continue;
        }
        if let Some(c) = candidate {
            let member = r["candidate_id"] == c["id"]
                || r["id"]
                    .as_str()
                    .is_some_and(|id| candidate_items.contains(id))
                || array(&r["ownership_candidate_ids"]).contains(&c["id"])
                || array(&r["candidate_couplings"])
                    .iter()
                    .any(|coupling| coupling["ownership_candidate_id"] == c["id"])
                || array(&r["item_ids"])
                    .iter()
                    .any(|i| i.as_str().is_some_and(|id| candidate_items.contains(id)));
            if !member {
                continue;
            }
        }
        selected.push(r.as_ref());
    }
    Ok(selected)
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[schemars(crate = "rmcp::schemars")]
#[serde(rename_all = "snake_case")]
pub enum ResponseMode {
    #[default]
    Full,
    Compact,
}
#[derive(Debug, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(untagged)]
pub enum SplitResponse {
    Full(Box<SuggestSplitEnvelope>),
    Compact(Box<Manifest>),
}
impl SplitResponse {
    pub fn error(&self) -> Option<&DomainError> {
        match self {
            Self::Full(r) => r.error.as_ref(),
            Self::Compact(r) => r.error.as_ref(),
        }
    }
}
#[derive(Debug, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct Manifest {
    pub schema_version: u8,
    pub envelope_kind: String,
    pub tool: String,
    pub response_mode: String,
    pub advisory: bool,
    pub analysis_status: String,
    pub manifest_complete: bool,
    pub root: Option<String>,
    pub snapshot_id: Option<String>,
    pub totals: Value,
    pub coverage: Value,
    pub candidate_summaries: Vec<Value>,
    pub draft_memberships: Vec<DraftMembership>,
    pub consequence_summaries: Vec<Value>,
    pub decision_groups: Vec<AdviceDecisionGroup>,
    pub detail_availability: Vec<Value>,
    pub retention: Retention,
    pub omissions: BTreeMap<String, usize>,
    pub error: Option<DomainError>,
    pub integrity: Integrity,
}
impl Manifest {
    fn from_full(full: &SuggestSplitEnvelope, retention: Retention) -> Self {
        let candidates: Vec<_> = full
            .ownership_candidates
            .iter()
            .map(|c| serde_json::to_value(c).expect("candidate JSON"))
            .collect();
        let memberships = full
            .drafts
            .iter()
            .map(|d| DraftMembership {
                id: d.id.clone(),
                source_snapshot_id: d.source_snapshot_id.clone(),
                unresolved_decision_ids: d.unresolved_decision_ids.clone(),
                groups: d
                    .groups
                    .iter()
                    .map(|g| GroupMembership {
                        consequence_summary: g.consequence_summary.clone(),
                        kind: g.kind.clone(),
                        destination_path: g.destination.as_ref().map(|d| d.path.clone()),
                        item_ids: g.item_ids.clone(),
                        overlap_ids: g.overlap_ids.clone(),
                        size_interpretation: g.size_interpretation.clone(),
                    })
                    .collect(),
            })
            .collect();
        let mut availability = Vec::new();
        for (collection, total) in [
            ("inventory", full.inventory.len()),
            ("groups", full.drafts.iter().map(|d| d.groups.len()).sum()),
            ("decisions", full.decisions.len()),
            ("advice_decisions", full.advice_decisions.len()),
            (
                "companions",
                full.ownership_candidates
                    .iter()
                    .map(|c| c.companions.len())
                    .sum(),
            ),
            (
                "boundary_observations",
                full.boundary_observations.records.len(),
            ),
            ("test_coupling", full.test_observations.records.len()),
            ("overlaps", full.overlaps.len()),
            ("chain_diagnostics", full.chain_diagnostics.len()),
        ] {
            availability.push(json!({"collection":collection,"total":total,"returned_records":0,"omitted_from_manifest":total,"retrievable":retention.state=="retained","permanently_unavailable":if retention.state=="retained" {0}else{total},"tool":if retention.state=="retained" {Some("get_split_detail")}else{None::<&str>}}));
        }
        Self {
            schema_version: 1,
            envelope_kind: "split_manifest".into(),
            tool: "suggest_split".into(),
            response_mode: "compact".into(),
            advisory: true,
            analysis_status: full.status.clone(),
            manifest_complete: true,
            root: full.root.clone(),
            snapshot_id: full.snapshot_id.clone(),
            totals: json!({"inventory":full.counts.inventory_items,"decisions":full.counts.decisions,"advice_decisions":full.counts.advice_decisions,"candidates":full.counts.ownership_candidates,"drafts":full.counts.drafts}),
            coverage: json!({"scope":full.coverage,"skipped":full.skipped,"truncation_reasons":full.truncation_reasons,"draft_eligibility":full.draft_eligibility,"partition_outcome":full.partition_outcome,"boundary":full.boundary_observations.coverage,"tests":full.test_observations.coverage,"effective_work_limits":full.effective_work_limits,"source":full.source,"local_forecasts":full.drafts.iter().map(|d|json!({"draft_id":d.id,"groups":d.groups.iter().map(|g|json!({"expected_to_block":g.expected_to_block,"assessment_scope":g.assessment_scope,"test_coupled":g.test_coupled})).collect::<Vec<_>>()})).collect::<Vec<_>>()}),
            consequence_summaries: full
                .ownership_candidates
                .iter()
                .map(|c| json!({"candidate_id":c.id,"summary":c.consequence_summary}))
                .collect(),
            candidate_summaries: candidates,
            draft_memberships: memberships,
            decision_groups: full.decision_groups.clone(),
            detail_availability: availability,
            retention,
            omissions: full.counts.omissions.clone(),
            error: full.error.clone(),
            integrity: full.integrity.clone(),
        }
    }
    fn set_retention(&mut self, retention: Retention) {
        for a in &mut self.detail_availability {
            let available = retention.state == "retained";
            a["retrievable"] = json!(available);
            a["permanently_unavailable"] = if available {
                json!(0)
            } else {
                a["total"].clone()
            };
            a["tool"] = if available {
                json!("get_split_detail")
            } else {
                Value::Null
            };
        }
        self.retention = retention;
    }
    fn overflow(&mut self) {
        self.manifest_complete = false;
        self.error = Some(DomainError::new(
            "ADVICE_MANIFEST_TOO_LARGE",
            "mandatory membership/consequence/coverage manifest cannot fit; no handle published",
        ));
        self.omissions
            .insert("candidate_summaries".into(), self.candidate_summaries.len());
        self.candidate_summaries.clear();
        self.omissions
            .insert("draft_memberships".into(), self.draft_memberships.len());
        self.draft_memberships.clear();
        self.omissions.insert(
            "consequence_summaries".into(),
            self.consequence_summaries.len(),
        );
        self.consequence_summaries.clear();
        self.omissions
            .insert("decision_groups".into(), self.decision_groups.len());
        self.decision_groups.clear();
        self.coverage = json!({"details":"unavailable_response_budget","scope":self.coverage["scope"],"partition_outcome":self.coverage["partition_outcome"]});
    }
}

pub(super) fn present(
    mut full: SuggestSplitEnvelope,
    evidence: Option<Evidence>,
    request: &SuggestSplitRequest,
    store: &Store,
    controls: Controls<'_>,
) -> SplitResponse {
    let budget = request
        .limits
        .response_bytes
        .clamp(64 * 1024, 16 * 1024 * 1024);
    let mut record = None;
    let mut retention = Retention::unavailable(store.limits, "analysis_incomplete");
    if !request.retain_snapshot {
        retention.state = "not_requested".into();
        retention.reason = None;
    } else if full.status == "complete"
        && full.error.is_none()
        && full.coverage.scope_exhaustive
        && let Some(evidence) = evidence
    {
        match store.prepare(&full, evidence, request, Instant::now(), controls) {
            Ok(prepared) => {
                retention = prepared.retention.clone();
                record = Some(prepared);
            }
            Err(reason) => retention = Retention::unavailable(store.limits, reason),
        }
    }
    if let Err(error) = controls.check() {
        record = None;
        if request.retain_snapshot {
            retention = Retention::unavailable(store.limits, "cancelled_before_publication");
        }
        if full.error.is_none() {
            full.incomplete(&error.code);
            full.error = Some(error);
        }
    }
    // Terminal output is still bounded after cancellation; it cannot publish a record.
    let terminal_flag = AtomicBool::new(false);
    let terminal = Controls {
        deadline: Instant::now() + Duration::from_secs(5),
        cancelled: &terminal_flag,
    };
    let mut response = if request.response_mode == ResponseMode::Compact {
        let mut manifest = Manifest::from_full(&full, retention.clone());
        if wire_bytes(&manifest) > budget {
            manifest.set_retention(if request.retain_snapshot {
                Retention::unavailable(store.limits, "response_budget")
            } else {
                retention.clone()
            });
            manifest.overflow();
            record = None;
        }
        SplitResponse::Compact(Box::new(manifest))
    } else {
        full.limits.response_bytes = budget;
        if request.retain_snapshot {
            full.retention = Some(retention.clone());
        }
        if request.retain_snapshot && full.inventory.len() > request.max_items {
            let omitted = full.inventory.len() - request.max_items;
            full.inventory.truncate(request.max_items);
            full.omit("inventory_items", omitted);
            full.incomplete("max_items");
            full.draft_eligibility.membership_complete = false;
        }
        full.shape_decisions(&request.limits);
        if let Err(error) = full.fit(if controls.check().is_ok() {
            controls
        } else {
            terminal
        }) {
            record = None;
            if full.error.is_none() {
                full.incomplete(&error.code);
                full.error = Some(error);
            }
            if request.retain_snapshot {
                full.retention = Some(Retention::unavailable(
                    store.limits,
                    "cancelled_before_publication",
                ));
            }
            let _ = full.fit(terminal);
        }
        // Large normalized options are mandatory retention metadata, not silently truncated.
        if full.wire_bytes() > budget {
            record = None;
            if request.retain_snapshot {
                full.retention = Some(Retention::unavailable(store.limits, "response_budget"));
            }
            let _ = full.fit(terminal);
        }
        SplitResponse::Full(Box::new(full))
    };
    if let Some(record) = record {
        let retention = store
            .publish(record, controls, Instant::now())
            .unwrap_or_else(|reason| Retention::unavailable(store.limits, reason));
        // Publication finalizes charge and fixed-width identity/expiry metadata outside the lock.
        match &mut response {
            SplitResponse::Full(full) => full.retention = Some(retention),
            SplitResponse::Compact(manifest) => manifest.set_retention(retention),
        }
    }
    response
}

#[cfg(test)]
mod tests;
