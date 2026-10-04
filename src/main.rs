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
    let server = rust_sitter_mcp::mcp::Server::new().context("set up Rust query engine")?;
    let service = server
        .clone()
        .serve(rmcp::transport::stdio())
        .await
        .context("initialize stdio MCP service")?;
    let outcome = service.waiting().await;
    server.shutdown().await;
    outcome.context("run stdio MCP service")?;
    Ok(())
}
