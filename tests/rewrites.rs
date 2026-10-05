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
        (
            "use crate::{source::{selected as alias, retained}};\nfn caller() { alias(); retained(); }\n",
            true,
            "use crate::{target::{selected as alias, retained}};",
            "use_path",
        ),
        (
            "use crate::{source::selected as alias, source::retained};\nfn caller() { alias(); retained(); }\n",
            true,
            "target::selected as alias",
            "use_path",
        ),
        (
            "use crate::source::{selected as alias, /* unrelated */ retained};\nfn caller() { alias(); retained(); }\n",
            false,
            "/* unrelated */ retained",
            "import_leaf_extract",
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
fn grouped_leaves_to_distinct_destinations_preserve_delimiters() {
    let repo = Fixture::generate();
    repo.write("cases/layout/lib.rs", "mod source;\nuse crate::source::{selected as alias, retained};\nfn caller() { alias(); retained(); }\n");
    repo.write(
        "cases/layout/source.rs",
        "pub(crate) fn selected() {}\npub(crate) fn retained() {}\n",
    );
    compile_layout(&repo);
    let result = run(
        &repo,
        json!({"repo_path":repo.0,"crate_root":"cases/layout/lib.rs","paths":["cases/layout"],"moves":[{"item":anchor(&repo,"cases/layout/source.rs","pub(crate) fn selected() {}"),"destination":{"kind":"new_sibling","path":"cases/layout/first.rs","parent_path":"cases/layout/lib.rs"}},{"item":anchor(&repo,"cases/layout/source.rs","pub(crate) fn retained() {}"),"destination":{"kind":"new_sibling","path":"cases/layout/second.rs","parent_path":"cases/layout/lib.rs"}}]}),
    );
    compile_layout(&apply(&repo, &result));
    assert_eq!(
        result["plan"]["rewrites"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|r| r["kind"] == "import_leaf_extract")
            .count(),
        2
    );
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
fn supported_choices_preserve_targets_provenance_and_recheck_aliases() {
    let repo = Fixture::generate();
    repo.write("cases/layout/lib.rs", "mod source;\n");
    repo.write(
        "cases/layout/source.rs",
        "fn selected() {}\nfn caller() { self::selected(); selected(); }\nfn collision() {}\n",
    );
    let args = json!({"repo_path":repo.0,"crate_root":"cases/layout/lib.rs","paths":["cases/layout"],"moves":[{"item":anchor(&repo,"cases/layout/source.rs","fn selected() {}"),"destination":{"kind":"new_sibling","path":"cases/layout/target.rs","parent_path":"cases/layout/lib.rs"}}]});
    let default = run(&repo, args.clone());
    let rewrites = default["plan"]["rewrites"].as_array().unwrap();
    let import = rewrites
        .iter()
        .find(|r| r["kind"] == "import_insert")
        .unwrap();
    for text in [
        "use super::target::selected;",
        "use crate::target::selected as chosen;",
        "super::target::selected",
    ] {
        let mut choice = args.clone();
        choice["rewrite_overrides"] =
            json!([{"target":import["target"],"action":"replace","replacement_text":text}]);
        let result = run(&repo, choice.clone());
        compile_layout(&apply(&repo, &result));
        assert!(
            result["plan"]["rewrites"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|r| r["selected_action"] == "replace" || r["after_text"] == "chosen")
                .all(|r| r["origin"] == "caller_override")
        );
        assert_eq!(result["plan"], run(&repo, choice)["plan"]);
    }
    let path = rewrites.iter().find(|r| r["kind"] == "path").unwrap();
    let mut aliases = args.clone();
    aliases["rewrite_overrides"] = json!([{"target":path["target"],"action":"replace","replacement_text":"chosen"},{"target":import["target"],"action":"replace","replacement_text":"use crate::target::selected as chosen;"}]);
    let chosen = run(&repo, aliases.clone());
    compile_layout(&apply(&repo, &chosen));
    let generated = chosen["plan"]["rewrites"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["kind"] == "path" && r["before_text"] == "selected")
        .unwrap();
    aliases["rewrite_overrides"].as_array_mut().unwrap().push(json!({"target":generated["target"],"action":"replace","replacement_text":"unrelated::injected"}));
    let invalid = run(&repo, aliases);
    withheld(&invalid);
    assert_eq!(invalid["error"]["code"], "INVALID_REWRITE_OVERRIDE");
    let mut choice = args.clone();
    choice["rewrite_overrides"] = json!([{"target":path["target"],"action":"replace","replacement_text":"super::target::selected"}]);
    let result = run(&repo, choice);
    compile_layout(&apply(&repo, &result));
    assert!(
        result["plan"]["rewrites"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["after_text"] == "super::target::selected"
                && r["origin"] == "caller_override")
    );
    let mut reject = args.clone();
    reject["rewrite_overrides"] = json!([{"target":path["target"],"action":"retain"}]);
    let blocked = run(&repo, reject);
    withheld(&blocked);
    assert!(
        blocked["plan"]["rewrites"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["target"] == path["target"]
                && r["after_text"] == r["before_text"]
                && r["selected_action"] == "retain"
                && !r["decision_ids"].as_array().unwrap().is_empty())
    );
    assert!(
        blocked["plan"]["decisions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["category"] == "unsupported_dependency_form"
                && d["selected_choice"] == "retain")
    );
    let mut collision = args.clone();
    collision["rewrite_overrides"] = json!([{"target":import["target"],"action":"replace","replacement_text":"use crate::target::selected as collision;"}]);
    let blocked = run(&repo, collision);
    withheld(&blocked);
    assert!(
        blocked["plan"]["decisions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["category"] == "binding_collision")
    );
    for text in [
        "use crate::source::collision;",
        "use crate::target::*;",
        "use crate::target::selected; fn injected() {}",
        "use crate::target::selected as chosen",
    ] {
        let mut invalid = args.clone();
        invalid["rewrite_overrides"] =
            json!([{"target":import["target"],"action":"replace","replacement_text":text}]);
        let result = run(&repo, invalid);
        withheld(&result);
        assert_eq!(
            result["error"]["code"], "INVALID_REWRITE_OVERRIDE",
            "{result}"
        );
    }
}
#[test]
fn local_uses_absolute_restrictions_and_unrelated_same_spelling() {
    let repo = Fixture::generate();
    repo.write(
        "cases/layout/lib.rs",
        "mod source;\nmod other;\npub use crate::other::selected as unrelated_public;\n",
    );
    repo.write("cases/layout/other.rs", "pub fn selected() {}\n");
    repo.write("cases/layout/source.rs", "fn selected() {}\nfn caller() { use crate::other::selected; selected(); }\nfn actual() { use self::selected as imported; imported(); }\nfn alias_caller() { use crate::source as alias; alias::selected(); }\n");
    compile_layout(&repo);
    let args = json!({"repo_path":repo.0,"crate_root":"cases/layout/lib.rs","paths":["cases/layout"],"moves":[{"item":anchor(&repo,"cases/layout/source.rs","fn selected() {}"),"destination":{"kind":"new_sibling","path":"cases/layout/target.rs","parent_path":"cases/layout/lib.rs"}}]});
    let result = run(&repo, args);
    compile_layout(&apply(&repo, &result));
    assert!(
        !result["plan"]["rewrites"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["kind"] == "import_insert")
    );
    let repo = Fixture::generate();
    repo.write("cases/layout/lib.rs", "mod a;\n");
    repo.write("cases/layout/a.rs", "mod source;\nmod destination;\n");
    repo.write(
        "cases/layout/a/source.rs",
        "pub(in crate::a) fn selected() {}\nfn caller() { selected(); }\n",
    );
    repo.write("cases/layout/a/destination.rs", "fn keep() {}\n");
    compile_layout(&repo);
    let result = run(
        &repo,
        json!({"repo_path":repo.0,"crate_root":"cases/layout/lib.rs","paths":["cases/layout"],"moves":[{"item":anchor(&repo,"cases/layout/a/source.rs","pub(in crate::a) fn selected() {}"),"destination":{"kind":"existing","path":"cases/layout/a/destination.rs"}}]}),
    );
    compile_layout(&apply(&repo, &result));
    assert!(
        !result["plan"]["rewrites"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["kind"] == "visibility")
    );
}
#[test]
fn constructor_and_alias_macro_uncertainties_remain_anchored() {
    for (source, selected, root, category) in [
        (
            "struct Selected(u8);\nfn caller() { let _ = Selected(1); }\n",
            "struct Selected(u8);",
            "mod source;\n",
            "visibility_context",
        ),
        (
            "pub(crate) fn selected() {}\n",
            "pub(crate) fn selected() {}",
            "mod source;\nuse crate::source::selected as alias;\nmacro_rules! call { ($f:ident) => { $f() }; }\nfn caller() { call!(alias); }\n",
            "macro_dependency",
        ),
        (
            "fn selected() {}\n#[cfg(any())] mod child { fn caller() { super::selected(); } }\n",
            "fn selected() {}",
            "mod source;\n",
            "module_context",
        ),
    ] {
        let repo = Fixture::generate();
        repo.write("cases/layout/lib.rs", root);
        repo.write("cases/layout/source.rs", source);
        compile_layout(&repo);
        let result = run(
            &repo,
            json!({"repo_path":repo.0,"crate_root":"cases/layout/lib.rs","paths":["cases/layout"],"moves":[{"item":anchor(&repo,"cases/layout/source.rs",selected),"destination":{"kind":"new_sibling","path":"cases/layout/target.rs","parent_path":"cases/layout/lib.rs"}}]}),
        );
        withheld(&result);
        assert!(
            result["plan"]["decisions"]
                .as_array()
                .unwrap()
                .iter()
                .any(|d| d["category"] == category && !d["anchors"].as_array().unwrap().is_empty()),
            "{result}"
        );
    }
}
#[test]
fn new_destination_alias_collision_and_multiple_virtual_edges() {
    let repo = Fixture::generate();
    repo.write("cases/layout/lib.rs", "mod a;\nmod b;\n");
    repo.write("cases/layout/a.rs", "pub(crate) mod source;\n");
    repo.write(
        "cases/layout/a/source.rs",
        "pub(crate) fn first() {}\npub(crate) fn second() {}\n",
    );
    repo.write(
        "cases/layout/b.rs",
        "fn caller() { crate::a::source::first(); crate::a::source::second(); }\n",
    );
    compile_layout(&repo);
    let result = run(
        &repo,
        json!({"repo_path":repo.0,"crate_root":"cases/layout/lib.rs","paths":["cases/layout"],"moves":[{"item":anchor(&repo,"cases/layout/a/source.rs","pub(crate) fn first() {}"),"destination":{"kind":"new_sibling","path":"cases/layout/a/one.rs","parent_path":"cases/layout/a.rs"}},{"item":anchor(&repo,"cases/layout/a/source.rs","pub(crate) fn second() {}"),"destination":{"kind":"new_sibling","path":"cases/layout/a/two.rs","parent_path":"cases/layout/a.rs"}}]}),
    );
    compile_layout(&apply(&repo, &result));
    for file in result["plan"]["created_files"].as_array().unwrap() {
        assert!(file["declaration_visibility_rewrite_id"].is_string());
    }
    repo.write("cases/layout/lib.rs", "mod source;\nmod bindings;\n");
    repo.write("cases/layout/bindings.rs", "pub(crate) fn helper() {}\n");
    repo.write(
        "cases/layout/source.rs",
        "use crate::bindings::helper;\nfn selected() { helper(); }\nfn occupied() {}\n",
    );
    let destination = json!({"kind":"new_sibling","path":"cases/layout/target.rs","parent_path":"cases/layout/lib.rs"});
    let args = json!({"repo_path":repo.0,"crate_root":"cases/layout/lib.rs","paths":["cases/layout"],"moves":[{"item":anchor(&repo,"cases/layout/source.rs","fn selected() { helper(); }"),"destination":destination},{"item":anchor(&repo,"cases/layout/source.rs","fn occupied() {}"),"destination":destination}]});
    let base = run(&repo, args.clone());
    let import = base["plan"]["rewrites"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["kind"] == "import_insert")
        .unwrap();
    let mut choice = args;
    choice["rewrite_overrides"] = json!([{"target":import["target"],"action":"replace","replacement_text":"use crate::bindings::helper as occupied;"}]);
    let blocked = run(&repo, choice);
    withheld(&blocked);
    assert!(
        blocked["plan"]["decisions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["category"] == "binding_collision")
    );
}
#[test]
fn stable_closure_locals_and_needed_free_bindings_are_distinguished() {
    let repo = Fixture::generate();
    repo.write("cases/layout/lib.rs", "mod source;\n");
    repo.write("cases/layout/source.rs", "fn helper() {}\nfn selected() { let local = |x: u8| x; let _ = local(1); let call = || helper(); call(); }\n");
    compile_layout(&repo);
    let selected = "fn selected() { let local = |x: u8| x; let _ = local(1); let call = || helper(); call(); }";
    let result = run(
        &repo,
        json!({"repo_path":repo.0,"crate_root":"cases/layout/lib.rs","paths":["cases/layout"],"moves":[{"item":anchor(&repo,"cases/layout/source.rs",selected),"destination":{"kind":"new_sibling","path":"cases/layout/target.rs","parent_path":"cases/layout/lib.rs"}}]}),
    );
    compile_layout(&apply(&repo, &result));
    assert_eq!(
        result["plan"]["rewrites"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|r| r["kind"] == "import_insert")
            .count(),
        1
    );
}
#[test]
fn grouped_local_imports_keep_their_binding_scope_after_context_changes() {
    let repo = Fixture::generate();
    repo.write("cases/layout/lib.rs", "mod source;\n");
    let selected =
        "fn selected() {\r\n use self::{helper, companion};\r\n helper(); companion();\r\n}";
    repo.write(
        "cases/layout/source.rs",
        &format!("fn helper() {{}}\nfn companion() {{}}\n{selected}\n"),
    );
    let destination = json!({"kind":"new_sibling","path":"cases/layout/target.rs","parent_path":"cases/layout/lib.rs"});
    for together in [false, true] {
        let mut moves = vec![
            json!({"item":anchor(&repo,"cases/layout/source.rs",selected),"destination":destination}),
        ];
        if together {
            moves.push(json!({"item":anchor(&repo,"cases/layout/source.rs","fn helper() {}"),"destination":destination}));
        }
        let args = json!({"repo_path":repo.0,"crate_root":"cases/layout/lib.rs","paths":["cases/layout"],"moves":moves});
        let result = run(&repo, args.clone());
        compile_layout(&apply(&repo, &result));
        if together {
            assert!(
                result["plan"]["rewrites"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter(|r| r["kind"] == "separator"
                        && r["target"]["binding"]
                            .as_str()
                            .is_some_and(|b| b.starts_with("local-import:")))
                    .all(|r| r["after_text"] == "\r\n")
            );
            let import = result["plan"]["rewrites"]
                .as_array()
                .unwrap()
                .iter()
                .find(|r| r["kind"] == "import_insert")
                .unwrap();
            let mut choice = args;
            choice["rewrite_overrides"] = json!([{"target":import["target"],"action":"replace","replacement_text":"use crate::source::companion as preserved;"}]);
            compile_layout(&apply(&repo, &run(&repo, choice)));
        }
    }
    repo.write("cases/layout/lib.rs", "mod source;\nmod other;\nuse crate::other::selected;\nfn caller() { use crate::source::{selected, retained}; selected(); retained(); }\n");
    repo.write(
        "cases/layout/source.rs",
        "pub(crate) fn selected() {}\npub(crate) fn retained() {}\n",
    );
    repo.write("cases/layout/other.rs", "pub(crate) fn selected() {}\n");
    compile_layout(&repo);
    let result = run(
        &repo,
        json!({"repo_path":repo.0,"crate_root":"cases/layout/lib.rs","paths":["cases/layout"],"moves":[{"item":anchor(&repo,"cases/layout/source.rs","pub(crate) fn selected() {}"),"destination":destination}]}),
    );
    compile_layout(&apply(&repo, &result));
}
#[test]
fn imported_module_prefixes_and_external_aliases_are_evidenced() {
    for (source, destination) in [
        (
            "use crate::bindings as namespace;\nuse namespace::helper as imported;\nfn selected() { imported(); }\n",
            "use crate::bindings as existing;\nuse existing::helper as imported;\n",
        ),
        (
            "use core::mem::size_of as imported;\nfn selected() { let _ = imported::<u8>(); }\n",
            "use core::mem::size_of as imported;\n",
        ),
    ] {
        let repo = Fixture::generate();
        repo.write(
            "cases/layout/lib.rs",
            "mod source;\nmod bindings;\nmod destination;\n",
        );
        repo.write("cases/layout/bindings.rs", "pub(crate) fn helper() {}\n");
        repo.write("cases/layout/source.rs", source);
        repo.write("cases/layout/destination.rs", destination);
        compile_layout(&repo);
        let selected = source.lines().last().unwrap();
        for dest in [
            json!({"kind":"existing","path":"cases/layout/destination.rs"}),
            json!({"kind":"new_sibling","path":"cases/layout/target.rs","parent_path":"cases/layout/lib.rs"}),
        ] {
            let result = run(
                &repo,
                json!({"repo_path":repo.0,"crate_root":"cases/layout/lib.rs","paths":["cases/layout"],"moves":[{"item":anchor(&repo,"cases/layout/source.rs",selected),"destination":dest}]}),
            );
            compile_layout(&apply(&repo, &result));
        }
    }
}
#[test]
fn needed_chained_reexports_have_the_exposure_decision() {
    let repo = Fixture::generate();
    repo.write(
        "cases/layout/lib.rs",
        "mod source;\nmod bindings;\nmod bridge;\n",
    );
    repo.write("cases/layout/bindings.rs", "pub(crate) fn helper() {}\n");
    repo.write(
        "cases/layout/bridge.rs",
        "pub(crate) use crate::bindings::helper;\n",
    );
    for source in [
        "use crate::bridge::helper;\nfn selected() { helper(); }\n",
        "fn selected() { crate::bridge::helper(); }\n",
    ] {
        repo.write("cases/layout/source.rs", source);
        compile_layout(&repo);
        let result = run(
            &repo,
            json!({"repo_path":repo.0,"crate_root":"cases/layout/lib.rs","paths":["cases/layout"],"moves":[{"item":anchor(&repo,"cases/layout/source.rs",source.lines().last().unwrap()),"destination":{"kind":"new_sibling","path":"cases/layout/target.rs","parent_path":"cases/layout/lib.rs"}}]}),
        );
        withheld(&result);
        assert!(
            result["plan"]["decisions"]
                .as_array()
                .unwrap()
                .iter()
                .any(|d| d["category"] == "reexport_dependency"
                    && d["anchors"][0]["path"] == "cases/layout/bridge.rs"),
            "{result}"
        );
    }
}
#[test]
fn conditional_dependencies_and_external_prefix_collisions_are_not_guessed() {
    let repo = Fixture::generate();
    repo.write("cases/layout/lib.rs", "mod source;\n");
    repo.write(
        "cases/layout/source.rs",
        "#[cfg(all())] fn helper() {}\nfn selected() { helper(); }\n",
    );
    compile_layout(&repo);
    let destination = json!({"kind":"new_sibling","path":"cases/layout/target.rs","parent_path":"cases/layout/lib.rs"});
    let result = run(
        &repo,
        json!({"repo_path":repo.0,"crate_root":"cases/layout/lib.rs","paths":["cases/layout"],"moves":[{"item":anchor(&repo,"cases/layout/source.rs","fn selected() { helper(); }"),"destination":destination}]}),
    );
    withheld(&result);
    assert!(
        result["plan"]["decisions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["category"] == "scope_dependency"
                && d["anchors"][0]["expected_text"] == "#[cfg(all())]")
    );
    repo.write("cases/layout/lib.rs", "mod source;\nmod second;\n");
    repo.write(
        "cases/layout/source.rs",
        "use core::mem::size_of;\nfn selected() { let _ = size_of::<u8>(); }\n",
    );
    repo.write("cases/layout/second.rs", "struct core;\n");
    compile_layout(&repo);
    let result = run(
        &repo,
        json!({"repo_path":repo.0,"crate_root":"cases/layout/lib.rs","paths":["cases/layout"],"moves":[{"item":anchor(&repo,"cases/layout/second.rs","struct core;"),"destination":destination},{"item":anchor(&repo,"cases/layout/source.rs","fn selected() { let _ = size_of::<u8>(); }"),"destination":destination}]}),
    );
    withheld(&result);
    assert!(
        result["plan"]["decisions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["category"] == "binding_collision")
    );
}
#[test]
fn category_summaries_coalesce_without_hiding_rejected_needs() {
    let repo = Fixture::generate();
    repo.write("cases/layout/lib.rs", "mod source;\n");
    repo.write(
        "cases/layout/source.rs",
        "fn first() {}\nfn second() {}\nfn caller() { first(); second(); }\n",
    );
    let destination = json!({"kind":"new_sibling","path":"cases/layout/target.rs","parent_path":"cases/layout/lib.rs"});
    let mut args = json!({"repo_path":repo.0,"crate_root":"cases/layout/lib.rs","paths":["cases/layout"],"moves":[{"item":anchor(&repo,"cases/layout/source.rs","fn first() {}"),"destination":destination},{"item":anchor(&repo,"cases/layout/source.rs","fn second() {}"),"destination":destination}]});
    let base = run(&repo, args.clone());
    args["rewrite_overrides"] = json!(
        base["plan"]["rewrites"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|r| r["kind"] == "import_insert")
            .map(|r| json!({"target":r["target"],"action":"retain"}))
            .collect::<Vec<_>>()
    );
    let blocked = run(&repo, args);
    withheld(&blocked);
    assert_eq!(
        blocked["plan"]["blockers"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|b| b["code"] == "UNSUPPORTED_DEPENDENCY_FORM")
            .count(),
        1
    );
    assert_eq!(
        blocked["plan"]["decisions"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|d| d["selected_choice"] == "retain"
                && !d["anchors"].as_array().unwrap().is_empty())
            .count(),
        2
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
