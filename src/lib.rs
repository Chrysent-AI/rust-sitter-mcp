//! Read-only structural search of written Rust syntax.
mod cursor;
pub mod edit;
pub mod engine;
mod items;
mod matching;
pub mod mcp;
pub mod move_plan;
mod patch;
mod pattern;
pub mod plan;
mod query;
pub mod result;
mod scope;
mod template;
mod trivia;
