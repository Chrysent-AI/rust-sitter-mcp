use crate::{
    engine::Engine,
    plan::{PlanEnvelope, ReplaceRequest},
    result::{DomainError, Limits, PatternRequest, SearchEnvelope, SearchRequest},
};
use rmcp::{
    RoleServer, ServerHandler,
    handler::server::{router::tool::ToolRouter, tool::ToolCallContext, wrapper::Parameters},
    model::{
        CallToolRequestParams, CallToolResponse, CallToolResult, Implementation, ListToolsResult,
        PaginatedRequestParams, ServerCapabilities, ServerConfig, Tool,
    },
    service::RequestContext,
    tool, tool_router,
};
use std::sync::{
    Arc, Mutex, Weak,
    atomic::{AtomicBool, Ordering},
};

#[derive(Clone)]
pub struct Server {
    router: ToolRouter<Self>,
    engine: Arc<Engine>,
    admission: Arc<tokio::sync::Semaphore>,
    active: Arc<Mutex<ActiveRequests>>,
}
#[derive(Default)]
struct ActiveRequests {
    cancelled: bool,
    flags: Vec<Weak<AtomicBool>>,
}
struct CancelOnDrop(Arc<AtomicBool>);
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}
fn wire(result: SearchEnvelope) -> CallToolResult {
    let failed = result.error.is_some();
    let value = serde_json::to_value(result).expect("serializable envelope");
    if failed {
        CallToolResult::structured_error(value)
    } else {
        CallToolResult::structured(value)
    }
}
impl Server {
    pub fn new() -> Result<Self, DomainError> {
        let launch = std::env::current_dir()
            .map_err(|_| DomainError::new("INTERNAL", "cannot capture launch directory"))?;
        Ok(Self {
            router: Self::tool_router(),
            engine: Arc::new(Engine::new(launch)?),
            admission: Arc::new(tokio::sync::Semaphore::new(1)),
            active: Arc::new(Mutex::new(ActiveRequests::default())),
        })
    }
    pub fn cancel_requests(&self) {
        let mut active = self.active.lock().expect("active lock");
        // Also cancel handlers already dispatched but not yet registered at EOF.
        active.cancelled = true;
        for flag in active.flags.iter().filter_map(Weak::upgrade) {
            flag.store(true, Ordering::Relaxed);
        }
    }
    async fn run_search(
        &self,
        request: SearchRequest,
        sugar: bool,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        let tool = if sugar { "search" } else { "search_query" };
        let failure = |limits, error| {
            let mut result = SearchEnvelope::failed(limits, None, error);
            result.tool = tool.into();
            wire(result)
        };
        let Ok(permit) = self.admission.clone().try_acquire_owned() else {
            return failure(
                request.limits,
                DomainError::new(
                    "BUSY",
                    "another engine call is running; retry after it finishes",
                ),
            );
        };
        let flag = Arc::new(AtomicBool::new(false));
        let _guard = CancelOnDrop(flag.clone());
        {
            let mut active = self.active.lock().expect("active lock");
            flag.store(active.cancelled, Ordering::Relaxed);
            active.flags.retain(|weak| weak.strong_count() > 0);
            active.flags.push(Arc::downgrade(&flag));
        }
        let request_id: String = format!("{:?}", context.id).chars().take(128).collect();
        let span = tracing::info_span!("search_call", tool, request = %request_id, paths_count = request.paths.as_ref().map_or(0, Vec::len), globs_count = request.globs.as_ref().map_or(0, Vec::len));
        let engine = self.engine.clone();
        let limits = request.limits.clone();
        let worker_flag = flag.clone();
        let mut job = tokio::task::spawn_blocking(move || {
            let _entered = span.enter();
            let _permit = permit;
            engine.search_interpreted(request, sugar, &worker_flag)
        });
        let result = tokio::select! {
            result = &mut job => result,
            _ = context.ct.cancelled() => { flag.store(true,Ordering::Relaxed); job.await }
        };
        match result {
            Ok(result) => wire(result),
            Err(_) => failure(
                limits,
                DomainError::new("INTERNAL", "blocking engine task failed"),
            ),
        }
    }
    async fn run_replace(
        &self,
        request: ReplaceRequest,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        let failure = |limits, error| plan_wire(PlanEnvelope::failed(limits, error));
        let Ok(permit) = self.admission.clone().try_acquire_owned() else {
            return failure(
                request.limits,
                DomainError::new(
                    "BUSY",
                    "another engine call is running; retry after it finishes",
                ),
            );
        };
        let flag = Arc::new(AtomicBool::new(false));
        let _guard = CancelOnDrop(flag.clone());
        {
            let mut active = self.active.lock().expect("active lock");
            flag.store(active.cancelled, Ordering::Relaxed);
            active.flags.retain(|weak| weak.strong_count() > 0);
            active.flags.push(Arc::downgrade(&flag));
        }
        let request_id: String = format!("{:?}", context.id).chars().take(128).collect();
        let span = tracing::info_span!("replace_call", tool="replace", request=%request_id, paths_count=request.paths.as_ref().map_or(0, Vec::len), globs_count=request.globs.as_ref().map_or(0, Vec::len));
        let engine = self.engine.clone();
        let limits = request.limits.clone();
        let worker_flag = flag.clone();
        let mut job = tokio::task::spawn_blocking(move || {
            let _entered = span.enter();
            let _permit = permit;
            engine.replace(request, &worker_flag)
        });
        let result = tokio::select! {
            result = &mut job => result,
            _ = context.ct.cancelled() => { flag.store(true, Ordering::Relaxed); job.await }
        };
        match result {
            Ok(result) => plan_wire(result),
            Err(_) => failure(
                limits,
                DomainError::new("INTERNAL", "blocking engine task failed"),
            ),
        }
    }
    pub async fn shutdown(&self) {
        self.cancel_requests();
        // The owned permit lives until all blocking workers and Git children have joined.
        let _permit = self.admission.acquire().await;
    }
}
fn plan_wire(result: PlanEnvelope) -> CallToolResult {
    let failed = result.error.is_some();
    let value = serde_json::to_value(result).expect("serializable plan");
    if failed {
        CallToolResult::structured_error(value)
    } else {
        CallToolResult::structured(value)
    }
}
#[tool_router]
impl Server {
    #[tool(name = "replace", description = "Read-only dry-run Rust expression replacement. Same sugar grammar as search. Bound $name copies exact capture bytes; no automatic parentheses, dedent or formatting. Original-coordinate anchors with expected_text select exact matches; omitted selection means all, [] means none. max_matches counts scope matches before selection. Keep trivia in place by default; unretained trivia, conflicts, syntax recovery, source changes and limits withhold ALL artifacts. Applicable plans include complete Git patch (three context lines) and original-byte JSON edits; server never applies them. Integrity is tree-sitter syntax only, semantic checking is not performed. Review artifacts and verify unchanged base bytes before external application.", output_schema = rmcp::handler::server::tool::schema_for_output::<PlanEnvelope>(), annotations(read_only_hint = true, destructive_hint = false, open_world_hint = false))]
    async fn replace(
        &self,
        Parameters(request): Parameters<ReplaceRequest>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        self.run_replace(request, context).await
    }
    #[tool(name = "search_query", description = "Read-only structural Rust search. Root each pattern at exactly one @match. Captures are arrays, not SSR unification. Supports Tree-sitter text predicates and #rust-arity? @arguments \"N\" only. Original half-open byte ranges; lines 1-based and UTF-8 columns 0-based. Context is exact complete lines. Limits may omit field text or return partial pages; repeat identical arguments with next_cursor (15-minute process-local series). No types, cfg evaluation or macro expansion; an empty search does not prove absence in generated code.", output_schema = rmcp::handler::server::tool::schema_for_output::<SearchEnvelope>(), annotations(read_only_hint = true, destructive_hint = false, open_world_hint = false))]
    async fn search_query(
        &self,
        Parameters(request): Parameters<SearchRequest>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        self.run_search(request, false, context).await
    }
    #[tool(name = "search", description = "Primary read-only Rust expression search. Examples: $a.unwrap(), $a.expect($b), pair($a,$a). Authored roots: $name, identifiers/self/scoped paths, literals (including byte/C prefixes), calls/fields/tuple fields, parentheses/unit/tuple/arrays/repeat arrays, unary/reference/try/await/index, binary/assignment/compound-assignment/range. Blocks/control flow/closures/structs/casts/generics/macros cannot be authored; use search_query. $name binds one expression node (even a whole source block/closure/generic call/macro), never a list. Allowed in expression operands, callee/receiver and each argument; field/method/path names are concrete. Names are ASCII identifiers excluding __ssr_. Repeated names require byte-identical source; distinct names bind independently. Concrete leaves/punctuation and exact arity must match; comment extras and whitespace gaps are ignored. One expression without a semicolon; no sequences, $$ escape or @capture annotation. Dollars inside literals/comments are literal. Original half-open byte ranges, 1-based lines and 0-based UTF-8 byte columns. Context/limits/cursors share search_query behavior; repeat identical arguments with next_cursor (15-minute process-local series). Written syntax only: no types, cfg evaluation or macro expansion; empty results do not prove absence in generated code.", output_schema = rmcp::handler::server::tool::schema_for_output::<SearchEnvelope>(), annotations(read_only_hint = true, destructive_hint = false, open_world_hint = false))]
    async fn search(
        &self,
        Parameters(request): Parameters<PatternRequest>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        self.run_search(request.into(), true, context).await
    }
}
impl ServerHandler for Server {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("rust-sitter-mcp", env!("VERSION_FULL")))
    }
    async fn list_tools(
        &self,
        _: Option<PaginatedRequestParams>,
        _: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, rmcp::ErrorData> {
        Ok(ListToolsResult::with_all_items(self.router.list_all()))
    }
    fn get_tool(&self, name: &str) -> Option<Tool> {
        self.router
            .list_all()
            .into_iter()
            .find(|tool| tool.name == name)
    }
    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, rmcp::ErrorData> {
        if request.name != "search_query" && request.name != "search" && request.name != "replace" {
            return Err(rmcp::ErrorData::method_not_found::<
                rmcp::model::CallToolRequestMethod,
            >());
        }
        let args = serde_json::Value::Object(request.arguments.clone().unwrap_or_default());
        let decoded = if serde_json::to_vec(&args).expect("JSON args").len() > 8 * 1024 * 1024 {
            Err("decoded arguments exceed 8 MiB".into())
        } else {
            if request.name == "replace" {
                serde_json::from_value::<ReplaceRequest>(args)
                    .map(|_| ())
                    .map_err(|e| e.to_string())
            } else if request.name == "search" {
                serde_json::from_value::<PatternRequest>(args)
                    .map(|_| ())
                    .map_err(|e| e.to_string())
            } else {
                serde_json::from_value::<SearchRequest>(args)
                    .map(|_| ())
                    .map_err(|e| e.to_string())
            }
        };
        if let Err(message) = decoded {
            if request.name == "replace" {
                return Ok(plan_wire(PlanEnvelope::failed(
                    Limits::default(),
                    DomainError::new("INVALID_PARAMS", message),
                ))
                .into());
            }
            let mut failed = SearchEnvelope::failed(
                Limits::default(),
                None,
                DomainError::new("INVALID_PARAMS", message),
            );
            failed.tool = request.name.to_string();
            return Ok(wire(failed).into());
        }
        self.router
            .call(ToolCallContext::new(self, request, context))
            .await
    }
}
