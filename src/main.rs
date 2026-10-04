use clap::Parser;
use rmcp::{
    RoleServer,
    service::{RxJsonRpcMessage, TxJsonRpcMessage},
    transport::{Transport, async_rw::AsyncRwTransport},
};
use rust_sitter_mcp::mcp::Server;

struct CancelOnClose<T> {
    inner: T,
    server: Server,
}
impl<T: Transport<RoleServer>> Transport<RoleServer> for CancelOnClose<T> {
    type Error = T::Error;

    fn send(
        &mut self,
        message: TxJsonRpcMessage<RoleServer>,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send + 'static {
        self.inner.send(message)
    }

    async fn receive(&mut self) -> Option<RxJsonRpcMessage<RoleServer>> {
        let message = self.inner.receive().await;
        if message.is_none() {
            // rmcp drains handlers after EOF; cancel before that drain starts.
            self.server.cancel_requests();
        }
        message
    }

    async fn close(&mut self) -> Result<(), Self::Error> {
        self.server.cancel_requests();
        self.inner.close().await
    }
}

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
    let server = Server::new().context("set up Rust query engine")?;
    let (input, output) = rmcp::transport::stdio();
    let transport = CancelOnClose {
        inner: AsyncRwTransport::new_server(input, output),
        server: server.clone(),
    };
    let service = server
        .clone()
        .serve(transport)
        .await
        .context("initialize stdio MCP service")?;
    let outcome = service.waiting().await;
    server.shutdown().await;
    outcome.context("run stdio MCP service")?;
    Ok(())
}
