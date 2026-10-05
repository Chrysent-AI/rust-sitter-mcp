#[path = "support/advice_flow.rs"]
mod advice_flow;
#[path = "support/fixture_gen.rs"]
mod fixture_gen;
#[path = "support/move_artifacts.rs"]
mod move_artifacts;
#[path = "support/stdio_client.rs"]
mod stdio_client;
use fixture_gen::{Fixture, observe};
use rust_sitter_mcp::{engine::Engine, split::SuggestSplitRequest};
use serde_json::{Value, json};
use std::{collections::BTreeSet, fs, sync::atomic::AtomicBool};
use stdio_client::Client;

fn args(repo: &Fixture, source: &str) -> Value {
    json!({"repo_path":repo.0,"crate_root":"src/lib.rs","source_path":source,"paths":["src"],"limits":{"text_bytes":0}})
}
fn run(repo: &Fixture, args: Value) -> Value {
    let request = serde_json::from_value(args).unwrap();
    serde_json::to_value(
        Engine::new(repo.0.clone())
            .unwrap()
            .suggest_split(request, &AtomicBool::new(false)),
    )
    .unwrap()
}
fn item<'a>(result: &'a Value, name: &str) -> &'a Value {
    result["inventory"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["name"] == name)
        .unwrap()
}
fn edge<'a>(result: &'a Value, from: &str, to: &str) -> Option<&'a Value> {
    let a = &item(result, from)["id"];
    let b = &item(result, to)["id"];
    result["signals"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["from_item_id"] == *a && s["to_item_id"] == *b)
}
#[test]
fn all_written_units_and_display_omission_preserve_complete_drafts() {
    let repo = Fixture::generate();
    let before = observe(&repo.0);
    let result = run(&repo, args(&repo, "src/inventory.rs"));
    advice_flow::complete(&result);
    let inventory = result["inventory"].as_array().unwrap();
    assert_eq!(inventory.len(), 18);
    assert_eq!(
        inventory
            .iter()
            .filter(|i| i["kind"] == "impl_item")
            .count(),
        3
    );
    assert!(
        inventory
            .iter()
            .filter(|i| i["kind"] == "impl_item")
            .all(|i| i["name"].is_null())
    );
    assert!(
        inventory
            .iter()
            .all(|i| i["name"] != "nested" && i["name"] != "member" && i["name"] != "GENERATED")
    );
    for kind in [
        "mod_item",
        "use_declaration",
        "extern_crate_declaration",
        "foreign_mod_item",
        "macro_definition",
        "expression_statement",
    ] {
        assert!(
            inventory
                .iter()
                .any(|i| i["kind"] == kind && !i["reasons"].as_array().unwrap().is_empty()),
            "missing {kind}"
        );
    }
    let impls = result["impl_contexts"].as_array().unwrap();
    assert_eq!(impls.len(), 3);
    assert!(
        impls
            .iter()
            .all(|record| record["written_type"].is_object())
    );
    assert_eq!(
        impls
            .iter()
            .filter(|record| record["written_trait"].is_object())
            .count(),
        1
    );
    assert_eq!(
        result["item_contexts"].as_array().unwrap().len(),
        inventory.len()
    );
    let source = fs::read_to_string(repo.0.join("src/inventory.rs")).unwrap();
    let mut end = 0;
    for unit in inventory {
        let start = unit["span"]["range"]["start_byte"].as_u64().unwrap() as usize;
        let stop = unit["span"]["range"]["end_byte"].as_u64().unwrap() as usize;
        assert!(start >= end && stop > start);
        assert_eq!(unit["span"]["text_bytes"], stop - start);
        assert_eq!(unit["bytes"], stop - start);
        assert_eq!(unit["span"]["text_omitted"], true);
        assert!(unit["span"]["text"].is_null());
        assert!(source.get(start..stop).is_some());
        end = stop;
    }
    assert!(
        !item(&result, "take")["attributes"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(!result["scope_trivia"].as_array().unwrap().is_empty());
    assert!(!result["drafts"].as_array().unwrap().is_empty());
    for draft in result["drafts"].as_array().unwrap() {
        let retain = &draft["groups"][0]["item_ids"];
        for unit in inventory
            .iter()
            .filter(|i| i["eligibility"] != "supported_unit")
        {
            assert!(retain.as_array().unwrap().contains(&unit["id"]));
        }
    }
    assert_eq!(observe(&repo.0), before);
}
#[test]
fn counted_candidates_exclude_independent_locals_and_unrelated_path_suffixes() {
    let repo = Fixture::generate();
    repo.write(
        "src/weak.rs",
        r#"fn target() {}
fn caller() { target(); target(); self::target(); crate::weak::target(); }
fn parameter(target: fn()) { target(); }
fn generic<target>() {}
fn simple_local() { let target = || {}; target(); }
fn closure() { let _ = |target: fn()| target(); }
fn nested() { fn target() {} target(); }
fn local_type() { struct target; let _ = target; }
fn uncertain(pair: (fn(), u8)) { let (target, _) = pair; target(); }
fn qualified() { unrelated::target(); }
fn nested_module() { mod inside { fn invoke() { self::target(); target(); } } }
fn tokens() { target!(); stringify!(target); let _ = "target"; /* target */ }
"#,
    );
    let result = run(&repo, args(&repo, "src/weak.rs"));
    advice_flow::complete(&result);
    let candidate = edge(&result, "caller", "target").unwrap();
    assert_eq!(candidate["count"], 4);
    assert_eq!(candidate["evidence"].as_array().unwrap().len(), 4);
    for control in [
        "parameter",
        "generic",
        "simple_local",
        "closure",
        "nested",
        "local_type",
        "uncertain",
        "qualified",
        "nested_module",
        "tokens",
    ] {
        assert!(
            edge(&result, control, "target").is_none(),
            "false edge from {control}"
        );
    }
    assert!(result["decisions"].as_array().unwrap().iter().any(|d| {
        d["category"] == "binding_collision"
            && d["item_ids"]
                .as_array()
                .unwrap()
                .contains(&item(&result, "uncertain")["id"])
    }));
}
#[test]
fn planted_cohesion_balanced_alternative_and_weak_fallback() {
    let repo = Fixture::generate();
    let result = run(&repo, args(&repo, "src/rich.rs"));
    advice_flow::complete(&result);
    assert_eq!(
        edge(&result, "alpha_read", "alpha_parse").unwrap()["count"],
        1
    );
    assert_eq!(
        edge(&result, "beta_write", "alpha_parse").unwrap()["count"],
        1
    );
    assert_eq!(result["drafts"].as_array().unwrap().len(), 2);
    let first = &result["drafts"][0];
    let groups = first["groups"].as_array().unwrap();
    let alpha = groups
        .iter()
        .find(|g| {
            g["item_ids"]
                .as_array()
                .unwrap()
                .contains(&item(&result, "alpha_read")["id"])
        })
        .unwrap();
    assert!(
        alpha["item_ids"]
            .as_array()
            .unwrap()
            .contains(&item(&result, "alpha_parse")["id"])
    );
    assert_eq!(alpha["confidence"]["level"], "high");
    assert_eq!(alpha["destination"]["path"], "src/alpha.rs");
    assert_eq!(alpha["destination"]["parent_path"], "src/lib.rs");
    let banners: Vec<_> = result["decisions"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|d| d["category"] == "trivia_ownership")
        .collect();
    assert_eq!(banners.len(), 2);
    assert!(
        banners
            .iter()
            .all(|d| d["selected_choice"] == "keep_in_place"
                && d["resolution"] == "choice_available"
                && d["blocks_applicability"] == false)
    );
    assert!(
        result["decisions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["resolution"] == "request_change_required")
    );
    let weak = run(&repo, args(&repo, "src/weak.rs"));
    advice_flow::complete(&weak);
    assert_eq!(weak["drafts"].as_array().unwrap().len(), 1);
    assert!(
        weak["drafts"][0]["rationale"]
            .as_str()
            .unwrap()
            .contains("balanced")
    );
    assert!(
        weak["drafts"][0]["groups"]
            .as_array()
            .unwrap()
            .iter()
            .all(|g| g["confidence"]["level"] == "low")
    );
    assert_eq!(
        weak["drafts"][0]["groups"][0]["item_ids"],
        json!([item(&weak, "apple")["id"]])
    );
}
#[test]
fn common_prefixes_duplicate_names_and_indivisible_size_are_honest() {
    let repo = Fixture::generate();
    repo.write(
        "src/weak.rs",
        "fn get_one() {}\nfn get_two() {}\nfn get_three() {}\n",
    );
    let common = run(&repo, args(&repo, "src/weak.rs"));
    assert!(
        common["drafts"][0]["rationale"]
            .as_str()
            .unwrap()
            .contains("balanced")
    );
    repo.write(
        "src/weak.rs",
        "fn same() {}\nfn same() {}\nfn consumer() { same(); }\n",
    );
    let duplicates = run(&repo, args(&repo, "src/weak.rs"));
    advice_flow::complete(&duplicates);
    assert_eq!(
        duplicates["signals"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|s| s["kind"] == "reference_candidate" && s["facts"]["ambiguous_binding"] == 1)
            .count(),
        2
    );
    assert!(
        duplicates["drafts"][0]["groups"]
            .as_array()
            .unwrap()
            .iter()
            .all(|g| g["confidence"]["level"] == "low")
    );
    repo.write(
        "src/weak.rs",
        &format!(
            "fn huge() {{ let _ = \"{}\"; }}\nfn small() {{}}\n",
            "x".repeat(65 * 1024)
        ),
    );
    let huge = run(&repo, args(&repo, "src/weak.rs"));
    advice_flow::complete(&huge);
    assert!(
        huge["drafts"][0]["groups"][0]["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|w| w.as_str().unwrap().contains("indivisible"))
    );
}
#[test]
fn word_prefixes_outer_attributes_and_doc_heading_signals() {
    let repo = Fixture::generate();
    repo.write("src/weak.rs", "/// # Parsing\n#[inline]\nfn HttpRead() {}\n/// # Parsing\n#[inline]\nfn HttpWrite() {}\nfn helper() {}\n");
    let result = run(&repo, args(&repo, "src/weak.rs"));
    advice_flow::complete(&result);
    for kind in ["name_prefix", "shared_attribute", "doc_heading"] {
        assert!(
            result["signals"]
                .as_array()
                .unwrap()
                .iter()
                .any(|s| s["kind"] == kind && s["item_ids"].as_array().unwrap().len() == 2),
            "{kind}: {result}"
        );
    }
    assert!(
        result["signals"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["kind"] == "name_prefix" && s["label"] == "http")
    );
}
#[test]
fn no_drafts_for_singleton_recovery_and_unsupported_layout() {
    let repo = Fixture::generate();
    for (source, reasons) in [
        ("src/single.rs", "fewer_than_two_eligible_items"),
        ("src/empty.rs", "empty_inventory"),
    ] {
        let result = run(&repo, args(&repo, source));
        advice_flow::complete(&result);
        assert!(result["drafts"].as_array().unwrap().is_empty());
        assert!(
            result["draft_eligibility"]["reasons"]
                .as_array()
                .unwrap()
                .contains(&json!(reasons))
        );
    }
    repo.write("src/weak.rs", "fn first() {}\nfn second() { let x = ; }\n");
    let recovered = run(&repo, args(&repo, "src/weak.rs"));
    advice_flow::complete(&recovered);
    assert_eq!(recovered["integrity"]["syntax"], "input_recovered");
    assert!(recovered["drafts"].as_array().unwrap().is_empty());
    let disconnected = run(
        &repo,
        json!({"repo_path":repo.0,"crate_root":"src/lib.rs","source_path":"cases/ambiguity/binding_collision.rs","paths":["src","cases/ambiguity/binding_collision.rs"]}),
    );
    advice_flow::complete(&disconnected);
    assert!(disconnected["drafts"].as_array().unwrap().is_empty());
    assert!(
        disconnected["draft_eligibility"]["reasons"]
            .as_array()
            .unwrap()
            .contains(&json!("unsupported_or_uncertain_ordinary_layout"))
    );
}
#[test]
fn admission_parent_formula_suffix_probing_and_exhaustion() {
    let repo = Fixture::generate();
    repo.write("src/alpha.rs", "fn occupied() {}\n");
    repo.write("src/alpha_2/mod.rs", "fn competing() {}\n");
    repo.write(
        "src/lib.rs",
        "mod rich;\nmod rewrite;\nmod inventory;\nconst alpha_3: u8 = 0;\n",
    );
    repo.write(".gitignore", "target/\nsrc/alpha_4.rs\n");
    let result = run(&repo, args(&repo, "src/rich.rs"));
    advice_flow::complete(&result);
    let alpha = result["drafts"][0]["groups"]
        .as_array()
        .unwrap()
        .iter()
        .find(|g| {
            g["item_ids"]
                .as_array()
                .unwrap()
                .contains(&item(&result, "alpha_read")["id"])
        })
        .unwrap();
    assert_eq!(alpha["destination"]["path"], "src/alpha_5.rs");
    // A parent's ordinary child formula, not the source's basename, determines the sibling parent.
    repo.write("src/lib.rs", "mod legacy;\n");
    repo.write("src/legacy/source.rs", "fn apple() {}\nfn zebra() {}\n");
    let legacy = run(&repo, args(&repo, "src/legacy/source.rs"));
    advice_flow::complete(&legacy);
    assert_eq!(
        legacy["drafts"][0]["groups"][1]["destination"]["parent_path"],
        "src/legacy/mod.rs"
    );
    let mut narrowed = args(&repo, "src/legacy/source.rs");
    narrowed["paths"] = json!([
        "src/lib.rs",
        "src/legacy/mod.rs",
        "src/legacy/source.rs",
        "src/legacy/destination.rs"
    ]);
    let filtered = run(&repo, narrowed);
    assert!(filtered["drafts"].as_array().unwrap().is_empty());
    // Exhaust all deterministic base/suffix candidates for a two-item fallback.
    for i in 1..=99 {
        let stem = if i == 1 {
            "source_part".into()
        } else {
            format!("source_part_{i}")
        };
        repo.write(&format!("src/legacy/{stem}.rs"), "fn occupied() {}\n");
    }
    let exhausted = run(&repo, args(&repo, "src/legacy/source.rs"));
    advice_flow::complete(&exhausted);
    assert!(exhausted["drafts"].as_array().unwrap().is_empty());
    assert!(
        exhausted["draft_eligibility"]["reasons"][0]
            .as_str()
            .unwrap()
            .contains("suffix_2_through_99")
    );
}
#[test]
fn incomplete_membership_and_wire_caps_never_leave_drafts_or_dangling_links() {
    let repo = Fixture::generate();
    let mut limited = args(&repo, "src/rich.rs");
    limited["max_items"] = json!(3);
    let result = run(&repo, limited);
    assert_eq!(result["status"], "partial");
    assert_eq!(result["inventory"].as_array().unwrap().len(), 3);
    assert_eq!(result["draft_eligibility"]["membership_complete"], false);
    assert!(
        result["counts"]["omissions"]["inventory_items"]
            .as_u64()
            .unwrap()
            > 0
    );
    assert!(result["drafts"].as_array().unwrap().is_empty());
    // Required membership/decision evidence, not merely source text, exceeds a small wire cap.
    let source: String = (0..150)
        .map(|i| format!("// --- section {i} ---\nfn unit_{i}() {{}}\n"))
        .collect();
    repo.write("src/weak.rs", &source);
    let mut args = args(&repo, "src/weak.rs");
    args["limits"]["response_bytes"] = json!(65536);
    let result = run(&repo, args);
    assert_eq!(result["status"], "partial");
    assert!(result["drafts"].as_array().unwrap().is_empty());
    assert!(result["counts"]["omissions"]["decisions"].as_u64().unwrap() > 0);
    assert!(
        result["counts"]["omissions"]["draft_decision_references"]
            .as_u64()
            .unwrap()
            > 0
    );
    assert!(
        result["inventory"]
            .as_array()
            .unwrap()
            .iter()
            .all(|i| i["signal_ids"].as_array().unwrap().is_empty())
    );
    let bytes = json!({"content":[{"type":"text","text":result.to_string()}],"structuredContent":result,"isError":false}).to_string().len() + 4096;
    assert!(bytes <= 65536, "{bytes}");
}
#[test]
fn display_only_wire_projection_preserves_drafts_and_context_coordinates() {
    let repo = Fixture::generate();
    repo.write(
        "src/weak.rs",
        &format!(
            "fn alpha_one() {{ let _ = \"{}\"; }}\nfn alpha_two() {{}}\nfn retained() {{}}",
            "x".repeat(40 * 1024)
        ),
    );
    let mut request = args(&repo, "src/weak.rs");
    request["limits"]["text_bytes"] = json!(65536);
    request["limits"]["response_bytes"] = json!(65536);
    let result = run(&repo, request);
    advice_flow::complete(&result);
    assert!(!result["drafts"].as_array().unwrap().is_empty());
    assert_eq!(result["draft_eligibility"]["membership_complete"], true);
    assert!(
        result["counts"]["omissions"]["display_text_fields"]
            .as_u64()
            .unwrap()
            > 0
    );
    assert!(result["counts"]["omissions"].get("drafts").is_none());
    let record = result["item_contexts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|record| record["item_id"] == item(&result, "alpha_two")["id"])
        .unwrap();
    assert_eq!(record["context"]["before"][0]["start"]["line"], 1);
    assert_eq!(record["context"]["after"][0]["end"]["line"], 3);
}
#[test]
fn reused_declaration_and_root_filename_fallback_are_evidenced() {
    let repo = Fixture::generate();
    repo.write("cases/layout/lib.rs", "mod source;\nmod source_part;\n");
    repo.write("cases/layout/source.rs", "fn one() {}\nfn two() {}\n");
    let advice = run(
        &repo,
        json!({"repo_path":repo.0,"crate_root":"cases/layout/lib.rs","source_path":"cases/layout/source.rs","paths":["cases/layout"]}),
    );
    advice_flow::complete(&advice);
    assert_eq!(
        advice["drafts"][0]["groups"][1]["destination"]["path"],
        "cases/layout/source_part.rs"
    );
    assert_eq!(
        advice["drafts"][0]["groups"][1]["destination"]["existing_declaration"]["span"]["text"],
        "mod source_part;"
    );
    repo.write("cases/layout/type.rs", "fn one() {}\nfn two() {}\n");
    let root = run(
        &repo,
        json!({"repo_path":repo.0,"crate_root":"cases/layout/type.rs","source_path":"cases/layout/type.rs","paths":["cases/layout"]}),
    );
    advice_flow::complete(&root);
    assert_eq!(
        root["drafts"][0]["groups"][1]["destination"]["path"],
        "cases/layout/split_part.rs"
    );
    assert_eq!(
        root["drafts"][0]["groups"][1]["destination"]["parent_path"],
        "cases/layout/type.rs"
    );
}
#[test]
fn sparse_reference_work_guard_is_incomplete_not_a_subset() {
    let repo = Fixture::generate();
    repo.write(
        "src/weak.rs",
        &format!(
            "fn target() {{}}\nfn caller() {{ {} }}\n",
            "self::target();".repeat(100_001)
        ),
    );
    let result = run(&repo, args(&repo, "src/weak.rs"));
    assert_eq!(result["status"], "partial");
    assert!(result["drafts"].as_array().unwrap().is_empty());
    assert!(
        result["truncation_reasons"]
            .as_array()
            .unwrap()
            .contains(&json!("reference_work_limit")),
        "{result}"
    );
    assert_eq!(result["integrity"]["semantic"], "not_performed");
}
#[test]
fn cancelled_invalid_and_strict_stdio_failures_are_typed() {
    let repo = Fixture::generate();
    let before = observe(&repo.0);
    let request: SuggestSplitRequest = serde_json::from_value(args(&repo, "src/rich.rs")).unwrap();
    let cancelled = Engine::new(repo.0.clone())
        .unwrap()
        .suggest_split(request, &AtomicBool::new(true));
    assert_eq!(cancelled.error.unwrap().code, "CANCELLED");
    assert!(cancelled.drafts.is_empty());
    let mut client = Client::new();
    for change in [
        json!({"apply":true}),
        json!({"draft_id":"draft/0"}),
        json!({"limits":{"unknown":1}}),
    ] {
        let mut request = args(&repo, "src/rich.rs");
        request
            .as_object_mut()
            .unwrap()
            .extend(change.as_object().unwrap().clone());
        let result = client.rpc(
            "tools/call",
            json!({"name":"suggest_split","arguments":request}),
        );
        assert_eq!(result["isError"], true);
        let advice = &result["structuredContent"];
        assert_eq!(advice["tool"], "suggest_split");
        assert_eq!(advice["error"]["code"], "INVALID_PARAMS");
        assert_eq!(advice["integrity"]["semantic"], "not_performed");
        assert!(advice.get("plan").is_none());
    }
    let mut invalid = args(&repo, "src/rich.rs");
    invalid["max_items"] = json!(0);
    assert_eq!(run(&repo, invalid)["error"]["code"], "INVALID_PARAMS");
    assert_eq!(observe(&repo.0), before);
}
#[test]
fn identical_requests_are_stable_across_fresh_processes_and_scope_order() {
    let repo = Fixture::generate();
    let mut first = Client::new();
    let a = first.call("suggest_split", args(&repo, "src/rich.rs"));
    assert_eq!(a, first.call("suggest_split", args(&repo, "src/rich.rs")));
    drop(first);
    let mut second = Client::new();
    let b = second.call("suggest_split", args(&repo, "src/rich.rs"));
    assert_eq!(a, b);
    let mut order_a = args(&repo, "src/rich.rs");
    order_a["paths"] = json!(["src", "src/rich.rs"]);
    let mut order_b = order_a.clone();
    order_b["paths"] = json!(["src/rich.rs", "src", "src"]);
    assert_eq!(run(&repo, order_a), run(&repo, order_b));
    let ids: BTreeSet<_> = a["decisions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids.len(), a["decisions"].as_array().unwrap().len());
}
#[test]
fn edited_membership_flow_and_staleness_use_only_explicit_moves() {
    let repo = Fixture::generate();
    let before = observe(&repo.0);
    let advice = run(&repo, args(&repo, "src/rich.rs"));
    let batch = advice_flow::edited_batch(&repo, &advice);
    let engine = Engine::new(repo.0.clone()).unwrap();
    let result = serde_json::to_value(engine.move_item(
        serde_json::from_value(batch.clone()).unwrap(),
        &AtomicBool::new(false),
    ))
    .unwrap();
    let copy = move_artifacts::apply(&repo, &result);
    assert!(
        fs::read_to_string(copy.0.join("src/edited_a.rs"))
            .unwrap()
            .contains("fn beta_write")
    );
    assert_eq!(observe(&repo.0), before);
    // Provenance is descriptive; even a real draft ID cannot authorize a stale anchor.
    repo.write(
        "src/rich.rs",
        &fs::read_to_string(repo.0.join("src/rich.rs"))
            .unwrap()
            .replace("alpha_parse() }", "alpha_parse() + 1 }"),
    );
    let stale = engine.move_item(
        serde_json::from_value(batch).unwrap(),
        &AtomicBool::new(false),
    );
    assert_eq!(stale.error.unwrap().code, "STALE_SELECTION");
    assert!(
        stale.plan.patch.is_none()
            && stale.plan.edits.is_none()
            && stale.plan.created_files.is_none()
    );
}

#[test]
#[ignore = "release-build ten fresh-server advice repetitions; not the routine gate"]
#[allow(clippy::assertions_on_constants)]
fn release_advice_workload_ten_runs() {
    assert!(!cfg!(debug_assertions), "run with --release");
    let repo = Fixture::generate();
    let mut corpus = vec![(
        "corpus/lib.rs".to_owned(),
        (0..99)
            .map(|i| format!("mod file_{i:03};\n"))
            .collect::<String>(),
    )];
    let items: String = (0..400)
        .map(|i| {
            let family = if i < 200 { "alpha" } else { "beta" };
            let next = if i % 2 == 0 { i + 1 } else { i - 1 };
            format!(
                "fn {family}_{i:03}() {{ {family}_{next:03}(); let _payload = \"{}\"; }}\n",
                "x".repeat(1800)
            )
        })
        .collect();
    for i in 0..99 {
        corpus.push((
            format!("corpus/file_{i:03}.rs"),
            if i == 0 { items.clone() } else { String::new() },
        ));
    }
    let mut remaining = 10 * 1024 * 1024;
    for (i, (path, source)) in corpus.iter().enumerate() {
        let length = if i == 99 {
            remaining
        } else {
            (96 * 1024).max(source.len() + 16)
        };
        remaining -= length;
        let padded = format!(
            "{source}\n\n/*{}*/\n",
            "p".repeat(length - source.len() - 7)
        );
        assert_eq!(padded.len(), length);
        if i == 1 {
            assert!(padded.len() <= 1024 * 1024);
        }
        repo.write(path, &padded);
    }
    assert_eq!(remaining, 0);
    let before = observe(&repo.0);
    let mut successes = 0;
    let mut previous = None;
    for repetition in 1..=10 {
        for (path, _) in &corpus {
            fs::read(repo.0.join(path)).unwrap();
        }
        let mut client = Client::new();
        let started = std::time::Instant::now();
        let result = client.call("suggest_split", json!({"repo_path":repo.0,"crate_root":"corpus/lib.rs","source_path":"corpus/file_000.rs","paths":["corpus"],"limits":{"text_bytes":0}}));
        let elapsed = started.elapsed().as_millis();
        advice_flow::complete(&result);
        assert_eq!(result["counts"]["eligible_files"], 100);
        assert_eq!(result["counts"]["inventory_items"], 400);
        assert_eq!(result["counts"]["reference_candidates"], 400);
        assert!(!result["drafts"].as_array().unwrap().is_empty());
        assert_eq!(observe(&repo.0), before);
        if let Some(old) = &previous {
            assert_eq!(&result, old);
        }
        let structured = result.to_string().len();
        let wire = json!({"content":[{"type":"text","text":result.to_string()}],"structuredContent":result,"isError":false}).to_string().len() + 4096;
        assert!(wire <= 2 * 1024 * 1024);
        eprintln!(
            "advice workload run={repetition} ms={elapsed} source_bytes={} corpus_bytes={} items={} candidates={} signals={} decisions={} drafts={} descriptor_bytes={} structured_bytes={structured} duplicated_wire_bytes={wire}",
            result["source"]["bytes"],
            10 * 1024 * 1024,
            result["counts"]["inventory_items"],
            result["counts"]["reference_candidates"],
            result["counts"]["signals"],
            result["counts"]["decisions"],
            result["counts"]["drafts"],
            result["counts"]["analysis_descriptor_bytes"]
        );
        previous = Some(result);
        if elapsed <= 5000 {
            successes += 1;
        }
    }
    assert!(successes >= 9, "only {successes}/10 within 5000 ms");
}
