#[path = "support/fixture_gen.rs"]
mod fixture_gen;
#[path = "support/move_artifacts.rs"]
mod move_artifacts;
use fixture_gen::{Fixture, observe};
use move_artifacts::{anchor, apply};
use rust_sitter_mcp::{engine::Engine, move_plan::MoveRequest};
use serde_json::{Value, json};
use std::{fs, process::Command, sync::atomic::AtomicBool};

fn run(repo: &Fixture, args: Value) -> Value {
    let before = observe(&repo.0);
    let request: MoveRequest = serde_json::from_value(args).unwrap();
    let result = Engine::new(repo.0.clone())
        .unwrap()
        .move_item(request, &AtomicBool::new(false));
    assert_eq!(observe(&repo.0), before);
    serde_json::to_value(result).unwrap()
}
fn fixture(item: &str) -> Fixture {
    let repo = Fixture::generate();
    repo.write("cases/facade/lib.rs", "pub mod worker;\nmod client;\n");
    repo.write("cases/facade/worker.rs", &format!("{item}\n"));
    repo.write(
        "cases/facade/client.rs",
        "use crate::worker::selected as work;\npub fn call() -> u8 { work() }\n",
    );
    repo
}
fn request(repo: &Fixture, item: &str) -> Value {
    json!({"repo_path":repo.0,"crate_root":"cases/facade/lib.rs","paths":["cases/facade"],"limits":{"diagnostic_count":1000},"moves":[{"item":anchor(repo,"cases/facade/worker.rs",item),"destination":{"kind":"new_child","parent_path":"cases/facade/worker.rs","path":"cases/facade/worker/detail.rs"}}]})
}
fn withheld(result: &Value) {
    assert_eq!(result["plan"]["applicable"], false, "{result}");
    for key in ["edits", "created_files", "patch"] {
        assert!(result["plan"][key].is_null(), "{result}");
    }
}
fn compile(copy: &Fixture, label: &str) {
    copy.write("facade-check/Cargo.toml", "[package]\nname = \"facade-check\"\nversion = \"0.1.0\"\nedition = \"2024\"\n[lib]\npath = \"../cases/facade/lib.rs\"\n");
    let output = Command::new("cargo")
        .current_dir(copy.0.join("facade-check"))
        .args(["check", "--all-targets", "--offline"])
        .output()
        .unwrap();
    fs::write(
        format!("/tmp/facade-{label}-check.log"),
        [&output.stdout[..], &output.stderr[..]].concat(),
    )
    .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
fn facade_request(repo: &Fixture, item: &str) -> Value {
    let mut args = request(repo, item);
    args["parent_facade"] = json!(true);
    args
}
#[test]
fn facade_visibility_preserves_private_parent_crate_and_public_regions() {
    for (visibility, facade, moved) in [
        ("", "use", "pub(super)"),
        ("pub(super) ", "pub(crate) use", "pub(crate)"),
        ("pub(crate) ", "pub(crate) use", "pub(crate)"),
        ("pub ", "pub use", "pub"),
    ] {
        let item = format!("{visibility}fn selected() -> u8 {{ 7 }}");
        let repo = fixture(&item);
        if visibility.is_empty() {
            repo.write("cases/facade/client.rs", "fn unrelated() {}\n");
        }
        repo.write(
            "cases/facade/worker.rs",
            &format!("{item}\nfn retained() -> u8 {{ selected() }}\n"),
        );
        let result = run(&repo, facade_request(&repo, &item));
        assert_eq!(result["plan"]["applicable"], true, "{visibility}: {result}");
        let copy = apply(&repo, &result);
        let parent = fs::read_to_string(copy.0.join("cases/facade/worker.rs")).unwrap();
        let child = fs::read_to_string(copy.0.join("cases/facade/worker/detail.rs")).unwrap();
        assert!(
            parent.contains(&format!("{facade} self::detail::selected;")),
            "{parent}"
        );
        assert!(child.contains(&format!("{moved} fn selected")), "{child}");
        assert!(!parent.contains("pub mod detail"));
        compile(&copy, "regions");
    }
}
#[test]
fn named_reexport_aliases_and_unit_tuple_constructors_keep_their_routes() {
    for item in [
        "pub struct Selected;",
        "pub struct Selected(pub u8);",
        "pub enum Selected { Value }",
    ] {
        let repo = fixture(item);
        repo.write(
            "cases/facade/lib.rs",
            "pub mod worker;\npub mod bridge;\nmod client;\n",
        );
        repo.write(
            "cases/facade/bridge.rs",
            "pub use crate::worker::Selected as Alias;\n",
        );
        let expression = if item.contains("pub u8") {
            "Alias(7)"
        } else if item.contains("enum") {
            "Alias::Value"
        } else {
            "Alias"
        };
        repo.write(
            "cases/facade/client.rs",
            &format!("use crate::bridge::Alias;\nfn call() {{ let _: Alias = {expression}; }}\n"),
        );
        let result = run(&repo, facade_request(&repo, item));
        assert_eq!(result["plan"]["applicable"], true, "{item}: {result}");
        let copy = apply(&repo, &result);
        for path in ["cases/facade/client.rs", "cases/facade/bridge.rs"] {
            assert_eq!(
                fs::read_to_string(copy.0.join(path)).unwrap(),
                fs::read_to_string(repo.0.join(path)).unwrap()
            );
        }
        compile(&copy, "constructors");
    }
}
#[test]
fn unique_direct_glob_keeps_the_same_exported_binding() {
    let item = "pub fn selected() -> u8 { 7 }";
    let repo = fixture(item);
    repo.write(
        "cases/facade/client.rs",
        "use crate::worker::*;\nfn call() -> u8 { selected() }\n",
    );
    let result = run(&repo, facade_request(&repo, item));
    assert_eq!(result["plan"]["applicable"], true, "{result}");
    let copy = apply(&repo, &result);
    assert_eq!(
        fs::read_to_string(copy.0.join("cases/facade/client.rs")).unwrap(),
        fs::read_to_string(repo.0.join("cases/facade/client.rs")).unwrap()
    );
    compile(&copy, "glob");
}
#[test]
fn conflicting_aliases_conditional_routes_namespaces_and_private_fields_refuse() {
    for case in [
        "alias",
        "glob",
        "conditional_item",
        "conditional_route",
        "macro_namespace",
        "private_field",
        "private_tuple",
        "private_consumer",
        "ambiguous_alias",
        "external_glob",
        "competing_glob",
        "conditional_glob",
    ] {
        let item = match case {
            "macro_namespace" => "macro_rules! selected { () => {} }",
            "private_field" => "pub struct selected { secret: u8 }",
            "private_tuple" => "pub struct selected(u8);",
            "private_consumer" => "fn selected() -> u8 { 7 }",
            _ => "pub fn selected() -> u8 { 7 }",
        };
        let repo = fixture(item);
        match case {
            "alias" => repo.write("cases/facade/worker.rs", &format!("use crate::other::selected;\n{item}\n")),
            "glob" => repo.write("cases/facade/worker.rs", &format!("use crate::other::*;\n{item}\n")),
            "conditional_item" => repo.write("cases/facade/worker.rs", &format!("#[cfg(feature = \"extra\")]\n{item}\n")),
            "conditional_route" => repo.write("cases/facade/lib.rs", "#[cfg(feature = \"extra\")] pub mod worker;\nmod client;\n"),
            "ambiguous_alias" => repo.write("cases/facade/client.rs", "use crate::worker::selected as work;\nuse crate::other::selected as work;\nfn call() -> u8 { work() }\n"),
            "external_glob" => repo.write("cases/facade/client.rs", "use external::*;\nuse crate::worker::*;\nfn call() -> u8 { selected() }\n"),
            "competing_glob" => { repo.write("cases/facade/lib.rs", "pub mod worker;\nmod client;\nmod other;\n"); repo.write("cases/facade/other.rs", "pub fn selected() -> u8 { 8 }\n"); repo.write("cases/facade/client.rs", "use crate::other::*;\nuse crate::worker::*;\nfn call() -> u8 { selected() }\n"); },
            "conditional_glob" => repo.write("cases/facade/client.rs", "#[cfg(feature = \"extra\")] use crate::worker::*;\nfn call() -> u8 { selected() }\n"),
            _ => {}
        }
        let result = run(&repo, facade_request(&repo, item));
        withheld(&result);
        assert!(
            result["plan"]["blockers"]
                .as_array()
                .is_some_and(|b| !b.is_empty())
                || !result["error"].is_null(),
            "{case}: {result}"
        );
    }
}
#[test]
fn facade_does_not_waive_macro_derive_or_moved_pattern_audits() {
    for item in [
        "pub fn selected() -> u8 { unknown!() }",
        "pub fn selected() -> u8 { matches!(1, 1) as u8 }",
        "pub enum selected { Value }",
    ] {
        let repo = fixture(item);
        if item.contains("enum") {
            repo.write(
                "cases/facade/worker.rs",
                &format!("#[derive(Unknown)]\n{item}\n"),
            );
            repo.write(
                "cases/facade/client.rs",
                "fn call(_: crate::worker::selected) {}\n",
            );
        }
        let result = run(&repo, facade_request(&repo, item));
        withheld(&result);
    }
}
#[test]
fn retaining_or_widening_facade_rewrites_withholds_artifacts() {
    let item = "fn selected() -> u8 { 7 }";
    let repo = fixture(item);
    repo.write("cases/facade/client.rs", "fn unrelated() {}\n");
    let args = facade_request(&repo, item);
    let baseline = run(&repo, args.clone());
    assert_eq!(baseline["plan"]["applicable"], true, "{baseline}");
    for kind in ["parent_facade", "visibility"] {
        let rewrite = baseline["plan"]["rewrites"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["kind"] == kind)
            .unwrap();
        let mut changed = args.clone();
        changed["rewrite_overrides"] =
            json!([{"target":rewrite["target"],"action":"retain","replacement_text":null}]);
        withheld(&run(&repo, changed));
        if kind == "visibility" {
            let mut changed = args.clone();
            changed["rewrite_overrides"] = json!([{"target":rewrite["target"],"action":"replace","replacement_text":"pub(crate) "}]);
            withheld(&run(&repo, changed));
        }
    }
}
#[test]
fn nested_super_and_self_regions_are_translated_without_widening() {
    for (visibility, facade, moved) in [
        ("pub(super)", "pub(super) use", "pub(in crate::outer)"),
        ("pub(self)", "use", "pub(super)"),
    ] {
        let repo = Fixture::generate();
        repo.write("cases/facade/lib.rs", "mod outer;\n");
        repo.write("cases/facade/outer.rs", "mod worker;\n");
        let item = format!("{visibility} fn selected() -> u8 {{ 7 }}");
        repo.write(
            "cases/facade/outer/worker.rs",
            &format!("{item}\nfn retained() -> u8 {{ selected() }}\n"),
        );
        let args = json!({"repo_path":repo.0,"crate_root":"cases/facade/lib.rs","paths":["cases/facade"],"parent_facade":true,"moves":[{"item":anchor(&repo,"cases/facade/outer/worker.rs",&item),"destination":{"kind":"new_child","parent_path":"cases/facade/outer/worker.rs","path":"cases/facade/outer/worker/detail.rs"}}]});
        let result = run(&repo, args);
        assert_eq!(result["plan"]["applicable"], true, "{result}");
        let copy = apply(&repo, &result);
        assert!(
            fs::read_to_string(copy.0.join("cases/facade/outer/worker.rs"))
                .unwrap()
                .contains(&format!("{facade} self::detail::selected;"))
        );
        assert!(
            fs::read_to_string(copy.0.join("cases/facade/outer/worker/detail.rs"))
                .unwrap()
                .contains(&format!("{moved} fn selected"))
        );
        compile(&copy, "nested-regions");
    }
}
#[test]
fn declared_active_conditions_do_not_authorize_conditional_facade_routes() {
    let item = "pub fn selected() -> u8 { 7 }";
    for case in ["item", "parent", "alias"] {
        let repo = fixture(item);
        match case {
            "item" => repo.write(
                "cases/facade/worker.rs",
                &format!("#[cfg(feature = \"extra\")]\n{item}\n"),
            ),
            "parent" => repo.write(
                "cases/facade/lib.rs",
                "#[cfg(feature = \"extra\")] pub mod worker;\nmod client;\n",
            ),
            "alias" => {
                repo.write(
                    "cases/facade/lib.rs",
                    "pub mod worker;\nmod client;\npub mod bridge;\n",
                );
                repo.write(
                    "cases/facade/bridge.rs",
                    "#[cfg(feature = \"extra\")] pub use crate::worker::selected as work;\n",
                );
                repo.write(
                    "cases/facade/client.rs",
                    "use crate::bridge::work;\nfn call() -> u8 { work() }\n",
                );
            }
            _ => unreachable!(),
        }
        let mut args = facade_request(&repo, item);
        args["semantic_configuration"] = json!({"crates":[{"name":"fixture","root_file":"cases/facade/lib.rs","edition":"2024","features":["extra"],"cfg":[],"dependencies":[]}]});
        withheld(&run(&repo, args));
    }
}
#[test]
fn a_facaded_batch_preserves_cross_item_access_and_final_local_binders() {
    let first = "pub fn selected() -> u8 { companion() }";
    let second = "fn companion() -> u8 { let (left, right) = (3, 4); left + right }";
    let repo = fixture(first);
    repo.write("cases/facade/worker.rs", &format!("{first}\n{second}\n"));
    let mut args = facade_request(&repo, first);
    let mut companion = args["moves"][0].clone();
    companion["item"] = anchor(&repo, "cases/facade/worker.rs", second);
    args["moves"].as_array_mut().unwrap().push(companion);
    let result = run(&repo, args);
    assert_eq!(result["plan"]["applicable"], true, "{result}");
    let copy = apply(&repo, &result);
    compile(&copy, "batch-binders");
}
#[test]
fn public_parent_facade_preserves_old_paths_in_a_disposable_crate() {
    let item = "pub fn selected() -> u8 { 7 }";
    let repo = fixture(item);
    repo.write(
        "cases/facade/worker.rs",
        &format!("{item}\npub fn retained() -> u8 {{ selected() }}\nmod tests;\n"),
    );
    repo.write(
        "cases/facade/worker/tests.rs",
        "use super::selected;\n#[test] fn old_path() { let _ = selected(); }\n",
    );
    let mut args = request(&repo, item);
    args["parent_facade"] = json!(true);
    let result = run(&repo, args);
    assert_eq!(result["plan"]["applicable"], true, "{result}");
    let copy = apply(&repo, &result);
    let parent = fs::read_to_string(copy.0.join("cases/facade/worker.rs")).unwrap();
    assert!(
        parent.contains("pub use self::detail::selected;"),
        "{parent}"
    );
    assert!(parent.contains("mod detail;"), "{parent}");
    assert!(!parent.contains("pub mod detail"));
    assert_eq!(
        fs::read_to_string(copy.0.join("cases/facade/client.rs")).unwrap(),
        fs::read_to_string(repo.0.join("cases/facade/client.rs")).unwrap()
    );
    copy.write("facade-check/Cargo.toml", "[package]\nname = \"facade-check\"\nversion = \"0.1.0\"\nedition = \"2024\"\n[lib]\npath = \"../cases/facade/lib.rs\"\n[[test]]\nname = \"external_client\"\npath = \"external_client.rs\"\n");
    copy.write("facade-check/external_client.rs", "use facade_check::worker::selected as old_name;\n#[test] fn old_export() { assert_eq!(old_name(), 7); }\n");
    let output = Command::new("cargo")
        .current_dir(copy.0.join("facade-check"))
        .args(["check", "--all-targets", "--offline"])
        .output()
        .unwrap();
    fs::write(
        "/tmp/facade-disposable-check.log",
        [&output.stdout[..], &output.stderr[..]].concat(),
    )
    .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
#[test]
fn selected_attribute_facade_admission_preserves_public_path_gating() {
    let item = "pub fn selected() -> u8 { 7 }";
    for (label, attribute) in [
        ("cfg_any", "#[cfg(any())]"),
        ("cfg_feature", "#[cfg(feature = \"x\")]"),
        ("cfg_attr", "#[cfg_attr(feature = \"x\", cfg(any()))]"),
        ("provider", "#[custom_attr]"),
        ("inline", "#[inline]"),
    ] {
        let repo = fixture(item);
        repo.write("cases/facade/worker.rs", &format!("{attribute}\n{item}\n"));
        let result = run(&repo, facade_request(&repo, item));
        println!("attribute case {label}: {result}");
        let has_facade = result["plan"]["rewrites"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["kind"] == "parent_facade");
        let has_public_path_need = result["plan"]["decisions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["reason"] == "public_path_change");
        if label == "inline" {
            assert_eq!(result["plan"]["applicable"], true, "{result}");
            assert!(has_facade, "{result}");
            assert!(!has_public_path_need, "{result}");
            let copy = apply(&repo, &result);
            let parent = fs::read_to_string(copy.0.join("cases/facade/worker.rs")).unwrap();
            let child = fs::read_to_string(copy.0.join("cases/facade/worker/detail.rs")).unwrap();
            assert!(
                parent.contains("pub use self::detail::selected;"),
                "{parent}"
            );
            assert!(child.contains(&format!("{attribute}\n{item}")), "{child}");
            compile(&copy, "inline");
        } else {
            withheld(&result);
            assert!(!has_facade, "{label}: {result}");
            assert!(has_public_path_need, "{label}: {result}");
            assert!(
                result["plan"]["decisions"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|d| d["reason"] == "conditional_or_inherited_context"
                        && d["refusal_basis"].as_array().is_some_and(|basis| basis
                            .iter()
                            .any(|b| b["class"] == "parent_facade_unproved"))),
                "{label}: {result}"
            );
        }
    }
}
#[test]
fn private_child_without_facade_retains_public_path_refusal() {
    let item = "pub fn selected() -> u8 { 7 }";
    let repo = fixture(item);
    let result = run(&repo, request(&repo, item));
    withheld(&result);
    assert!(
        result["plan"]["decisions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["reason"] == "public_path_change"),
        "{result}"
    );
}
