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

const SELECTED: &str = "fn selected(value: crate::Outcome) -> crate::Outcome { match value { crate::Outcome::Pass => crate::Outcome::Pass, crate::Outcome::Fail(number) => crate::Outcome::Fail(number) } }";

fn fixture(declarations: &str) -> Fixture {
    let repo = Fixture::generate();
    repo.write(
        "cases/layout/lib.rs",
        &format!("mod source;\n{declarations}\n"),
    );
    repo.write("cases/layout/source.rs", SELECTED);
    repo
}
fn request(repo: &Fixture) -> Value {
    request_for(repo, SELECTED)
}
fn request_for(repo: &Fixture, selected: &str) -> Value {
    json!({"repo_path":repo.0,"crate_root":"cases/layout/lib.rs","paths":["cases/layout"],
        "resolve_semantic":true,"limits":{"time_budget_ms":30000,"diagnostic_count":1000},
        "semantic_configuration":{"crates":[{"name":"fixture","root_file":"cases/layout/lib.rs","edition":"2024","features":[],"cfg":[],"dependencies":[]}]},
        "moves":[{"item":move_artifacts::anchor(repo,"cases/layout/source.rs",selected),"destination":{"kind":"new_sibling","path":"cases/layout/moved.rs","parent_path":"cases/layout/lib.rs"}}]})
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

#[test]
fn stable_variant_identity_survives_own_and_unrelated_additive_derives() {
    for declarations in [
        "#[derive(Clone)] pub enum Outcome { Pass, Fail(u32) }",
        "#[derive(Clone)] pub enum Outcome { Pass, Fail(u32) } #[derive(Custom)] struct Other;",
        "#[derive(Custom)] pub enum Outcome { Pass, Fail(u32) }",
        "#[derive(custom::Serialize, r#Custom,)] pub enum Outcome { Pass, Fail(u32) }",
        "#[cfg_attr(all(), derive(custom::Serialize))] pub enum Outcome { Pass, Fail(u32) }",
    ] {
        let repo = fixture(declarations);
        let result = run(&repo, request(&repo));
        assert_eq!(result["status"], "complete", "{result}");
        assert_eq!(
            result["plan"]["applicable"], true,
            "{declarations}: {result}"
        );
        for artifact in ["edits", "created_files", "patch"] {
            assert!(!result["plan"][artifact].is_null());
        }
        let variants: Vec<_> = result["plan"]["binding_proofs"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|p| p["classification"] == "variant_path")
            .collect();
        assert_eq!(variants.len(), 4, "{result}");
        for proof in variants {
            assert_eq!(proof["class"], "ra_resolved");
            assert_eq!(proof["source_access"], true);
            assert_eq!(proof["final_access"], true);
            assert_eq!(
                proof["declaration"]["anchor"]["expected_text"],
                proof["final_declaration"]["anchor"]["expected_text"]
            );
            assert_eq!(
                proof["declaration"]["crate_origin"],
                proof["final_declaration"]["crate_origin"]
            );
            assert_eq!(
                proof["original_receiver"]["declaration"]["anchor"]["expected_text"],
                "Outcome"
            );
            assert_eq!(proof["adjusted_receiver"], proof["original_receiver"]);
            assert!(proof["basis"].as_str().unwrap().contains(
                "nominal identity via stable written declaration or explicit import route"
            ));
        }
        let evaluations = result["plan"]["resolution_coverage"]["context_evaluations"]
            .as_array()
            .unwrap();
        for revision in ["original", "final"] {
            assert!(
                evaluations.iter().any(|e| e["revision"] == revision
                    && e["status"] == "admitted"
                    && e["fact_class"] == "nominal_identity"
                    && e["basis"]
                        .as_str()
                        .unwrap()
                        .contains("derive-generated imports can override globs")),
                "{result}"
            );
        }
        // Exercise exact published bytes, not just a true applicability bit.
        let copy = move_artifacts::apply(&repo, &result);
        assert!(
            std::fs::read_to_string(copy.0.join("cases/layout/moved.rs"))
                .unwrap()
                .contains(SELECTED)
        );
    }
}

/// A real proc macro: rustc expands this workspace, while the bounded RA graph
/// deliberately does not. The competing explicit import must not become a proof
/// of the glob's unexpanded identity.
fn injecting_fixture(import: &str, selected: &str) -> Fixture {
    let repo = Fixture::generate();
    repo.write("cases/layout/Cargo.toml", "[package]\nname = \"derive-binding-fixture\"\nversion = \"0.0.0\"\nedition = \"2024\"\n[lib]\npath = \"lib.rs\"\n[dependencies]\ninjector = { path = \"injector\" }\n[workspace]\nmembers = [\"injector\"]\n");
    repo.write("cases/layout/injector/Cargo.toml", "[package]\nname = \"injector\"\nversion = \"0.0.0\"\nedition = \"2024\"\n[lib]\nproc-macro = true\n");
    repo.write("cases/layout/injector/src/lib.rs", "extern crate proc_macro;\n#[proc_macro_derive(Inject)]\npub fn inject(_: proc_macro::TokenStream) -> proc_macro::TokenStream { \"use crate::b::B::V;\".parse().unwrap() }\n#[proc_macro_derive(InjectEnum)]\npub fn inject_enum(_: proc_macro::TokenStream) -> proc_macro::TokenStream { \"pub use crate::b::B as A;\".parse().unwrap() }\n");
    repo.write("cases/layout/lib.rs", "#![allow(dead_code, unused_imports)]\nmod source;\npub enum A { V }\npub mod a { pub enum A { V } }\npub mod b { pub enum B { V } }\n");
    repo.write("cases/layout/source.rs", &format!("{import}\n{selected}\n"));
    repo
}
fn cargo_check(repo: &Fixture) {
    let result = std::process::Command::new("cargo")
        .args(["check", "--workspace", "--offline", "--quiet"])
        .current_dir(repo.0.join("cases/layout"))
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
}

#[test]
fn generated_explicit_import_overriding_glob_withholds_variant_proof() {
    for (imports, selected, occurrence) in [
        (
            "use crate::A::*; use injector::Inject; #[derive(Inject)] struct Other;",
            "fn selected() -> crate::b::B { V }",
            "V",
        ),
        (
            "use crate::a::*; use injector::InjectEnum; #[derive(InjectEnum)] struct Other;",
            "fn selected() -> crate::b::B { A::V }",
            "A::V",
        ),
        // An explicit local import does not make a glob-dependent re-export
        // source stable against imports emitted in that source module.
        (
            "use crate::routes::A;",
            "fn selected() -> crate::b::B { A::V }",
            "A::V",
        ),
    ] {
        let repo = injecting_fixture(imports, selected);
        if imports.contains("routes") {
            repo.write("cases/layout/lib.rs", "#![allow(dead_code, unused_imports)]\nmod source; pub mod a { pub enum A { V } } pub mod b { pub enum B { V } } pub mod routes { pub use crate::a::*; use injector::InjectEnum; #[derive(InjectEnum)] struct Other; }\n");
        }
        let root = std::fs::read_to_string(repo.0.join("cases/layout/lib.rs")).unwrap();
        repo.write("cases/layout/lib.rs", &format!("{root}mod destination;\n"));
        repo.write(
            "cases/layout/destination.rs",
            if occurrence == "V" {
                "use crate::A::*;\n"
            } else {
                "use crate::a::A;\n"
            },
        );
        // The return type proves which enum rustc actually chose after expansion.
        // Destination bindings deliberately preserve RA's unexpanded identity,
        // so agreement between overlays cannot substitute for the admission rule.
        cargo_check(&repo);
        let mut args = request_for(&repo, selected);
        args["moves"][0]["destination"] =
            json!({"kind":"existing","path":"cases/layout/destination.rs"});
        let result = run(&repo, args);
        assert_eq!(result["status"], "complete", "{result}");
        assert_eq!(result["plan"]["applicable"], false, "{result}");
        for artifact in ["edits", "created_files", "patch"] {
            assert!(result["plan"][artifact].is_null(), "{result}");
        }
        assert!(
            !result["plan"]["binding_proofs"]
                .as_array()
                .unwrap()
                .iter()
                .any(|p| p["classification"] == "variant_path"),
            "{result}"
        );
        // Bare glob references already have a syntactic veto and never enter RA;
        // qualified paths exercise the stronger nominal-identity admission.
        if occurrence == "V" {
            assert!(
                result["plan"]["decisions"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|d| d["refusal_basis"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .any(|b| b["class"] == "glob_import")),
                "{result}"
            );
            continue;
        }
        let evaluations = result["plan"]["resolution_coverage"]["context_evaluations"]
            .as_array()
            .unwrap();
        assert!(
            evaluations.iter().any(|e| e["kind"] == "binding"
                && e["status"] == "skipped"
                && e["reason"] == "stable_written_identity_unproved"
                && e["anchor"]["expected_text"] == occurrence),
            "{result}"
        );
        if imports.contains("derive") {
            assert!(
                evaluations
                    .iter()
                    .any(|e| e["reason"] == "non_builtin_derive"
                        && e["status"] == "skipped"
                        && e["anchor"]["expected_text"]
                            .as_str()
                            .unwrap()
                            .contains("derive(Inject")),
                "{result}"
            );
        }
        assert!(
            result["plan"]["decisions"].as_array().unwrap().iter().any(
                |d| d["blocks_applicability"] == true
                    && d["refusal_basis"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .any(|b| b["class"] == "semantic_source_fact_unproved")
            ),
            "{result}"
        );
    }
}

#[test]
fn explicit_enum_binding_stays_stable_with_real_derive_generated_import() {
    for imports in [
        "use crate::A; use injector::Inject; #[derive(Inject)] struct Other;",
        "use crate::{A as Explicit}; use injector::Inject; #[derive(Inject)] struct Other;",
        "use crate::{routes::{A}}; use injector::Inject; #[derive(Inject)] struct Other;",
    ] {
        let base = if imports.contains("Explicit") {
            "Explicit"
        } else {
            "A"
        };
        let selected = format!(
            "fn selected(value: {base}) -> {base} {{ match value {{ {base}::V => {base}::V }} }}"
        );
        let repo = injecting_fixture(imports, &selected);
        if imports.contains("routes") {
            repo.write("cases/layout/lib.rs", "#![allow(dead_code, unused_imports)]\nmod source; mod destination; pub enum A { V } pub mod b { pub enum B { V } } pub mod routes { pub use crate::A; }\n");
            repo.write("cases/layout/destination.rs", "use crate::A;\n");
        }
        cargo_check(&repo);
        let mut args = request_for(&repo, &selected);
        if imports.contains("routes") {
            // Supply a written destination binding to isolate nested-use
            // identity from the separate import-repair capability boundary.
            args["moves"][0]["destination"] =
                json!({"kind":"existing","path":"cases/layout/destination.rs"});
        }
        let result = run(&repo, args);
        assert_eq!(result["plan"]["applicable"], true, "{result}");
        let proof = result["plan"]["binding_proofs"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["classification"] == "variant_path")
            .unwrap();
        assert!(
            proof["basis"]
                .as_str()
                .unwrap()
                .contains("stable written declaration or explicit import route")
        );
        assert_eq!(
            proof["original_receiver"]["declaration"]["anchor"]["expected_text"],
            "A"
        );
        let copy = move_artifacts::apply(&repo, &result);
        cargo_check(&copy);
    }
}

#[test]
fn unknown_conditions_attributes_missing_enums_and_malformed_derives_still_block() {
    for declarations in [
        "",
        "pub enum Outcome { #[cfg(undeclared)] Pass, Fail(u32) }",
        "pub enum Outcome { Pass, Fail(#[cfg_attr(undeclared, allow(dead_code))] u32) }",
        "pub enum Outcome<T> { Pass, Fail(T) }",
        "#[cfg_attr(undeclared, derive(Custom))] pub enum Outcome { Pass, Fail(u32) }",
        "#[arbitrary] pub enum Outcome { Pass, Fail(u32) }",
        "#[derive(Custom Other)] pub enum Outcome { Pass, Fail(u32) }",
        "#[derive()] pub enum Outcome { Pass, Fail(u32) }",
        "#[derive(custom::)] pub enum Outcome { Pass, Fail(u32) }",
        "#[derive(custom: :Serialize)] pub enum Outcome { Pass, Fail(u32) }",
        "#[derive(,Custom)] pub enum Outcome { Pass, Fail(u32) }",
    ] {
        let repo = fixture(declarations);
        let result = run(&repo, request(&repo));
        assert_eq!(
            result["plan"]["applicable"], false,
            "{declarations}: {result}"
        );
        for artifact in ["edits", "created_files", "patch"] {
            assert!(result["plan"][artifact].is_null());
        }
        assert!(
            !result["plan"]["binding_proofs"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|p| p["classification"] == "variant_path"),
            "{result}"
        );
        if declarations.contains("cfg_attr") {
            assert!(
                result["plan"]["resolution_coverage"]["context_evaluations"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|e| e["reason"] == "undeclared_cfg_atom"
                        && e["fact_class"] == "nominal_identity"),
                "{result}"
            );
        }
    }
}

#[test]
fn final_variant_shadow_or_unknown_cfg_attribute_is_not_original_identity() {
    for destination in [
        "pub enum Outcome { Pass, Fail(u32) }",
        "#[cfg_attr(undeclared, derive(Custom))] struct Other;",
    ] {
        let repo = fixture("pub enum Outcome { Pass, Fail(u32) }");
        repo.write(
            "cases/layout/lib.rs",
            "mod source; mod destination; pub enum Outcome { Pass, Fail(u32) }",
        );
        repo.write("cases/layout/destination.rs", destination);
        // The explicit crate path does not see an unrelated same-name destination
        // enum; a local path does, and needs must compare their actual identities.
        let text = SELECTED.replace("crate::Outcome", "Outcome");
        let mut args = request(&repo);
        repo.write(
            "cases/layout/source.rs",
            &format!("use crate::Outcome; {text}"),
        );
        args["moves"][0]["item"] = move_artifacts::anchor(&repo, "cases/layout/source.rs", &text);
        args["moves"][0]["destination"] =
            json!({"kind":"existing","path":"cases/layout/destination.rs"});
        let result = run(&repo, args);
        assert_eq!(
            result["plan"]["applicable"], false,
            "{destination}: {result}"
        );
        assert!(result["plan"]["patch"].is_null());
    }
}
