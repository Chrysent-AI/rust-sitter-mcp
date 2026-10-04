use crate::engine::{SearchRequest, SearchResult};
use rmcp::{
    ServerHandler,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{CallToolResult, Implementation, ServerCapabilities, ServerConfig},
    tool, tool_handler, tool_router,
};
use std::sync::Arc;

#[derive(Clone)]
pub struct Server {
    router: ToolRouter<Self>,
    admission: Arc<tokio::sync::Semaphore>,
}

impl Default for Server {
    fn default() -> Self {
        Self {
            router: Self::tool_router(),
            admission: Arc::new(tokio::sync::Semaphore::new(1)),
        }
    }
}

#[tool_router]
impl Server {
    #[tool(name = "search_query", description = "Read-only structural search of written Rust syntax using a raw Tree-sitter query rooted at @match; no types, cfg evaluation or macro expansion.", output_schema = rmcp::handler::server::tool::schema_for_output::<SearchResult>(), annotations(read_only_hint = true, destructive_hint = false, open_world_hint = false))]
    async fn search_query(&self, Parameters(request): Parameters<SearchRequest>) -> CallToolResult {
        let Ok(permit) = self.admission.clone().try_acquire_owned() else {
            return CallToolResult::structured_error(serde_json::json!({"error":{"code":"BUSY"}}));
        };
        let result = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            crate::engine::search(request)
        })
        .await;
        match result {
            Ok(Ok(result)) => {
                tracing::info!(
                    tool = "search_query",
                    returned = result.matches.len(),
                    "search complete"
                );
                CallToolResult::structured(
                    serde_json::to_value(result).expect("serializable search result"),
                )
            }
            other => {
                CallToolResult::structured_error(serde_json::json!({"error":format!("{other:?}")}))
            }
        }
    }
}

#[tool_handler(router = self.router)]
impl ServerHandler for Server {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("rust-sitter-mcp", env!("VERSION_FULL")))
    }
}
