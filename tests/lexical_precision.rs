#[path = "support/fixture_gen.rs"]
mod fixture_gen;
#[path = "support/move_artifacts.rs"]
mod move_artifacts;
#[path = "support/spawner_precision.rs"]
mod spawner_precision;
use fixture_gen::{Fixture, observe};
use move_artifacts::anchor;
use rust_sitter_mcp::{engine::Engine, move_plan::MoveRequest, split::SuggestSplitRequest};
use serde_json::{Value, json};
use std::{collections::BTreeMap, fs, process::Command, sync::atomic::AtomicBool};

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
    let mut wrong_root = request(
        &repo,
        "const TRANSCRIPT_MAX_RAW_LINE: usize = 16 * 1024 * 1024;",
    );
    wrong_root["crate_root"] = json!("cases/precision/main.rs");
    withheld(&run(&repo, wrong_root));
}

fn advice(repo: &Fixture) -> Value {
    let before = observe(&repo.0);
    let request: SuggestSplitRequest = serde_json::from_value(json!({"repo_path":repo.0,"crate_root":"cases/precision/lib.rs","source_path":SOURCE,"paths":["cases/precision"],"limits":{"text_bytes":0}})).unwrap();
    let result = Engine::new(repo.0.clone())
        .unwrap()
        .suggest_split(request, &AtomicBool::new(false));
    assert_eq!(observe(&repo.0), before);
    serde_json::to_value(result).unwrap()
}
fn withheld(result: &Value) {
    assert_eq!(result["plan"]["applicable"], false, "{result}");
    for field in ["edits", "created_files", "patch"] {
        assert!(result["plan"][field].is_null(), "{result}");
    }
}
fn compile(repo: &Fixture) {
    // Never compile or create build artifacts in the caller repository.
    let copy = repo.copy();
    copy.write("Cargo.toml", "[package]\nname = \"fixture-corpus\"\nversion = \"0.0.0\"\nedition = \"2024\"\n[lib]\npath = \"cases/precision/lib.rs\"\n");
    let result = Command::new("cargo")
        .current_dir(&copy.0)
        .args(["check", "--locked", "--offline", "--quiet"])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
}
#[test]
fn sanitizer_replica_exposes_candidate_and_retains_true_blockers() {
    let source = format!(
        "{}\n\n{}\n",
        include_str!("fixtures/spawner_precision/sanitizers.rs"),
        r#"fn compact_transcript_event(event_type: Option<&str>) {
    match event_type {
        Some(event_type @ ("message_start" | "message_end" | "turn_end")) => {
            compact_transcript_message_field(obj, "message");
            if event_type == "message_end" {
                for key in ["stopReason", "stop_reason"] {
                    sanitize_transcript_field(obj, key, sanitized_transcript_stop_reason);
                }
            }
        }
        _ => {}
    }
}"#
    );
    let repo = spawner_precision::load(&source);
    let mut args = request(&repo, "fn sanitized_transcript_stop_reason");
    // Use the complete original written functions, not reconstructed source.
    args["moves"] = json!(["sanitized_transcript_stop_reason", "sanitized_transcript_error", "sanitized_transcript_terminal_value"].map(|name| {
        let start = source.find(&format!("fn {name}(")).unwrap();
        let rest = &source[start..];
        let end = rest.find("\n\nfn ").unwrap_or(rest.len());
        json!({"item":anchor(&repo, SOURCE, rest[..end].trim_end()),"destination":{"kind":"new_sibling","path":"cases/precision/subagent/sanitized_transcript.rs","parent_path":"cases/precision/subagent/mod.rs"}})
    }));
    let result = run(&repo, args);
    withheld(&result);
    let binder = source.find("|raw|").unwrap() + 1;
    assert!(
        !result["plan"]["decisions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["anchors"][0]["range"]["start_byte"] == binder),
        "{result}"
    );
    let mut reasons = BTreeMap::<String, usize>::new();
    let mut categories = BTreeMap::<String, usize>::new();
    for decision in result["plan"]["decisions"].as_array().unwrap() {
        if decision["blocks_applicability"] == true {
            assert_ne!(decision["reason"], "lexical_context_unproved", "{decision}");
            assert!(!decision["anchors"].as_array().unwrap().is_empty());
            *reasons
                .entry(decision["reason"].as_str().unwrap().into())
                .or_default() += 1;
            *categories
                .entry(decision["category"].as_str().unwrap().into())
                .or_default() += 1;
        }
    }
    for category in [
        "unsupported_dependency_form",
        "scope_dependency",
        "visibility_context",
    ] {
        assert!(categories.contains_key(category), "{result}");
    }
    for reason in [
        "external_or_missing_binding",
        "conditional_or_inherited_context",
        "member_or_constructor_unproved",
    ] {
        assert!(reasons.contains_key(reason), "{result}");
    }
    println!("sanitizer replica remaining typed blockers: {reasons:?}; categories: {categories:?}");
    let advice = advice(&repo);
    for reference in [
        "sanitized_transcript_stop_reason(value)",
        "sanitized_transcript_stop_reason);",
    ] {
        let at = source.find(reference).unwrap();
        assert!(
            !advice["decisions"]
                .as_array()
                .unwrap()
                .iter()
                .any(|d| d["reason"] == "lexical_context_unproved"
                    && d["anchors"][0]["span"]["range"]["start_byte"] == at)
        );
        assert!(
            advice["signals"]
                .as_array()
                .unwrap()
                .iter()
                .any(|s| s["kind"] == "reference_candidate"
                    && s["evidence"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|e| e["range"]["start_byte"] == at)),
            "{advice}"
        );
    }
}
#[test]
fn closure_binders_are_not_dependencies_but_body_and_type_references_are() {
    for (closure, dependency) in [
        ("|raw| raw", None),
        ("|raw| free(raw)", Some("free")),
        ("|| raw()", Some("raw")),
        ("|raw: ExternalType| raw", Some("ExternalType")),
        ("|(ExternalType(raw),)| raw", Some("ExternalType")),
        ("|ExternalType { raw }| raw", Some("ExternalType")),
    ] {
        let selected = format!("fn selected() {{ let _ = {closure}; }}");
        let repo = spawner_precision::load(&selected);
        let result = run(&repo, request(&repo, &selected));
        assert_eq!(result["plan"]["integrity"]["semantic"], "not_performed");
        let decisions = result["plan"]["decisions"].as_array().unwrap();
        if let Some(dependency) = dependency {
            withheld(&result);
            assert!(
                decisions
                    .iter()
                    .any(|d| d["reason"] == "external_or_missing_binding"
                        && d["anchors"][0]["expected_text"] == dependency),
                "{closure}: {result}"
            );
        } else {
            assert_eq!(result["plan"]["applicable"], true, "{closure}: {result}");
        }
        if closure.starts_with("|raw") {
            let binder = selected.find("raw").unwrap();
            assert!(
                !decisions
                    .iter()
                    .any(|d| d["reason"] == "external_or_missing_binding"
                        && d["anchors"][0]["range"]["start_byte"] == binder),
                "{closure}: {result}"
            );
        }
    }
}
#[test]
fn admitted_patterns_repair_only_free_accesses_and_compile_after_application() {
    for local in [
        "let (other, mut another) = (1, 2); selected();",
        "for other in [1] { selected(); }",
        "if let Some(other) = Some(1) { selected(); }",
        "while let Some(other) = None::<u8> { selected(); }",
        "match Some(1) { Some(other) => selected(), _ => {} }",
        "{ let (mut selected,) = (|| {},); selected(); }",
        "{ let (ref selected,) = (|| {},); selected(); }",
        "for mut selected in [|| {}] { selected(); }",
        "if let Some(ref selected) = Some(|| {}) { selected(); }",
        "match Some(|| {}) { Some(ref selected) => selected(), _ => {} }",
        "let closure = move |selected: fn()| selected();",
        "let (r#other,) = (1,); selected();",
    ] {
        let source =
            format!("fn selected() {{}}\nfn caller() {{ {local} }}\nfn free() {{ selected(); }}\n");
        let repo = spawner_precision::load(&source);
        let mut args = request(&repo, "fn selected() {}");
        let result = run(&repo, args.clone());
        assert_eq!(result["plan"]["applicable"], true, "{local}: {result}");
        let import = result["plan"]["rewrites"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["kind"] == "import_insert")
            .unwrap();
        args["rewrite_overrides"] = json!([{"target":import["target"],"action":"replace","replacement_text":"use crate::subagent::probe_constants::selected as relocated;"}]);
        let aliased = run(&repo, args);
        let copy = move_artifacts::apply(&repo, &aliased);
        compile(&copy);
        let after = fs::read_to_string(copy.0.join(SOURCE)).unwrap();
        assert!(after.contains("fn free() { relocated(); }"), "{after}");
        if local.contains("mut selected")
            || local.contains("ref selected")
            || local.contains("selected: fn()")
        {
            assert!(after.contains(local), "{after}");
        } else {
            assert!(
                after.contains(
                    &local
                        .replace("selected();", "relocated();")
                        .replace("=> selected()", "=> relocated()")
                ),
                "{after}"
            );
        }
    }
}
#[test]
fn captured_literal_alternatives_preserve_local_and_free_accesses() {
    let moved = "fn selected() -> &'static str { \"message_end\" }";
    for (arm, expected_arm) in [
        (
            "Some(event_type @ (\"message_start\" | \"message_end\" | \"turn_end\")) if selected() == event_type => { let _ = selected(); }",
            "Some(event_type @ (\"message_start\" | \"message_end\" | \"turn_end\")) if relocated() == event_type => { let _ = relocated(); }",
        ),
        (
            "Some(selected @ (\"message_start\" | \"message_end\" | \"turn_end\")) if selected == \"message_end\" => { let _ = selected; }",
            "Some(selected @ (\"message_start\" | \"message_end\" | \"turn_end\")) if selected == \"message_end\" => { let _ = selected; }",
        ),
    ] {
        let source = format!(
            "{moved}\nfn caller() {{ match Some(selected()) {{ {arm}, _ => {{ let _ = selected(); }} }} }}\nfn free() {{ let _ = selected(); }}\n"
        );
        let repo = spawner_precision::load(&source);
        let mut args = request(&repo, moved);
        let result = run(&repo, args.clone());
        assert_eq!(result["plan"]["applicable"], true, "{result}");
        assert_eq!(result["plan"]["integrity"]["semantic"], "not_performed");
        assert!(
            !result["plan"]["decisions"]
                .as_array()
                .unwrap()
                .iter()
                .any(|d| d["reason"] == "lexical_context_unproved")
        );
        let import = result["plan"]["rewrites"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["kind"] == "import_insert")
            .unwrap();
        args["rewrite_overrides"] = json!([{"target":import["target"],"action":"replace","replacement_text":"use crate::subagent::probe_constants::selected as relocated;"}]);
        let aliased = run(&repo, args);
        let copy = move_artifacts::apply(&repo, &aliased);
        compile(&copy);
        let after = fs::read_to_string(copy.0.join(SOURCE)).unwrap();
        assert!(after.contains(expected_arm), "{after}");
        assert!(after.contains("match Some(relocated())"), "{after}");
        assert!(after.contains("_ => { let _ = relocated(); }"), "{after}");
        assert!(
            after.contains("fn free() { let _ = relocated(); }"),
            "{after}"
        );
    }
}
#[test]
fn ambiguity_matrix_agrees_between_advice_and_move_with_anchored_witnesses() {
    for (caller, reason) in [
        (
            "fn caller(pair: (fn(),)) { let (selected,) = pair; selected(); }",
            "identifier_pattern_binding_or_constant",
        ),
        (
            "fn caller(selected: fn()) { for selected in values { selected(); } }",
            "identifier_pattern_binding_or_constant",
        ),
        (
            "fn caller() { if let Some(selected) = value { selected(); } }",
            "identifier_pattern_binding_or_constant",
        ),
        (
            "fn caller() { while let Some(selected) = value { selected(); } }",
            "identifier_pattern_binding_or_constant",
        ),
        (
            "fn caller() { match value { Some(selected) => selected(), _ => {} } }",
            "identifier_pattern_binding_or_constant",
        ),
        (
            "fn caller() { let (mut selected, p!()) = value; selected(); }",
            "unsupported_pattern",
        ),
        (
            "fn caller() { match value { other @ Some(selected) => selected(), _ => {} } }",
            "identifier_pattern_binding_or_constant",
        ),
        (
            "fn caller() { match value { selected @ Some(selected) => selected(), _ => {} } }",
            "identifier_pattern_binding_or_constant",
        ),
        (
            "fn caller() { match value { selected @ p!() => selected(), _ => {} } }",
            "unsupported_pattern",
        ),
        (
            "fn caller() { match value { other @ (A(mut selected) | B(mut selected)) => selected(), _ => {} } }",
            "unsupported_pattern",
        ),
        (
            "fn caller() { match value { selected @ 1 | 2 => selected(), _ => {} } }",
            "unsupported_pattern",
        ),
        (
            "fn caller() { match value { selected @ (1 | 2) if let Some(x) = value && x > 0 => selected(), _ => {} } }",
            "unsupported_pattern",
        ),
        (
            "fn caller() { if let Some(other) = value && other { selected(); } }",
            "unsupported_pattern",
        ),
        ("fn caller() { m!(); selected(); }", "unsupported_pattern"),
        ("fn caller() { selected(); m!(); }", "unsupported_pattern"),
        (
            "fn caller() { m!(); let selected = value; selected(); }",
            "unsupported_pattern",
        ),
        (
            "fn caller() { let selected = value; selected(); m!(); }",
            "unsupported_pattern",
        ),
        (
            "fn caller() { #[cfg(any())] let other = value; selected(); }",
            "conditional_local_context",
        ),
        (
            "fn caller() { use crate::other::*; selected(); }",
            "relevant_local_import",
        ),
    ] {
        let source = format!("fn selected() {{}}\n{caller}\n");
        let repo = spawner_precision::load(&source);
        let result = run(&repo, request(&repo, "fn selected() {}"));
        withheld(&result);
        let decision = result["plan"]["decisions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|d| d["reason"] == "lexical_context_unproved")
            .unwrap();
        let witness = &decision["lexical_uncertainty"];
        assert_eq!(witness["reason"], reason, "{result}");
        assert_eq!(witness["spelling"], "selected");
        assert_eq!(witness["scope"]["path"], SOURCE);
        assert_eq!(decision["anchors"][0]["expected_text"], "selected");
        let range = &witness["pattern"]["range"];
        assert!(
            !source[range["start_byte"].as_u64().unwrap() as usize
                ..range["end_byte"].as_u64().unwrap() as usize]
                .is_empty()
        );
        let advice = advice(&repo);
        let advice_decision = advice["decisions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|d| d["reason"] == "lexical_context_unproved")
            .unwrap();
        assert_eq!(advice_decision["lexical_uncertainty"], *witness);
        assert!(advice_decision["evidence"][0]["text"].is_null());
        assert_eq!(
            advice_decision["anchors"][0]["span"]["range"],
            decision["anchors"][0]["range"]
        );
        assert_eq!(result, run(&repo, request(&repo, "fn selected() {}")));
    }
}
#[test]
fn binding_scope_boundaries_keep_real_file_consumers_repairable() {
    for caller in [
        "let selected = selected;",
        "for selected in [selected] {}",
        "if let Some(selected) = Some(selected()) {} else { selected(); }",
        "while let Some(selected) = Some(selected()) { break; }",
        "match Some(1) { Some(selected) => {}, _ => selected() }",
    ] {
        let repo =
            spawner_precision::load(&format!("fn selected() {{}}\nfn caller() {{ {caller} }}\n"));
        let result = run(&repo, request(&repo, "fn selected() {}"));
        let copy = move_artifacts::apply(&repo, &result);
        compile(&copy);
        assert!(
            fs::read_to_string(copy.0.join(SOURCE))
                .unwrap()
                .contains(caller)
        );
        assert!(
            result["plan"]["rewrites"]
                .as_array()
                .unwrap()
                .iter()
                .any(|r| r["kind"] == "import_insert"),
            "{result}"
        );
    }
}
#[test]
fn match_guard_references_are_repaired_and_ambiguous_guard_bindings_still_block() {
    for (pattern, applicable) in [("Some(other)", true), ("Some(selected)", false)] {
        let source = format!(
            "fn selected() -> bool {{ true }}\nfn caller() {{ match Some(1) {{ {pattern} if selected() => {{}}, _ => {{}} }} }}\n"
        );
        let repo = spawner_precision::load(&source);
        let mut args = request(&repo, "fn selected() -> bool { true }");
        let result = run(&repo, args.clone());
        assert_eq!(result["plan"]["applicable"], applicable, "{result}");
        if applicable {
            let import = result["plan"]["rewrites"]
                .as_array()
                .unwrap()
                .iter()
                .find(|r| r["kind"] == "import_insert")
                .unwrap();
            args["rewrite_overrides"] = json!([{"target":import["target"],"action":"replace","replacement_text":"use crate::subagent::probe_constants::selected as relocated;"}]);
            let copy = move_artifacts::apply(&repo, &run(&repo, args));
            compile(&copy);
            assert!(
                fs::read_to_string(copy.0.join(SOURCE))
                    .unwrap()
                    .contains("if relocated() =>")
            );
            let result = advice(&repo);
            let at = source.find("selected() =>").unwrap();
            assert!(
                result["signals"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|s| s["kind"] == "reference_candidate"
                        && s["evidence"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .any(|e| e["range"]["start_byte"] == at)),
                "{result}"
            );
        } else {
            withheld(&result);
            assert!(
                result["plan"]["decisions"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|d| d["lexical_uncertainty"]["reason"]
                        == "identifier_pattern_binding_or_constant"),
                "{result}"
            );
        }
    }
}
#[test]
fn moved_forced_composite_locals_pass_the_dependency_prefilter() {
    for text in [
        "fn moved() { let (mut selected,) = (|| {},); selected(); }",
        "fn moved() { let (ref selected,) = (|| {},); selected(); }",
        "fn moved<T>(selected: T) -> T { selected }",
    ] {
        let repo = spawner_precision::load(text);
        let result = run(&repo, request(&repo, text));
        let copy = move_artifacts::apply(&repo, &result);
        compile(&copy);
        assert!(
            !result["plan"]["rewrites"]
                .as_array()
                .unwrap()
                .iter()
                .any(|r| r["kind"] == "import_insert")
        );
    }
}
#[test]
fn value_pattern_cannot_prove_a_same_spelled_type_reference_independent() {
    let moved = "fn moved() { let (mut Widget,) = (1,); let _: Widget = 2; }";
    let source = format!("pub(crate) type Widget = u8;\n{moved}\n");
    let repo = spawner_precision::load(&source);
    let result = run(&repo, request(&repo, moved));
    withheld(&result);
    assert_eq!(result["plan"]["integrity"]["semantic"], "not_performed");
    let decision = result["plan"]["decisions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["reason"] == "lexical_context_unproved")
        .unwrap();
    assert_eq!(decision["blocks_applicability"], true);
    let at = source.find("_: Widget").unwrap() + 3;
    assert_eq!(decision["anchors"][0]["path"], SOURCE);
    assert_eq!(decision["anchors"][0]["expected_text"], "Widget");
    assert_eq!(
        decision["anchors"][0]["range"],
        json!({"start_byte": at, "end_byte": at + "Widget".len()})
    );
    let witness = &decision["lexical_uncertainty"];
    assert_eq!(witness["spelling"], "Widget");
    assert_eq!(witness["reason"], "value_binding_in_type_position");
    for (field, text) in [
        ("scope", "let (mut Widget,) = (1,);"),
        ("pattern", "(mut Widget,)"),
    ] {
        assert_eq!(witness[field]["path"], SOURCE);
        let range = &witness[field]["range"];
        assert_eq!(
            &source[range["start_byte"].as_u64().unwrap() as usize
                ..range["end_byte"].as_u64().unwrap() as usize],
            text
        );
    }
    let advice = advice(&repo);
    let advised = advice["decisions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["reason"] == "lexical_context_unproved")
        .unwrap();
    assert_eq!(advised["lexical_uncertainty"], *witness);
    assert_eq!(
        advised["anchors"][0]["span"]["range"],
        decision["anchors"][0]["range"]
    );
    assert_eq!(result, run(&repo, request(&repo, moved)));
}
#[test]
fn actual_local_type_bindings_remain_independent_after_moving() {
    for moved in [
        "fn moved<Widget>(value: Widget) -> Widget { value }",
        "fn moved() { type Widget = u8; let _: Widget = 2; }",
    ] {
        let source = format!("pub(crate) type Widget = u8;\n{moved}\n");
        let repo = spawner_precision::load(&source);
        let result = run(&repo, request(&repo, moved));
        assert_eq!(result["plan"]["applicable"], true, "{result}");
        assert_eq!(result["plan"]["integrity"]["semantic"], "not_performed");
        assert!(
            result["plan"]["decisions"].as_array().unwrap().is_empty(),
            "{result}"
        );
        assert!(
            !result["plan"]["rewrites"]
                .as_array()
                .unwrap()
                .iter()
                .any(|r| r["kind"] == "import_insert")
        );
        let copy = move_artifacts::apply(&repo, &result);
        compile(&copy);
        assert!(
            fs::read_to_string(copy.0.join("cases/precision/subagent/probe_constants.rs"))
                .unwrap()
                .contains(moved)
        );
    }
}
#[test]
fn local_import_proof_does_not_bypass_a_nearer_ambiguous_pattern() {
    let source = "fn selected() {}\nfn caller(pair: (fn(),)) { use crate::subagent::spawner::selected; { let (selected,) = pair; selected(); } }\n";
    let repo = spawner_precision::load(source);
    let result = run(&repo, request(&repo, "fn selected() {}"));
    withheld(&result);
    assert!(result["plan"]["decisions"].as_array().unwrap().iter().any(|d| d["lexical_uncertainty"]["reason"] == "identifier_pattern_binding_or_constant"), "{result}");
}
#[test]
fn path_prefix_and_caller_selected_alias_fail_with_specific_witnesses() {
    let repo = spawner_precision::load(
        "fn selected() {}\nuse crate::subagent::spawner as prefix;\nfn caller(pair: (fn(),)) { let (prefix,) = pair; prefix::selected(); }\n",
    );
    let result = run(&repo, request(&repo, "fn selected() {}"));
    withheld(&result);
    let decision = result["plan"]["decisions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["lexical_uncertainty"]["spelling"] == "prefix")
        .unwrap();
    assert_eq!(
        decision["lexical_uncertainty"]["reason"],
        "identifier_pattern_binding_or_constant"
    );
    assert_eq!(decision["anchors"][0]["expected_text"], "selected");

    let repo = spawner_precision::load(
        "fn selected() {}\nfn caller(pair: (fn(),)) { let (shadowed,) = pair; selected(); }\n",
    );
    let mut args = request(&repo, "fn selected() {}");
    let result = run(&repo, args.clone());
    assert_eq!(result["plan"]["applicable"], true, "{result}");
    let import = result["plan"]["rewrites"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["kind"] == "import_insert")
        .unwrap();
    args["rewrite_overrides"] = json!([{"target":import["target"],"action":"replace","replacement_text":"use crate::subagent::probe_constants::selected as shadowed;"}]);
    let result = run(&repo, args);
    withheld(&result);
    assert!(
        result["plan"]["decisions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["reason"] == "final_alias_conflict"
                && d["lexical_uncertainty"]["spelling"] == "shadowed"
                && d["lexical_uncertainty"]["reason"] == "identifier_pattern_binding_or_constant"),
        "{result}"
    );
}
#[test]
fn mandatory_lexical_witnesses_survive_display_caps_or_are_explicitly_omitted() {
    let source = format!(
        "fn selected() {{}}\nfn caller(pair: (fn(),)) {{ let (selected,) = pair; {} }}\n",
        "selected();".repeat(200)
    );
    let repo = spawner_precision::load(&source);
    let mut args = request(&repo, "fn selected() {}");
    args["limits"]["diagnostic_count"] = json!(0);
    let counts_only = run(&repo, args.clone());
    withheld(&counts_only);
    assert_eq!(counts_only["plan"]["decisions"], json!([]));
    assert_eq!(counts_only["counts"]["omissions"]["decisions"], 200);
    assert_eq!(counts_only["plan"]["decision_groups"][0]["count"], 200);
    assert_eq!(
        counts_only["plan"]["decision_groups"][0]["reason"],
        "lexical_context_unproved"
    );
    args["limits"]["diagnostic_count"] = json!(256);
    let result = run(&repo, args.clone());
    withheld(&result);
    assert_eq!(result["plan"]["decisions"].as_array().unwrap().len(), 200);
    assert!(result["plan"]["decisions"].as_array().unwrap().iter().all(|d| d["lexical_uncertainty"]["reason"] == "identifier_pattern_binding_or_constant"));
    args["limits"]["response_bytes"] = json!(65536);
    let result = run(&repo, args);
    withheld(&result);
    assert_eq!(result["status"], "partial");
    assert!(result["plan"]["decisions"].as_array().unwrap().is_empty());
    assert_eq!(result["counts"]["omissions"]["decisions"], 200);

    let request: SuggestSplitRequest = serde_json::from_value(json!({"repo_path":repo.0,"crate_root":"cases/precision/lib.rs","source_path":SOURCE,"paths":["cases/precision"],"limits":{"text_bytes":0,"diagnostic_count":0,"response_bytes":65536}})).unwrap();
    let before = observe(&repo.0);
    let result = Engine::new(repo.0.clone())
        .unwrap()
        .suggest_split(request, &AtomicBool::new(false));
    assert_eq!(observe(&repo.0), before);
    let result = serde_json::to_value(result).unwrap();
    assert_eq!(result["status"], "partial", "{result}");
    assert!(result["drafts"].as_array().unwrap().is_empty());
    assert!(result["decisions"].as_array().unwrap().is_empty());
    assert!(result["counts"]["omissions"]["decisions"].as_u64().unwrap() >= 200);
}
