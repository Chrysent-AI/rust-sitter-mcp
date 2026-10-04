use clap::Parser;

#[derive(Parser)]
#[command(name = "rust-sitter-mcp", version = env!("VERSION_FULL"))]
#[command(about = "Structural Rust search and refactoring proposals (package skeleton)")]
struct Cli {}

fn main() {
    Cli::parse();
}
