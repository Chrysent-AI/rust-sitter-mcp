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

const COMPOUND: &str = "all(feature = \"tokio\", any(feature = \"http1\", feature = \"http2\"))";

fn run(root: &str, source: &str, selected: &str, config: Value) -> Value {
    let repo = Fixture::generate();
    repo.write("cases/layout/lib.rs", root);
    repo.write("cases/layout/source.rs", source);
    let args = json!({"repo_path":repo.0,"crate_root":"cases/layout/lib.rs",
        "paths":["cases/layout"],"resolve_semantic":false,"semantic_configuration":config,
        "assume_standard_prelude":true,"limits":{"diagnostic_count":1000},
        "moves":[{"item":move_artifacts::anchor(&repo,"cases/layout/source.rs",selected),
            "destination":{"kind":"new_sibling","path":"cases/layout/moved.rs","parent_path":"cases/layout/lib.rs"}}]});
    let before = observe(&repo.0);
    let result = Engine::new(repo.0.clone()).unwrap().move_item(
        serde_json::from_value::<MoveRequest>(args).unwrap(),
        &AtomicBool::new(false),
    );
    assert_eq!(observe(&repo.0), before);
    let result = serde_json::to_value(result).unwrap();
    assert_eq!(result["status"], "complete", "{result}");
    assert_eq!(result["plan"]["integrity"]["semantic"], "not_performed");
    if result["plan"]["applicable"] == true {
        move_artifacts::apply(&repo, &result);
    } else {
        assert!(result["plan"]["patch"].is_null());
    }
    result
}

fn config(as_atoms: bool) -> Value {
    let names = ["tokio", "http1", "http2"];
    json!({"crates":[{"name":"fixture","root_file":"cases/layout/lib.rs","edition":"2024",
        "features":if as_atoms { vec![] } else { names.to_vec() },
        "cfg":if as_atoms { names.iter().map(|f| json!({"key":"feature","value":f})).collect::<Vec<_>>() } else { vec![] },
        "dependencies":[]}]})
}

#[test]
fn written_selected_conditions_share_positive_only_predicate_semantics() {
    let selected = "fn selected() -> u32 { 1 }";
    for atoms in [false, true] {
        for attribute in [
            format!("#[cfg({COMPOUND})]"),
            format!("#[cfg_attr(feature = \"tokio\", cfg({COMPOUND}))]"),
            "#[cfg_attr(not(feature = \"tokio\"), arbitrary_provider)]".into(),
            "#[cfg_attr(feature = \"tokio\", cfg_attr(all(), allow(dead_code)))]".into(),
        ] {
            let source = format!("{attribute}\n{selected}\n");
            let result = run("mod source;\n", &source, selected, config(atoms));
            assert_eq!(result["plan"]["applicable"], true, "{attribute}: {result}");
            assert!(result["plan"].get("resolution_coverage").is_none());
        }
    }
}

#[test]
fn written_cfg_entries_and_boolean_literals_are_positive_evidence() {
    let selected = "fn selected() -> u32 { 1 }";
    let mut config = config(false);
    config["crates"][0]["cfg"] = json!([
        {"key":"unix","value":null}, {"key":"target_os","value":"linux"}
    ]);
    for predicate in [
        "all(unix, target_os = \"linux\", true)",
        "not(false)",
        "all()",
    ] {
        let result = run(
            "mod source;\n",
            &format!("#[cfg({predicate})]\n{selected}\n"),
            selected,
            config.clone(),
        );
        assert_eq!(result["plan"]["applicable"], true, "{result}");
    }
    let result = run(
        "mod source;\n",
        &format!("#[cfg(target_os = \"unknown\")]\n{selected}\n"),
        selected,
        config,
    );
    assert_eq!(result["plan"]["applicable"], false, "{result}");
}

#[test]
fn written_unknown_inactive_and_provider_attributes_stay_anchored() {
    let selected = "fn selected() -> u32 { 1 }";
    for attribute in [
        "#[cfg(feature = \"missing\")]",
        "#[cfg(unknown_key)]",
        "#[cfg(any(feature = \"http1\", unknown_key))]",
        "#[cfg(not(feature = \"tokio\"))]",
        "#[cfg_attr(feature = \"tokio\", cfg(any()))]",
        "#[cfg_attr(feature = \"tokio\", arbitrary_provider)]",
        "#[cfg_attr(feature = \"tokio\", derive(Clone))]",
        "#[cfg(not(feature = \"tokio\", feature = \"http1\"))]",
        "#[cfg_attr(feature = \"tokio\",)]",
        "#[cfg_attr(unknown_key, allow(dead_code))]",
    ] {
        let result = run(
            "mod source;\n",
            &format!("{attribute}\n{selected}\n"),
            selected,
            config(false),
        );
        assert_eq!(result["plan"]["applicable"], false, "{attribute}: {result}");
        assert!(
            result["plan"]["decisions"]
                .as_array()
                .unwrap()
                .iter()
                .any(|d| d["reason"] == "conditional_or_inherited_context"
                    && d["unresolved_consequence"]
                        .as_str()
                        .unwrap()
                        .contains("declared configuration consulted")
                    && d["refusal_basis"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|b| b["class"] == "conditional_context"
                            && b["anchor"]["range"]["start_byte"] == 0)),
            "{result}"
        );
    }
}

#[test]
fn conditional_admission_does_not_erase_an_independent_provider() {
    let selected = "fn selected() -> u32 { 1 }";
    let result = run(
        "mod source;\n",
        &format!(
            "#[cfg_attr(not(feature = \"tokio\"), allow(dead_code))]\n#[arbitrary_provider]\n{selected}\n"
        ),
        selected,
        config(false),
    );
    assert_eq!(result["plan"]["applicable"], false, "{result}");
    assert!(
        result["plan"]["decisions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["refusal_basis"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|b| b["class"] == "conditional_context"
                    && b["anchor"]["range"]["start_byte"]
                        .as_u64()
                        .is_some_and(|n| n > 0))),
        "{result}"
    );
}

#[test]
fn written_chain_inline_import_and_required_binding_conditions_are_consulted() {
    let selected = "fn selected() -> u32 { 1 }";
    let source = format!(
        "{selected}\n#[cfg({COMPOUND})]\nmod consumers {{ use super::selected; fn call() -> u32 {{ selected() }} }}\n"
    );
    let result = run(
        &format!("#[cfg({COMPOUND})]\nmod source;\n"),
        &source,
        selected,
        config(false),
    );
    assert_eq!(result["plan"]["applicable"], true, "{result}");

    let selected = "fn selected() -> Local { Local }";
    let source = format!("#[cfg({COMPOUND})]\nstruct Local;\n{selected}\n");
    let result = run("mod source;\n", &source, selected, config(false));
    assert_eq!(result["plan"]["applicable"], true, "{result}");

    let source = format!("#[cfg({COMPOUND})]\nuse crate::Local;\n{selected}\n");
    let result = run(
        "pub struct Local;\nmod source;\n",
        &source,
        selected,
        config(false),
    );
    assert_eq!(result["plan"]["applicable"], true, "{result}");
}

#[test]
fn declared_cfg_can_admit_named_reexports_but_not_unknown_or_inactive_leaves() {
    let selected = "fn selected() -> Alias { Alias }";
    for (predicate, applicable) in [
        (COMPOUND, true),
        ("unknown_key", false),
        ("not(feature = \"tokio\")", false),
    ] {
        let source = format!("#[cfg({predicate})]\npub use crate::Local as Alias;\n{selected}\n");
        let result = run(
            "pub struct Local;\nmod source;\n",
            &source,
            selected,
            config(false),
        );
        assert_eq!(result["plan"]["applicable"], applicable, "{result}");
    }
}

#[test]
fn written_prelude_scopes_consult_cfg_without_dropping_shadow_names() {
    let selected = "fn selected() -> Option<u8> { None }";
    for attribute in [
        format!("#![cfg({COMPOUND})]"),
        "#![cfg_attr(not(feature = \"tokio\"), no_implicit_prelude)]".into(),
    ] {
        let result = run(
            "mod source;\n",
            &format!("{attribute}\n{selected}\n"),
            selected,
            config(false),
        );
        assert_eq!(result["plan"]["applicable"], true, "{result}");
    }
    for attribute in [
        "#[cfg(not(feature = \"tokio\"))]",
        "#[cfg(feature = \"tokio\")]",
    ] {
        let result = run(
            "mod source;\n",
            &format!("{attribute}\nstruct Option;\n{selected}\n"),
            selected,
            config(false),
        );
        assert_eq!(result["plan"]["applicable"], false, "{result}");
    }
    let result = run(
        "mod source;\n",
        &format!("#![cfg_attr(feature = \"tokio\", no_implicit_prelude)]\n{selected}\n"),
        selected,
        config(false),
    );
    assert_eq!(result["plan"]["applicable"], false, "{result}");
}

#[test]
fn lexical_cfg_can_discharge_context_but_not_an_inactive_required_binding() {
    let selected = "fn selected() -> Option<u8> { #[cfg(feature = \"tokio\")] struct Other; None }";
    let result = run("mod source;\n", selected, selected, config(false));
    assert_eq!(result["plan"]["applicable"], true, "{result}");
    for cfg in [
        "feature = \"missing\"",
        "unknown_key",
        "not(feature = \"tokio\")",
    ] {
        let selected = format!("fn selected() {{ #[cfg({cfg})] struct Local; let _: Local; }}");
        let result = run("mod source;\n", &selected, &selected, config(false));
        assert_eq!(result["plan"]["applicable"], false, "{result}");
    }
}

#[test]
fn absent_duplicate_mismatched_and_invalid_configuration_do_not_admit_cfg() {
    let selected = "fn selected() -> u32 { 1 }";
    let source = format!("#[cfg({COMPOUND})]\n{selected}\n");
    let good = config(false);
    let mut wrong = good.clone();
    wrong["crates"][0]["root_file"] = json!("elsewhere.rs");
    let mut duplicate = good.clone();
    duplicate["crates"]
        .as_array_mut()
        .unwrap()
        .push(good["crates"][0].clone());
    let mut invalid = good.clone();
    invalid["crates"][0]["edition"] = json!("invalid");
    for config in [Value::Null, wrong, duplicate, invalid] {
        let result = run("mod source;\n", &source, selected, config);
        assert_eq!(result["plan"]["applicable"], false, "{result}");
    }
}
