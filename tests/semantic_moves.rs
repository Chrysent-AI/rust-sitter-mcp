#[allow(dead_code)]
#[path = "support/fixture_gen.rs"]
mod fixture_gen;
#[allow(dead_code)]
#[path = "support/move_artifacts.rs"]
mod move_artifacts;
use fixture_gen::{Fixture, observe};
use move_artifacts::anchor;
use rust_sitter_mcp::{engine::Engine, move_plan::MoveRequest};
use serde_json::{Value, json};
use std::sync::atomic::AtomicBool;

#[test]
fn invalid_semantic_configuration_is_actionable_before_resolution() {
    let text = "fn selected(value: crate::Value) -> u32 { value.read() }";
    let repo = fixture(text);
    let mut args = request(&repo, text);
    args["resolve_semantic"] = json!(true);
    args["semantic_configuration"] = config();
    args["semantic_configuration"]["crates"][0]["name"] = json!("fixture-package");
    let result = run(&repo, args);
    assert_eq!(result["error"]["code"], "INVALID_PARAMS", "{result}");
    assert_eq!(
        result["error"]["field"],
        "semantic_configuration.crates[0].name"
    );
    assert!(
        result["error"]["message"]
            .as_str()
            .unwrap()
            .contains("fixture-package")
    );
    assert_eq!(result["plan"]["integrity"]["semantic"], "not_performed");
    assert!(result["plan"]["resolution_coverage"].is_null());
}

#[test]
fn semantic_configuration_error_matrix_includes_empty_candidate_batches() {
    for text in [
        "fn selected() {}",
        "fn selected(value: crate::Value) -> u32 { value.read() }",
    ] {
        let repo = fixture(text);
        let base = config();
        let entry = base["crates"][0].clone();
        let mut cases = Vec::new();
        for (key, value, why) in [
            ("name", json!("fixture-package"), "Cargo package"),
            ("name", json!("9bad"), "identifier"),
            ("name", json!(""), "identifier"),
            ("root_file", json!("cases/layout/missing.rs"), "admitted"),
            ("edition", json!("2099"), "2015"),
        ] {
            let mut c = base.clone();
            c["crates"][0][key] = value.clone();
            cases.push((
                c,
                format!("semantic_configuration.crates[0].{key}"),
                value.as_str().unwrap().to_owned(),
                why,
            ));
        }
        for (key, value, why) in [
            ("name", json!("fixture"), "duplicate crate name"),
            (
                "root_file",
                json!("cases/layout/lib.rs"),
                "duplicate crate root",
            ),
        ] {
            let mut second = entry.clone();
            second["name"] = json!("other");
            second["root_file"] = json!("cases/layout/destination.rs");
            second[key] = value.clone();
            cases.push((
                json!({"crates":[entry.clone(),second]}),
                format!("semantic_configuration.crates[1].{key}"),
                value.as_str().unwrap().to_owned(),
                why,
            ));
        }
        for (target, why) in [("absent", "explicitly configured"), ("fixture", "cycle")] {
            let mut c = base.clone();
            c["crates"][0]["dependencies"] = json!([{"name":"edge","crate_name":target}]);
            cases.push((
                c,
                "semantic_configuration.crates[0].dependencies[0].crate_name".into(),
                target.into(),
                why,
            ));
        }
        let mut second = entry.clone();
        second["name"] = json!("other");
        second["root_file"] = json!("cases/layout/destination.rs");
        second["dependencies"] = json!([{"name":"back","crate_name":"fixture"}]);
        let mut first = entry.clone();
        first["dependencies"] = json!([{"name":"forward","crate_name":"other"}]);
        cases.push((
            json!({"crates":[first,second]}),
            "semantic_configuration.crates[1].dependencies[0].crate_name".into(),
            "fixture".into(),
            "cycle",
        ));
        for (configuration, field, value, why) in cases {
            let mut args = request(&repo, text);
            args["resolve_semantic"] = json!(true);
            args["semantic_configuration"] = configuration.clone();
            let result = run(&repo, args.clone());
            assert_eq!(result["error"]["code"], "INVALID_PARAMS", "{result}");
            assert_eq!(result["error"]["field"], field, "{result}");
            let message = result["error"]["message"].as_str().unwrap();
            assert!(
                message.contains(&value) && message.contains(why),
                "{message}"
            );
            assert_eq!(result["plan"]["integrity"]["semantic"], "not_performed");
            assert!(result["plan"]["resolution_coverage"].is_null());
            args["resolve_semantic"] = json!(false);
            let off = run(&repo, args);
            assert!(off["error"].is_null(), "{off}");
            assert_eq!(off["plan"]["integrity"]["semantic"], "not_performed");
            assert!(off["plan"]["resolution_coverage"].is_null());
        }
        let mut args = request(&repo, text);
        args["resolve_semantic"] = json!(true);
        args["semantic_configuration"] = base;
        args["semantic_configuration"]["crates"][0]["name"] = json!("fixture_package");
        let valid = run(&repo, args);
        assert_eq!(valid["plan"]["applicable"], true, "{valid}");
        assert_eq!(
            valid["plan"]["integrity"]["semantic"],
            if text.contains("value.read") {
                "resolution_performed"
            } else {
                "not_performed"
            }
        );
    }
}

fn config() -> Value {
    json!({"crates":[{"name":"fixture","root_file":"cases/layout/lib.rs","edition":"2024","features":[],"cfg":[],"dependencies":[]}]})
}
fn fixture(text: &str) -> Fixture {
    let repo = Fixture::generate();
    repo.write("cases/layout/lib.rs", "mod source;\nmod destination;\npub struct Value { pub number: u32 }\nimpl Value { pub fn read(&self) -> u32 { self.number } pub fn new() -> Value { Value { number: 7 } } }\n");
    repo.write("cases/layout/source.rs", text);
    repo.write("cases/layout/destination.rs", "fn retained() {}\n");
    repo
}
fn request(repo: &Fixture, text: &str) -> Value {
    json!({"repo_path":repo.0,"crate_root":"cases/layout/lib.rs","paths":["cases/layout"],"limits":{"time_budget_ms":30000,"diagnostic_count":1000},
        "moves":[{"item":anchor(repo,"cases/layout/source.rs",text),"destination":{"kind":"new_sibling","path":"cases/layout/moved.rs","parent_path":"cases/layout/lib.rs"}}]})
}
fn run(repo: &Fixture, args: Value) -> Value {
    let before = observe(&repo.0);
    let result = Engine::new(repo.0.clone()).unwrap().move_item(
        serde_json::from_value::<MoveRequest>(args).unwrap(),
        &AtomicBool::new(false),
    );
    assert_eq!(observe(&repo.0), before);
    serde_json::to_value(result).unwrap()
}
fn resolved(repo: &Fixture, mut args: Value) -> Value {
    args["resolve_semantic"] = json!(true);
    args["semantic_configuration"] = config();
    run(repo, args)
}
fn reason(value: &Value, expected: &str) -> bool {
    value["plan"]["decisions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|d| d["reason"] == expected && d["blocks_applicability"] == true)
}

fn assert_default_off(repo: &Fixture, args: &Value) {
    let omitted = run(repo, args.clone());
    let mut explicit = args.clone();
    explicit["resolve_semantic"] = json!(false);
    explicit["semantic_configuration"] = config();
    assert_eq!(
        serde_json::to_vec(&omitted).unwrap(),
        serde_json::to_vec(&run(repo, explicit)).unwrap()
    );
    assert_eq!(omitted["plan"]["applicable"], false, "{omitted}");
    assert_eq!(omitted["plan"]["integrity"]["semantic"], "not_performed");
    assert!(omitted["plan"]["resolution_coverage"].is_null());
}

fn assert_written_prefix_repair(repo: &Fixture, consumer: &str, routes: &[(&str, &str, usize)]) {
    let args = request(repo, "pub(crate) fn selected() {}");
    assert_default_off(repo, &args);
    let on = resolved(repo, args);
    assert!(on["error"].is_null(), "{on}");
    assert_eq!(on["plan"]["applicable"], true, "{on}");
    // Written repair consumes the need: it is not a both-overlay semantic proof.
    assert_eq!(on["plan"]["integrity"]["semantic"], "not_performed");
    for field in ["binding_proofs", "resolution_coverage"] {
        assert!(on["plan"][field].is_null(), "{on}");
    }
    assert!(on["coverage"]["ra_resolved"].is_null(), "{on}");
    let edits: Vec<_> = on["plan"]["edits"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["path"] == consumer)
        .collect();
    assert_eq!(edits.len(), 1, "{on}");
    let original = std::fs::read_to_string(repo.0.join(consumer)).unwrap();
    let start = original.find("source::selected").unwrap();
    assert_eq!(
        edits[0]["range"],
        json!({"start_byte":start,"end_byte":start+16})
    );
    assert_eq!(edits[0]["original_text"], "source::selected");
    assert_eq!(edits[0]["replacement_text"], "crate::moved::selected");
    let rewrite = on["plan"]["rewrites"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["kind"] == "path" && r["target"]["anchor"]["path"] == consumer)
        .unwrap();
    assert!(rewrite["anchors"].as_array().unwrap().contains(&anchor(
        repo,
        consumer,
        "source::selected"
    )));
    for (path, text, line) in routes {
        let expected = anchor(repo, path, text);
        assert!(
            rewrite["anchors"].as_array().unwrap().contains(&expected),
            "{on}"
        );
        let source = std::fs::read_to_string(repo.0.join(path)).unwrap();
        let offset = expected["range"]["start_byte"].as_u64().unwrap() as usize;
        assert_eq!(
            source[..offset].bytes().filter(|b| *b == b'\n').count() + 1,
            *line
        );
    }
    let before = observe(&repo.0);
    let copy = move_artifacts::apply(repo, &on);
    assert_eq!(
        std::fs::read_to_string(copy.0.join(consumer)).unwrap(),
        original.replacen("source::selected", "crate::moved::selected", 1)
    );
    copy.write("Cargo.toml", "[package]\nname = \"fixture-corpus\"\nversion = \"0.0.0\"\nedition = \"2024\"\n[lib]\npath = \"cases/layout/lib.rs\"\n");
    let check = std::process::Command::new("cargo")
        .current_dir(&copy.0)
        .args(["check", "--locked", "--offline", "--quiet"])
        .output()
        .unwrap();
    assert!(
        check.status.success(),
        "{}",
        String::from_utf8_lossy(&check.stderr)
    );
    assert_eq!(observe(&repo.0), before);
    eprintln!(
        "written prefix repair applied and cargo check passed: {consumer}; routes={routes:?}"
    );
}

#[test]
fn glob_qualified_prefix_repairs_the_original_consumer_fixture() {
    let repo = fixture("pub(crate) fn selected() {}");
    repo.write("cases/layout/lib.rs", "mod source; mod consumer;");
    repo.write(
        "cases/layout/consumer.rs",
        "use super::*; #[cfg(all())] fn caller() { source::selected(); }",
    );
    assert_written_prefix_repair(
        &repo,
        "cases/layout/consumer.rs",
        &[
            ("cases/layout/lib.rs", "mod source;", 1),
            ("cases/layout/consumer.rs", "use super::*;", 1),
        ],
    );
}

#[test]
fn glob_qualified_prefix_repairs_imported_grouped_and_named_module_routes() {
    for (parent, bridge, consumer) in [
        ("use crate::source;", "", "use super::*;"),
        ("use crate::{source::{self}};", "", "use super::*;"),
        (
            "use crate::{source::{self, retained}};",
            "",
            "use super::*;",
        ),
        (
            "use crate::bridge::source;",
            "pub(crate) use crate::source;",
            "use super::*;",
        ),
        (
            "use crate::source;",
            "",
            "use crate::parent;\nuse parent::*;",
        ),
    ] {
        let repo = fixture("pub(crate) fn selected() {}");
        repo.write(
            "cases/layout/source.rs",
            "pub(crate) fn selected() {}\npub(crate) fn retained() {}\n",
        );
        repo.write(
            "cases/layout/lib.rs",
            "mod source;\nmod parent;\nmod bridge;\n",
        );
        repo.write(
            "cases/layout/parent.rs",
            &format!("{parent}\nmod consumer;\n"),
        );
        repo.write("cases/layout/bridge.rs", bridge);
        repo.write(
            "cases/layout/parent/consumer.rs",
            &format!("{consumer}\nfn caller() {{ source::selected(); }}\n"),
        );
        let glob = consumer.lines().last().unwrap();
        let mut routes = vec![
            ("cases/layout/lib.rs", "mod source;", 1),
            ("cases/layout/parent.rs", parent, 1),
            (
                "cases/layout/parent/consumer.rs",
                glob,
                consumer.lines().count(),
            ),
        ];
        if !bridge.is_empty() {
            routes.push(("cases/layout/bridge.rs", bridge, 1));
        }
        if consumer.contains("parent::*") {
            routes.push(("cases/layout/parent/consumer.rs", "use crate::parent;", 1));
        }
        assert_written_prefix_repair(&repo, "cases/layout/parent/consumer.rs", &routes);
    }
}

#[test]
fn glob_qualified_prefix_repairs_associated_function_consumers() {
    for attributes in ["", "#[cfg(all())]", "#[allow(dead_code)] #[cfg(all())]"] {
        let repo = fixture("pub(crate) fn selected() {}");
        repo.write("cases/layout/lib.rs", "mod source; mod consumer;");
        repo.write(
            "cases/layout/consumer.rs",
            &format!(
                "use super::*; struct Caller; impl Caller {{ {attributes} fn caller() {{ source::selected(); }} }}"
            ),
        );
        assert_written_prefix_repair(
            &repo,
            "cases/layout/consumer.rs",
            &[
                ("cases/layout/lib.rs", "mod source;", 1),
                ("cases/layout/consumer.rs", "use super::*;", 1),
            ],
        );
    }
}

#[test]
fn glob_qualified_prefix_refuses_unproved_enclosing_consumer_attributes() {
    for (label, consumer) in [
        (
            "inactive method cfg",
            "struct Caller; impl Caller { #[cfg(any())] fn caller() { source::selected(); } }",
        ),
        (
            "unknown method cfg",
            "struct Caller; impl Caller { #[cfg(unknown)] fn caller() { source::selected(); } }",
        ),
        (
            "replacing method attribute",
            "struct Caller; impl Caller { #[unknown] fn caller() { source::selected(); } }",
        ),
        (
            "method cfg_attr",
            "struct Caller; impl Caller { #[cfg_attr(all(), unknown)] fn caller() { source::selected(); } }",
        ),
        (
            "replacing method attribute before active cfg",
            "struct Caller; impl Caller { #[unknown] #[cfg(all())] fn caller() { source::selected(); } }",
        ),
        (
            "inactive method cfg before active cfg",
            "struct Caller; impl Caller { #[cfg(any())] #[cfg(all())] fn caller() { source::selected(); } }",
        ),
        (
            "active impl cfg",
            "struct Caller; #[cfg(all())] impl Caller { fn caller() { source::selected(); } }",
        ),
        (
            "unknown impl cfg",
            "struct Caller; #[cfg(unknown)] impl Caller { fn caller() { source::selected(); } }",
        ),
        (
            "replacing impl attribute",
            "struct Caller; #[unknown] impl Caller { fn caller() { source::selected(); } }",
        ),
        (
            "impl cfg_attr",
            "struct Caller; #[cfg_attr(all(), unknown)] impl Caller { fn caller() { source::selected(); } }",
        ),
        (
            "nested impl cfg",
            "fn outer() { struct Caller; #[cfg(all())] impl Caller { fn caller() { source::selected(); } } }",
        ),
        (
            "nested module cfg",
            "#[cfg(all())] mod nested { use super::*; fn caller() { source::selected(); } }",
        ),
        (
            "replacing closure attribute",
            "fn caller() { let call = #[unknown] || { source::selected(); }; }",
        ),
        (
            "impl inner attribute",
            "struct Caller; impl Caller { #![unknown] fn caller() { source::selected(); } }",
        ),
        (
            "outer function cfg around nested consumer",
            "#[cfg(all())] fn outer() { fn caller() { source::selected(); } }",
        ),
    ] {
        let repo = fixture("pub(crate) fn selected() {}");
        repo.write("cases/layout/lib.rs", "mod source; mod consumer;");
        repo.write(
            "cases/layout/consumer.rs",
            &format!("use super::*; {consumer}"),
        );
        let mut args = request(&repo, "pub(crate) fn selected() {}");
        // Keep cfg evidence fixed: lexical refusals disclose its consultation
        // independently of whether semantic resolution is enabled.
        args["semantic_configuration"] = config();
        assert_prefix_request_refused(&repo, "cases/layout/consumer.rs", label, args);
    }
}

fn assert_prefix_refused(repo: &Fixture, consumer: &str, label: &str) {
    assert_prefix_request_refused(
        repo,
        consumer,
        label,
        request(repo, "pub(crate) fn selected() {}"),
    );
}

fn assert_prefix_request_refused(repo: &Fixture, consumer: &str, label: &str, args: Value) {
    assert_default_off(repo, &args);
    let on = resolved(repo, args);
    assert!(on["error"].is_null(), "{label}: {on}");
    assert_eq!(on["plan"]["applicable"], false, "{label}: {on}");
    for field in ["edits", "created_files", "patch"] {
        assert!(on["plan"][field].is_null(), "{label}: {on}");
    }
    assert!(
        on["plan"]["decisions"].as_array().unwrap().iter().any(|d| {
            d["blocks_applicability"] == true
                && matches!(
                    d["reason"].as_str(),
                    Some(
                        "external_or_missing_binding"
                            | "lexical_context_unproved"
                            | "conditional_or_inherited_context"
                    )
                )
                && d["anchors"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|a| a["path"] == consumer)
        }),
        "{label}: {on}"
    );
    assert!(
        !on["plan"]["rewrites"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| { r["kind"] == "path" && r["target"]["anchor"]["path"] == consumer }),
        "refused route must not plan a consumer repair: {label}: {on}"
    );
    eprintln!("qualified prefix refused: {label}");
}

#[test]
fn glob_qualified_prefix_refusal_matrix() {
    for (label, parent, bridge, consumer) in [
        (
            "competing imports",
            "use crate::source; use crate::other::source;",
            "",
            "use super::*; fn caller() { source::selected(); }",
        ),
        (
            "duplicate aliases",
            "use crate::source; use crate::{source};",
            "",
            "use super::*; fn caller() { source::selected(); }",
        ),
        (
            "missing route",
            "use crate::absent::source;",
            "",
            "use super::*; fn caller() { source::selected(); }",
        ),
        (
            "conditional provider",
            "#[cfg(all())] use crate::source;",
            "",
            "use super::*; fn caller() { source::selected(); }",
        ),
        (
            "recursive named route",
            "use crate::bridge::source;",
            "pub(crate) use crate::parent::source;",
            "use super::*; fn caller() { source::selected(); }",
        ),
        (
            "glob forwarding",
            "use crate::bridge::*;",
            "pub(crate) use crate::source;",
            "use super::*; fn caller() { source::selected(); }",
        ),
        (
            "renamed import",
            "use crate::source as renamed;",
            "",
            "use super::*; fn caller() { renamed::selected(); }",
        ),
        (
            "same-spelling rename",
            "use crate::source as source;",
            "",
            "use super::*; fn caller() { source::selected(); }",
        ),
        (
            "renamed intermediate",
            "use crate::bridge::source;",
            "pub(crate) use crate::source as source;",
            "use super::*; fn caller() { source::selected(); }",
        ),
        (
            "competing consumer globs",
            "use crate::source;",
            "",
            "use super::*; use crate::other::*; fn caller() { source::selected(); }",
        ),
        (
            "competing provider glob",
            "use crate::source; use crate::other::*;",
            "",
            "use super::*; fn caller() { source::selected(); }",
        ),
        (
            "provider macro",
            "use crate::source; unknown!();",
            "",
            "use super::*; fn caller() { source::selected(); }",
        ),
        (
            "consumer macro",
            "use crate::source;",
            "",
            "use super::*; unknown!(); fn caller() { source::selected(); }",
        ),
        (
            "attribute provider",
            "use crate::source; #[unknown] struct Generated;",
            "",
            "use super::*; fn caller() { source::selected(); }",
        ),
        (
            "derive provider",
            "use crate::source; #[derive(Unknown)] struct Generated;",
            "",
            "use super::*; fn caller() { source::selected(); }",
        ),
        (
            "block-local glob",
            "use crate::source;",
            "",
            "use super::*; fn caller() { use crate::other::*; source::selected(); }",
        ),
        (
            "block-local shadow",
            "use crate::source;",
            "",
            "use super::*; fn caller() { use crate::absent as source; source::selected(); }",
        ),
        (
            "nonparent glob",
            "use crate::source;",
            "",
            "use crate::*; fn caller() { source::selected(); }",
        ),
        (
            "grouped glob",
            "use crate::source;",
            "",
            "use super::{*}; fn caller() { source::selected(); }",
        ),
        (
            "conditional glob",
            "use crate::source;",
            "",
            "#[cfg(all())] use super::*; fn caller() { source::selected(); }",
        ),
        (
            "unknown consumer cfg",
            "use crate::source;",
            "",
            "use super::*; #[cfg(unknown)] fn caller() { source::selected(); }",
        ),
        (
            "inactive consumer cfg",
            "use crate::source;",
            "",
            "use super::*; #[cfg(any())] fn caller() { source::selected(); }",
        ),
        (
            "other function cfg",
            "use crate::source;",
            "",
            "use super::*; #[cfg(all())] fn other() {} fn caller() { source::selected(); }",
        ),
        (
            "consumer cfg_attr",
            "use crate::source;",
            "",
            "use super::*; #[cfg_attr(all(), inline)] fn caller() { source::selected(); }",
        ),
        (
            "consumer replacing attribute",
            "use crate::source;",
            "",
            "use super::*; #[unknown] fn caller() { source::selected(); }",
        ),
    ] {
        let repo = fixture("pub(crate) fn selected() {}");
        repo.write(
            "cases/layout/lib.rs",
            "mod source; mod parent; mod bridge; mod other;",
        );
        repo.write(
            "cases/layout/parent.rs",
            &format!("{parent}\nmod consumer;\n"),
        );
        repo.write("cases/layout/bridge.rs", bridge);
        repo.write(
            "cases/layout/other.rs",
            "pub(crate) mod source { pub(crate) fn selected() {} }",
        );
        repo.write("cases/layout/parent/consumer.rs", consumer);
        assert_prefix_refused(&repo, "cases/layout/parent/consumer.rs", label);
    }
}

#[test]
fn glob_qualified_prefix_bounds_named_route_depth() {
    let repo = fixture("pub(crate) fn selected() {}");
    let mut root = String::from("mod source; mod parent;\n");
    for i in 0..18 {
        root.push_str(&format!("mod link{i};\n"));
        repo.write(
            &format!("cases/layout/link{i}.rs"),
            &if i == 17 {
                "pub(crate) use crate::source;".into()
            } else {
                format!("pub(crate) use crate::link{}::source;", i + 1)
            },
        );
    }
    repo.write("cases/layout/lib.rs", &root);
    repo.write(
        "cases/layout/parent.rs",
        "use crate::link0::source; mod consumer;",
    );
    repo.write(
        "cases/layout/parent/consumer.rs",
        "use super::*; fn caller() { source::selected(); }",
    );
    assert_prefix_refused(
        &repo,
        "cases/layout/parent/consumer.rs",
        "over-depth named route",
    );
}

#[test]
fn cross_file_missing_qualified_prefix_keeps_source_fact_refusal() {
    let repo = fixture("pub(crate) fn selected() {}");
    repo.write("cases/layout/lib.rs", "mod source; mod consumer;");
    repo.write(
        "cases/layout/consumer.rs",
        "use super::*; #[cfg(all())] fn caller() { absent::selected(); }",
    );
    assert_prefix_refused(&repo, "cases/layout/consumer.rs", "missing prefix");
    let on = resolved(&repo, request(&repo, "pub(crate) fn selected() {}"));
    assert_eq!(on["plan"]["integrity"]["semantic"], "resolution_performed");
    assert!(on["coverage"]["ra_resolved"].is_null(), "{on}");
    assert_eq!(on["plan"]["resolution_coverage"]["decisions"], 0, "{on}");
    assert!(on["plan"]["binding_proofs"].is_null(), "{on}");
    assert!(
        on["plan"]["decisions"].as_array().unwrap().iter().any(|d| {
            d["anchors"].as_array().unwrap().iter().any(|a| {
                a["path"] == "cases/layout/consumer.rs" && a["expected_text"] == "selected"
            }) && d["refusal_basis"]
                .as_array()
                .unwrap()
                .iter()
                .any(|b| b["class"] == "semantic_source_fact_unproved")
        }),
        "{on}"
    );
}

#[test]
fn macro_free_batch_resolves_inherent_constructor_method_and_field_with_full_evidence() {
    let text = "fn selected(value: crate::Value) -> u32 { let constructed = crate::Value::new(); value.read() + constructed.number }";
    let repo = fixture(text);
    let args = request(&repo, text);
    let off = run(&repo, args.clone());
    assert!(!off["plan"]["applicable"].as_bool().unwrap());
    assert!(reason(&off, "member_or_constructor_unproved"), "{off}");
    let mut explicit_off = args.clone();
    explicit_off["resolve_semantic"] = json!(false);
    explicit_off["semantic_configuration"] = config();
    assert_eq!(
        serde_json::to_vec(&off).unwrap(),
        serde_json::to_vec(&run(&repo, explicit_off)).unwrap()
    );
    let on = resolved(&repo, args);
    assert_eq!(on["plan"]["applicable"], true, "{on}");
    assert_eq!(on["plan"]["integrity"]["semantic"], "resolution_performed");
    let proofs = on["plan"]["binding_proofs"].as_array().unwrap();
    assert!(proofs.len() >= 2, "{on}");
    assert_eq!(on["coverage"]["ra_resolved"], proofs.len());
    for p in proofs {
        assert_eq!(p["class"], "ra_resolved");
        assert_eq!(p["source_access"], true);
        assert_eq!(p["final_access"], true);
        assert!(p["declaration"]["anchor"]["expected_text"].is_string());
        assert_eq!(p["declaration"]["crate_origin"], "fixture");
        assert!(p["original_receiver"].is_object());
        assert!(p["adjusted_receiver"].is_object());
        assert!(
            p["coverage"]["semantic_input_digest"]
                .as_str()
                .unwrap()
                .starts_with("sha1:")
        );
        assert!(p["coverage"]["final_overlay_digest"].is_string());
        assert_eq!(p["coverage"]["configuration"], config());
        assert_eq!(p["final_anchor"]["path"], "cases/layout/moved.rs");
    }
    assert!(
        on["plan"]["resolution_coverage"]["statement"]
            .as_str()
            .unwrap()
            .contains("compilation/equivalence not performed")
    );
}

#[test]
fn explicit_dependency_graph_discharges_external_bindings_and_inherent_calls() {
    let text = "fn selected(é: external::Value) -> u32 { let made = external::Value::new(); é.read() + made.number }";
    let repo = fixture(text);
    repo.write(
        "cases/layout/external.rs",
        "pub struct Value { pub number: u32 } impl Value { pub fn read(&self) -> u32 { self.number } pub fn new() -> Value { Value { number: 7 } } }",
    );
    let args = request(&repo, text);
    let off = run(&repo, args.clone());
    assert!(reason(&off, "external_or_missing_binding"), "{off}");
    assert!(
        off["plan"]["decisions"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|d| d["blocks_applicability"] == true)
            .all(|d| matches!(
                d["reason"].as_str(),
                Some("external_or_missing_binding" | "member_or_constructor_unproved")
            )),
        "{off}"
    );
    let mut args = args;
    args["resolve_semantic"] = json!(true);
    let mut config = config();
    config["crates"][0]["dependencies"] = json!([{"name":"external","crate_name":"external"}]);
    config["crates"].as_array_mut().unwrap().push(json!({"name":"external","root_file":"cases/layout/external.rs","edition":"2024","features":[],"cfg":[],"dependencies":[]}));
    args["semantic_configuration"] = config;
    let on = run(&repo, args);
    assert_eq!(on["plan"]["applicable"], true, "{on}");
    let proofs = on["plan"]["binding_proofs"].as_array().unwrap();
    assert!(proofs.len() >= 2, "{on}");
    assert!(
        proofs
            .iter()
            .all(|p| p["declaration"]["crate_origin"] == "external")
    );
    assert!(
        proofs
            .iter()
            .any(|p| p["classification"] == "inherent_function")
    );
    assert!(
        proofs
            .iter()
            .any(|p| p["classification"] == "type_or_constructor")
    );
    // Apply only to a disposable test copy; the analysis itself stayed read-only.
    let copy = move_artifacts::apply(&repo, &on);
    assert!(copy.0.join("cases/layout/moved.rs").exists());
}

#[test]
fn unrelated_macro_body_and_proc_macro_registration_do_not_key_on_crate_kind() {
    let text = "fn selected(value: crate::Value) -> u32 { value.read() }";
    let repo = fixture(text);
    repo.write(
        "cases/layout/source.rs",
        &format!("#[proc_macro]\nfn registration() {{ unknown!(); }}\n{text}"),
    );
    let on = resolved(&repo, request(&repo, text));
    assert_eq!(on["plan"]["applicable"], true, "{on}");
}

#[test]
fn inaccessible_private_method_remains_a_named_blocker() {
    let text = "fn selected(value: Value) -> u32 { value.read() }";
    let repo = fixture(text);
    repo.write(
        "cases/layout/source.rs",
        &format!("pub struct Value; impl Value {{ fn read(&self) -> u32 {{ 7 }} }}\n{text}"),
    );
    let value = resolved(&repo, request(&repo, text));
    assert_eq!(value["plan"]["applicable"], false, "{value}");
    assert!(reason(&value, "member_or_constructor_unproved"), "{value}");
    assert!(value["plan"]["patch"].is_null());
}

fn assert_anchored_access_refusal(value: &Value) {
    assert_eq!(value["plan"]["applicable"], false, "{value}");
    for field in ["edits", "created_files", "patch"] {
        assert!(value["plan"][field].is_null(), "{value}");
    }
    assert!(
        value["plan"]["decisions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| {
                d["reason"] == "member_or_constructor_unproved"
                    && d["blocks_applicability"] == true
                    && d["anchors"].as_array().unwrap().iter().any(|a| {
                        a["path"] == "cases/layout/source.rs"
                            && !a["expected_text"].as_str().unwrap().is_empty()
                    })
            }),
        "{value}"
    );
}

#[test]
fn private_field_access_is_not_unlocked_by_a_type_visibility_explanation() {
    let text = "fn selected(value: Value) -> u32 { value.number }";
    let repo = fixture(text);
    repo.write(
        "cases/layout/source.rs",
        &format!("struct Value {{ number: u32 }}\n{text}"),
    );
    let value = resolved(&repo, request(&repo, text));
    assert_anchored_access_refusal(&value);
}

#[test]
fn generic_and_trait_object_receivers_refuse_without_candidate_shortcuts() {
    for text in [
        "fn selected<T: crate::Read>(value: T) -> u32 { value.read() }",
        "fn selected(value: &dyn crate::Read) -> u32 { value.read() }",
        "fn selected(value: crate::Value) -> u32 { value.trait_read() }",
        "fn selected() { let value = missing(); value.read(); }",
    ] {
        let repo = fixture(text);
        repo.write("cases/layout/lib.rs", "mod source; pub trait Read { fn read(&self) -> u32; fn trait_read(&self) -> u32; } pub struct Value; impl Read for Value { fn read(&self) -> u32 { 1 } fn trait_read(&self) -> u32 { 1 } }");
        let value = resolved(&repo, request(&repo, text));
        assert_anchored_access_refusal(&value);
    }
}

#[test]
fn final_overlay_not_original_scope_controls_resolution() {
    let text = "fn selected(value: crate::Value) -> u32 { value.read() }";
    let repo = fixture(text);
    let first = resolved(&repo, request(&repo, text));
    assert_eq!(first["plan"]["applicable"], true, "{first}");
    let target = first["plan"]["rewrites"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["kind"] == "module_declaration")
        .unwrap()["target"]
        .clone();
    let mut args = request(&repo, text);
    args["rewrite_overrides"] = json!([{"target":target,"action":"retain"}]);
    let changed = resolved(&repo, args);
    assert_eq!(changed["plan"]["applicable"], false, "{changed}");
    assert!(
        reason(&changed, "member_or_constructor_unproved"),
        "{changed}"
    );
    assert!(changed["plan"]["patch"].is_null());
}

#[test]
fn inaccessible_constructor_fields_and_recovered_dependency_text_refuse() {
    let text = "fn selected() -> crate::source::Value { crate::source::Value { number: 7 } }";
    let repo = fixture(text);
    repo.write(
        "cases/layout/source.rs",
        &format!("pub struct Value {{ number: u32 }}\n{text}"),
    );
    let value = resolved(&repo, request(&repo, text));
    assert_anchored_access_refusal(&value);
    let text = "fn selected(value: crate::Value) -> u32 { value.read() }";
    let repo = fixture(text);
    repo.write("cases/layout/lib.rs", "mod source; pub struct Value; impl Value { pub fn read(&self) -> u32 { 7 } } fn recovered() { @ }");
    let value = resolved(&repo, request(&repo, text));
    assert_eq!(value["plan"]["applicable"], false, "{value}");
    assert!(reason(&value, "member_or_constructor_unproved"), "{value}");
}

#[test]
fn oversized_resolution_audit_withholds_artifacts_and_counts_omitted_evidence() {
    let text = "fn selected(value: crate::Value) -> u32 { value.read() }";
    let repo = fixture(text);
    let mut args = request(&repo, text);
    args["resolve_semantic"] = json!(true);
    args["limits"]["response_bytes"] = json!(65536);
    let mut configuration = config();
    configuration["crates"][0]["features"] = json!(
        (0..256)
            .map(|i| format!("{i}{}", "x".repeat(240)))
            .collect::<Vec<_>>()
    );
    args["semantic_configuration"] = configuration;
    let result = Engine::new(repo.0.clone()).unwrap().move_item(
        serde_json::from_value(args).unwrap(),
        &AtomicBool::new(false),
    );
    assert!(result.wire_bytes() <= result.limits.response_bytes);
    let value = serde_json::to_value(result).unwrap();
    assert_eq!(value["plan"]["applicable"], false, "{value}");
    assert_eq!(value["coverage"]["ra_resolved"], 1);
    assert_eq!(value["counts"]["omissions"]["binding_proofs"], 1);
    assert_eq!(value["counts"]["omissions"]["resolution_coverage"], 1);
    assert!(value["plan"]["patch"].is_null());
}

#[test]
fn configuration_changes_fingerprint_and_missing_graph_keeps_blockers() {
    let text = "fn selected(value: crate::Value) -> u32 { value.read() }";
    let repo = fixture(text);
    let mut args = request(&repo, text);
    args["resolve_semantic"] = json!(true);
    let missing = run(&repo, args.clone());
    assert!(reason(&missing, "member_or_constructor_unproved"));
    assert_eq!(missing["plan"]["integrity"]["semantic"], "not_performed");
    args["semantic_configuration"] = config();
    let before = run(&repo, args.clone());
    args["semantic_configuration"]["crates"][0]["features"] = json!(["extra"]);
    let after = run(&repo, args.clone());
    assert_eq!(after["plan"]["applicable"], true, "{after}");
    assert_eq!(before["snapshot_id"], after["snapshot_id"]);
    assert_ne!(
        before["plan"]["resolution_coverage"]["semantic_input_digest"],
        after["plan"]["resolution_coverage"]["semantic_input_digest"]
    );
    args["semantic_configuration"]["crates"][0]["root_file"] = json!("cases/layout/missing.rs");
    let missing = run(&repo, args);
    assert_eq!(
        missing["error"]["field"],
        "semantic_configuration.crates[0].root_file"
    );
}

#[test]
fn frozen_serde_helper_bodies_stay_blocked_after_inert_chain_metadata_admission() {
    let frozen: Value =
        serde_json::from_str(include_str!("fixtures/replay-corpus/serde-attr-lit-2.json")).unwrap();
    let repo = Fixture::generate();
    // Recreate the falsification boundary using the frozen, exact helper bodies;
    // the external checkout is exercised separately by the read-only replay rig.
    repo.write(
        "serde_derive/src/lib.rs",
        "#![cfg_attr(not(check_cfg), allow(unexpected_cfgs))]\nmod internals;\n",
    );
    repo.write("serde_derive/src/internals/mod.rs", "mod attr;\n");
    let texts: Vec<_> = frozen["request"]["moves"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["item"]["expected_text"].as_str().unwrap())
        .collect();
    repo.write("serde_derive/src/internals/attr.rs", &texts.join("\n\n"));
    let mut args = frozen["request"].clone();
    args["repo_path"] = json!(repo.0);
    args["resolve_semantic"] = json!(true);
    let mut configuration = config();
    configuration["crates"][0]["root_file"] = args["crate_root"].clone();
    args["semantic_configuration"] = configuration;
    for m in args["moves"].as_array_mut().unwrap() {
        m["item"] = anchor(
            &repo,
            "serde_derive/src/internals/attr.rs",
            m["item"]["expected_text"].as_str().unwrap(),
        );
    }
    let value = run(&repo, args);
    assert_eq!(value["plan"]["applicable"], false, "{value}");
    assert_eq!(value["plan"]["chain_diagnostics"], json!([]), "{value}");
    assert!(reason(&value, "macro_context_unexamined"));
    assert!(reason(&value, "member_or_constructor_unproved"));
    assert!(value["plan"]["patch"].is_null());
}

#[test]
fn macro_touching_item_keeps_its_typed_refusal() {
    let text = "fn selected(value: crate::Value) -> u32 { unknown!(value.read()) }";
    let repo = fixture(text);
    let value = resolved(&repo, request(&repo, text));
    assert_eq!(value["plan"]["applicable"], false, "{value}");
    assert!(reason(&value, "macro_context_unexamined"), "{value}");
}
