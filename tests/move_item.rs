#[path = "support/chain_fixture.rs"]
mod chain_fixture;
#[path = "support/move_chain_tests.rs"]
mod chain_tests;
#[path = "support/derive_tests.rs"]
mod derive_tests;
#[path = "support/fixture_gen.rs"]
mod fixture_gen;
#[path = "support/move_artifacts.rs"]
mod move_artifacts;
#[path = "support/prelude_scope_tests.rs"]
mod prelude_scope_tests;
#[path = "support/prelude_tests.rs"]
mod prelude_tests;
#[path = "support/stdio_client.rs"]
mod stdio_client;
#[path = "support/test_consumers.rs"]
mod test_consumers;
use fixture_gen::{Fixture, observe};
use move_artifacts::{anchor, apply};
use rust_sitter_mcp::{engine::Engine, move_plan::MoveRequest};
use serde_json::{Value, json};
use std::{fs, os::unix::fs::PermissionsExt, sync::atomic::AtomicBool};

fn fixture(source: &str) -> Fixture {
    let repo = Fixture::generate();
    repo.write("cases/layout/lib.rs", "mod source;\nmod destination;\n");
    repo.write("cases/layout/source.rs", source);
    repo.write("cases/layout/destination.rs", "fn keep() {}\n");
    repo
}
fn request(repo: &Fixture, moves: Value) -> Value {
    json!({"repo_path":repo.0,"crate_root":"cases/layout/lib.rs","paths":["cases/layout"],"moves":moves})
}
fn new(path: &str) -> Value {
    json!({"kind":"new_sibling","path":path,"parent_path":"cases/layout/lib.rs"})
}
fn entry(repo: &Fixture, text: &str, destination: Value) -> Value {
    json!({"item":anchor(repo,"cases/layout/source.rs",text),"destination":destination})
}
fn run(repo: &Fixture, args: Value) -> Value {
    let before = observe(&repo.0);
    let request: MoveRequest = serde_json::from_value(args).unwrap();
    let result = Engine::new(repo.0.clone())
        .unwrap()
        .move_item(request, &AtomicBool::new(false));
    assert!(result.wire_bytes() <= result.limits.response_bytes);
    assert_eq!(observe(&repo.0), before);
    serde_json::to_value(result).unwrap()
}
fn withheld(value: &Value) {
    assert_eq!(value["plan"]["applicable"], false, "{value}");
    for field in ["edits", "created_files", "patch"] {
        assert!(value["plan"][field].is_null(), "{value}");
    }
    assert_eq!(value["plan"]["integrity"]["semantic"], "not_performed");
}
fn code(value: &Value, expected: &str) {
    withheld(value);
    assert!(
        value["error"]["code"] == expected
            || value["plan"]["blockers"]
                .as_array()
                .unwrap()
                .iter()
                .any(|b| b["code"] == expected),
        "expected {expected}: {value}"
    );
}

#[test]
fn every_supported_kind_and_simultaneous_original_order() {
    let texts = [
        "fn selected() {}",
        "struct Record;",
        "enum Choice { First, Second }",
        "union Bits { number: u32 }",
        "trait Value { fn value(&self) -> u8; }",
        "impl Record { fn member(&self) {} }",
        "type Count = usize;",
        "const DEFAULT: u8 = 1;",
        "static LABEL: &str = \"é\";",
    ];
    let repo = fixture(&texts.join("\n"));
    let before = observe(&repo.0);
    let moves: Vec<_> = texts
        .iter()
        .rev()
        .map(|text| entry(&repo, text, new("cases/layout/new_file.rs")))
        .collect();
    let result = run(&repo, request(&repo, json!(moves)));
    assert_eq!(result["plan"]["selected_count"], texts.len());
    let copy = apply(&repo, &result);
    let content = fs::read_to_string(copy.0.join("cases/layout/new_file.rs")).unwrap();
    assert_eq!(content, format!("{}\n", texts.join("\n")));
    for record in result["plan"]["moves"].as_array().unwrap() {
        assert_eq!(record["item"]["eligibility"], "supported_unit");
    }
    assert!(repo.0.join("cases/layout/source.rs").exists());
    assert_eq!(observe(&repo.0), before);
}

#[test]
fn trivia_newlines_unicode_internal_scopes_and_fragmented_replay() {
    let text = "fn selected() {\r\n    #![allow(unused_variables)]\r\n    // internal\r\n    let é = 1;\n    let x = é;\r\n}";
    let repo = fixture(&format!(
        "//! file docs\r\n#![allow(dead_code)]\r\nfn retained() {{}}\r\n// leading\r\n/** outer docs */\r\n// --- retained banner ---\r\n#[inline]\r\n// --- retained banner two ---\r\n{text} // trailing"
    ));
    let before = observe(&repo.0);
    let args = request(
        &repo,
        json!([entry(&repo, text, new("cases/layout/bytes.rs"))]),
    );
    let result = run(&repo, args.clone());
    let copy = apply(&repo, &result);
    let content = fs::read_to_string(copy.0.join("cases/layout/bytes.rs")).unwrap();
    assert!(
        content.contains(text)
            && content.contains("/** outer docs */")
            && content.contains("// trailing")
    );
    assert!(!content.contains("file docs") && !content.contains("retained banner"));
    let retained = fs::read_to_string(copy.0.join("cases/layout/source.rs")).unwrap();
    assert!(retained.contains("file docs") && retained.contains("retained banner"));
    let rewrites = result["plan"]["rewrites"].as_array().unwrap();
    assert!(rewrites.iter().filter(|r| r["kind"] == "separator" && r["target"]["boundary_role"] == "before_payload").count() >= 2);
    let targets: std::collections::BTreeSet<_> =
        rewrites.iter().map(|r| r["target"].to_string()).collect();
    assert_eq!(targets.len(), rewrites.len());
    let mut replay = args;
    replay["rewrite_overrides"] = json!(
        rewrites
            .iter()
            .map(|r| json!({"target":r["target"],"action":"accept_default"}))
            .collect::<Vec<_>>()
    );
    assert_eq!(result["plan"], run(&repo, replay)["plan"]);
    assert_eq!(observe(&repo.0), before);
}

#[test]
fn ambiguous_carry_overrides_relevance_and_protected_refusal() {
    let repo = fixture("//! protected\nfn retained() {}\n// --- banner ---\n\nfn selected() {}\n");
    let item = anchor(&repo, "cases/layout/source.rs", "fn selected() {}");
    let args = request(
        &repo,
        json!([entry(
            &repo,
            "fn selected() {}",
            new("cases/layout/new_file.rs")
        )]),
    );
    let base = run(&repo, args.clone());
    let trivia = base["plan"]["trivia_decisions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["classification"] == "ambiguous")
        .unwrap();
    let source = fs::read_to_string(repo.0.join("cases/layout/source.rs")).unwrap();
    let span = &trivia["span"]["range"];
    let banner = json!({"path":"cases/layout/source.rs","range":span,"expected_text":&source[span["start_byte"].as_u64().unwrap() as usize..span["end_byte"].as_u64().unwrap() as usize]});
    let mut carried = args.clone();
    carried["trivia_overrides"] =
        json!([{"trivia":banner,"disposition":"carry_with_item","target_item":item}]);
    let result = run(&repo, carried.clone());
    let copy = apply(&repo, &result);
    assert!(
        fs::read_to_string(copy.0.join("cases/layout/new_file.rs"))
            .unwrap()
            .starts_with("// --- banner ---")
    );
    assert!(
        !fs::read_to_string(copy.0.join("cases/layout/source.rs"))
            .unwrap()
            .contains("banner")
    );
    carried["trivia_overrides"][0]["disposition"] = json!("keep_in_place");
    code(&run(&repo, carried), "INVALID_MOVE_TRIVIA_OVERRIDE");
    let protected = base["plan"]["trivia_decisions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["classification"] == "scope")
        .unwrap();
    let r = &protected["span"]["range"];
    let mut rejected = args.clone();
    rejected["trivia_overrides"] = json!([{"trivia":{"path":"cases/layout/source.rs","range":r,"expected_text":&source[r["start_byte"].as_u64().unwrap() as usize..r["end_byte"].as_u64().unwrap() as usize]},"disposition":"carry_with_item","target_item":item}]);
    code(&run(&repo, rejected), "UNSUPPORTED_TRIVIA_DISPOSITION");
    repo.write(
        "cases/layout/source.rs",
        &format!("{source}\nfn farther() {{}}\n// --- distant ---\nfn last() {{}}\n"),
    );
    let mut distant = args.clone();
    distant["trivia_overrides"] = json!([{"trivia":anchor(&repo,"cases/layout/source.rs","// --- distant ---\n"),"disposition":"carry_with_item","target_item":item}]);
    code(&run(&repo, distant), "INVALID_MOVE_TRIVIA_OVERRIDE");
    repo.write(
        "cases/layout/unrelated.rs",
        "// --- unrelated ---\nfn other() {}\n",
    );
    let mut unrelated = args;
    unrelated["trivia_overrides"] = json!([{"trivia":anchor(&repo,"cases/layout/unrelated.rs","// --- unrelated ---\n"),"disposition":"carry_with_item","target_item":item}]);
    code(&run(&repo, unrelated), "INVALID_MOVE_TRIVIA_OVERRIDE");
}

#[test]
fn insertion_both_boundaries_replay_and_touching_sources() {
    let repo = fixture("fn selected() {}");
    repo.write(
        "cases/layout/destination.rs",
        "fn first() {} /* retained */fn keep() {}",
    );
    let mut destination = json!({"kind":"existing","path":"cases/layout/destination.rs","before_item":anchor(&repo,"cases/layout/destination.rs","fn keep() {}")});
    let args = request(
        &repo,
        json!([entry(&repo, "fn selected() {}", destination.clone())]),
    );
    let result = run(&repo, args.clone());
    apply(&repo, &result);
    let separators: Vec<_> = result["plan"]["rewrites"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|r| r["kind"] == "separator")
        .collect();
    assert_eq!(separators.len(), 2);
    assert!(
        separators
            .iter()
            .any(|r| r["target"]["boundary_role"] == "before_payload")
    );
    assert!(
        separators
            .iter()
            .any(|r| r["target"]["boundary_role"] == "after_payload")
    );
    for sep in &separators {
        let mut replay = args.clone();
        replay["rewrite_overrides"] =
            json!([{"target":sep["target"],"action":"replace","replacement_text":"\r\n"}]);
        apply(&repo, &run(&repo, replay));
        let mut unsafe_choice = args.clone();
        unsafe_choice["rewrite_overrides"] = json!([{"target":sep["target"],"action":"retain"}]);
        withheld(&run(&repo, unsafe_choice));
    }
    repo.write("cases/layout/source.rs", "fn selected() {}fn retained() {}");
    repo.write("cases/layout/destination.rs", "fn incoming() {}");
    destination = json!({"kind":"existing","path":"cases/layout/destination.rs"});
    let args = request(
        &repo,
        json!([entry(&repo,"fn selected() {}",destination),{"item":anchor(&repo,"cases/layout/destination.rs","fn incoming() {}"),"destination":{"kind":"existing","path":"cases/layout/source.rs","before_item":anchor(&repo,"cases/layout/source.rs","fn retained() {}")}}]),
    );
    let result = run(&repo, args);
    apply(&repo, &result);
    assert_eq!(result["plan"]["edits"].as_array().unwrap().len(), 2);
}

#[test]
fn destination_adversary_matrix_and_walker_parity() {
    let repo = fixture("fn selected() {}\nfn other() {}\n");
    let make = |path: &str| request(&repo, json!([entry(&repo, "fn selected() {}", new(path))]));
    for (path, expected) in [
        ("../escape.rs", "INVALID_DESTINATION"),
        ("/tmp/escape.rs", "INVALID_DESTINATION"),
        ("cases/layout/bad-name.rs", "INVALID_NEW_FILE_NAME"),
        ("cases/layout/fn.rs", "INVALID_NEW_FILE_NAME"),
        ("cases/layout/mod.rs", "INVALID_NEW_FILE_NAME"),
        ("cases/layout/_.rs", "INVALID_NEW_FILE_NAME"),
        ("cases/layout/union.rs", "INVALID_NEW_FILE_NAME"),
        ("cases/layout/destination.rs", "DESTINATION_ALREADY_EXISTS"),
        ("cases/layout/missing/new.rs", "INVALID_DESTINATION"),
    ] {
        let result = run(&repo, make(path));
        code(&result, expected);
        assert_eq!(result["error"]["field"], "moves[0].destination.path");
    }
    let mut batch = make("cases/layout/valid.rs");
    batch["moves"].as_array_mut().unwrap().push(entry(
        &repo,
        "fn other() {}",
        new("cases/layout/bad-name.rs"),
    ));
    let result = run(&repo, batch);
    code(&result, "INVALID_NEW_FILE_NAME");
    assert_eq!(result["error"]["field"], "moves[1].destination.path");
    let mut wrong = make("cases/layout/valid.rs");
    wrong["moves"][0]["destination"]["parent_path"] = json!("cases/layout/source.rs");
    code(&run(&repo, wrong), "INVALID_DECLARATION_PARENT");
    repo.write("cases/layout/Clash.rs", "fn x() {}");
    code(
        &run(&repo, make("cases/layout/clash.rs")),
        "DESTINATION_ALREADY_EXISTS",
    );
    repo.write("cases/layout/competing/mod.rs", "fn x() {}");
    code(
        &run(&repo, make("cases/layout/competing.rs")),
        "MODULE_DECLARATION_CONFLICT",
    );
    fs::create_dir(repo.0.join("cases/layout/occupied.rs")).unwrap();
    code(
        &run(&repo, make("cases/layout/occupied.rs")),
        "DESTINATION_ALREADY_EXISTS",
    );
    std::os::unix::fs::symlink("source.rs", repo.0.join("cases/layout/link.rs")).unwrap();
    code(
        &run(&repo, make("cases/layout/link.rs")),
        "DESTINATION_ALREADY_EXISTS",
    );
    let mut filtered = make("cases/layout/excluded.rs");
    filtered["globs"] = json!(["cases/layout/lib.rs", "cases/layout/source.rs"]);
    code(&run(&repo, filtered), "INVALID_DESTINATION");
    repo.write(
        "cases/layout/.gitignore",
        "blocked.rs\nallowed*.rs\n!allowed.rs\n",
    );
    code(
        &run(&repo, make("cases/layout/blocked.rs")),
        "INVALID_DESTINATION",
    );
    apply(&repo, &run(&repo, make("cases/layout/allowed.rs")));
    // Prospective admission and ordinary walker agree for an eligible/ignored equivalent file.
    repo.write("cases/layout/allowed.rs", "fn equivalent() {}");
    repo.write("cases/layout/blocked.rs", "fn ignored() {}");
    let engine = Engine::new(repo.0.clone()).unwrap();
    let search = engine.search(
        serde_json::from_value(
            json!({"repo_path":repo.0,"paths":["cases/layout"],"query":"(function_item) @match"}),
        )
        .unwrap(),
        &AtomicBool::new(false),
    );
    assert!(
        search
            .matches
            .iter()
            .any(|m| m.path == "cases/layout/allowed.rs")
    );
    assert!(
        !search
            .matches
            .iter()
            .any(|m| m.path == "cases/layout/blocked.rs")
    );
}

#[test]
fn definite_selection_errors_limits_noop_and_whole_batch_refusal() {
    let repo = fixture("fn selected() {}\nfn other() {}\n");
    let args = request(
        &repo,
        json!([entry(
            &repo,
            "fn selected() {}",
            new("cases/layout/new_file.rs")
        )]),
    );
    let mut duplicate = args.clone();
    duplicate["moves"]
        .as_array_mut()
        .unwrap()
        .push(args["moves"][0].clone());
    code(&run(&repo, duplicate), "DUPLICATE_MOVE");
    let mut stale = args.clone();
    stale["moves"][0]["item"]["expected_text"] = json!("stale");
    code(&run(&repo, stale), "STALE_SELECTION");
    let mut partial = args.clone();
    partial["moves"][0]["item"] = anchor(&repo, "cases/layout/source.rs", "selected");
    code(&run(&repo, partial), "INVALID_ITEM_SELECTION");
    let mut same = args.clone();
    same["moves"][0]["destination"] = json!({"kind":"existing","path":"cases/layout/source.rs"});
    code(&run(&repo, same), "INVALID_DESTINATION");
    let noop = run(&repo, request(&repo, json!([])));
    assert_eq!(noop["plan"]["applicable"], true);
    assert_eq!(noop["plan"]["edits"], json!([]));
    assert_eq!(noop["plan"]["created_files"], json!([]));
    assert_eq!(noop["plan"]["patch"], "");
    assert_eq!(noop["plan"]["chain_diagnostics"], json!([]));
    assert_eq!(noop["plan"]["decisions"], json!([]));
    let mut capped = args.clone();
    capped["moves"].as_array_mut().unwrap().push(entry(
        &repo,
        "fn other() {}",
        new("cases/layout/new_file.rs"),
    ));
    capped["max_moves"] = json!(1);
    code(&run(&repo, capped), "max_moves");
    let mut output = args.clone();
    repo.write(
        "cases/layout/source.rs",
        &format!("fn selected() {{ let x = \"{}\"; }}", "a".repeat(24000)),
    );
    let text = fs::read_to_string(repo.0.join("cases/layout/source.rs")).unwrap();
    output["moves"][0]["item"] = anchor(&repo, "cases/layout/source.rs", &text);
    output["limits"] = json!({"text_bytes":0,"response_bytes":65536});
    code(&run(&repo, output), "response_bytes");
    repo.write(
        "cases/layout/source.rs",
        "fn selected() {}\nfn bad() { @ }\n",
    );
    code(&run(&repo, args.clone()), "PREEXISTING_SYNTAX_ERROR");
    let cancelled = Engine::new(repo.0.clone()).unwrap().move_item(
        serde_json::from_value(args).unwrap(),
        &AtomicBool::new(true),
    );
    assert_eq!(cancelled.error.unwrap().code, "CANCELLED");
    assert!(cancelled.plan.chain_diagnostics.is_empty());
    assert!(cancelled.plan.decisions.is_empty());
    assert!(
        cancelled.plan.edits.is_none()
            && cancelled.plan.created_files.is_none()
            && cancelled.plan.patch.is_none()
    );
}

#[test]
fn quoted_directories_legacy_layout_root_extraction_and_override_errors() {
    let repo = fixture("fn selected() {}");
    let dir = "cases/quo té\t\"\\";
    let root = format!("{dir}/lib.rs");
    let source = format!("{dir}/source.rs");
    let target = format!("{dir}/new_file.rs");
    repo.write(&root, "mod source;\n");
    repo.write(&source, "fn selected() {} // trailing");
    fs::set_permissions(repo.0.join(&source), fs::Permissions::from_mode(0o755)).unwrap();
    let args = json!({"repo_path":repo.0,"crate_root":root,"paths":[dir],"moves":[{"item":anchor(&repo,&source,"fn selected() {}"),"destination":{"kind":"new_sibling","path":target,"parent_path":root}}]});
    apply(&repo, &run(&repo, args));
    let legacy = json!({"repo_path":repo.0,"crate_root":"src/lib.rs","paths":["src"],"moves":[{"item":anchor(&repo,"src/legacy/source.rs","pub(crate) fn legacy_source() {}"),"destination":{"kind":"new_sibling","path":"src/legacy/new_file.rs","parent_path":"src/legacy/mod.rs"}}]});
    apply(&repo, &run(&repo, legacy));
    repo.write("cases/layout/lib.rs", "fn target() {}");
    let args = request(
        &repo,
        json!([{"item":anchor(&repo,"cases/layout/lib.rs","fn target() {}"),"destination":new("cases/layout/target.rs")} ]),
    );
    let base = run(&repo, args.clone());
    apply(&repo, &base);
    let module = base["plan"]["rewrites"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["kind"] == "module_declaration")
        .unwrap();
    for (action, replacement, expected) in [
        ("retain", None, "MODULE_CONTEXT"),
        (
            "replace",
            Some("pub mod target;"),
            "INVALID_REWRITE_OVERRIDE",
        ),
    ] {
        let mut altered = args.clone();
        let mut choice = json!({"target":module["target"],"action":action});
        if let Some(text) = replacement {
            choice["replacement_text"] = json!(text);
        }
        altered["rewrite_overrides"] = json!([choice]);
        code(&run(&repo, altered), expected);
    }
    let mut stale = args.clone();
    let mut choice = json!({"target":module["target"],"action":"accept_default"});
    choice["target"]["items"][0]["expected_text"] = json!("stale");
    stale["rewrite_overrides"] = json!([choice]);
    code(&run(&repo, stale), "STALE_REWRITE_OVERRIDE");
    let mut duplicate = args;
    let choice = json!({"target":module["target"],"action":"accept_default"});
    duplicate["rewrite_overrides"] = json!([choice, choice]);
    code(&run(&repo, duplicate), "INVALID_REWRITE_OVERRIDE");
}

#[test]
fn visibility_guards_use_syntax_not_whitespace_spelling() {
    let repo = fixture("pub fn selected() {}");
    repo.write(
        "cases/layout/lib.rs",
        "pub\tmod source;\nmod destination;\n",
    );
    let args = request(
        &repo,
        json!([entry(
            &repo,
            "pub fn selected() {}",
            new("cases/layout/target.rs")
        )]),
    );
    for declaration in [
        "pub\tmod source;\nmod destination;\n",
        "pub/* api */mod source;\nmod destination;\n",
    ] {
        repo.write("cases/layout/lib.rs", declaration);
        code(&run(&repo, args.clone()), "REEXPORT_DEPENDENCY");
    }
    repo.write(
        "cases/layout/lib.rs",
        "mod source;\nmod destination;\npub /* scope */ ( self ) mod target;\n",
    );
    repo.write("cases/layout/source.rs", "fn selected() {}");
    code(
        &run(
            &repo,
            request(
                &repo,
                json!([entry(
                    &repo,
                    "fn selected() {}",
                    new("cases/layout/target.rs")
                )]),
            ),
        ),
        "VISIBILITY_CONTEXT",
    );
    repo.write("cases/layout/lib.rs", "mod source;\nmod destination;\n");
    repo.write("cases/layout/source.rs", "pub ( self ) fn selected() {}");
    code(
        &run(
            &repo,
            request(
                &repo,
                json!([entry(
                    &repo,
                    "pub ( self ) fn selected() {}",
                    new("cases/layout/target.rs")
                )]),
            ),
        ),
        "VISIBILITY_CONTEXT",
    );
    let written = "pub /* local */ ( crate ) fn selected() {}";
    repo.write("cases/layout/source.rs", written);
    let result = run(
        &repo,
        request(
            &repo,
            json!([entry(&repo, written, new("cases/layout/target.rs"))]),
        ),
    );
    assert_eq!(
        result["plan"]["moves"][0]["item"]["visibility"],
        "pub /* local */ ( crate )"
    );
    apply(&repo, &result);
}

#[test]
fn unrelated_module_aliases_do_not_block_moves() {
    for import in [
        "use crate::source as source_alias;\n",
        "use crate::{source as source_alias};\n",
    ] {
        for consumer in ["", "fn caller() { source_alias::retained(); }\n"] {
            let selected = "pub(crate) fn selected() {}";
            let repo = fixture(&format!("{selected}\npub(crate) fn retained() {{}}\n"));
            let root = format!("mod source;\nmod destination;\n{import}{consumer}");
            repo.write("cases/layout/lib.rs", &root);
            for destination in [
                new("cases/layout/target.rs"),
                json!({"kind":"existing","path":"cases/layout/destination.rs"}),
            ] {
                let target = destination["path"].as_str().unwrap().to_owned();
                let result = run(
                    &repo,
                    request(&repo, json!([entry(&repo, selected, destination)])),
                );
                assert!(result["plan"]["blockers"].as_array().unwrap().is_empty());
                let copy = apply(&repo, &result);
                let applied_root = fs::read_to_string(copy.0.join("cases/layout/lib.rs")).unwrap();
                assert_eq!(applied_root.replace("mod target;\n", ""), root);
                assert!(
                    fs::read_to_string(copy.0.join(target))
                        .unwrap()
                        .contains(selected)
                );
                assert_eq!(
                    fs::read_to_string(copy.0.join("cases/layout/source.rs")).unwrap(),
                    "\npub(crate) fn retained() {}\n"
                );
            }
        }
    }
}

#[test]
fn ordinary_module_alias_consumers_repair_but_globs_still_block() {
    for import in [
        "use crate::source as source_alias;\n",
        "use crate::{source as source_alias};\n",
    ] {
        for (consumer, category, evidence) in [
            (
                "fn caller() { source_alias::selected(); }\n",
                "UNSUPPORTED_DEPENDENCY_FORM",
                "selected",
            ),
            (
                "use source_alias::*;\n",
                "GLOB_DEPENDENCY",
                "use source_alias::*;",
            ),
        ] {
            let selected = "pub(crate) fn selected() {}";
            let repo = fixture(selected);
            repo.write(
                "cases/layout/lib.rs",
                &format!("mod source;\nmod destination;\n{import}{consumer}"),
            );
            let result = run(
                &repo,
                request(
                    &repo,
                    json!([entry(&repo, selected, new("cases/layout/target.rs"))]),
                ),
            );
            if category == "UNSUPPORTED_DEPENDENCY_FORM" {
                let copy = apply(&repo, &result);
                assert!(
                    fs::read_to_string(copy.0.join("cases/layout/lib.rs"))
                        .unwrap()
                        .contains("crate::target::selected()")
                );
                continue;
            }
            code(&result, category);
            let item_id = &result["plan"]["moves"][0]["item"]["id"];
            assert!(
                result["plan"]["decisions"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|d| {
                        d["blocks_applicability"] == true
                            && d["item_ids"].as_array().unwrap().contains(item_id)
                            && d["anchors"].as_array().unwrap().iter().any(|a| {
                                a["path"] == "cases/layout/lib.rs" && a["expected_text"] == evidence
                            })
                    }),
                "{result}"
            );
        }
    }
}

#[test]
fn collision_arrivals_parent_context_and_dependency_taxonomy() {
    let repo = fixture("fn selected() {}\n");
    repo.write(
        "cases/layout/lib.rs",
        "mod source;\nmod destination;\nmod second;\n",
    );
    repo.write("cases/layout/second.rs", "fn selected() {}\n");
    let collision = request(
        &repo,
        json!([entry(&repo,"fn selected() {}",new("cases/layout/target.rs")),{"item":anchor(&repo,"cases/layout/second.rs","fn selected() {}"),"destination":new("cases/layout/target.rs")} ]),
    );
    code(&run(&repo, collision), "BINDING_COLLISION");
    repo.write("cases/layout/second.rs", "fn target() {}\n");
    let parent_collision = request(
        &repo,
        json!([entry(&repo,"fn selected() {}",new("cases/layout/target.rs")),{"item":anchor(&repo,"cases/layout/second.rs","fn target() {}"),"destination":{"kind":"existing","path":"cases/layout/lib.rs"}}]),
    );
    code(&run(&repo, parent_collision), "BINDING_COLLISION");
    let clean = request(
        &repo,
        json!([entry(
            &repo,
            "fn selected() {}",
            new("cases/layout/target.rs")
        )]),
    );
    repo.write(
        "cases/layout/lib.rs",
        "#![no_implicit_prelude]\nmod source;\nmod destination;\n",
    );
    code(&run(&repo, clean.clone()), "CRATE_IDENTITY_UNCERTAIN");
    repo.write(
        "cases/layout/lib.rs",
        "mod source;\nmod destination;\npub(self) mod target;\n",
    );
    code(&run(&repo, clean), "VISIBILITY_CONTEXT");
    for (source, text, category) in [
        ("mod selected;", "mod selected;", "MODULE_CONTEXT"),
        ("use core::mem;", "use core::mem;", "SCOPE_DEPENDENCY"),
        (
            "macro_rules! selected { () => {}; }",
            "macro_rules! selected { () => {}; }",
            "MACRO_DEPENDENCY",
        ),
        (
            "macro_rules! call { ($f:ident) => { $f() }; } fn selected() {} fn caller() { call!(selected); }",
            "fn selected() {}",
            "MACRO_DEPENDENCY",
        ),
        (
            "pub(super) fn selected() {}",
            "pub(super) fn selected() {}",
            "VISIBILITY_CONTEXT",
        ),
        (
            "#[cfg(any())] fn selected() {}",
            "fn selected() {}",
            "SCOPE_DEPENDENCY",
        ),
        (
            "fn selected() { missing(); }",
            "fn selected() { missing(); }",
            "UNSUPPORTED_DEPENDENCY_FORM",
        ),
        (
            "use somewhere::*; fn selected() { missing(); }",
            "fn selected() { missing(); }",
            "GLOB_DEPENDENCY",
        ),
        (
            "fn selected() {} pub use self::selected as exposed;",
            "fn selected() {}",
            "REEXPORT_DEPENDENCY",
        ),
    ] {
        repo.write("cases/layout/lib.rs", "mod source;\nmod destination;\n");
        repo.write("cases/layout/source.rs", source);
        let args = request(
            &repo,
            json!([entry(&repo, text, new("cases/layout/target.rs"))]),
        );
        code(&run(&repo, args), category);
    }
    repo.write(
        "cases/layout/source.rs",
        "use somewhere::*; macro_rules! unrelated { () => { \"selected\" }; } fn selected() {}",
    );
    apply(
        &repo,
        &run(
            &repo,
            request(
                &repo,
                json!([entry(
                    &repo,
                    "fn selected() {}",
                    new("cases/layout/target.rs")
                )]),
            ),
        ),
    );
}
