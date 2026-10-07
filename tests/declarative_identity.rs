#[allow(dead_code)]
#[path = "support/fixture_gen.rs"]
mod fixture_gen;
#[allow(dead_code)]
#[path = "support/move_artifacts.rs"]
mod move_artifacts;
use fixture_gen::{Fixture, observe};
use rust_sitter_mcp::{engine::Engine, move_plan::MoveRequest};
use serde_json::{Value, json};
use std::sync::atomic::AtomicBool;

const WRAPPER: &str = "macro_rules! uuid_id { ($name:ident) => { #[derive(Clone, Copy, Debug, Deserialize, Serialize)] #[serde(transparent)] pub struct $name(Opaque); impl $name { pub fn new() -> Self { loop {} } pub fn read(&self) -> u32 { 1 } } }; }";
const SELECTED: &str = "fn selected(item: crate::route::ItemId, dispatch: crate::route::DispatchId) -> crate::route::ItemId { item }";
fn fixture(definition: &str, text: &str) -> Fixture {
    let repo = Fixture::generate();
    repo.write(
        "cases/layout/lib.rs",
        "mod source; mod model; pub mod route;",
    );
    repo.write(
        "cases/layout/model.rs",
        &format!("{definition}\nuuid_id!(ItemId); uuid_id!(DispatchId);"),
    );
    repo.write(
        "cases/layout/route.rs",
        "pub use crate::model::{ItemId, DispatchId};",
    );
    repo.write("cases/layout/source.rs", text);
    repo
}
fn request(repo: &Fixture, text: &str) -> Value {
    json!({"repo_path":repo.0,"crate_root":"cases/layout/lib.rs","paths":["cases/layout"],"limits":{"time_budget_ms":30000,"diagnostic_count":1000},"resolve_semantic":true,
        "semantic_configuration":{"crates":[{"name":"fixture","root_file":"cases/layout/lib.rs","edition":"2024","features":[],"cfg":[{"key":"declared","value":null}],"dependencies":[]}]},
        "moves":[{"item":move_artifacts::anchor(repo,"cases/layout/source.rs",text),"destination":{"kind":"new_sibling","path":"cases/layout/moved.rs","parent_path":"cases/layout/lib.rs"}}]})
}
fn run(repo: &Fixture, args: Value) -> Value {
    let before = observe(&repo.0);
    let result = Engine::new(repo.0.clone()).unwrap().move_item(
        serde_json::from_value::<MoveRequest>(args).unwrap(),
        &AtomicBool::new(false),
    );
    assert_eq!(observe(&repo.0), before, "analysis modified admitted bytes");
    serde_json::to_value(result).unwrap()
}
fn blocked(value: &Value) {
    assert_eq!(value["plan"]["applicable"], false, "{value}");
    for key in ["edits", "created_files", "patch"] {
        assert!(value["plan"][key].is_null(), "{value}");
    }
}

#[test]
fn wrapper_signature_identity_has_both_written_pairs_without_generated_field_inference() {
    let repo = fixture(WRAPPER, SELECTED);
    let mut off = request(&repo, SELECTED);
    off["resolve_semantic"] = json!(false);
    blocked(&run(&repo, off));
    let value = run(&repo, request(&repo, SELECTED));
    assert_eq!(value["plan"]["applicable"], true, "{value}");
    let proofs: Vec<_> = value["plan"]["binding_proofs"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|p| p["class"] == "declarative_macro_identity")
        .collect();
    assert_eq!(proofs.len(), 3, "{value}");
    for proof in proofs {
        assert_eq!(proof["classification"], "declaration_identity");
        assert!(
            proof["basis"]
                .as_str()
                .unwrap()
                .contains("bounded declarative-macro expansion of written tokens")
        );
        for key in [
            "original_receiver",
            "adjusted_receiver",
            "final_original_receiver",
            "final_adjusted_receiver",
        ] {
            assert!(proof[key].is_null());
        }
        for key in ["declaration", "final_declaration"] {
            assert_eq!(
                proof[key]["declarative_macro"]["definition"]["expected_text"],
                WRAPPER
            );
            let name = proof[key]["anchor"]["expected_text"].as_str().unwrap();
            assert_eq!(
                proof[key]["declarative_macro"]["invocation"]["expected_text"],
                format!("uuid_id!({name});")
            );
        }
    }
    let copy = move_artifacts::apply(&repo, &value);
    assert!(
        std::fs::read_to_string(copy.0.join("cases/layout/moved.rs"))
            .unwrap()
            .contains(SELECTED)
    );
    let evaluations = value["plan"]["resolution_coverage"]["context_evaluations"]
        .as_array()
        .unwrap();
    for revision in ["original", "final"] {
        assert!(evaluations.iter().any(|e| e["revision"] == revision
            && e["kind"] == "declarative_macro_definition"
            && e["anchor"]["expected_text"] == WRAPPER));
    }
}

#[test]
fn explicit_imports_preserve_bare_signature_identities_in_the_final_sibling() {
    let text = "fn selected(item: ItemId, dispatch: DispatchId) -> ItemId { item }";
    let repo = fixture(WRAPPER, text);
    repo.write(
        "cases/layout/source.rs",
        &format!("use crate::route::{{ItemId, DispatchId}}; {text}"),
    );
    let value = run(&repo, request(&repo, text));
    assert_eq!(value["plan"]["applicable"], true, "{value}");
    assert!(
        value["plan"]["rewrites"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["kind"] == "import_insert"
                && r["rationale"]
                    .as_str()
                    .unwrap()
                    .contains("pending both-overlay declaration identity"))
    );
}

#[test]
fn unproved_or_conditioned_facade_keeps_original_reference_coordinates_and_artifact_veto() {
    let text = "fn selected(item: ItemId) {}";
    for route in [
        "pub use crate::route::ItemId;",
        "#[cfg(declared)] pub use crate::model::ItemId;",
    ] {
        let repo = fixture(WRAPPER, text);
        repo.write(
            "cases/layout/source.rs",
            &format!(
                "// {}\nuse crate::route::ItemId; {text}",
                "padding".repeat(30)
            ),
        );
        repo.write("cases/layout/route.rs", route);
        let value = run(&repo, request(&repo, text));
        blocked(&value);
        assert!(
            value["plan"]["binding_proofs"]
                .as_array()
                .into_iter()
                .flatten()
                .all(|p| p["class"] != "declarative_macro_identity")
        );
        if !route.starts_with("#[") {
            assert!(
                value["plan"]["decisions"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|d| d["anchors"][0]["path"] == "cases/layout/source.rs"
                        && d["anchors"][0]["expected_text"] == "ItemId"),
                "{value}"
            );
        }
    }
}

#[test]
fn conditional_complex_recursive_and_namespace_producing_expansions_disclose_refusal() {
    let huge = format!(
        "macro_rules! uuid_id {{ ($name:ident) => {{ pub struct $name {{ {} }} }}; }}",
        "pub field: u32,".repeat(1000)
    );
    let nested = format!(
        "macro_rules! uuid_id {{ ($name:ident) => {{ pub struct $name; impl $name {{ fn nesting() {{ let _ = {}0{}; }} }} }}; }}",
        "(".repeat(33),
        ")".repeat(33)
    );
    for (definition, reason) in [
        (
            format!("#[cfg(declared)] {WRAPPER}"),
            "declarative_conditional_context",
        ),
        (
            format!("#[cfg_attr(any(), allow(dead_code))] {WRAPPER}"),
            "declarative_conditional_context",
        ),
        (
            "macro_rules! uuid_id { ($($name:ident),*) => { $(pub struct $name;)* }; }".into(),
            "declarative_fragment_limit",
        ),
        (
            "macro_rules! uuid_id { ($name:ident) => { uuid_id!($name); }; }".into(),
            "declarative_recursion_limit",
        ),
        (
            "macro_rules! uuid_id { ($name:ident) => { pub struct $name; use other::*; }; }".into(),
            "declarative_output_unproved",
        ),
        (
            "macro_rules! uuid_id { ($name:ident) => { #[custom] pub struct $name; }; }".into(),
            "declarative_output_unproved",
        ),
        (huge, "declarative_token_limit"),
        (nested, "declarative_nesting_limit"),
    ] {
        let repo = fixture(&definition, SELECTED);
        let value = run(&repo, request(&repo, SELECTED));
        blocked(&value);
        assert!(
            value["plan"]["binding_proofs"]
                .as_array()
                .into_iter()
                .flatten()
                .all(|p| p["class"] != "declarative_macro_identity"),
            "{value}"
        );
        assert!(
            value["plan"]["resolution_coverage"]["context_evaluations"]
                .as_array()
                .unwrap()
                .iter()
                .any(|e| e["status"] == "skipped"
                    && e["reason"] == reason
                    && e["anchor"]["expected_text"]
                        .as_str()
                        .unwrap()
                        .starts_with("uuid_id!")),
            "{reason}: {value}"
        );
    }
}

#[test]
fn generated_members_constructors_and_variant_facts_remain_blocked() {
    for (definition, text) in [
        (
            WRAPPER,
            "fn selected(value: crate::route::ItemId) -> u32 { value.read() }",
        ),
        (
            WRAPPER,
            "fn selected() { let _ = crate::route::ItemId::new(); }",
        ),
        (
            "macro_rules! uuid_id { ($name:ident) => { pub struct $name { pub number: u32 } }; }",
            "fn selected(value: crate::route::ItemId) -> u32 { value.number }",
        ),
        (
            "macro_rules! uuid_id { ($name:ident) => { pub struct $name; }; }",
            "fn selected() { let _ = crate::route::ItemId; }",
        ),
        (
            "macro_rules! uuid_id { ($name:ident) => { pub enum $name { Unit } }; }",
            "fn selected() { let _ = crate::route::ItemId::Unit; }",
        ),
    ] {
        let repo = fixture(definition, text);
        let value = run(&repo, request(&repo, text));
        blocked(&value);
        assert!(
            value["plan"]["binding_proofs"]
                .as_array()
                .into_iter()
                .flatten()
                .all(|p| p["classification"] == "declaration_identity"),
            "{value}"
        );
    }
}

#[test]
fn procedural_and_unresolved_macro_generated_names_never_have_identity_evidence() {
    for definition in [
        "",
        "#[proc_macro] pub fn uuid_id(input: TokenStream) -> TokenStream { input }",
    ] {
        let repo = fixture(definition, SELECTED);
        let value = run(&repo, request(&repo, SELECTED));
        blocked(&value);
        assert!(
            value["plan"]["binding_proofs"]
                .as_array()
                .into_iter()
                .flatten()
                .next()
                .is_none(),
            "{value}"
        );
    }
}
