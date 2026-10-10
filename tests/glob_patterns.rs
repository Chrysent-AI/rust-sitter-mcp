#[path = "support/fixture_gen.rs"]
mod fixture_gen;
#[path = "support/move_artifacts.rs"]
mod move_artifacts;
use fixture_gen::{Fixture, observe};
use move_artifacts::anchor;
use rust_sitter_mcp::{engine::Engine, move_plan::MoveRequest};
use serde_json::{Value, json};
use std::sync::atomic::AtomicBool;

const FUNCTION: &str = "fn selected(input: u8) -> u8 { let [binding] = [input]; binding + 1 }";
const ROOT: &str = "cases/patterns/lib.rs";
const SOURCE: &str = "cases/patterns/source.rs";
const DESTINATION: &str = "cases/patterns/destination.rs";
const EXPORTS: &str = "cases/patterns/exports.rs";
fn fixture(import: &str, exports: &str, destination: &str, function: &str) -> Fixture {
    let repo = Fixture::generate();
    repo.write(
        ROOT,
        "mod source; mod destination; pub(crate) mod exports;\n",
    );
    repo.write(SOURCE, &format!("{import}\n{function}\n"));
    repo.write(DESTINATION, destination);
    repo.write(EXPORTS, exports);
    repo
}
fn request(repo: &Fixture, function: &str) -> Value {
    json!({"repo_path":repo.0,"crate_root":ROOT,"paths":["cases/patterns"],"limits":{"text_bytes":0,"diagnostic_count":100000},"moves":[{"item":anchor(repo,SOURCE,function),"destination":{"kind":"existing","path":DESTINATION}}]})
}
fn run(repo: &Fixture, args: Value) -> Value {
    let before = observe(&repo.0);
    let request: MoveRequest = serde_json::from_value(args).unwrap();
    let result = serde_json::to_value(
        Engine::new(repo.0.clone())
            .unwrap()
            .move_item(request, &AtomicBool::new(false)),
    )
    .unwrap();
    assert_eq!(observe(&repo.0), before);
    assert_eq!(result["plan"]["integrity"]["semantic"], "not_performed");
    result
}
fn withheld(result: &Value) {
    assert_eq!(result["plan"]["applicable"], false, "{result}");
    for field in ["edits", "created_files", "patch", "directory_preconditions"] {
        assert!(result["plan"][field].is_null(), "{field}: {result}");
    }
}
fn anchored(result: &Value, function: &str) {
    withheld(result);
    assert!(
        result["plan"]["decisions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["reason"] == "lexical_context_unproved"
                && d["anchors"][0]["path"] == SOURCE
                && d["anchors"][0]["expected_text"] == "binding"
                && d["item_ids"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|id| id.as_str().unwrap().starts_with("i/"))),
        "{function}: {result}"
    );
}
#[test]
fn direct_unrelated_export_allows_an_applicable_lossless_move() {
    let repo = fixture(
        "use crate::exports::*;",
        "pub(crate) const other: u8 = 1;\n",
        "",
        FUNCTION,
    );
    let result = run(&repo, request(&repo, FUNCTION));
    assert_eq!(result["plan"]["applicable"], true, "{result}");
    let applied = move_artifacts::apply(&repo, &result);
    assert!(
        std::fs::read_to_string(applied.0.join(DESTINATION))
            .unwrap()
            .contains(FUNCTION)
    );
    let copy = applied.copy();
    let output = std::process::Command::new("rustc")
        .current_dir(&copy.0)
        .args([
            "--edition=2024",
            "--crate-type=lib",
            "--emit=metadata",
            ROOT,
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
#[test]
fn supported_namespace_distinctions_grouped_and_multiple_routes() {
    for exports in [
        "pub(crate) fn binding() {}",
        "pub(crate) type binding = u8;",
        "pub(crate) struct binding { pub(crate) field: u8 }",
        "pub(crate) enum Choice { binding }",
    ] {
        let repo = fixture(
            "use crate::{exports::*, exports::*};",
            exports,
            "",
            FUNCTION,
        );
        let result = run(&repo, request(&repo, FUNCTION));
        assert_eq!(result["plan"]["applicable"], true, "{exports}: {result}");
    }
    let repo = fixture(
        "use crate::exports::*; use crate::second::*;",
        "pub(crate) const other: u8 = 1;",
        "",
        FUNCTION,
    );
    repo.write(
        ROOT,
        "mod source; mod destination; pub(crate) mod exports; pub(crate) mod second;",
    );
    repo.write(
        "cases/patterns/second.rs",
        "pub(crate) const extra: u8 = 2;",
    );
    assert_eq!(
        run(&repo, request(&repo, FUNCTION))["plan"]["applicable"],
        true
    );
}
#[test]
fn source_constants_constructors_and_named_imports_remain_uncertain() {
    for exports in [
        "pub(crate) const binding: u8 = 1;",
        "pub(crate) static binding: u8 = 1;",
        "pub(crate) struct binding;",
        "pub(crate) struct binding(pub(crate) u8);",
        "const binding: u8 = 1;",
    ] {
        let repo = fixture("use crate::exports::*;", exports, "", FUNCTION);
        anchored(&run(&repo, request(&repo, FUNCTION)), FUNCTION);
    }
    let repo = fixture(
        "use crate::exports::*; use crate::exports::other as binding;",
        "pub(crate) const other: u8 = 1;",
        "",
        FUNCTION,
    );
    anchored(&run(&repo, request(&repo, FUNCTION)), FUNCTION);
}
#[test]
fn destination_competitors_and_unknown_producers_are_final_refusals() {
    for destination in [
        "const binding: u8 = 0;",
        "struct binding;",
        "struct binding(u8);",
        "use crate::exports::other as binding;",
        "use missing::*;",
        "produce!();",
        "#[provider]\nfn unrelated() {}",
        "#[cfg(feature = \"optional\")] const binding: u8 = 0;",
    ] {
        let repo = fixture(
            "use crate::exports::*;",
            "pub(crate) const other: u8 = 1;",
            destination,
            FUNCTION,
        );
        anchored(&run(&repo, request(&repo, FUNCTION)), FUNCTION);
    }
}
#[test]
fn unread_binders_are_revalidated_and_unassisted_history_is_unchanged() {
    let function = "fn selected(input: u8) -> u8 { let [binding] = [input]; input + 1 }";
    let repo = fixture(
        "use crate::exports::*;",
        "pub(crate) const other: u8 = 1;",
        "const binding: u8 = 0;",
        function,
    );
    anchored(&run(&repo, request(&repo, function)), function);
    let repo = fixture(
        "",
        "pub(crate) const other: u8 = 1;",
        "const binding: u8 = 0;",
        FUNCTION,
    );
    // This pre-existing unassisted relocation limitation is not claimed repaired.
    assert_eq!(
        run(&repo, request(&repo, FUNCTION))["plan"]["applicable"],
        true
    );
}
#[test]
fn exporter_or_destination_companion_mutations_use_the_complete_overlay() {
    for destination in [EXPORTS, DESTINATION] {
        let repo = fixture(
            "use crate::exports::*;",
            "pub(crate) const other: u8 = 1;",
            "use crate::exports::*;",
            FUNCTION,
        );
        repo.write(
            ROOT,
            "mod source; mod destination; mod companion; pub(crate) mod exports;",
        );
        repo.write("cases/patterns/companion.rs", "const binding: u8 = 0;");
        let mut args = request(&repo, FUNCTION);
        args["moves"].as_array_mut().unwrap().push(json!({"item":anchor(&repo,"cases/patterns/companion.rs","const binding: u8 = 0;"),"destination":{"kind":"existing","path":destination}}));
        anchored(&run(&repo, args), FUNCTION);
    }
}
#[test]
fn unknown_routes_attributes_recovery_and_forwarding_never_prove_absence() {
    for exports in [
        "pub(crate) use crate::missing::*;",
        "pub(crate) use crate::source::*;",
        "pub(crate) use crate::source::selected;",
        "produce!();",
        "macro_rules! produce { () => {} }",
        "#[provider]\npub(crate) fn other() {}",
        "#[derive(Clone)]\npub(crate) struct Other;",
        "#[cfg(feature = \"optional\")]\npub(crate) const other: u8 = 1;",
        "pub(crate) const other: u8 = ;",
    ] {
        let repo = fixture("use crate::exports::*;", exports, "", FUNCTION);
        withheld(&run(&repo, request(&repo, FUNCTION)));
    }
    for import in [
        "use missing::*;",
        "use crate::exports::Choice::*;",
        "#[cfg(feature = \"optional\")]\nuse crate::exports::*;",
        "use crate::exports::*; use missing::*;",
    ] {
        let repo = fixture(import, "pub(crate) enum Choice { Other }", "", FUNCTION);
        withheld(&run(&repo, request(&repo, FUNCTION)));
    }
}
#[test]
fn ordinary_inline_and_named_alias_routes_are_supported() {
    let repo = fixture(
        "use crate::exports::{nested::*};",
        "pub(crate) mod nested { pub(crate) const other: u8 = 1; }",
        "",
        FUNCTION,
    );
    assert_eq!(
        run(&repo, request(&repo, FUNCTION))["plan"]["applicable"],
        true
    );
    let repo = fixture(
        "use crate::alias::*;",
        "pub(crate) const other: u8 = 1;",
        "",
        FUNCTION,
    );
    repo.write(
        ROOT,
        "mod source; mod destination; pub(crate) mod exports; use crate::exports as alias;",
    );
    assert_eq!(
        run(&repo, request(&repo, FUNCTION))["plan"]["applicable"],
        true
    );
}
#[test]
fn unadmitted_competing_and_conditional_module_layouts_remain_unknown() {
    for root in [
        "mod source; mod destination; #[cfg(feature = \"optional\")] pub(crate) mod exports;",
        "mod source; mod destination; #[path = \"exports.rs\"] pub(crate) mod exports;",
    ] {
        let repo = fixture(
            "use crate::exports::*;",
            "pub(crate) const other: u8 = 1;",
            "",
            FUNCTION,
        );
        repo.write(ROOT, root);
        withheld(&run(&repo, request(&repo, FUNCTION)));
    }
    let repo = fixture(
        "use crate::exports::*;",
        "pub(crate) const other: u8 = 1;",
        "",
        FUNCTION,
    );
    repo.write(
        "cases/patterns/exports/mod.rs",
        "pub(crate) const other: u8 = 1;",
    );
    withheld(&run(&repo, request(&repo, FUNCTION)));
    let repo = fixture(
        "use crate::exports::*;",
        "pub(crate) const other: u8 = 1;",
        "",
        FUNCTION,
    );
    let mut args = request(&repo, FUNCTION);
    args["paths"] = json!([ROOT, SOURCE, DESTINATION]);
    withheld(&run(&repo, args));
}
#[test]
fn nested_scopes_preserve_binding_identity_and_block_local_routes() {
    for function in [
        "fn selected(input: u8) -> u8 { let [binding] = [input]; { let binding = 2; binding + 1 }; binding + 1 }",
        "fn selected(input: u8) -> u8 { let (binding,) = (input,); binding + 1 }",
    ] {
        let repo = fixture(
            "use crate::exports::*;",
            "pub(crate) const other: u8 = 1;",
            "",
            function,
        );
        let result = run(&repo, request(&repo, function));
        assert_eq!(result["plan"]["applicable"], true, "{function}: {result}");
    }
    // A copied block import retains its independent unsupported-import veto,
    // even though the following composite binder itself has direct glob proof.
    let function = "fn selected(input: u8) -> u8 { use crate::exports::*; let [binding] = [input]; binding + 1 }";
    let repo = fixture(
        "use crate::exports::*;",
        "pub(crate) const other: u8 = 1;",
        "",
        function,
    );
    let result = run(&repo, request(&repo, function));
    withheld(&result);
    assert!(
        result["plan"]["decisions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["reason"] == "unsupported_construct")
    );
    assert!(
        !result["plan"]["decisions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["reason"] == "lexical_context_unproved"
                && d["anchors"][0]["expected_text"] == "binding")
    );
    for function in [
        "fn selected(input: u8) -> u8 { { const binding: u8 = 0; let [binding] = [input]; binding + 1 } }",
        "fn selected(input: u8) -> u8 { use crate::exports as local; use local::*; let [binding] = [input]; binding + 1 }",
        "fn selected(input: u8) -> u8 { use crate::exports::*; let [binding] = [input]; produce!(); binding + 1 }",
    ] {
        let repo = fixture(
            "use crate::exports::*;",
            "pub(crate) const other: u8 = 1;",
            "",
            function,
        );
        withheld(&run(&repo, request(&repo, function)));
    }
}
#[test]
fn inaccessible_and_provider_uncertain_alias_hops_are_unknown() {
    for route in [
        "mod hidden { mod exports { pub(crate) const other: u8 = 1; } }",
        "mod hidden { #[provider] /* attached */ pub(crate) use crate::exports as exports_alias; }",
        "mod hidden { #[cfg(feature = \"optional\")] pub(crate) use crate::exports as exports_alias; }",
    ] {
        let import = if route.contains("exports_alias") {
            "use crate::hidden::exports_alias::*;"
        } else {
            "use crate::hidden::exports::*;"
        };
        let repo = fixture(import, "pub(crate) const other: u8 = 1;", "", FUNCTION);
        repo.write(
            ROOT,
            &format!("mod source; mod destination; pub(crate) mod exports; {route}"),
        );
        withheld(&run(&repo, request(&repo, FUNCTION)));
    }
}
#[test]
fn synthesized_child_declarations_are_validated_and_schema_three_withholds_totally() {
    let repo = fixture(
        "use crate::exports::*;",
        "pub(crate) const other: u8 = 1;",
        "",
        FUNCTION,
    );
    let mut args = request(&repo, FUNCTION);
    args["moves"][0]["destination"] =
        json!({"kind":"new_child","path":"cases/patterns/source/new.rs","parent_path":SOURCE});
    let result = run(&repo, args);
    assert_eq!(result["schema_version"], 3);
    assert_eq!(result["plan"]["applicable"], true, "{result}");
    let repo = fixture(
        "use crate::exports::*;",
        "pub(crate) const binding: u8 = 0;",
        "",
        FUNCTION,
    );
    let mut args = request(&repo, FUNCTION);
    args["moves"][0]["destination"] =
        json!({"kind":"new_child","path":"cases/patterns/source/new.rs","parent_path":SOURCE});
    let result = run(&repo, args);
    assert_eq!(result["schema_version"], 3);
    withheld(&result);
}
