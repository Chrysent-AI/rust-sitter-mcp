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

const SELECTED: &str = "fn selected(value: crate::Value) -> u32 { value.read() }";

fn configuration() -> Value {
    json!({"crates":[{"name":"fixture","root_file":"cases/layout/lib.rs","edition":"2024","features":["enabled"],"cfg":[{"key":"declared","value":null},{"key":"target_os","value":"linux"}],"dependencies":[]}]})
}
fn fixture(attributes: &str) -> Fixture {
    let repo = Fixture::generate();
    repo.write("cases/layout/lib.rs", &format!("mod source;\n{attributes}\npub struct Value {{ pub number: u32 }}\nimpl Value {{ pub fn read(&self) -> u32 {{ self.number }} }}\n"));
    repo.write("cases/layout/source.rs", SELECTED);
    repo
}
fn request(repo: &Fixture, text: &str) -> Value {
    json!({"repo_path":repo.0,"crate_root":"cases/layout/lib.rs","paths":["cases/layout"],"limits":{"time_budget_ms":30000,"diagnostic_count":1000},"resolve_semantic":true,"semantic_configuration":configuration(),
        "moves":[{"item":move_artifacts::anchor(repo,"cases/layout/source.rs",text),"destination":{"kind":"new_sibling","path":"cases/layout/moved.rs","parent_path":"cases/layout/lib.rs"}}]})
}
fn run(repo: &Fixture, args: Value) -> Value {
    let before = observe(&repo.0);
    let result = Engine::new(repo.0.clone()).unwrap().move_item(
        serde_json::from_value::<MoveRequest>(args).unwrap(),
        &AtomicBool::new(false),
    );
    assert_eq!(observe(&repo.0), before, "analysis modified caller source");
    serde_json::to_value(result).unwrap()
}
fn evaluations(value: &Value) -> &Vec<Value> {
    value["plan"]["resolution_coverage"]["context_evaluations"]
        .as_array()
        .unwrap()
}
fn evidence(value: &Value, text: &str, status: &str, result: Value) -> bool {
    evaluations(value).iter().any(|e| {
        e["anchor"]["expected_text"] == text && e["status"] == status && e["value"] == result
    })
}

#[test]
fn built_in_derives_and_declared_cfg_no_longer_poison_inherent_resolution() {
    let attr = "#[cfg(all(declared, feature = \"enabled\", target_os = \"linux\"))]\n#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Ord, PartialOrd, Hash,)]";
    let repo = fixture(attr);
    let mut off = request(&repo, SELECTED);
    off["resolve_semantic"] = json!(false);
    assert_eq!(run(&repo, off)["plan"]["applicable"], false);
    let value = run(&repo, request(&repo, SELECTED));
    assert_eq!(value["plan"]["applicable"], true, "{value}");
    for atom in ["declared", "feature = \"enabled\"", "target_os = \"linux\""] {
        assert!(evidence(&value, atom, "evaluated", json!(true)), "{value}");
    }
    for revision in ["original", "final"] {
        assert!(evaluations(&value).iter().any(|e| e["revision"] == revision
            && e["anchor"]["expected_text"] == attr.split('\n').nth(1).unwrap()
            && e["status"] == "admitted"));
    }
    for proof in value["plan"]["binding_proofs"].as_array().unwrap() {
        assert_eq!(
            proof["coverage"]["context_evaluations"],
            value["plan"]["resolution_coverage"]["context_evaluations"]
        );
    }
}

#[test]
fn cfg_attr_evaluates_before_admitting_derive_or_skipping_inactive_payload() {
    for (attr, inactive) in [
        ("#[cfg_attr(declared, derive(Clone, Debug))]", false),
        (
            "#[cfg_attr(feature = \"enabled\", derive(r#Clone /* comment */, Copy,))]",
            false,
        ),
        ("#[cfg_attr(not(declared), derive(Custom))]", true),
        (
            "#[cfg_attr(all(), cfg_attr(any(), arbitrary_attribute))]",
            true,
        ),
    ] {
        let repo = fixture(attr);
        let value = run(&repo, request(&repo, SELECTED));
        assert_eq!(value["plan"]["applicable"], true, "{attr}: {value}");
        assert!(evidence(&value, attr, "admitted", Value::Null));
        if inactive {
            assert!(evaluations(&value).iter().any(|e| e["status"] == "inactive" && e["reason"] == "cfg_attr_condition_false"), "{value}");
        }
    }
}

#[test]
fn unknown_cfg_custom_or_malformed_derives_and_module_macros_keep_vetoes() {
    for (attributes, why) in [
        ("#[derive(Custom)]", "non_builtin_derive"),
        (
            "macro_rules! Clone { () => {} } #[derive(Clone)]",
            "non_builtin_derive",
        ),
        ("#[derive(Clone, custom::Debug)]", "non_builtin_derive"),
        ("#[derive(Clone Debug)]", "non_builtin_derive"),
        ("#[derive()]", "unparseable_derive"),
        ("#[cfg(undeclared)]", "undeclared_cfg_atom"),
        ("#[cfg(feature = \"missing\")]", "undeclared_cfg_atom"),
        (
            "#[cfg_attr(undeclared, derive(Clone))]",
            "undeclared_cfg_atom",
        ),
        (
            "#[cfg_attr(declared, derive(Custom))]",
            "non_builtin_derive",
        ),
        ("#[cfg(any(declared, undeclared))]", "undeclared_cfg_atom"),
        ("#[cfg(arbitrary(declared))]", "unsupported_cfg_predicate"),
        ("module_macro!();", "module_macro_invocation"),
    ] {
        let repo = fixture(attributes);
        let value = run(&repo, request(&repo, SELECTED));
        assert_eq!(value["plan"]["applicable"], false, "{attributes}: {value}");
        // A nominal attribute proof is independent of method resolution: a
        // custom derive cannot rewrite Value, but its generated impls are unknown.
        assert!(
            value["plan"]["binding_proofs"]
                .as_array()
                .into_iter()
                .flatten()
                .all(|p| {
                    p["classification"] == "context_attribute"
                        && p["basis"]
                            .as_str()
                            .unwrap()
                            .contains("nominal identity via stable written declaration or explicit import route")
                }),
            "{value}"
        );
        assert!(value["plan"]["patch"].is_null());
        assert!(
            evaluations(&value)
                .iter()
                .any(|e| e["status"] == "skipped" && e["reason"] == why),
            "{attributes}: {value}"
        );
        assert!(
            value["plan"]["decisions"]
                .as_array()
                .unwrap()
                .iter()
                .any(|d| d["refusal_basis"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|b| b["class"] == "semantic_source_fact_unproved")),
            "{value}"
        );
    }
}

#[test]
fn target_custom_derive_still_vetoes_trait_method_facts() {
    let repo = fixture("#[derive(Custom)]");
    repo.write("cases/layout/lib.rs", "mod source; #[derive(Custom)] pub struct Value; pub trait Read { fn read(&self) -> u32; } impl Read for Value { fn read(&self) -> u32 { 1 } }");
    repo.write(
        "cases/layout/source.rs",
        &format!("use crate::Read;\n{SELECTED}"),
    );
    let value = run(&repo, request(&repo, SELECTED));
    assert_eq!(value["plan"]["applicable"], false, "{value}");
    assert!(value["plan"]["edits"].is_null());
    assert!(value["plan"]["created_files"].is_null());
    assert!(value["plan"]["patch"].is_null());
    assert!(
        evaluations(&value).iter().any(|e| {
            e["anchor"]["expected_text"] == "#[derive(Custom)]"
                && e["fact_class"] == "generated_items"
                && e["status"] == "skipped"
                && e["reason"] == "non_builtin_derive"
        }),
        "{value}"
    );
    assert!(
        !value["plan"]["binding_proofs"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|p| p["anchor"]["expected_text"] == "value.read"),
        "{value}"
    );
}

#[test]
fn generated_item_guard_reaches_receiver_declarations_in_other_modules() {
    for derive in ["Clone", "Custom"] {
        for text in [
            "fn selected(value: crate::model::Value) -> u32 { value.read() }",
            "fn selected(value: crate::model::Value) -> u32 { crate::model::Value::read(&value) }",
        ] {
            let repo = Fixture::generate();
            repo.write(
                "cases/layout/lib.rs",
                "mod source; pub mod model; impl model::Value { pub fn read(&self) -> u32 { 1 } }",
            );
            repo.write(
                "cases/layout/model.rs",
                &format!("#[derive({derive})] pub struct Value;"),
            );
            repo.write("cases/layout/source.rs", text);
            let value = run(&repo, request(&repo, text));
            assert_eq!(
                value["plan"]["applicable"],
                derive == "Clone",
                "{derive}: {value}"
            );
            if derive == "Custom" {
                assert!(value["plan"]["patch"].is_null());
                assert!(
                    evaluations(&value).iter().any(|e| {
                        e["anchor"]["path"] == "cases/layout/model.rs"
                            && e["fact_class"] == "generated_items"
                            && e["reason"] == "non_builtin_derive"
                            && e["status"] == "skipped"
                    }),
                    "{value}"
                );
            }
        }
    }
}

#[test]
fn known_off_sibling_is_information_not_a_context_veto() {
    let repo = fixture("");
    repo.write("cases/layout/lib.rs", "mod source; #[cfg(not(declared))] struct Disabled; pub struct Value { pub number: u32 } impl Value { pub fn read(&self) -> u32 { self.number } }\n");
    let value = run(&repo, request(&repo, SELECTED));
    assert_eq!(value["plan"]["applicable"], true, "{value}");
    assert!(evidence(&value, "not(declared)", "evaluated", json!(false)));
}

#[test]
fn final_destination_context_is_evaluated_independently() {
    let repo = fixture("#[derive(Clone)]");
    repo.write(
        "cases/layout/destination.rs",
        "#[cfg_attr(undeclared, derive(Clone))] struct Other;\n",
    );
    repo.write("cases/layout/lib.rs", "mod source; mod destination; #[derive(Clone)] pub struct Value { pub number: u32 } impl Value { pub fn read(&self) -> u32 { self.number } }");
    let mut args = request(&repo, SELECTED);
    args["moves"][0]["destination"] =
        json!({"kind":"existing","path":"cases/layout/destination.rs"});
    let value = run(&repo, args);
    assert_eq!(value["plan"]["applicable"], false, "{value}");
    assert!(
        evaluations(&value)
            .iter()
            .any(|e| e["revision"] == "final" && e["reason"] == "undeclared_cfg_atom")
    );
    assert!(
        value["plan"]["decisions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["refusal_basis"]
                .as_array()
                .unwrap()
                .iter()
                .any(|b| b["class"] == "semantic_final_fact_unproved"))
    );
}

#[test]
fn effect_scope_dependency_and_variant_pattern_have_independent_proofs() {
    let repo = Fixture::generate();
    repo.write("cases/layout/lib.rs", "mod source; mod scheduler;\n");
    repo.write(
        "cases/layout/scheduler.rs",
        "pub mod event; pub use event::Effect; #[derive(Clone, Copy, Debug)] struct Chain;\n",
    );
    repo.write(
        "cases/layout/scheduler/event.rs",
        "#[derive(Clone, Debug, Eq, PartialEq)] pub enum Effect { RecordTransition(u32) }\n",
    );
    let text = "fn selected(effect: crate::scheduler::Effect) { let crate::scheduler::Effect::RecordTransition(value) = effect; }";
    repo.write("cases/layout/source.rs", text);
    let mut args = request(&repo, text);
    args["resolve_semantic"] = json!(false);
    let off = run(&repo, args.clone());
    assert!(
        off["plan"]["decisions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["category"] == "scope_dependency"
                && d["anchors"][0]["expected_text"] == "#[derive(Clone, Debug, Eq, PartialEq)]"),
        "{off}"
    );
    args["resolve_semantic"] = json!(true);
    let on = run(&repo, args);
    assert!(
        !on["plan"]["decisions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["category"] == "scope_dependency"
                && d["anchors"][0]["expected_text"] == "#[derive(Clone, Debug, Eq, PartialEq)]"),
        "{on}"
    );
    assert!(
        on["plan"]["binding_proofs"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["classification"] == "context_attribute"
                && p["anchor"]["expected_text"] == "#[derive(Clone, Debug, Eq, PartialEq)]"),
        "{on}"
    );
    assert_eq!(on["plan"]["applicable"], true, "{on}");
    assert!(
        on["plan"]["binding_proofs"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| {
                p["classification"] == "variant_path"
                    && p["anchor"]["expected_text"] == "crate::scheduler::Effect::RecordTransition"
            }),
        "{on}"
    );
}

#[test]
fn attribute_discharge_preserves_required_visibility_repairs() {
    let repo = Fixture::generate();
    repo.write("cases/layout/lib.rs", "mod source; mod destination;\n");
    let text = "fn selected(value: Value) {}";
    repo.write(
        "cases/layout/source.rs",
        &format!("#[derive(Clone)] struct Value;\n{text}"),
    );
    repo.write("cases/layout/destination.rs", "fn retained() {}\n");
    let mut args = request(&repo, text);
    args["moves"][0]["destination"] =
        json!({"kind":"existing","path":"cases/layout/destination.rs"});
    let value = run(&repo, args);
    assert_eq!(value["plan"]["applicable"], true, "{value}");
    assert!(
        value["plan"]["rewrites"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["kind"] == "visibility" && r["after_text"] == "pub(crate) "),
        "{value}"
    );
    assert!(
        value["plan"]["binding_proofs"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["classification"] == "context_attribute")
    );
    let copy = move_artifacts::apply(&repo, &value);
    assert!(
        std::fs::read_to_string(copy.0.join("cases/layout/source.rs"))
            .unwrap()
            .contains("pub(crate) struct Value")
    );
}
