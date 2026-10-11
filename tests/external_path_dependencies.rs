#[allow(dead_code)]
#[path = "support/fixture_gen.rs"]
mod fixture_gen;
#[allow(dead_code)]
#[path = "support/move_artifacts.rs"]
mod move_artifacts;
use fixture_gen::{Fixture, observe};
use rust_sitter_mcp::{engine::Engine, move_plan::MoveRequest, split::SuggestSplitRequest};
use serde_json::{Value, json};
use std::{fs, process::Command, sync::atomic::AtomicBool};

const SELECTED: &str = "pub fn selected() -> u32 { dep::Thing::new().configure() }";
fn fixture() -> (Fixture, Fixture) {
    let repo = Fixture::generate();
    let dep = Fixture::empty();
    dep.write(
        "Cargo.toml",
        "[package]\nname = \"dependency-library\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    );
    dep.write("src/lib.rs", "pub struct Thing; impl Thing { pub fn new() -> Self { Self } pub fn configure(&self) -> u32 { 7 } }\n");
    repo.write("Cargo.toml", &format!("[package]\nname = \"caller\"\nversion = \"0.1.0\"\nedition = \"2024\"\n[dependencies]\ndep = {{ package = \"dependency-library\", path = {:?} }}\n", dep.0.canonicalize().unwrap().to_str().unwrap()));
    repo.write("src/lib.rs", "mod source;\n");
    repo.write("src/source.rs", &format!("{SELECTED}\n"));
    let output = Command::new("cargo")
        .args(["generate-lockfile", "--offline"])
        .current_dir(&repo.0)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    (repo, dep)
}
fn configuration() -> Value {
    json!({"crates":[{"name":"caller","root_file":"src/lib.rs","edition":"2024","features":[],"cfg":[],"dependencies":[]}]})
}
fn request(repo: &Fixture) -> Value {
    json!({"repo_path":repo.0,"crate_root":"src/lib.rs","paths":["src"],"resolve_semantic":true,
        "external_path_dependencies":true,"semantic_configuration":configuration(),"limits":{"time_budget_ms":30000,"diagnostic_count":1000},
        "moves":[{"item":move_artifacts::anchor(repo,"src/source.rs",SELECTED),"destination":{"kind":"new_sibling","path":"src/moved.rs","parent_path":"src/lib.rs"}}]})
}
fn run(repo: &Fixture, dep: &Fixture, args: Value) -> Value {
    let before = observe(&repo.0);
    let dependency = observe(&dep.0);
    let result = Engine::new(repo.0.clone()).unwrap().move_item(
        serde_json::from_value::<MoveRequest>(args).unwrap(),
        &AtomicBool::new(false),
    );
    assert_eq!(observe(&repo.0), before, "caller tree changed");
    assert_eq!(observe(&dep.0), dependency, "dependency tree changed");
    serde_json::to_value(result).unwrap()
}
#[test]
fn out_of_tree_inherent_method_has_real_both_overlay_proof_and_compilable_patch() {
    let (repo, dep) = fixture();
    let args = request(&repo);
    let mut off = args.clone();
    off.as_object_mut()
        .unwrap()
        .remove("external_path_dependencies");
    let implicit = run(&repo, &dep, off.clone());
    off["external_path_dependencies"] = json!(false);
    assert_eq!(implicit, run(&repo, &dep, off));
    assert_eq!(implicit["plan"]["applicable"], false);
    assert!(implicit["coverage"].get("external_dependencies").is_none());
    let value = run(&repo, &dep, args);
    assert_eq!(value["plan"]["applicable"], true, "{value}");
    let roots = value["coverage"]["external_dependencies"]["consulted_roots"]
        .as_array()
        .unwrap();
    assert_eq!(roots.len(), 1);
    assert_eq!(
        roots[0]["root"],
        dep.0.canonicalize().unwrap().to_str().unwrap()
    );
    assert_eq!(roots[0]["package"], "dependency-library");
    assert_eq!(value["coverage"]["external_dependencies"]["complete"], true);
    assert!(
        value["coverage"]["external_dependencies"]["corpus_digest"]
            .as_str()
            .unwrap()
            .starts_with("sha1:")
    );
    let proofs = value["plan"]["binding_proofs"].as_array().unwrap();
    assert_eq!(value["coverage"]["ra_resolved"], proofs.len());
    assert!(
        proofs
            .iter()
            .any(|p| p["declaration"]["anchor"]["expected_text"] == "configure")
    );
    for proof in proofs {
        assert_eq!(proof["class"], "ra_resolved");
        assert_eq!(proof["classification"], "inherent_function");
        assert_eq!(proof["source_access"], true);
        assert_eq!(proof["final_access"], true);
        assert_eq!(proof["declaration"], proof["final_declaration"]);
        assert!(
            proof["declaration"]["anchor"]["path"]
                .as_str()
                .unwrap()
                .starts_with("@external/0/")
        );
        assert_eq!(proof["original_receiver"], proof["final_original_receiver"]);
    }
    let before = observe(&repo.0);
    let dependency = observe(&dep.0);
    let copy = move_artifacts::apply(&repo, &value);
    let output = Command::new("cargo")
        .args(["check", "--offline", "--locked"])
        .current_dir(&copy.0)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(observe(&repo.0), before);
    assert_eq!(observe(&dep.0), dependency);
}
#[test]
fn disjoint_enclosing_workspace_inherited_path_method_has_an_applicable_plan() {
    let workspace = Fixture::empty();
    let (mut repo, dep) = fixture();
    fs::rename(&repo.0, workspace.0.join("member")).unwrap();
    repo.0 = workspace.0.join("member");
    repo.write("Cargo.toml", "[package]\nname='caller'\nversion='0.1.0'\nedition='2024'\n[dependencies]\ndep.workspace=true\n");
    workspace.write("Cargo.toml", &format!("[workspace]\nmembers=['member']\nresolver='3'\n[workspace.dependencies]\ndep={{package='dependency-library',path='../{}'}}\n", dep.0.file_name().unwrap().to_str().unwrap()));
    let output = Command::new("cargo")
        .args(["generate-lockfile", "--offline"])
        .current_dir(&workspace.0)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    fs::rename(repo.0.join("Cargo.lock"), repo.0.join("unused.lock")).unwrap();
    let before = observe(&workspace.0);
    let value = run(&repo, &dep, request(&repo));
    assert_eq!(value["plan"]["applicable"], true, "{value}");
    assert_eq!(value["coverage"]["external_dependencies"]["complete"], true);
    assert_eq!(
        value["coverage"]["external_dependencies"]["consulted_roots"][0]["root"],
        dep.0.canonicalize().unwrap().to_str().unwrap()
    );
    let proofs = value["plan"]["binding_proofs"].as_array().unwrap();
    assert!(
        proofs
            .iter()
            .any(|proof| proof["declaration"]["anchor"]["expected_text"] == "configure")
    );
    for proof in proofs {
        assert_eq!(proof["class"], "ra_resolved");
        assert_eq!(proof["source_access"], true);
        assert_eq!(proof["final_access"], true);
        assert_eq!(proof["declaration"], proof["final_declaration"]);
    }
    // Applicable publication includes Corpus::recheck and a second discovery.
    assert_eq!(observe(&workspace.0), before, "enclosing workspace changed");
}
#[test]
fn path_discovery_refuses_missing_root_stale_lock_configuration_and_external_edit_abuse() {
    for case in [
        "missing_root",
        "stale_lock",
        "config",
        "registry",
        "vendor",
        "transitive",
        "escape",
        "destination",
        "selection",
        "budget",
        "build_script",
        "private_method",
        "generic_method",
        "symlink",
        "caller_wrapper",
        "field_fact",
        "namespace",
    ] {
        let (repo, dep) = fixture();
        let mut args = request(&repo);
        match case {
            "missing_root" => { fs::rename(dep.0.join("Cargo.toml"), dep.0.join("missing.toml")).unwrap(); },
            "stale_lock" => { repo.write("Cargo.lock", "version = 4\n[[package]]\nname = \"caller\"\nversion = \"9.0.0\"\n"); },
            "config" => args["semantic_configuration"]["crates"][0]["features"] = json!(["invented"]),
            "registry" => repo.write("Cargo.toml", "[package]\nname='caller'\nversion='0.1.0'\nedition='2024'\n[dependencies]\nitoa='=1.0.15'\n"),
            "vendor" => repo.write(".cargo/config.toml", "[source.crates-io]\nreplace-with='local'\n[source.local]\ndirectory='vendor'\n"),
            "transitive" => { let text = fs::read_to_string(dep.0.join("Cargo.toml")).unwrap(); dep.write("Cargo.toml", &format!("{text}\n[dependencies]\nitoa='=1.0.15'\n")); },
            "escape" => dep.write("src/lib.rs", "#[path = \"../../escape.rs\"] mod escape;\n"),
            "destination" => args["moves"][0]["destination"]["path"] = json!(dep.0.join("src/new.rs")),
            "selection" => args["moves"][0]["item"]["path"] = json!("@external/0/src/lib.rs"),
            "budget" => args["limits"]["max_file_bytes"] = json!(32),
            "build_script" => dep.write("build.rs", "fn main() { std::fs::write(\"executed\", \"no\").unwrap(); }"),
            "private_method" => dep.write("src/lib.rs", "pub struct Thing; impl Thing { pub fn new() -> Self { Self } fn configure(&self) -> u32 { 7 } }"),
            "generic_method" => dep.write("src/lib.rs", "pub struct Thing; impl Thing { pub fn new() -> Self { Self } pub fn configure<T>(&self) -> u32 { 7 } }"),
            "symlink" => { std::os::unix::fs::symlink(repo.0.join("src/lib.rs"), dep.0.join("src/escaped.rs")).unwrap(); },
            "namespace" => { repo.write("@external/0/src/lib.rs", "pub struct Collision;"); args["paths"] = json!(["src", "@external"]); },
            "caller_wrapper" => {
                use std::os::unix::fs::PermissionsExt;
                repo.write("wrapper.sh", "#!/bin/sh\nprintf forbidden > caller_write_marker\nexit 1\n");
                fs::set_permissions(repo.0.join("wrapper.sh"), fs::Permissions::from_mode(0o755)).unwrap();
                repo.write(".cargo/config.toml", &format!("[build]\nrustc-wrapper={:?}\n", repo.0.join("wrapper.sh").to_str().unwrap()));
            },
            "field_fact" => {
                dep.write("src/lib.rs", "pub struct Thing { number: u32 } impl Thing { pub fn new() -> Self { Self { number: 7 } } }");
                let text = "pub fn selected() -> u32 { dep::Thing::new().number }";
                repo.write("src/source.rs", text);
                args["moves"][0]["item"] = move_artifacts::anchor(&repo, "src/source.rs", text);
            },
            _ => unreachable!(),
        }
        let value = run(&repo, &dep, args);
        assert_eq!(value["plan"]["applicable"], false, "{case}: {value}");
        assert!(value["plan"]["patch"].is_null(), "{case}: {value}");
        assert!(!dep.0.join("executed").exists());
        assert!(!repo.0.join("caller_write_marker").exists());
        if case == "field_fact" {
            assert!(
                value["plan"]["decisions"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|d| d["refusal_basis"].as_array().is_some_and(|basis| basis
                        .iter()
                        .any(|b| b["class"] == "semantic_source_fact_unproved"))),
                "{value}"
            );
        }
    }
}
fn selected_request(repo: &Fixture, text: &str) -> Value {
    let mut args = request(repo);
    repo.write("src/source.rs", text);
    args["moves"][0]["item"] = move_artifacts::anchor(repo, "src/source.rs", text);
    args
}

fn assert_default_off(repo: &Fixture, dep: &Fixture, args: &Value) {
    let mut off = args.clone();
    off.as_object_mut()
        .unwrap()
        .remove("external_path_dependencies");
    let implicit = run(repo, dep, off.clone());
    off["external_path_dependencies"] = json!(false);
    assert_eq!(
        serde_json::to_vec(&implicit).unwrap(),
        serde_json::to_vec(&run(repo, dep, off)).unwrap(),
        "default-off serialized bytes"
    );
    assert!(implicit["coverage"].get("external_dependencies").is_none());
    assert_eq!(implicit["coverage"]["ra_resolved"].as_u64().unwrap_or(0), 0);
}

fn check_copy(repo: &Fixture, dep: &Fixture, value: &Value) {
    let before = (observe(&repo.0), observe(&dep.0));
    let copy = move_artifacts::apply(repo, value);
    let output = Command::new("cargo")
        .args(["check", "--offline", "--locked"])
        .env("CARGO_TARGET_DIR", copy.0.join("target"))
        .current_dir(&copy.0)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!((observe(&repo.0), observe(&dep.0)), before);
}

#[test]
fn public_written_fields_and_struct_constructors_keep_external_identity() {
    for (library, selected, classification, name, receiver, count) in [
        (
            "pub struct Thing { pub number: u32 } impl Thing { pub fn new() -> Self { Self { number: 7 } } }",
            "pub fn selected() -> u32 { let thing = dep::Thing::new(); let reference = &&thing; reference.number }",
            "field",
            "number",
            "Thing",
            2,
        ),
        (
            "pub struct Thing { pub number: u32 }",
            "pub fn selected() -> u32 { let thing = dep::Thing { number: 7 }; thing.number }",
            "type_or_constructor",
            "Thing",
            "Thing",
            2,
        ),
        (
            "pub struct Tuple(pub u32);",
            "pub fn selected() -> u32 { let _ = dep::Tuple(7); 7 }",
            "type_or_constructor",
            "Tuple",
            "Tuple",
            1,
        ),
    ] {
        let (repo, dep) = fixture();
        dep.write("src/lib.rs", library);
        let args = selected_request(&repo, selected);
        assert_default_off(&repo, &dep, &args);
        let value = run(&repo, &dep, args);
        assert_eq!(value["plan"]["applicable"], true, "{value}");
        let proofs = value["plan"]["binding_proofs"].as_array().unwrap();
        assert_eq!(proofs.len(), count, "{classification}/{name}: {value}");
        assert_eq!(value["coverage"]["ra_resolved"], count);
        assert_eq!(
            value["plan"]["resolution_coverage"]["decisions"],
            proofs.len()
        );
        let proof = proofs
            .iter()
            .find(|p| p["classification"] == classification)
            .unwrap();
        assert_eq!(proof["declaration"]["anchor"]["expected_text"], name);
        for proof in proofs {
            assert_eq!(proof["class"], "ra_resolved");
            assert_eq!(proof["source_access"], true);
            assert_eq!(proof["final_access"], true);
            assert_eq!(proof["declaration"], proof["final_declaration"]);
            assert!(
                proof["declaration"]["anchor"]["path"]
                    .as_str()
                    .unwrap()
                    .starts_with("@external/0/")
            );
            assert_eq!(proof["original_receiver"], proof["final_original_receiver"]);
            assert_eq!(proof["adjusted_receiver"], proof["final_adjusted_receiver"]);
            assert_eq!(
                proof["original_receiver"]["declaration"]["anchor"]["expected_text"],
                receiver
            );
            assert!(
                proof["original_receiver"]["declaration"]["anchor"]["path"]
                    .as_str()
                    .unwrap()
                    .starts_with("@external/0/")
            );
        }
        if classification == "field" {
            assert_eq!(
                proof["original_receiver"]["references"],
                json!(["shared", "shared"])
            );
        }
        assert!(
            value["plan"]["created_files"]
                .as_array()
                .unwrap()
                .iter()
                .all(|file| !file["path"].as_str().unwrap().starts_with("@external/"))
        );
        check_copy(&repo, &dep, &value);
        eprintln!(
            "{classification}/{name}: {count} real proofs; both identities match; disposable copy compiles; default-off bytes equal"
        );
    }
}

#[test]
fn unsupported_external_fields_constructors_and_nonconstruction_paths_have_honest_counts() {
    for (case, library, selected, count, refusal) in [
        (
            "private_field",
            "pub struct Thing { number: u32 } impl Thing { pub fn new() -> Self { Self { number: 7 } } }",
            "pub fn selected() -> u32 { dep::Thing::new().number }",
            1,
            "semantic_source_fact_unproved",
        ),
        (
            "private_record",
            "pub struct Thing { number: u32 }",
            "pub fn selected() -> u32 { let _ = dep::Thing { number: 7 }; 7 }",
            0,
            "semantic_source_fact_unproved",
        ),
        (
            "private_tuple",
            "pub struct Tuple(u32);",
            "pub fn selected() -> u32 { let _ = dep::Tuple(7); 7 }",
            0,
            "semantic_source_fact_unproved",
        ),
        (
            "generic_constructor",
            "pub struct Thing<T>(pub T);",
            "pub fn selected() -> u32 { let _ = dep::Thing::<u32>(7); 7 }",
            0,
            "semantic_source_fact_unproved",
        ),
        (
            "trait_constructor",
            "pub struct Thing; pub trait Create { fn new() -> Self; } impl Create for Thing { fn new() -> Self { Self } }",
            "pub fn selected() -> u32 { let _ = <dep::Thing as dep::Create>::new(); 7 }",
            0,
            "semantic_source_fact_unproved",
        ),
        (
            "generated_constructor",
            "macro_rules! make { () => { pub struct Thing { pub number: u32 } }; } make!();",
            "pub fn selected() -> u32 { let _ = dep::Thing { number: 7 }; 7 }",
            0,
            "semantic_source_fact_unproved",
        ),
        (
            "unknown_context",
            "#[cfg(unknown)] pub struct Thing { pub number: u32 }",
            "pub fn selected() -> u32 { let _ = dep::Thing { number: 7 }; 7 }",
            0,
            "semantic_source_fact_unproved",
        ),
        (
            "type_path",
            "pub struct Thing;",
            "pub fn selected(_: dep::Thing) -> u32 { 7 }",
            0,
            "external_fact_unsupported",
        ),
        (
            "type_argument_inside_call",
            "pub struct Thing;",
            "pub fn selected() -> u32 { fn consume<T>() {} consume::<dep::Thing>(); 7 }",
            0,
            "semantic_source_fact_unproved",
        ),
        (
            "constructor_value_inside_call",
            "pub struct Tuple(pub u32);",
            "pub fn selected() -> u32 { let _ = (dep::Tuple, 7); 7 }",
            0,
            "external_fact_unsupported",
        ),
        (
            "variant_path",
            "pub enum Thing { Value }",
            "pub fn selected() -> u32 { let _ = dep::Thing::Value; 7 }",
            0,
            "external_fact_unsupported",
        ),
        (
            "generic_field_receiver",
            "pub struct Thing<T> { pub number: T }",
            "pub fn selected<T>(thing: dep::Thing<T>) -> u32 { let _ = thing.number; 7 }",
            0,
            "semantic_source_fact_unproved",
        ),
        (
            "generated_field",
            "macro_rules! make { () => { pub struct Thing { pub number: u32 } }; } make!();",
            "pub fn selected() -> u32 { let thing = dep::Thing { number: 7 }; thing.number }",
            0,
            "semantic_source_fact_unproved",
        ),
        (
            "conditional_field_constructor",
            "pub struct Thing { #[cfg(unknown)] pub number: u32 }",
            "pub fn selected() -> u32 { let _ = dep::Thing { number: 7 }; 7 }",
            0,
            "external_fact_unsupported",
        ),
        (
            "conditional_field_access",
            "pub struct Thing { #[cfg(unknown)] pub number: u32 } impl Thing { pub fn new() -> Self { Self { number: 7 } } }",
            "pub fn selected() -> u32 { dep::Thing::new().number }",
            1,
            "semantic_source_fact_unproved",
        ),
        (
            "provider_uncertain_constructor",
            "#[derive(provider::Custom)] pub struct Thing { pub number: u32 }",
            "pub fn selected() -> u32 { let _ = dep::Thing { number: 7 }; 7 }",
            0,
            "semantic_source_fact_unproved",
        ),
    ] {
        let (repo, dep) = fixture();
        dep.write("src/lib.rs", library);
        let args = selected_request(&repo, selected);
        assert_default_off(&repo, &dep, &args);
        let value = run(&repo, &dep, args);
        assert!(value["error"].is_null(), "{case}: {value}");
        assert_eq!(value["plan"]["applicable"], false, "{case}: {value}");
        assert!(value["plan"]["patch"].is_null());
        assert_eq!(
            value["coverage"]["ra_resolved"].as_u64().unwrap_or(0),
            count as u64,
            "{case}: {value}"
        );
        assert_eq!(
            value["plan"]["binding_proofs"]
                .as_array()
                .map_or(0, Vec::len),
            count
        );
        assert_eq!(value["plan"]["resolution_coverage"]["decisions"], count);
        assert!(
            value["plan"]["decisions"]
                .as_array()
                .unwrap()
                .iter()
                .any(|decision| decision["refusal_basis"]
                    .as_array()
                    .is_some_and(|basis| basis.iter().any(|b| b["class"] == refusal))),
            "{case}: {value}"
        );
        eprintln!("{case}: {count} real proofs; {refusal}; no patch; default-off bytes equal");
    }
}

#[test]
fn external_fields_and_constructors_refuse_an_unproved_final_context() {
    for selected in [
        "pub fn selected() -> u32 { dep::Thing::new().number }",
        "pub fn selected() -> u32 { let _ = dep::Thing { number: 7 }; 7 }",
        "pub fn selected() -> u32 { let _ = dep::Tuple(7); 7 }",
    ] {
        let (repo, dep) = fixture();
        dep.write("src/lib.rs", "pub struct Thing { pub number: u32 } pub struct Tuple(pub u32); impl Thing { pub fn new() -> Self { Self { number: 7 } } }");
        let mut args = selected_request(&repo, selected);
        repo.write("src/lib.rs", "mod source;\nmod destination;\n");
        repo.write("src/destination.rs", "#[cfg(unknown)] fn unrelated() {}\n");
        args["moves"][0]["destination"] = json!({"kind":"existing","path":"src/destination.rs"});
        assert_default_off(&repo, &dep, &args);
        let value = run(&repo, &dep, args);
        assert!(value["error"].is_null(), "{value}");
        assert_eq!(value["plan"]["applicable"], false, "{value}");
        assert_eq!(value["coverage"]["ra_resolved"].as_u64().unwrap_or(0), 0);
        assert_eq!(
            value["plan"]["binding_proofs"]
                .as_array()
                .map_or(0, Vec::len),
            0
        );
        assert!(value["plan"]["patch"].is_null());
        assert!(
            value["plan"]["decisions"]
                .as_array()
                .unwrap()
                .iter()
                .any(|d| d["refusal_basis"].as_array().is_some_and(|b| b
                    .iter()
                    .any(|b| b["class"] == "semantic_final_fact_unproved"))),
            "{value}"
        );
    }
}

#[test]
fn advice_discloses_sources_but_never_claims_resolution_or_admits_external_inventory() {
    let (repo, dep) = fixture();
    let before = observe(&repo.0);
    let dependency = observe(&dep.0);
    let result = Engine::new(repo.0.clone()).unwrap().suggest_split(
        serde_json::from_value::<SuggestSplitRequest>(json!({"repo_path":repo.0,"crate_root":"src/lib.rs","source_path":"src/source.rs","paths":["src/lib.rs","src/source.rs"],"external_path_dependencies":true,"semantic_configuration":configuration(),"limits":{"time_budget_ms":30000}})).unwrap(), &AtomicBool::new(false));
    let value = serde_json::to_value(result).unwrap();
    assert!(value["error"].is_null(), "{value}");
    assert_eq!(
        value["coverage"]["external_dependencies"]["consulted_roots"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(value["integrity"]["semantic"], "not_performed");
    assert!(
        value["inventory"]
            .as_array()
            .unwrap()
            .iter()
            .all(|i| i["path"] == "src/source.rs")
    );
    assert_eq!(observe(&repo.0), before);
    assert_eq!(observe(&dep.0), dependency);
}
