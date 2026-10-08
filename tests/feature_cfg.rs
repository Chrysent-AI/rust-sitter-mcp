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

fn run(attribute: &str, features_as_atoms: bool, resolve: bool) -> (Fixture, Value) {
    let repo = Fixture::generate();
    repo.write("cases/layout/lib.rs", "mod source;\n");
    let selected = "fn selected() -> u32 { 1 }";
    repo.write(
        "cases/layout/source.rs",
        &format!("{attribute}\n{selected}\n"),
    );
    let names = ["tokio", "http1", "http2"];
    let config = json!({"crates":[{
        "name":"fixture", "root_file":"cases/layout/lib.rs", "edition":"2024",
        "features": if features_as_atoms { Vec::<&str>::new() } else { names.to_vec() },
        "cfg": if features_as_atoms {
            names.iter().map(|name| json!({"key":"feature","value":name})).collect::<Vec<_>>()
        } else { vec![] }, "dependencies":[]
    }]});
    let args = json!({"repo_path":repo.0,"crate_root":"cases/layout/lib.rs",
        "paths":["cases/layout"],"resolve_semantic":resolve,"semantic_configuration":config,
        "moves":[{"item":move_artifacts::anchor(&repo,"cases/layout/source.rs",selected),
            "destination":{"kind":"new_sibling","path":"cases/layout/moved.rs","parent_path":"cases/layout/lib.rs"}}]});
    let before = observe(&repo.0);
    let result = Engine::new(repo.0.clone()).unwrap().move_item(
        serde_json::from_value::<MoveRequest>(args).unwrap(),
        &AtomicBool::new(false),
    );
    assert_eq!(observe(&repo.0), before, "analysis modified caller source");
    (repo, serde_json::to_value(result).unwrap())
}

#[test]
fn selected_feature_attributes_reach_both_overlay_context_checks() {
    for as_atoms in [false, true] {
        for attr in [
            "#[cfg(feature = \"http1\")]",
            "#[cfg(all(feature = \"tokio\", any(feature = \"http1\", feature = \"http2\")))]",
            "#[cfg_attr(feature = \"tokio\", cfg(any(feature = \"http1\", feature = \"http2\")))]",
            "#[cfg_attr(not(feature = \"tokio\"), arbitrary_provider)]",
        ] {
            let (repo, result) = run(attr, as_atoms, true);
            assert_eq!(result["plan"]["applicable"], true, "{result}");
            let proofs = result["plan"]["binding_proofs"].as_array().unwrap();
            assert!(
                proofs
                    .iter()
                    .any(|p| p["classification"] == "context_attribute"
                        && p["anchor"]["expected_text"] == attr
                        && p["final_anchor"]["expected_text"] == attr)
            );
            let evaluations = result["plan"]["resolution_coverage"]["context_evaluations"]
                .as_array()
                .unwrap();
            for revision in ["original", "final"] {
                assert!(evaluations.iter().any(|e| e["revision"] == revision
                    && e["anchor"]["expected_text"] == attr
                    && e["status"] == "admitted"));
            }
            let copy = move_artifacts::apply(&repo, &result);
            let moved = std::fs::read_to_string(copy.0.join("cases/layout/moved.rs")).unwrap();
            assert!(moved.contains(attr), "attached cfg must move losslessly");
            let check = std::process::Command::new("rustc")
                .args([
                    "--edition=2024",
                    "--crate-type=lib",
                    "--cfg",
                    "feature=\"tokio\"",
                    "--cfg",
                    "feature=\"http1\"",
                    "--cfg",
                    "feature=\"http2\"",
                ])
                .arg(copy.0.join("cases/layout/lib.rs"))
                .arg("-o")
                .arg(copy.0.join("checked.rlib"))
                .output()
                .unwrap();
            assert!(
                check.status.success(),
                "{}",
                String::from_utf8_lossy(&check.stderr)
            );
        }
    }
}

#[test]
fn undeclared_inactive_malformed_and_provider_contexts_remain_anchored() {
    for (attr, reason) in [
        ("#[cfg(feature = \"missing\")]", "undeclared_cfg_atom"),
        ("#[cfg(unknown_key)]", "undeclared_cfg_atom"),
        (
            "#[cfg(any(feature = \"http1\", unknown_key))]",
            "undeclared_cfg_atom",
        ),
        (
            "#[cfg(not(feature = \"tokio\"))]",
            "inactive_written_binding",
        ),
        (
            "#[cfg_attr(feature = \"tokio\", cfg(any()))]",
            "inactive_written_binding",
        ),
        (
            "#[cfg_attr(feature = \"tokio\", arbitrary_provider)]",
            "unsupported_attribute",
        ),
        (
            "#[cfg(not(feature = \"tokio\", feature = \"http1\"))]",
            "unsupported_cfg_predicate",
        ),
    ] {
        let (_, result) = run(attr, true, true);
        assert_eq!(result["plan"]["applicable"], false, "{result}");
        assert!(result["plan"]["patch"].is_null());
        let evaluations = result["plan"]["resolution_coverage"]["context_evaluations"]
            .as_array()
            .unwrap();
        assert!(
            evaluations
                .iter()
                .any(|e| e["reason"] == reason && e["anchor"]["range"]["start_byte"].is_number()),
            "{result}"
        );
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
                        && b["anchor"]["range"]["start_byte"] == 0))
        );
    }
}

#[test]
fn feature_context_admission_requires_the_resolution_opt_in() {
    let (_, result) = run("#[cfg(feature = \"http1\")]", true, false);
    assert_eq!(result["plan"]["applicable"], false);
    assert_eq!(result["plan"]["integrity"]["semantic"], "not_performed");
}
