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

const WRAPPER: &str = "macro_rules! uuid_id { ($name:ident) => { #[derive(Clone, Copy, Debug)] pub struct $name(Opaque); impl $name { pub fn new() -> Self { loop {} } pub fn read(&self) -> u32 { 1 } } }; }";
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
fn nine_helper_free_wrapper_occurrences_still_have_both_overlay_identities() {
    let params = (0..9)
        .map(|i| format!("p{i}: crate::route::ItemId"))
        .collect::<Vec<_>>()
        .join(", ");
    let text = format!("fn selected({params}) {{}}");
    let repo = fixture(WRAPPER, &text);
    let value = run(&repo, request(&repo, &text));
    assert_eq!(value["plan"]["applicable"], true, "{value}");
    let proofs: Vec<_> = value["plan"]["binding_proofs"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|p| p["class"] == "declarative_macro_identity")
        .collect();
    assert_eq!(proofs.len(), 9, "{value}");
    assert!(
        proofs
            .iter()
            .all(|p| p["declaration"]["declarative_macro"].is_object()
                && p["final_declaration"]["declarative_macro"].is_object())
    );
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
            "declarative_attribute_provider_uncertain",
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
fn nested_expansion_discloses_the_outer_written_depth_cap_without_identity() {
    let text = "fn selected(item: crate::route::ItemId) -> crate::route::ItemId { item }";
    let inner = "macro_rules! inner { ($name:ident) => { pub struct $name; }; }";
    for (outer, nested) in [
        (
            "macro_rules! make { ($name:ident) => { inner!($name); }; }",
            true,
        ),
        (
            "macro_rules! make { ($name:ident) => { pub struct $name; }; }",
            false,
        ),
    ] {
        let repo = fixture("", text);
        repo.write(
            "cases/layout/model.rs",
            &format!("{inner}\n{outer}\nmake!(ItemId);"),
        );
        repo.write("cases/layout/route.rs", "pub use crate::model::ItemId;");
        let value = run(&repo, request(&repo, text));
        let proofs: Vec<_> = value["plan"]["binding_proofs"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|p| p["class"] == "declarative_macro_identity")
            .collect();
        if !nested {
            assert_eq!(value["plan"]["applicable"], true, "{value}");
            assert_eq!(proofs.len(), 2, "{value}");
            continue;
        }
        blocked(&value);
        assert!(proofs.is_empty(), "{value}");
        assert!(
            value["plan"]["decisions"]
                .as_array()
                .unwrap()
                .iter()
                .flat_map(|d| d["refusal_basis"].as_array().into_iter().flatten())
                .any(|b| b["class"] == "semantic_source_fact_unproved"),
            "{value}"
        );
        let evaluations = value["plan"]["resolution_coverage"]["context_evaluations"]
            .as_array()
            .unwrap();
        for (kind, text) in [
            ("declarative_macro", "make!(ItemId);"),
            ("declarative_macro_definition", outer),
        ] {
            let anchor = move_artifacts::anchor(&repo, "cases/layout/model.rs", text);
            assert!(
                evaluations.iter().any(|e| e["revision"] == "original"
                    && e["kind"] == kind
                    && e["fact_class"] == "nominal_identity"
                    && e["status"] == "skipped"
                    && e["reason"] == "declarative_recursion_limit"
                    && e["anchor"] == anchor),
                "{value}"
            );
        }
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
fn provider_uncertain_attributes_refuse_with_written_definition_and_named_disclosure() {
    for attrs in [
        "#[derive(Serialize)] #[serde(transparent)]",
        "#[serde(transparent)] #[derive(Serialize)]",
        "#[derive(Clone)] #[serde(transparent)]",
        "#[derive(Serialize, Deserialize)] #[serde(transparent)]",
        "#[derive(Serialize)] #[provider::serde(transparent)]",
        "#[serde = \"transparent\"]",
        "#[helper]",
        "#[derive(Serialize)]",
        "#[derive(provider::Clone)]",
    ] {
        let definition = format!(
            "macro_rules! uuid_id {{ ($name:ident) => {{ {attrs} pub struct $name(Opaque); }}; }}"
        );
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
        let evaluations = value["plan"]["resolution_coverage"]["context_evaluations"]
            .as_array()
            .unwrap();
        let anchor = move_artifacts::anchor(&repo, "cases/layout/model.rs", &definition);
        let name = if attrs.contains("serde") {
            "serde"
        } else if attrs.contains("helper") {
            "helper"
        } else {
            "derive"
        };
        assert!(
            evaluations.iter().any(|e| e["revision"] == "original"
                && e["kind"] == "declarative_macro_definition"
                && e["status"] == "skipped"
                && e["reason"] == "declarative_attribute_provider_uncertain"
                && e["anchor"] == anchor
                && e["basis"]
                    .as_str()
                    .unwrap()
                    .contains("provider-uncertain attribute")
                && e["basis"].as_str().unwrap().contains(name)),
            "{value}"
        );
        assert!(
            evaluations.iter().any(|e| e["kind"] == "declarative_macro"
                && e["reason"] == "declarative_attribute_provider_uncertain"),
            "{value}"
        );
    }
}

#[test]
fn helper_attributes_on_written_owners_and_shadowed_builtin_derives_refuse() {
    let text = "fn selected(value: crate::route::ItemId) -> crate::route::ItemId { value }";
    for owner in [
        "definition",
        "invocation",
        "module",
        "crate",
        "shadowed_derive",
    ] {
        let repo = fixture(WRAPPER, text);
        match owner {
            "definition" => repo.write("cases/layout/model.rs", &format!("#[helper] {WRAPPER} uuid_id!(ItemId); uuid_id!(DispatchId);")),
            "invocation" => repo.write("cases/layout/model.rs", &format!("{WRAPPER} #[helper] uuid_id!(ItemId); uuid_id!(DispatchId);")),
            "module" => repo.write("cases/layout/lib.rs", "mod source; #[helper] mod model; pub mod route;"),
            "crate" => repo.write("cases/layout/lib.rs", "#![helper] mod source; mod model; pub mod route;"),
            "shadowed_derive" => repo.write("cases/layout/model.rs", &format!("macro_rules! Clone {{ () => {{}}; }} {WRAPPER} uuid_id!(ItemId); uuid_id!(DispatchId);")),
            _ => unreachable!(),
        }
        let value = run(&repo, request(&repo, text));
        blocked(&value);
        assert!(
            value["plan"]["binding_proofs"]
                .as_array()
                .into_iter()
                .flatten()
                .all(|p| p["class"] != "declarative_macro_identity"),
            "{owner}: {value}"
        );
        if owner == "crate" {
            // The ordinary chain gate refuses before semantic admission is reached.
            assert!(
                value["plan"]["decisions"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|d| d["anchors"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|a| a["expected_text"] == "#![helper]")),
                "{value}"
            );
            continue;
        }
        assert!(
            value["plan"]["resolution_coverage"]["context_evaluations"]
                .as_array()
                .unwrap()
                .iter()
                .any(|e| e["status"] == "skipped"
                    && (e["reason"] == "declarative_attribute_provider_uncertain"
                        || e["reason"] == "unsupported_attribute")),
            "{owner}: {value}"
        );
    }
}

const HELPERS: &str = "macro_rules! uuid_id { ($name:ident) => { #[derive(Serialize, Deserialize)] #[serde(transparent)] pub struct $name(Opaque); impl $name { pub fn new() -> Self { loop {} } pub fn read(&self) -> u32 { 1 } } }; }";

fn assumed_request(repo: &Fixture, text: &str) -> Value {
    let mut args = request(repo, text);
    args["assume_declared_helpers"] = json!(true);
    args
}

#[test]
fn declared_helpers_are_conditional_identities_with_separate_coverage_and_exact_off_behavior() {
    for attrs in [
        "#[derive(Serialize, Deserialize)] #[serde(transparent)]",
        "#[serde(transparent)] #[derive(Serialize, Deserialize)]",
        "#[helper]",
        "#[provider::helper(value)] #[derive(provider::Serialize)]",
    ] {
        let definition = HELPERS.replace(
            "#[derive(Serialize, Deserialize)] #[serde(transparent)]",
            attrs,
        );
        let text = "fn selected(item: ItemId, dispatch: DispatchId) -> ItemId { item }";
        let repo = fixture(&definition, text);
        repo.write(
            "cases/layout/source.rs",
            &format!("use crate::route::{{ItemId, DispatchId}}; {text}"),
        );
        let omitted = run(&repo, request(&repo, text));
        let mut off = request(&repo, text);
        off["assume_declared_helpers"] = json!(false);
        assert_eq!(
            serde_json::to_string(&omitted).unwrap(),
            serde_json::to_string(&run(&repo, off)).unwrap()
        );
        blocked(&omitted);
        let value = run(&repo, assumed_request(&repo, text));
        assert_eq!(value["plan"]["applicable"], true, "{value}");
        assert_eq!(value["coverage"]["assumed_declared_identity"], 3);
        assert!(value["coverage"].get("ra_resolved").is_none());
        assert_eq!(value["plan"]["resolution_coverage"]["decisions"], 3);
        let proofs = value["plan"]["binding_proofs"].as_array().unwrap();
        assert_eq!(proofs.len(), 3);
        for proof in proofs {
            assert_eq!(proof["class"], "assumed_declared_identity");
            assert_eq!(proof["classification"], "declaration_identity");
            let basis = proof["basis"].as_str().unwrap();
            for statement in [
                "registered derive helper",
                "identity assumed with the type",
                "not engine-classified",
                "no procedural expansion, macro hygiene",
                "compilation or equivalence claims",
            ] {
                assert!(basis.contains(statement), "{proof}");
            }
            for key in ["declaration", "final_declaration"] {
                assert_eq!(
                    proof[key]["declarative_macro"]["definition"]["expected_text"],
                    definition
                );
                let name = proof[key]["anchor"]["expected_text"].as_str().unwrap();
                assert_eq!(
                    proof[key]["declarative_macro"]["invocation"]["expected_text"],
                    format!("uuid_id!({name});")
                );
            }
            assert!(proof["original_receiver"].is_null());
            assert!(proof["final_original_receiver"].is_null());
        }
        let evaluations = value["plan"]["resolution_coverage"]["context_evaluations"]
            .as_array()
            .unwrap();
        for revision in ["original", "final"] {
            assert!(evaluations.iter().any(|e| {
                e["revision"] == revision
                    && e["kind"] == "declarative_macro_definition"
                    && e["reason"] == "assumed_declared_helpers"
                    && e["basis"]
                        .as_str()
                        .unwrap()
                        .contains("caller-assumed helper")
            }));
        }
        move_artifacts::apply(&repo, &value);
    }
}

#[test]
fn nine_helper_bearing_id_signature_occurrences_discharge_only_with_the_explicit_assumption() {
    let params = (0..9)
        .map(|i| {
            format!(
                "p{i}: crate::route::{}",
                if i % 2 == 0 { "ItemId" } else { "DispatchId" }
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    let text = format!("fn selected({params}) {{}}");
    let repo = fixture(HELPERS, &text);
    blocked(&run(&repo, request(&repo, &text)));
    let value = run(&repo, assumed_request(&repo, &text));
    assert_eq!(value["plan"]["applicable"], true, "{value}");
    assert_eq!(value["coverage"]["assumed_declared_identity"], 9);
    assert!(value["coverage"].get("ra_resolved").is_none());
    assert!(
        value["plan"]["binding_proofs"]
            .as_array()
            .unwrap()
            .iter()
            .all(|p| p["class"] == "assumed_declared_identity"
                && p["declaration"]["declarative_macro"].is_object()
                && p["final_declaration"]["declarative_macro"].is_object())
    );
    move_artifacts::apply(&repo, &value);
}

#[test]
fn assumption_counts_do_not_inflate_real_proofs_and_survive_diagnostic_omission() {
    let text = "fn selected(item: crate::route::ItemId, ordinary: crate::Plain) -> crate::route::ItemId { let _ = ordinary.read(); item }";
    let repo = fixture(HELPERS, text);
    repo.write(
        "cases/layout/lib.rs",
        "mod source; mod model; pub mod route; pub struct Plain; impl Plain { pub fn read(&self) -> u32 { 1 } }",
    );
    let value = run(&repo, assumed_request(&repo, text));
    assert_eq!(value["plan"]["applicable"], true, "{value}");
    assert_eq!(value["coverage"]["assumed_declared_identity"], 2);
    assert_eq!(value["coverage"]["ra_resolved"], 1);
    assert_eq!(value["plan"]["resolution_coverage"]["decisions"], 3);
    move_artifacts::apply(&repo, &value);
    let mut capped = assumed_request(&repo, text);
    capped["limits"]["response_bytes"] = json!(65_536);
    let capped = run(&repo, capped);
    assert!(
        capped["plan"]["binding_proofs"]
            .as_array()
            .into_iter()
            .flatten()
            .next()
            .is_none()
    );
    assert_eq!(
        capped["coverage"]["assumed_declared_identity"], 2,
        "{capped}"
    );
    assert_eq!(capped["coverage"]["ra_resolved"], 1);
    assert_eq!(capped["counts"]["omissions"]["binding_proofs"], 3);

    // A written nominal identity depending on an assumed sibling namespace audit
    // is conditional too; it must not leak into the real-resolution counter.
    repo.write(
        "cases/layout/model.rs",
        &format!("{HELPERS} uuid_id!(ItemId); uuid_id!(DispatchId); pub enum Plain {{ Unit }}"),
    );
    repo.write(
        "cases/layout/route.rs",
        "pub use crate::model::{ItemId, DispatchId, Plain};",
    );
    let text = "fn selected() { let _ = crate::route::Plain::Unit; }";
    repo.write("cases/layout/source.rs", text);
    let dependent = run(&repo, assumed_request(&repo, text));
    assert_eq!(dependent["plan"]["applicable"], true, "{dependent}");
    assert_eq!(dependent["coverage"]["assumed_declared_identity"], 1);
    assert!(dependent["coverage"].get("ra_resolved").is_none());
}

#[test]
fn declared_helper_flag_preserves_all_other_expansion_and_generated_fact_vetoes() {
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
        (format!("#[cfg(declared)] {HELPERS}"), "declarative_conditional_context"),
        (format!("#[cfg_attr(any(), allow(dead_code))] {HELPERS}"), "declarative_conditional_context"),
        (huge, "declarative_token_limit"),
        (nested, "declarative_nesting_limit"),
        ("macro_rules! uuid_id { ($($name:ident),*) => { $(pub struct $name;)* }; }".into(), "declarative_fragment_limit"),
        ("macro_rules! uuid_id { ($name:ident) => { uuid_id!($name); }; }".into(), "declarative_recursion_limit"),
        ("macro_rules! uuid_id { ($name:ident) => { pub struct $name; use other::*; }; }".into(), "declarative_output_unproved"),
        ("macro_rules! uuid_id { ($name:ident) => { #[cfg(declared)] #[helper] pub struct $name; }; }".into(), "declarative_attribute_provider_uncertain"),
        ("macro_rules! uuid_id { ($name:ident) => { #[derive(Serialize)] pub struct $name; }; }".into(), "declarative_attribute_provider_uncertain"),
        ("macro_rules! Clone { () => {}; } macro_rules! uuid_id { ($name:ident) => { #[helper] #[derive(Clone, Serialize)] pub struct $name; }; }".into(), "declarative_attribute_provider_uncertain"),
        ("macro_rules! uuid_id { ($name:ident) => { #[helper] #[derive(Serialize = value)] pub struct $name; }; }".into(), "declarative_attribute_provider_uncertain"),
        ("macro_rules! uuid_id { ($name:ident) => { #[cfg_attr(any(), helper)] pub struct $name; }; }".into(), "declarative_attribute_provider_uncertain"),
        ("macro_rules! uuid_id { ($name:ident) => { #[no_implicit_prelude] #[helper] pub struct $name; }; }".into(), "declarative_attribute_provider_uncertain"),
    ] {
        let repo = fixture(&definition, SELECTED);
        let value = run(&repo, assumed_request(&repo, SELECTED));
        blocked(&value);
        assert!(value["plan"]["binding_proofs"].as_array().into_iter().flatten().next().is_none(), "{value}");
        assert!(value["plan"]["resolution_coverage"]["context_evaluations"].as_array().unwrap().iter().any(|e| e["status"] == "skipped" && e["reason"] == reason), "{reason}: {value}");
    }
    for text in [
        "fn selected(value: crate::route::ItemId) -> u32 { value.read() }",
        "fn selected() { let _ = crate::route::ItemId::new(); }",
        "fn selected() { let _ = crate::route::ItemId(Opaque); }",
        "fn selected(value: crate::route::ItemId) { let _ = value.0; }",
    ] {
        let repo = fixture(HELPERS, text);
        blocked(&run(&repo, assumed_request(&repo, text)));
    }
    let definition =
        "macro_rules! uuid_id { ($name:ident) => { #[helper] pub enum $name { Unit } }; }";
    let text = "fn selected() { let _ = crate::route::ItemId::Unit; }";
    let repo = fixture(definition, text);
    blocked(&run(&repo, assumed_request(&repo, text)));
    for definition in [
        "",
        "#[proc_macro] pub fn uuid_id(input: TokenStream) -> TokenStream { input }",
    ] {
        let repo = fixture(definition, SELECTED);
        let value = run(&repo, assumed_request(&repo, SELECTED));
        blocked(&value);
        assert!(
            value["plan"]["binding_proofs"]
                .as_array()
                .into_iter()
                .flatten()
                .next()
                .is_none()
        );
    }
    let repo = fixture(HELPERS, SELECTED);
    for field in ["resolve_semantic", "semantic_configuration"] {
        let mut args = assumed_request(&repo, SELECTED);
        args[field] = if field == "resolve_semantic" {
            json!(false)
        } else {
            Value::Null
        };
        blocked(&run(&repo, args));
    }
}

#[test]
fn declared_helper_flag_keeps_helper_free_proofs_real_and_written_owner_attributes_blocked() {
    let repo = fixture(WRAPPER, SELECTED);
    let value = run(&repo, assumed_request(&repo, SELECTED));
    assert_eq!(value["plan"]["applicable"], true, "{value}");
    assert!(value["coverage"].get("assumed_declared_identity").is_none());
    assert_eq!(value["coverage"]["ra_resolved"], 3);
    for owner in [
        "definition",
        "invocation",
        "module",
        "crate",
        "shadowed_derive",
    ] {
        let repo = fixture(WRAPPER, SELECTED);
        match owner {
            "definition" => repo.write("cases/layout/model.rs", &format!("#[helper] {WRAPPER} uuid_id!(ItemId); uuid_id!(DispatchId);")),
            "invocation" => repo.write("cases/layout/model.rs", &format!("{WRAPPER} #[helper] uuid_id!(ItemId); uuid_id!(DispatchId);")),
            "module" => repo.write("cases/layout/lib.rs", "mod source; #[helper] mod model; pub mod route;"),
            "crate" => repo.write("cases/layout/lib.rs", "#![helper] mod source; mod model; pub mod route;"),
            "shadowed_derive" => repo.write("cases/layout/model.rs", &format!("macro_rules! Clone {{ () => {{}}; }} {WRAPPER} uuid_id!(ItemId); uuid_id!(DispatchId);")),
            _ => unreachable!(),
        }
        blocked(&run(&repo, assumed_request(&repo, SELECTED)));
    }
}

#[test]
fn declared_helper_schema_is_default_false_and_tool_description_discloses_the_assumption() {
    #[path = "support/stdio_client.rs"]
    mod stdio_client;
    let mut client = stdio_client::Client::new();
    let tools = client.rpc("tools/list", json!({}));
    let tool = tools["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == "move_item")
        .unwrap();
    assert_eq!(
        tool["inputSchema"]["properties"]["assume_declared_helpers"]["type"],
        "boolean"
    );
    assert_eq!(
        tool["inputSchema"]["properties"]["assume_declared_helpers"]["default"],
        false
    );
    for term in [
        "assume_declared_helpers",
        "assumed_declared_identity",
        "registered derive helper",
    ] {
        assert!(tool["description"].as_str().unwrap().contains(term));
    }
    let repo = fixture(HELPERS, SELECTED);
    let value = client.call("move_item", assumed_request(&repo, SELECTED));
    assert_eq!(value["plan"]["applicable"], true, "{value}");
    assert_eq!(value["coverage"]["assumed_declared_identity"], 3);
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
