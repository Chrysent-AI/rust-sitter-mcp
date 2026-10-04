use rmcp::schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[schemars(crate = "rmcp::schemars")]
#[serde(default, deny_unknown_fields)]
pub struct Context {
    /// Actual source lines before the match (default 2, maximum 20).
    pub before_lines: usize,
    /// Actual source lines after the match (default 2, maximum 20).
    pub after_lines: usize,
}
impl Default for Context {
    fn default() -> Self {
        Self {
            before_lines: 2,
            after_lines: 2,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[schemars(crate = "rmcp::schemars")]
#[serde(default, deny_unknown_fields)]
pub struct Limits {
    pub max_file_bytes: usize,
    pub max_files: usize,
    pub max_source_bytes: usize,
    pub time_budget_ms: u64,
    pub query_state_limit: u32,
    pub response_bytes: usize,
    pub diagnostic_count: usize,
    pub text_bytes: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            max_file_bytes: 2 * 1024 * 1024,
            max_files: 20_000,
            max_source_bytes: 256 * 1024 * 1024,
            time_budget_ms: 60_000,
            query_state_limit: 4096,
            response_bytes: 2 * 1024 * 1024,
            diagnostic_count: 64,
            text_bytes: 8192,
        }
    }
}
impl Limits {
    pub fn validate(&self) -> Result<(), DomainError> {
        let valid = (1..=16 * 1024 * 1024).contains(&self.max_file_bytes)
            && (1..=100_000).contains(&self.max_files)
            && (1..=512 * 1024 * 1024).contains(&self.max_source_bytes)
            && (1..=300_000).contains(&self.time_budget_ms)
            && (1..=65_536).contains(&self.query_state_limit)
            && (64 * 1024..=16 * 1024 * 1024).contains(&self.response_bytes)
            && self.diagnostic_count <= 256
            && self.text_bytes <= 64 * 1024;
        if valid {
            Ok(())
        } else {
            Err(DomainError::new(
                "INVALID_PARAMS",
                "limits outside documented ranges",
            ))
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(deny_unknown_fields)]
pub struct SearchRequest {
    /// Required existing path inside a Git worktree; relative paths use the immutable launch directory.
    pub repo_path: String,
    /// Rust Tree-sitter query, at most 64 KiB/64 patterns/64 capture names. Each rooted pattern needs exactly one @match. Text predicates and #rust-arity? only; no directives. Written syntax only, no expanded macro bodies or semantic analysis.
    pub query: String,
    /// Union of existing root-relative files/directories. No symlink components or '..'. ANDed with globs and discovery; empty arrays are invalid.
    pub paths: Option<Vec<String>>,
    /// Positive root-anchored gitignore-style inclusion globs; union then AND with paths. '*' does not cross '/', '**' does. No negation, braces or directory-only patterns.
    pub globs: Option<Vec<String>>,
    #[serde(default)]
    pub context: Context,
    #[serde(default)]
    pub limits: Limits,
    /// Default 100, range 1–1000. Repeat the identical effective request with next_cursor to continue.
    #[serde(default = "default_page_size")]
    pub page_size: usize,
    pub cursor: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(deny_unknown_fields)]
pub struct PatternRequest {
    /// Required existing path inside a Git worktree; relative paths use the immutable launch directory.
    pub repo_path: String,
    /// One Rust expression, no trailing semicolon. $name binds one expression node; repeated names require byte-identical source. Field/method/path names are concrete; sequences and @capture annotations are unsupported. Dollars inside Rust literals/comments are literal; no $$ escape. At most 64 KiB, 4,096 significant nodes and 64 metavariable occurrences.
    pub pattern: String,
    /// Union of existing root-relative files/directories. No symlink components or '..'. ANDed with globs and discovery; empty arrays are invalid.
    pub paths: Option<Vec<String>>,
    /// Positive root-anchored gitignore-style inclusion globs; union then AND with paths. '*' does not cross '/', '**' does. No negation, braces or directory-only patterns.
    pub globs: Option<Vec<String>>,
    #[serde(default)]
    pub context: Context,
    #[serde(default)]
    pub limits: Limits,
    /// Default 100, range 1–1000. Repeat the identical effective request with next_cursor to continue.
    #[serde(default = "default_page_size")]
    pub page_size: usize,
    pub cursor: Option<String>,
}
impl From<PatternRequest> for SearchRequest {
    fn from(request: PatternRequest) -> Self {
        Self {
            repo_path: request.repo_path,
            query: request.pattern,
            paths: request.paths,
            globs: request.globs,
            context: request.context,
            limits: request.limits,
            page_size: request.page_size,
            cursor: request.cursor,
        }
    }
}
fn default_page_size() -> usize {
    100
}

#[derive(Debug, Clone, Serialize, JsonSchema, thiserror::Error)]
#[schemars(crate = "rmcp::schemars")]
#[error("{code}: {message}")]
pub struct DomainError {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub byte_offset: Option<usize>,
}
impl DomainError {
    pub fn new(code: &str, message: impl AsRef<str>) -> Self {
        let message: String = message.as_ref().chars().take(1024).collect();
        Self {
            code: code.into(),
            message,
            field: None,
            byte_offset: None,
        }
    }
    pub fn at_pattern(mut self, byte_offset: usize) -> Self {
        self.field = Some("pattern".into());
        self.byte_offset = Some(byte_offset);
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, PartialOrd, Ord)]
#[schemars(crate = "rmcp::schemars")]
#[serde(deny_unknown_fields)]
pub struct ByteRange {
    pub start_byte: usize,
    pub end_byte: usize,
}
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct Position {
    pub line: usize,
    pub byte_column: usize,
}
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct SourceSlice {
    pub range: ByteRange,
    pub start: Position,
    pub end: Position,
    pub text: Option<String>,
    pub text_bytes: usize,
    pub text_omitted: bool,
}
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct SyntaxFlags {
    pub file_has_recovery: bool,
    pub subtree_has_recovery: bool,
    pub enclosing_has_recovery: bool,
}
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct ContextLines {
    pub before: Vec<SourceSlice>,
    pub after: Vec<SourceSlice>,
    pub before_clipped: usize,
    pub after_clipped: usize,
    pub before_omitted: usize,
    pub after_omitted: usize,
}
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct TriviaDecision {
    pub id: String,
    pub path: String,
    pub span: SourceSlice,
    pub reason: String,
    pub default_disposition: String,
}
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct MatchRecord {
    pub id: String,
    pub path: String,
    pub span: SourceSlice,
    pub captures: BTreeMap<String, Option<Vec<SourceSlice>>>,
    pub captures_omitted: BTreeMap<String, usize>,
    pub capture_occurrences: usize,
    pub capture_occurrences_omitted: usize,
    pub pattern_index: usize,
    pub syntax: SyntaxFlags,
    pub context: ContextLines,
    pub trivia_decisions: Vec<TriviaDecision>,
    pub trivia_decisions_omitted: usize,
}
impl MatchRecord {
    pub fn descriptor(&mut self) {
        self.span.text = None;
        self.span.text_omitted = true;
        self.captures.clear();
        self.captures_omitted.clear();
        self.capture_occurrences_omitted = self.capture_occurrences;
        self.context.before_omitted += self.context.before.len();
        self.context.before.clear();
        self.context.after_omitted += self.context.after.len();
        self.context.after.clear();
        self.trivia_decisions_omitted += self.trivia_decisions.len();
        self.trivia_decisions.clear();
    }
}
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct Diagnostic {
    pub path: String,
    pub code: String,
    pub range: ByteRange,
}
#[derive(Debug, Clone, Default, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct Coverage {
    pub scan_exhausted: bool,
    pub eligible_scan_complete: bool,
    pub scope_exhaustive: bool,
}
#[derive(Debug, Clone, Default, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct Counts {
    pub discovered_files: usize,
    pub scanned_files: usize,
    pub eligible_files: usize,
    pub matched_files: usize,
    pub observed_matches: usize,
    pub returned_matches: usize,
    pub total_matches: Option<usize>,
    pub total_is_exact: bool,
}
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct SkipExample {
    pub path: Option<String>,
    pub detail_code: String,
    pub path_bytes_hex: Option<String>,
}
#[derive(Debug, Clone, Default, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct SkipReason {
    pub count: u64,
    pub examples: Vec<SkipExample>,
    pub examples_omitted: u64,
    pub count_saturated: bool,
}
pub type Skipped = BTreeMap<String, SkipReason>;
pub fn skipped_map() -> Skipped {
    [
        "oversized",
        "binary",
        "non_utf8",
        "unreadable",
        "symlink",
        "nested_repository",
        "non_utf8_path",
        "traversal_error",
    ]
    .into_iter()
    .map(|key| (key.into(), SkipReason::default()))
    .collect()
}
pub fn skip(
    skipped: &mut Skipped,
    reason: &str,
    path: &std::path::Path,
    detail: &str,
    example_budget: &mut usize,
) {
    let record = skipped.get_mut(reason).expect("fixed skip reason");
    record.count_saturated |= record.count == u64::MAX;
    record.count = record.count.saturating_add(1);
    if record.examples.len() < 3 && *example_budget > 0 {
        *example_budget -= 1;
        use std::os::unix::ffi::OsStrExt;
        let path_text = path.to_str().map(str::to_owned);
        let hex = path_text.is_none().then(|| {
            path.as_os_str()
                .as_bytes()
                .iter()
                .take(128)
                .map(|b| format!("{b:02x}"))
                .collect()
        });
        record.examples.push(SkipExample {
            path: path_text,
            detail_code: detail.into(),
            path_bytes_hex: hex,
        });
    } else {
        record.examples_omitted = record.examples_omitted.saturating_add(1);
    }
}
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct SearchEnvelope {
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
    pub matches: Vec<MatchRecord>,
    pub next_cursor: Option<String>,
    pub has_more: Option<bool>,
}
impl SearchEnvelope {
    pub fn empty(limits: Limits) -> Self {
        Self {
            schema_version: 1,
            tool: "search_query".into(),
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
            matches: Vec::new(),
            next_cursor: None,
            has_more: Some(false),
        }
    }
    pub fn failed(limits: Limits, root: Option<String>, error: DomainError) -> Self {
        let mut result = Self::empty(limits);
        result.status = "failed".into();
        result.root = root;
        result.error = Some(error);
        result.has_more = None;
        result
    }
    /// Exact structured/text duplication plus a conservative JSON-RPC framing reserve.
    pub fn wire_bytes(&self) -> usize {
        let value = serde_json::to_value(self).expect("serializable envelope");
        serde_json::to_vec(&serde_json::json!({"content":[{"type":"text","text":value.to_string()}],"structuredContent":value,"isError":self.error.is_some()})).expect("serializable wire").len() + 4096
    }
}
