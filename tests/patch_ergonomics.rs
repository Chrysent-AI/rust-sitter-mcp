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
    let result = Engine::new(repo.0.clone()).unwrap().move_item(
        serde_json::from_value::<MoveRequest>(args).unwrap(),
        &AtomicBool::new(false),
    );
    assert_eq!(observe(&repo.0), before, "planning must be read-only");
    serde_json::to_value(result).unwrap()
}
fn compile(repo: &Fixture) {
    repo.write("Cargo.toml", "[package]\nname = \"fixture-corpus\"\nversion = \"0.0.0\"\nedition = \"2024\"\n[lib]\npath = \"cases/layout/lib.rs\"\n");
    let check = Command::new("cargo")
        .current_dir(&repo.0)
        .args(["check", "--locked", "--offline", "--quiet"])
        .output()
        .unwrap();
    assert!(
        check.status.success(),
        "{}",
        String::from_utf8_lossy(&check.stderr)
    );
}
fn args(repo: &Fixture, source: &str, item: &str) -> Value {
    json!({"repo_path":repo.0,"crate_root":"cases/layout/lib.rs","paths":["cases/layout"],"moves":[{
        "item":anchor(repo,source,item),"destination":{"kind":"new_sibling","path":"cases/layout/target.rs","parent_path":"cases/layout/lib.rs"}
    }]})
}
fn text(repo: &Fixture, path: &str) -> String {
    fs::read_to_string(repo.0.join(path)).unwrap()
}
fn gap(plan: &Value) -> &Value {
    plan["decisions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["category"] == "removal_gap")
        .unwrap()
}
fn refused(result: &Value, code: &str) {
    assert_eq!(result["error"]["code"], code, "{result}");
    assert_eq!(result["plan"]["applicable"], false);
    for field in ["edits", "created_files", "patch"] {
        assert!(result["plan"][field].is_null());
    }
}

#[test]
fn idiomatic_move_applies_compiles_and_gap_collapse_is_explicit_replay() {
    let repo = Fixture::generate();
    let root = "mod source;\nmod destination; // sibling\n\n#[cfg(test)]\nmod tests { #[test] fn smoke() {} }\n";
    let item = "fn selected() -> u8 { 7 }";
    let source = format!(
        "#![allow(dead_code, unused_imports)]\nuse std::cmp::max;\nuse std::cmp::min;\n\nfn keep() {{}}\n\n{item}\n\nfn caller() -> u8 {{ selected() }}\n"
    );
    repo.write("cases/layout/lib.rs", root);
    repo.write("cases/layout/source.rs", &source);
    repo.write("cases/layout/destination.rs", "");
    let request = args(&repo, "cases/layout/source.rs", item);
    let result = run(&repo, request.clone());
    let copy = apply(&repo, &result); // Independently reconstructs JSON, git apply --check, git apply.
    compile(&copy);
    assert_eq!(
        text(&copy, "cases/layout/target.rs"),
        "pub(crate) fn selected() -> u8 { 7 }\n"
    );
    assert_eq!(
        text(&copy, "cases/layout/lib.rs"),
        root.replace("// sibling\n", "// sibling\nmod target;\n")
    );
    let expected = source
        .replace(
            "use std::cmp::min;\n",
            "use std::cmp::min;\nuse crate::target::selected;\n",
        )
        .replace(item, "");
    assert_eq!(text(&copy, "cases/layout/source.rs"), expected);
    assert!(expected.contains("use std::cmp::max;\nuse std::cmp::min;\n"));
    assert!(expected.contains("fn keep() {}\n\n\n\nfn caller()"));
    let choice = gap(&result["plan"]);
    assert_eq!(choice["reason"], "removal_gap_choice");
    assert_eq!(choice["blocks_applicability"], false);
    assert_eq!(choice["removal_gap"]["before_text"], "\n\n\n\n");
    assert_eq!(choice["removal_gap"]["after_text"], "\n\n");
    assert_eq!(
        choice["removal_gap"]["default_disposition"],
        "keep_in_place"
    );
    assert_eq!(
        choice["removal_gap"]["selected_disposition"],
        "keep_in_place"
    );
    assert!(
        !result["plan"]["rewrites"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["kind"] == "removal_gap")
    );
    for action in ["retain", "accept_default"] {
        let mut replay = request.clone();
        replay["rewrite_overrides"] =
            json!([{"target":choice["action"]["target"],"action":action}]);
        let kept = run(&repo, replay);
        for artifact in ["edits", "created_files", "patch", "origins", "rewrites"] {
            assert_eq!(kept["plan"][artifact], result["plan"][artifact]);
        }
    }
    let mut replay = request;
    replay["rewrite_overrides"] = json!([{"target":choice["action"]["target"],"action":"replace","replacement_text":choice["removal_gap"]["after_text"]}]);
    let collapsed = run(&repo, replay.clone());
    assert_eq!(run(&repo, replay)["plan"], collapsed["plan"]);
    let collapsed_copy = apply(&repo, &collapsed);
    compile(&collapsed_copy);
    assert_eq!(
        text(&collapsed_copy, "cases/layout/source.rs"),
        expected.replace("fn keep() {}\n\n\n\n", "fn keep() {}\n\n")
    );
    assert_eq!(
        gap(&collapsed["plan"])["removal_gap"]["selected_disposition"],
        "collapse"
    );
    let audit = collapsed["plan"]["rewrites"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["kind"] == "removal_gap")
        .unwrap();
    assert_eq!(audit["origin"], "caller_override");
    assert_eq!(audit["default_action"], "retain");
    assert_eq!(audit["after_text"], "\n\n");
    assert!(audit["before_text"].as_str().unwrap().contains(item));
    assert_eq!(
        audit["decision_ids"],
        json!([gap(&collapsed["plan"])["id"]])
    );
}

#[test]
fn declaration_fallbacks_preserve_test_attachments_and_eof() {
    for root in [
        "fn keep() {}\n\n/// Tests remain attached.\n#[cfg(test)]\nmod tests { #[test] fn smoke() {} }\n",
        "fn keep() {}\n",
        "mod source; // keep comment\r\n\r\n#[cfg(test)]\r\nmod tests {}\r\n",
    ] {
        let repo = Fixture::generate();
        let item = "fn selected() {}";
        // Without sibling declarations the root itself supplies the selected item.
        let source = if root.starts_with("mod source;") {
            "cases/layout/source.rs"
        } else {
            "cases/layout/lib.rs"
        };
        repo.write(
            "cases/layout/lib.rs",
            &if source.ends_with("source.rs") {
                root.into()
            } else {
                format!("{item}\n{root}")
            },
        );
        repo.write("cases/layout/source.rs", &format!("{item}\n"));
        let result = run(&repo, args(&repo, source, item));
        let copy = apply(&repo, &result);
        compile(&copy);
        let output = text(&copy, "cases/layout/lib.rs");
        if root.contains("#[cfg(test)]") {
            assert!(output.find("mod target;").unwrap() < output.find("#[cfg(test)]").unwrap());
            if root.contains("///") {
                assert!(output.contains("/// Tests remain attached.\n#[cfg(test)]\nmod tests"));
            }
        } else {
            assert!(output.ends_with("mod target;\n"));
        }
        assert!(
            !result["plan"]["rewrites"]
                .as_array()
                .unwrap()
                .iter()
                .find(|r| r["kind"] == "module_declaration")
                .unwrap()["anchors"]
                .as_array()
                .unwrap()
                .is_empty()
        );
    }
}

#[test]
fn created_eof_separators_preserve_crlf_and_unterminated_comments() {
    for (source, ending) in [
        ("fn keep() {}\r\nfn selected() {}\r\n", "\r\n"),
        ("fn keep() {}\r\nfn selected() {} // tail", "\n"),
        ("fn selected() {} // tail\r\n", "\r\n"),
    ] {
        let repo = Fixture::generate();
        repo.write("cases/layout/lib.rs", "mod source;\n");
        repo.write("cases/layout/source.rs", source);
        let request = args(&repo, "cases/layout/source.rs", "fn selected() {}");
        let result = run(&repo, request.clone());
        let copy = apply(&repo, &result);
        compile(&copy);
        let content = text(&copy, "cases/layout/target.rs");
        assert!(content.ends_with(ending) && !content.ends_with(&format!("{ending}{ending}")));
        let separator = result["plan"]["rewrites"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| {
                r["kind"] == "separator"
                    && r["target"]["path"] == "cases/layout/target.rs"
                    && r["target"]["boundary_role"] == "after_payload"
            })
            .unwrap();
        let mut rejected = request;
        rejected["rewrite_overrides"] = json!([{"target":separator["target"],"action":"retain"}]);
        let blocked = run(&repo, rejected);
        assert_eq!(blocked["plan"]["applicable"], false);
        assert!(blocked["plan"]["patch"].is_null());
    }
}

#[test]
fn gap_choices_are_bounded_stale_safe_and_cannot_inject_or_touch_unrelated_gaps() {
    let repo = Fixture::generate();
    repo.write("cases/layout/lib.rs", "mod source;\n");
    repo.write(
        "cases/layout/source.rs",
        "fn first() {}\n\nfn selected() {}\n\nfn second() {}\n\n\nfn last() {}\n",
    );
    let request = args(&repo, "cases/layout/source.rs", "fn selected() {}");
    let base = run(&repo, request.clone());
    assert_eq!(
        base["plan"]["decisions"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|d| d["category"] == "removal_gap")
            .count(),
        1
    );
    let choice = gap(&base["plan"]);
    for replacement in ["fn injected() {}", "\n", "\n\n\n"] {
        let mut replay = request.clone();
        replay["rewrite_overrides"] = json!([{"target":choice["action"]["target"],"action":"replace","replacement_text":replacement}]);
        refused(&run(&repo, replay), "INVALID_REWRITE_OVERRIDE");
    }
    let mut duplicate = request.clone();
    duplicate["rewrite_overrides"] = json!([{"target":choice["action"]["target"],"action":"retain"},{"target":choice["action"]["target"],"action":"retain"}]);
    refused(&run(&repo, duplicate), "INVALID_REWRITE_OVERRIDE");
    let mut replay = request;
    replay["rewrite_overrides"] = json!([{"target":choice["action"]["target"],"action":"replace","replacement_text":choice["removal_gap"]["after_text"]}]);
    let result = run(&repo, replay.clone());
    let copy = apply(&repo, &result);
    assert!(text(&copy, "cases/layout/source.rs").contains("fn second() {}\n\n\nfn last() {}"));
    repo.write(
        "cases/layout/source.rs",
        "fn first() {}\n\nfn selected() {}\n \nfn second() {}\n\n\nfn last() {}\n",
    );
    refused(&run(&repo, replay), "STALE_REWRITE_OVERRIDE");
}

#[test]
fn adjacent_removals_bof_and_crlf_gaps_collapse_without_losing_comment_bytes() {
    for source in [
        "fn first() {}\n\nfn selected() {}\n\nfn another() {}\n\nfn last() {}\n",
        "\nfn selected() {}\n\nfn another() {}\n\nfn last() {}\n",
        "fn first() {} // retained\r\n\r\nfn selected() {}\r\n\r\nfn another() {}\r\n\r\nfn last() {}\r\n",
    ] {
        let repo = Fixture::generate();
        repo.write("cases/layout/lib.rs", "mod source;\n");
        repo.write("cases/layout/source.rs", source);
        let mut request = args(&repo, "cases/layout/source.rs", "fn selected() {}");
        request["moves"]
            .as_array_mut()
            .unwrap()
            .push(args(&repo, "cases/layout/source.rs", "fn another() {}")["moves"][0].clone());
        let base = run(&repo, request.clone());
        let choice = gap(&base["plan"]);
        request["rewrite_overrides"] = json!([{"target":choice["action"]["target"],"action":"replace","replacement_text":choice["removal_gap"]["after_text"]}]);
        let copy = apply(&repo, &run(&repo, request));
        compile(&copy);
        let output = text(&copy, "cases/layout/source.rs");
        assert!(!output.contains("\n\n\n") && !output.contains("\r\n\r\n\r\n"));
        if source.contains("retained") {
            assert!(output.contains("// retained\r\n"));
        }
        if source.starts_with('\n') {
            assert!(output.starts_with("\nfn last()"));
        }
    }
}

#[test]
fn import_boundaries_preserve_attached_docs_attributes_and_trailing_comments() {
    for (imports, eol) in [
        (
            "use std::cmp::max;\n#[allow(unused_imports)]\nuse std::cmp::min; // attached\n",
            "\n",
        ),
        (
            "use std::cmp::max;\r\n#[allow(unused_imports)]\r\nuse std::cmp::min; // attached\r\n",
            "\r\n",
        ),
        ("", "\n"),
    ] {
        let repo = Fixture::generate();
        let item = "fn selected() {}";
        repo.write("cases/layout/lib.rs", "mod source;\n");
        let source = format!(
            "#![allow(dead_code, unused_imports)]{eol}{imports}/// Selected docs.{eol}{item}{eol}/// Caller docs.{eol}#[allow(dead_code)]{eol}fn caller() {{ selected(); }}{eol}"
        );
        repo.write("cases/layout/source.rs", &source);
        let result = run(&repo, args(&repo, "cases/layout/source.rs", item));
        let copy = apply(&repo, &result);
        compile(&copy);
        let output = text(&copy, "cases/layout/source.rs");
        assert!(output.contains(imports));
        assert!(output.contains(&format!(
            "/// Caller docs.{eol}#[allow(dead_code)]{eol}fn caller()"
        )));
        if !imports.is_empty() {
            assert!(output.contains(&format!("{imports}use crate::target::selected;{eol}")));
        } else {
            assert!(output.starts_with(
                "#![allow(dead_code, unused_imports)]\nuse crate::target::selected;\n"
            ));
        }
        assert!(text(&copy, "cases/layout/target.rs").starts_with("/// Selected docs."));
    }
}
