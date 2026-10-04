#[path = "support/fixture_gen.rs"]
mod fixture_gen;
#[path = "support/move_artifacts.rs"]
mod move_artifacts;
use fixture_gen::{Fixture, observe};
use move_artifacts::{anchor, apply};
use rust_sitter_mcp::{engine::Engine, move_plan::MoveRequest};
use serde_json::{Value, json};
use std::{fs, process::Command, sync::atomic::AtomicBool};
fn run(repo: &Fixture, args: Value) -> Value {
    let before = observe(&repo.0);
    let request: MoveRequest = serde_json::from_value(args).unwrap();
    let result = Engine::new(repo.0.clone())
        .unwrap()
        .move_item(request, &AtomicBool::new(false));
    assert_eq!(observe(&repo.0), before);
    serde_json::to_value(result).unwrap()
}
fn withheld(value: &Value) {
    assert_eq!(value["plan"]["applicable"], false, "{value}");
    for field in ["edits", "created_files", "patch"] {
        assert!(value["plan"][field].is_null(), "{value}");
    }
}
#[test]
fn private_remaining_caller_vertical_slice() {
    let repo = Fixture::generate();
    repo.write("cases/layout/lib.rs", "mod source;\n");
    repo.write("cases/layout/source.rs", "fn helper() -> u8 { 1 }\n/// Private docs.\nfn private() -> u8 { helper() }\nfn caller() -> u8 { private() + self::private() }\n");
    let args = json!({"repo_path":repo.0,"crate_root":"cases/layout/lib.rs","paths":["cases/layout"],"moves":[{"item":anchor(&repo,"cases/layout/source.rs","fn private() -> u8 { helper() }"),"destination":{"kind":"new_sibling","path":"cases/layout/target.rs","parent_path":"cases/layout/lib.rs"}}]});
    let result = run(&repo, args.clone());
    let copy = apply(&repo, &result);
    let content = fs::read_to_string(copy.0.join("cases/layout/target.rs")).unwrap();
    assert!(
        content.contains("use crate::source::helper;") && content.contains("pub(crate) fn private")
    );
    let source = fs::read_to_string(copy.0.join("cases/layout/source.rs")).unwrap();
    assert!(
        source.contains("use crate::target::private;")
            && source.contains("crate::target::private()")
    );
    assert!(source.contains("pub(crate) fn helper"));
    assert!(
        !fs::read_to_string(copy.0.join("cases/layout/lib.rs"))
            .unwrap()
            .contains("pub(crate) mod target")
    );
    let check = Command::new("rustc")
        .current_dir(&copy.0)
        .args([
            "--edition=2024",
            "--crate-type=lib",
            "cases/layout/lib.rs",
            "--out-dir",
        ])
        .arg(&copy.0)
        .output()
        .unwrap();
    assert!(
        check.status.success(),
        "{}",
        String::from_utf8_lossy(&check.stderr)
    );
    let rewrites = result["plan"]["rewrites"].as_array().unwrap();
    for kind in ["path", "import_insert", "visibility"] {
        assert!(rewrites.iter().any(|r| r["kind"] == kind));
    }
    let mut replay = args.clone();
    replay["rewrite_overrides"] = json!(
        rewrites
            .iter()
            .map(|r| json!({"target":r["target"],"action":"accept_default"}))
            .collect::<Vec<_>>()
    );
    assert_eq!(result["plan"], run(&repo, replay)["plan"]);
    for rewrite in rewrites.iter().filter(|r| r["kind"] != "separator") {
        let mut reject = args.clone();
        reject["rewrite_overrides"] = json!([{"target":rewrite["target"],"action":"retain"}]);
        withheld(&run(&repo, reject));
    }
}
