//! Read-only structural search of written Rust syntax.
mod cursor;
pub mod edit;
pub mod engine;
mod matching;
pub mod mcp;
mod patch;
mod pattern;
pub mod plan;
mod query;
pub mod result;
mod scope;
mod template;
mod trivia;
