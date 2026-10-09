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
    assert!(reason(&missing, "member_or_constructor_unproved"));
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
