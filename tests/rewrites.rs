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
fn compile_layout(repo: &Fixture) {
    let copy = repo.copy();
    copy.write("Cargo.toml", "[package]\nname = \"fixture-corpus\"\nversion = \"0.0.0\"\nedition = \"2024\"\n[lib]\npath = \"cases/layout/lib.rs\"\n");
    let check = Command::new("cargo")
        .current_dir(&copy.0)
        .args(["check", "--locked", "--offline", "--quiet"])
        .output()
        .unwrap();
    assert!(
        check.status.success(),
        "{}",
        String::from_utf8_lossy(&check.stderr)
    );
}
#[test]
fn full_module_chain_and_declaration_visibility_links() {
    for reused in [false, true] {
        let repo = Fixture::generate();
        repo.write("cases/layout/lib.rs", "mod a;\nmod b;\n");
        repo.write(
            "cases/layout/a.rs",
            if reused {
                "pub(crate) mod nested;\nmod target;\n"
            } else {
                "pub(crate) mod nested;\n"
            },
        );
        repo.write("cases/layout/a/nested.rs", "pub(crate) fn selected() {}\n");
        repo.write(
            "cases/layout/b.rs",
            "fn caller() { crate::a::nested::selected(); }\n",
        );
        if !reused {
            compile_layout(&repo);
        }
        let args = json!({"repo_path":repo.0,"crate_root":"cases/layout/lib.rs","paths":["cases/layout"],"moves":[{"item":anchor(&repo,"cases/layout/a/nested.rs","pub(crate) fn selected() {}"),"destination":{"kind":"new_sibling","path":"cases/layout/a/target.rs","parent_path":"cases/layout/a.rs"}}]});
        let result = run(&repo, args.clone());
        let copy = apply(&repo, &result);
        compile_layout(&copy);
        assert!(
            fs::read_to_string(copy.0.join("cases/layout/a.rs"))
                .unwrap()
                .contains("pub(crate) mod target;")
        );
        assert_eq!(
            fs::read_to_string(copy.0.join("cases/layout/lib.rs")).unwrap(),
            "mod a;\nmod b;\n"
        );
        let file = &result["plan"]["created_files"][0];
        let visibility = result["plan"]["rewrites"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["id"] == file["declaration_visibility_rewrite_id"])
            .unwrap();
        assert_eq!(visibility["kind"], "visibility");
        let mut rejected = args;
        rejected["rewrite_overrides"] = json!([{"target":visibility["target"],"action":"retain"}]);
        withheld(&run(&repo, rejected));
    }
}
#[test]
fn independent_lexical_bindings_do_not_create_access_needs() {
    for caller in [
        "fn caller() { fn selected() {} selected(); }",
        "fn caller(selected: fn()) { selected(); }",
        "fn caller() { let selected = || {}; selected(); }",
    ] {
        let repo = Fixture::generate();
        repo.write("cases/layout/lib.rs", "mod source;\n");
        repo.write(
            "cases/layout/source.rs",
            &format!("fn selected() {{}}\n{caller}\n"),
        );
        compile_layout(&repo);
        let result = run(
            &repo,
            json!({"repo_path":repo.0,"crate_root":"cases/layout/lib.rs","paths":["cases/layout"],"moves":[{"item":anchor(&repo,"cases/layout/source.rs","fn selected() {}"),"destination":{"kind":"new_sibling","path":"cases/layout/target.rs","parent_path":"cases/layout/lib.rs"}}]}),
        );
        let copy = apply(&repo, &result);
        compile_layout(&copy);
        assert!(
            !result["plan"]["rewrites"]
                .as_array()
                .unwrap()
                .iter()
                .any(|r| r["kind"] == "visibility" || r["kind"] == "import_insert")
        );
    }
    let repo = Fixture::generate();
    repo.write("cases/layout/lib.rs", "mod source;\n");
    repo.write(
        "cases/layout/source.rs",
        "fn selected() {}\nfn caller(pair: (fn(),)) { let (selected,) = pair; selected(); }\n",
    );
    compile_layout(&repo);
    let result = run(
        &repo,
        json!({"repo_path":repo.0,"crate_root":"cases/layout/lib.rs","paths":["cases/layout"],"moves":[{"item":anchor(&repo,"cases/layout/source.rs","fn selected() {}"),"destination":{"kind":"new_sibling","path":"cases/layout/target.rs","parent_path":"cases/layout/lib.rs"}}]}),
    );
    withheld(&result);
    assert!(
        result["plan"]["decisions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["category"] == "binding_collision"
                && !d["anchors"].as_array().unwrap().is_empty())
    );
}
#[test]
fn imported_dependencies_reuse_dedup_and_collisions() {
    for destination in ["new", "existing", "collision"] {
        let repo = Fixture::generate();
        repo.write(
            "cases/layout/lib.rs",
            "mod source;\nmod bindings;\nmod destination;\n",
        );
        repo.write(
            "cases/layout/bindings.rs",
            "pub(crate) fn helper() -> u8 { 1 }\n",
        );
        repo.write(
            "cases/layout/source.rs",
            "use crate::bindings::helper;\nfn selected() -> u8 { helper() + helper() }\n",
        );
        repo.write(
            "cases/layout/destination.rs",
            if destination == "collision" {
                "fn helper() -> u8 { 2 }\n"
            } else {
                "use crate::bindings::{helper};\nfn keep() {}\n"
            },
        );
        compile_layout(&repo);
        let target = if destination == "new" {
            json!({"kind":"new_sibling","path":"cases/layout/target.rs","parent_path":"cases/layout/lib.rs"})
        } else {
            json!({"kind":"existing","path":"cases/layout/destination.rs"})
        };
        let result = run(
            &repo,
            json!({"repo_path":repo.0,"crate_root":"cases/layout/lib.rs","paths":["cases/layout"],"moves":[{"item":anchor(&repo,"cases/layout/source.rs","fn selected() -> u8 { helper() + helper() }"),"destination":target}]}),
        );
        if destination == "collision" {
            withheld(&result);
            continue;
        }
        let copy = apply(&repo, &result);
        compile_layout(&copy);
        assert_eq!(
            result["plan"]["rewrites"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|r| r["kind"] == "import_insert")
                .count(),
            usize::from(destination == "new")
        );
    }
}
#[test]
fn explicit_alias_group_prefix_and_leaf_extraction() {
    for (imports, selections, expected, kind) in [
        (
            "use crate::source::selected as alias;\nfn caller() { alias(); }\n",
            false,
            "use crate::target::selected as alias;",
            "use_path",
        ),
        (
            "use crate::source::{selected as alias, retained};\nfn caller() { alias(); retained(); }\n",
            false,
            "use crate::target::selected as alias;",
            "import_leaf_extract",
        ),
        (
            "use crate::{source::selected as alias, source::retained};\nfn caller() { alias(); retained(); }\n",
            false,
            "target::selected as alias",
            "use_path",
        ),
        (
            "use crate::source::{selected as alias, retained};\nfn caller() { alias(); retained(); }\n",
            true,
            "use crate::target::{selected as alias, retained};",
            "use_path",
        ),
    ] {
        let repo = Fixture::generate();
        repo.write("cases/layout/lib.rs", &format!("mod source;\n{imports}"));
        repo.write(
            "cases/layout/source.rs",
            "pub(crate) fn selected() {}\npub(crate) fn retained() {}\n",
        );
        compile_layout(&repo);
        let destination = json!({"kind":"new_sibling","path":"cases/layout/target.rs","parent_path":"cases/layout/lib.rs"});
        let mut moves = vec![
            json!({"item":anchor(&repo,"cases/layout/source.rs","pub(crate) fn selected() {}"),"destination":destination}),
        ];
        if selections {
            moves.push(json!({"item":anchor(&repo,"cases/layout/source.rs","pub(crate) fn retained() {}"),"destination":destination}));
        }
        let result = run(
            &repo,
            json!({"repo_path":repo.0,"crate_root":"cases/layout/lib.rs","paths":["cases/layout"],"moves":moves}),
        );
        let copy = apply(&repo, &result);
        compile_layout(&copy);
        assert!(
            fs::read_to_string(copy.0.join("cases/layout/lib.rs"))
                .unwrap()
                .contains(expected)
        );
        assert!(
            result["plan"]["rewrites"]
                .as_array()
                .unwrap()
                .iter()
                .any(|r| r["kind"] == kind)
        );
    }
}
#[test]
fn inline_consumers_use_lexical_self_super_context() {
    let repo = Fixture::generate();
    repo.write("cases/layout/lib.rs", "mod source;\n");
    repo.write(
        "cases/layout/source.rs",
        "fn selected() {}\nmod child { fn caller() { super::selected(); } }\n",
    );
    compile_layout(&repo);
    let result = run(
        &repo,
        json!({"repo_path":repo.0,"crate_root":"cases/layout/lib.rs","paths":["cases/layout"],"moves":[{"item":anchor(&repo,"cases/layout/source.rs","fn selected() {}"),"destination":{"kind":"new_sibling","path":"cases/layout/target.rs","parent_path":"cases/layout/lib.rs"}}]}),
    );
    let copy = apply(&repo, &result);
    compile_layout(&copy);
    assert!(
        fs::read_to_string(copy.0.join("cases/layout/source.rs"))
            .unwrap()
            .contains("crate::target::selected()")
    );
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
