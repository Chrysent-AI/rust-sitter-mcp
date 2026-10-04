use clap::Parser;

#[derive(Parser)]
#[command(name = "rust-sitter-mcp", version = env!("VERSION_FULL"))]
#[command(about = "Read-only structural Rust search over stdio MCP")]
struct Cli {}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    use anyhow::Context;
    use rmcp::ServiceExt;
    Cli::parse();
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .init();
    let service = rust_sitter_mcp::mcp::Server::default()
        .serve(rmcp::transport::stdio())
        .await
        .context("initialize stdio MCP service")?;
    service.waiting().await.context("run stdio MCP service")?;
    Ok(())
}
