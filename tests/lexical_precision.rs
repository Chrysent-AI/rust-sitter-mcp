#[path = "support/fixture_gen.rs"]
mod fixture_gen;
#[path = "support/move_artifacts.rs"]
mod move_artifacts;
#[path = "support/spawner_precision.rs"]
mod spawner_precision;
use fixture_gen::{Fixture, observe};
use move_artifacts::anchor;
use rust_sitter_mcp::{engine::Engine, move_plan::MoveRequest};
use serde_json::{Value, json};
use std::sync::atomic::AtomicBool;

const SOURCE: &str = "cases/precision/subagent/spawner.rs";
fn run(repo: &Fixture, args: Value) -> Value {
    let before = observe(&repo.0);
    let request: MoveRequest = serde_json::from_value(args).unwrap();
    let result = Engine::new(repo.0.clone())
        .unwrap()
        .move_item(request, &AtomicBool::new(false));
    assert_eq!(observe(&repo.0), before);
    serde_json::to_value(result).unwrap()
}
fn request(repo: &Fixture, text: &str) -> Value {
    json!({"repo_path":repo.0,"crate_root":"cases/precision/lib.rs","paths":["cases/precision"],"limits":{"text_bytes":0},"moves":[{"item":anchor(repo,SOURCE,text),"destination":{"kind":"new_sibling","path":"cases/precision/subagent/probe_constants.rs","parent_path":"cases/precision/subagent/mod.rs"}}]})
}
#[test]
fn raw_line_replica_applies_losslessly() {
    let repo = spawner_precision::load(include_str!("fixtures/spawner_precision/raw_line.rs"));
    let result = run(
        &repo,
        request(
            &repo,
            "const TRANSCRIPT_MAX_RAW_LINE: usize = 16 * 1024 * 1024;",
        ),
    );
    assert_eq!(result["plan"]["applicable"], true, "{result}");
    assert_eq!(result["plan"]["integrity"]["semantic"], "not_performed");
    let applied = move_artifacts::apply(&repo, &result);
    let source = std::fs::read_to_string(applied.0.join(SOURCE)).unwrap();
    assert!(source.contains("use crate::subagent::probe_constants::TRANSCRIPT_MAX_RAW_LINE;"));
    assert!(source.contains("stdout_carry.len() > TRANSCRIPT_MAX_RAW_LINE"));
    assert_eq!(
        result,
        run(
            &repo,
            request(
                &repo,
                "const TRANSCRIPT_MAX_RAW_LINE: usize = 16 * 1024 * 1024;"
            )
        )
    );
}
