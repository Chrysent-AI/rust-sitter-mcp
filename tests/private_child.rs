#[path = "support/fixture_gen.rs"]
mod fixture_gen;
#[path = "support/move_artifacts.rs"]
mod move_artifacts;
#[path = "support/stdio_client.rs"]
mod stdio_client;
use fixture_gen::{Fixture, observe};
use move_artifacts::{anchor, apply};
use rust_sitter_mcp::{engine::Engine, move_plan::MoveRequest};
use serde_json::{Value, json};
use std::{
    fs,
    os::unix::fs::{PermissionsExt, symlink},
    sync::atomic::AtomicBool,
};
use stdio_client::Client;

fn fixture(layout: &str, source: &str) -> (Fixture, String, String) {
    let repo = Fixture::generate();
    let parent = match layout {
        "flat" => "cases/child/worker.rs",
        "mod" => "cases/child/worker/mod.rs",
        "root" => "cases/child/lib.rs",
        _ => unreachable!(),
    };
    if layout != "root" {
        repo.write("cases/child/lib.rs", "mod worker;\n");
    }
    repo.write(parent, source);
    let destination = if layout == "root" {
        "cases/child/detail.rs"
    } else {
        "cases/child/worker/detail.rs"
    };
    (repo, parent.into(), destination.into())
}
fn request(repo: &Fixture, parent: &str, dest: &str, texts: &[&str]) -> Value {
    json!({"repo_path":repo.0,"crate_root":"cases/child/lib.rs","paths":["cases/child"],"limits":{"diagnostic_count":1000},"moves":texts.iter().map(|text| json!({"item":anchor(repo,parent,text),"destination":{"kind":"new_child","parent_path":parent,"path":dest}})).collect::<Vec<_>>()})
}
fn run(repo: &Fixture, args: Value) -> Value {
    let before = observe(&repo.0);
    let result = Engine::new(repo.0.clone()).unwrap().move_item(
        serde_json::from_value::<MoveRequest>(args).unwrap(),
        &AtomicBool::new(false),
    );
    assert_eq!(observe(&repo.0), before);
    assert!(result.wire_bytes() <= result.limits.response_bytes);
    serde_json::to_value(result).unwrap()
}
fn withheld(result: &Value) {
    assert_eq!(result["schema_version"], 3, "{result}");
    assert_eq!(result["plan"]["applicable"], false, "{result}");
    assert!(result["plan"].get("directory_preconditions").is_some());
    for field in ["edits", "created_files", "directory_preconditions", "patch"] {
        assert!(result["plan"][field].is_null(), "{field}: {result}");
    }
}
#[test]
fn real_stdio_whole_function_layouts_absent_directory_and_narrow_facade_repair() {
    for layout in ["flat", "mod", "root"] {
        let text = "fn helper() -> u8 { 7 }";
        let facade = "pub fn facade() -> u8 { helper() }";
        let (repo, parent, destination) =
            fixture(layout, &format!("//! retained docs\n{text}\n{facade}\n"));
        let before = observe(&repo.0);
        if layout == "flat" {
            assert!(!repo.0.join("cases/child/worker").exists());
        }
        let result =
            Client::new().call("move_item", request(&repo, &parent, &destination, &[text]));
        assert_eq!(result["schema_version"], 3);
        let copy = apply(&repo, &result);
        assert_eq!(observe(&repo.0), before);
        assert_eq!(
            result["plan"]["directory_preconditions"][0]["observed_state"],
            if layout == "flat" {
                "absent"
            } else {
                "existing_directory"
            }
        );
        assert_eq!(
            result["plan"]["directory_preconditions"][0]["basis"]["kind"],
            match layout {
                "flat" => "flat_file_child",
                "mod" => "mod_rs_child",
                _ => "crate_root_child",
            }
        );
        let retained = fs::read_to_string(copy.0.join(&parent)).unwrap();
        assert!(retained.contains(facade));
        assert!(retained.contains("mod detail;"));
        assert!(!retained.contains("pub mod detail"));
        let child = fs::read_to_string(copy.0.join(destination)).unwrap();
        assert!(
            child.contains(if layout == "root" {
                "pub(crate) fn helper"
            } else {
                "pub(super) fn helper"
            }),
            "{child}"
        );
        let visibility = result["plan"]["rewrites"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["kind"] == "visibility")
            .unwrap();
        assert!(
            !visibility["visibility"]["consumers"]
                .as_array()
                .unwrap()
                .is_empty()
        );
    }
}
#[test]
fn real_stdio_generic_associated_wrapper_bytes_and_parent_imports_are_not_inherited() {
    for layout in ["flat", "mod", "root"] {
        let header = "impl<T> Record<T> where T: crate::Bound ";
        let member = "fn identity(value: T) -> T { value }";
        let source =
            format!("use crate::Record;\r\n{header}{{\r\n    /// café λ\r\n    {member}\r\n}}");
        let (repo, parent, destination) = fixture(layout, &source);
        if layout == "root" {
            repo.write(&parent, &format!("pub trait Bound {{}} pub struct Record<T>(T);\r\n{header}{{\r\n    /// café λ\r\n    {member}\r\n}}"));
        } else {
            repo.write(
                "cases/child/lib.rs",
                "mod worker; pub trait Bound {} pub struct Record<T>(T);\n",
            );
        }
        let mut args = request(&repo, &parent, &destination, &[member]);
        args["moves"][0]["enclosing_impl"] = anchor(&repo, &parent, header);
        let result = Client::new().call("move_item", args);
        let copy = apply(&repo, &result);
        let child = fs::read_to_string(copy.0.join(destination)).unwrap();
        assert!(child.contains(header), "{child}");
        assert!(child.contains("/// café λ\r\n"));
        assert!(
            child.contains(if layout == "root" {
                "pub(crate) fn identity"
            } else {
                "pub(super) fn identity"
            }),
            "{child}"
        );
        assert!(
            child.contains("use crate::Record;"),
            "explicit binding route required: {child}"
        );
    }
}
#[test]
fn existing_directory_private_dangling_reuse_same_batch_and_exact_trivia() {
    for declaration in ["", "mod detail;\r\n"] {
        let texts = ["fn first() { let é = 1; let x = é; }", "fn second() {}"];
        let (repo, parent, destination) = fixture(
            "flat",
            &format!(
                "//! retained\r\n{declaration}/** docs */\r\n#[inline]\r\n{} // trailing\r\n{}",
                texts[0], texts[1]
            ),
        );
        fs::create_dir(repo.0.join("cases/child/worker")).unwrap();
        fs::set_permissions(repo.0.join(&parent), fs::Permissions::from_mode(0o755)).unwrap();
        let result = run(&repo, request(&repo, &parent, &destination, &texts));
        let copy = apply(&repo, &result);
        assert_eq!(result["plan"]["created_files"].as_array().unwrap().len(), 1);
        assert_eq!(
            result["plan"]["directory_preconditions"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            result["plan"]["created_files"][0]["declaration_link"]["kind"],
            if declaration.is_empty() {
                "synthesized"
            } else {
                "reused"
            }
        );
        let child = fs::read_to_string(copy.0.join(destination)).unwrap();
        assert!(child.contains("/** docs */\r\n#[inline]\r\n"));
        assert!(child.contains("// trailing\r\n"));
        assert!(child.contains(texts[0]) && child.contains(texts[1]));
        assert_eq!(
            fs::metadata(copy.0.join(parent))
                .unwrap()
                .permissions()
                .mode()
                & 0o111,
            0o111
        );
    }
}
#[test]
fn mixed_batches_version_selection_and_all_artifact_withholding() {
    let text = "fn selected() {}";
    let (repo, parent, destination) = fixture("flat", text);
    repo.write("cases/child/lib.rs", "mod worker; mod other;\n");
    repo.write("cases/child/other.rs", "fn another() {}\n");
    let mut args = request(&repo, &parent, &destination, &[text]);
    args["moves"].as_array_mut().unwrap().push(json!({"item":anchor(&repo,"cases/child/other.rs","fn another() {}"),"destination":{"kind":"new_sibling","parent_path":"cases/child/lib.rs","path":"cases/child/sibling.rs"}}));
    let complete = run(&repo, args.clone());
    apply(&repo, &complete);
    assert_eq!(
        complete["plan"]["directory_preconditions"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    let mut legacy = args.clone();
    legacy["moves"].as_array_mut().unwrap().remove(0);
    let result = run(&repo, legacy);
    assert_eq!(result["schema_version"], 2);
    assert!(result["plan"].get("directory_preconditions").is_none());
    apply(&repo, &result);
    let mut blocked = args.clone();
    repo.write("cases/child/other.rs", "fn another() { missing!(); }\n");
    blocked["moves"][1]["item"] = anchor(
        &repo,
        "cases/child/other.rs",
        "fn another() { missing!(); }",
    );
    let result = run(&repo, blocked);
    assert_eq!(result["plan"]["state"], "blocked");
    withheld(&result);
    args["moves"][1]["item"]["expected_text"] = json!("stale");
    withheld(&run(&repo, args));
    let mut invalid = request(&repo, &parent, &destination, &[text]);
    invalid["max_moves"] = json!(0);
    withheld(&run(&repo, invalid));
    let mut invalid = request(&repo, &parent, &destination, &[text]);
    invalid["moves"][0]["destination"]["parent_path"] = json!("cases/child/lib.rs");
    withheld(&run(&repo, invalid));
}
#[test]
fn child_safety_admission_geometry_declarations_and_proof_matrix() {
    for case in [
        "ignored_dir",
        "ignored_leaf",
        "pruned_negation",
        "glob",
        "symlink",
        "non_dir",
        "case_dir",
        "case_leaf",
        "nested",
        "competing",
        "competing_case",
        "competing_symlink",
        "competing_nested",
        "hard_boundary",
        "public",
        "restricted",
        "inline",
        "remapped",
        "cfg_unknown",
        "duplicate",
        "parent_unadmitted",
        "root_unadmitted",
        "wrong_path",
        "new_mod",
        "missing_chain",
        "public_path",
        "macro",
        "field",
        "outside",
    ] {
        let text = "fn selected() {}";
        let (repo, parent, destination) = fixture("flat", text);
        let mut args = request(&repo, &parent, &destination, &[text]);
        match case {
            "ignored_dir" => repo.write("cases/child/.gitignore", "worker/\n"),
            "ignored_leaf" => repo.write("cases/child/.gitignore", "worker/detail.rs\n"),
            "pruned_negation" => {
                repo.write("cases/child/.gitignore", "worker/\n!worker/detail.rs\n")
            }
            "glob" => args["globs"] = json!(["cases/child/*.rs"]),
            "symlink" => symlink(".", repo.0.join("cases/child/worker")).unwrap(),
            "non_dir" => repo.write("cases/child/worker", "not a directory"),
            "case_dir" => fs::create_dir(repo.0.join("cases/child/Worker")).unwrap(),
            "case_leaf" => repo.write("cases/child/worker/Detail.rs", ""),
            "nested" => {
                fs::create_dir(repo.0.join("cases/child/worker")).unwrap();
                fs::create_dir(repo.0.join("cases/child/worker/.git")).unwrap();
            }
            "competing" => repo.write("cases/child/worker/detail/mod.rs", ""),
            "competing_case" => repo.write("cases/child/worker/Detail/mod.rs", ""),
            "competing_symlink" => {
                fs::create_dir(repo.0.join("cases/child/worker")).unwrap();
                symlink(".", repo.0.join("cases/child/worker/detail")).unwrap();
            }
            "competing_nested" => repo.write("cases/child/worker/detail/.git", "boundary"),
            "hard_boundary" => {
                repo.write("cases/child/target.rs", text);
                args["moves"][0]["item"] = anchor(&repo, "cases/child/target.rs", text);
                args["moves"][0]["destination"] = json!({"kind":"new_child","parent_path":"cases/child/target.rs","path":"cases/child/target/detail.rs"});
            }
            "public" => repo.write(&parent, &format!("pub mod detail;\n{text}")),
            "restricted" => repo.write(&parent, &format!("pub(super) mod detail;\n{text}")),
            "inline" => repo.write(&parent, &format!("mod detail {{}}\n{text}")),
            "remapped" => repo.write(
                &parent,
                &format!("#[path=\"elsewhere.rs\"] mod detail;\n{text}"),
            ),
            "cfg_unknown" => repo.write(
                &parent,
                &format!("#[cfg(feature=\"flag\")] mod detail;\n{text}"),
            ),
            "duplicate" => repo.write(&parent, &format!("mod detail; mod detail;\n{text}")),
            "parent_unadmitted" => args["paths"] = json!(["cases/child/lib.rs"]),
            "root_unadmitted" => args["paths"] = json!([parent]),
            "wrong_path" => {
                args["moves"][0]["destination"]["path"] = json!("cases/child/detail.rs")
            }
            "new_mod" => {
                args["moves"][0]["destination"]["path"] = json!("cases/child/worker/detail/mod.rs")
            }
            "missing_chain" => repo.write("cases/child/lib.rs", ""),
            "public_path" => {
                repo.write("cases/child/lib.rs", "pub mod worker;");
                repo.write(&parent, "pub fn selected() {}");
                args["moves"][0]["item"] = anchor(&repo, &parent, "pub fn selected() {}");
            }
            "macro" => {
                repo.write(&parent, "fn selected() { missing!(); }");
                args["moves"][0]["item"] = anchor(&repo, &parent, "fn selected() { missing!(); }");
            }
            "field" => {
                repo.write(
                    &parent,
                    "struct Record { value: u8 } fn selected(r: Record) -> u8 { r.value }",
                );
                args["moves"][0]["item"] =
                    anchor(&repo, &parent, "fn selected(r: Record) -> u8 { r.value }");
            }
            "outside" => {
                repo.write(
                    "cases/child/lib.rs",
                    "mod worker; fn outside() { crate::worker::selected(); }",
                );
            }
            _ => unreachable!(),
        }
        // Re-anchor after introducing declaration prologues.
        if [
            "public",
            "restricted",
            "inline",
            "remapped",
            "cfg_unknown",
            "duplicate",
        ]
        .contains(&case)
        {
            args["moves"][0]["item"] = anchor(&repo, &parent, text);
        }
        let result = run(&repo, args);
        withheld(&result);
    }
}
#[test]
fn configured_private_dangling_declaration_reuses_only_proved_active_context() {
    let text = "fn selected() {}";
    let (repo, parent, destination) = fixture(
        "flat",
        &format!("#[cfg(feature=\"flag\")] mod detail;\n{text}"),
    );
    let mut args = request(&repo, &parent, &destination, &[text]);
    args["semantic_configuration"] = json!({"crates":[{"name":"fixture","root_file":"cases/child/lib.rs","edition":"2024","features":["flag"],"cfg":[],"dependencies":[]}]});
    let result = run(&repo, args);
    apply(&repo, &result);
    assert_eq!(
        result["plan"]["created_files"][0]["declaration_link"]["kind"],
        "reused"
    );
}
#[test]
fn real_stdio_export_new_child_is_inactive_and_exact_roundtrip() {
    let text = "fn selected() {}";
    let (repo, parent, destination) = fixture("flat", text);
    let before = observe(&repo.0);
    let mut client = Client::new();
    let tools = client.rpc("tools/list", json!({}));
    assert_eq!(tools["tools"].as_array().unwrap().len(), 7);
    for name in ["move_item", "export_move_request"] {
        let tool = tools["tools"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["name"] == name)
            .unwrap();
        assert!(tool["inputSchema"].to_string().contains("new_child"));
        if name == "move_item" {
            assert!(
                tool["outputSchema"]
                    .to_string()
                    .contains("directory_preconditions")
            );
        }
    }
    let advice = client.call("suggest_split",json!({"repo_path":repo.0,"crate_root":"cases/child/lib.rs","source_path":parent,"paths":["cases/child"],"retain_snapshot":true}));
    let item = &advice["inventory"][0];
    let exported = client.call("export_move_request",json!({"analysis_handle":advice["retention"]["analysis_handle"],"snapshot_id":advice["snapshot_id"],"selection":[{"unit_ref":{"analysis_id":advice["retention"]["analysis_id"],"item_id":item["id"]},"destination":{"kind":"new_child","parent_path":parent,"path":destination}}]}));
    assert_eq!(exported["submitted"], false);
    assert_eq!(exported["applicability"], "not_assessed");
    let hand = request(&repo, &parent, &destination, &[text]);
    assert_eq!(exported["request"]["moves"], hand["moves"]);
    let result = client.call("move_item", exported["request"].clone());
    apply(&repo, &result);
    assert_eq!(observe(&repo.0), before);
}

#[test]
fn schema_consumers_reject_unknown_or_child_versions_before_partial_application() {
    let (repo, parent, destination) = fixture("flat", "fn selected() {}");
    let before = observe(&repo.0);
    let mut client = Client::new();
    let args = request(&repo, &parent, &destination, &["fn selected() {}"]);
    let result = client.call("move_item", args.clone());
    assert_eq!(
        move_artifacts::require_schema(&result, &[2]),
        Err("unsupported artifact schema")
    );
    assert_eq!(move_artifacts::require_schema(&result, &[2, 3]), Ok(3));
    let mut unknown = result.clone();
    unknown["schema_version"] = json!(4);
    assert!(move_artifacts::require_schema(&unknown, &[2, 3]).is_err());
    let mut typed_failure = args;
    typed_failure["max_moves"] = json!(0);
    let failure = client.rpc(
        "tools/call",
        json!({"name":"move_item","arguments":typed_failure}),
    );
    assert_eq!(failure["isError"], true);
    withheld(&failure["structuredContent"]);
    assert_eq!(observe(&repo.0), before);
}

#[test]
fn repository_root_child_uses_existing_root_directory_without_fictional_ancestors() {
    let repo = Fixture::empty();
    repo.git(&["init", "--quiet", "--template=", "--initial-branch=main"]);
    let text = "fn standalone_root_helper() {}";
    repo.write("lib.rs", text);
    let result = Client::new().call("move_item", json!({"repo_path":repo.0,"crate_root":"lib.rs","paths":["."],"moves":[{"item":anchor(&repo,"lib.rs",text),"destination":{"kind":"new_child","parent_path":"lib.rs","path":"detail.rs"}}]}));
    apply(&repo, &result);
    assert_eq!(result["plan"]["directory_preconditions"][0]["path"], ".");
    assert_eq!(
        result["plan"]["directory_preconditions"][0]["observed_state"],
        "existing_directory"
    );
}

#[test]
fn shared_absent_directory_has_one_record_multiple_creations_and_batch_aliases_refuse() {
    let texts = ["fn first() {}", "fn second() {}"];
    let (repo, parent, destination) = fixture("flat", &texts.join("\n"));
    let mut args = request(&repo, &parent, &destination, &texts);
    args["moves"][1]["destination"]["path"] = json!("cases/child/worker/other.rs");
    let result = run(&repo, args.clone());
    apply(&repo, &result);
    assert_eq!(
        result["plan"]["directory_preconditions"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        result["plan"]["directory_preconditions"][0]["dependent_created_file_ids"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    args["moves"][1]["destination"]["path"] = json!("cases/child/worker/Detail.rs");
    withheld(&run(&repo, args));
}
