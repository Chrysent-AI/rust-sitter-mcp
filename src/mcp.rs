use crate::{
    engine::Engine,
    result::{DomainError, Limits, SearchEnvelope, SearchRequest},
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
    active: Arc<Mutex<Vec<Weak<AtomicBool>>>>,
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
            active: Arc::new(Mutex::new(Vec::new())),
        })
    }
    pub async fn shutdown(&self) {
        for flag in self
            .active
            .lock()
            .expect("active lock")
            .iter()
            .filter_map(Weak::upgrade)
        {
            flag.store(true, Ordering::Relaxed);
        }
        // The owned permit lives until all blocking workers and Git children have joined.
        let _permit = self.admission.acquire().await;
    }
}
#[tool_router]
impl Server {
    #[tool(name = "search_query", description = "Read-only structural Rust search. Root each pattern at exactly one @match. Captures are arrays, not SSR unification. Supports Tree-sitter text predicates and #rust-arity? @arguments \"N\" only. Original half-open byte ranges; lines 1-based and UTF-8 columns 0-based. Context is exact complete lines. Limits may omit field text or return partial pages; repeat identical arguments with next_cursor (15-minute process-local series). No types, cfg evaluation or macro expansion; an empty search does not prove absence in generated code.", output_schema = rmcp::handler::server::tool::schema_for_output::<SearchEnvelope>(), annotations(read_only_hint = true, destructive_hint = false, open_world_hint = false))]
    async fn search_query(
        &self,
        Parameters(request): Parameters<SearchRequest>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        let Ok(permit) = self.admission.clone().try_acquire_owned() else {
            return wire(SearchEnvelope::failed(
                request.limits,
                None,
                DomainError::new(
                    "BUSY",
                    "another engine call is running; retry after it finishes",
                ),
            ));
        };
        let flag = Arc::new(AtomicBool::new(false));
        let _guard = CancelOnDrop(flag.clone());
        {
            let mut active = self.active.lock().expect("active lock");
            active.retain(|weak| weak.strong_count() > 0);
            active.push(Arc::downgrade(&flag));
        }
        let request_id: String = format!("{:?}", context.id).chars().take(128).collect();
        let span = tracing::info_span!("search_call", tool = "search_query", request = %request_id, paths_count = request.paths.as_ref().map_or(0, Vec::len), globs_count = request.globs.as_ref().map_or(0, Vec::len));
        let engine = self.engine.clone();
        let limits = request.limits.clone();
        let worker_flag = flag.clone();
        let mut job = tokio::task::spawn_blocking(move || {
            let _entered = span.enter();
            let _permit = permit;
            engine.search(request, &worker_flag)
        });
        let result = tokio::select! {
            result = &mut job => result,
            _ = context.ct.cancelled() => { flag.store(true,Ordering::Relaxed); job.await }
        };
        match result {
            Ok(result) => wire(result),
            Err(_) => wire(SearchEnvelope::failed(
                limits,
                None,
                DomainError::new("INTERNAL", "blocking engine task failed"),
            )),
        }
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
        if request.name != "search_query" {
            return Err(rmcp::ErrorData::method_not_found::<
                rmcp::model::CallToolRequestMethod,
            >());
        }
        let args = serde_json::Value::Object(request.arguments.clone().unwrap_or_default());
        let decoded = if serde_json::to_vec(&args).expect("JSON args").len() > 8 * 1024 * 1024 {
            Err("decoded arguments exceed 8 MiB".into())
        } else {
            serde_json::from_value::<SearchRequest>(args).map_err(|e| e.to_string())
        };
        if let Err(message) = decoded {
            return Ok(wire(SearchEnvelope::failed(
                Limits::default(),
                None,
                DomainError::new("INVALID_PARAMS", message),
            ))
            .into());
        }
        self.router
            .call(ToolCallContext::new(self, request, context))
            .await
    }
}
