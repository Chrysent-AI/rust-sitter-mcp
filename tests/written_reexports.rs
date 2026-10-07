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
    assert_eq!(observe(&repo.0), before, "planning must remain read-only");
    serde_json::to_value(result).unwrap()
}
fn request(repo: &Fixture, source: &str, selected: &str) -> Value {
    json!({"repo_path":repo.0,"crate_root":"cases/layout/lib.rs","paths":["cases/layout"],"limits":{"diagnostic_count":1000},"moves":[{"item":anchor(repo,source,selected),"destination":{"kind":"new_sibling","path":"cases/layout/target.rs","parent_path":"cases/layout/lib.rs"}}]})
}
fn compile(repo: &Fixture) {
    let output = Command::new("rustc")
        .current_dir(&repo.0)
        .args([
            "--edition=2024",
            "--crate-type=lib",
            "cases/layout/lib.rs",
            "--out-dir",
        ])
        .arg(&repo.0)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
fn withheld(result: &Value) {
    assert_eq!(result["status"], "complete", "{result}");
    assert_eq!(result["plan"]["applicable"], false, "{result}");
    for key in ["edits", "created_files", "patch"] {
        assert!(result["plan"][key].is_null(), "{result}");
    }
}
fn written(result: &Value) -> Vec<&Value> {
    result["plan"]["rewrites"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|r| {
            r["evidence"]
                .as_array()
                .unwrap()
                .contains(&json!("written_reexport"))
        })
        .collect()
}
fn assert_anchor(repo: &Fixture, rewrite: &Value, path: &str, text: &str) {
    let expected = anchor(repo, path, text);
    assert!(
        rewrite["anchors"].as_array().unwrap().contains(&expected),
        "missing anchor {expected}: {rewrite}"
    );
}

#[test]
fn named_renamed_and_source_public_leaves_import_the_canonical_declaration() {
    for visibility in ["pub", "pub(crate)"] {
        for renamed in [false, true] {
            for source_public in [false, true] {
                let repo = Fixture::generate();
                repo.write(
                    "cases/layout/lib.rs",
                    "mod source;\nmod bindings;\nmod bridge;\n",
                );
                let declaration = "pub fn helper() -> u8 { 1 }";
                repo.write("cases/layout/bindings.rs", &format!("{declaration}\n"));
                let binding = if renamed { "alias" } else { "helper" };
                let hop = format!(
                    "{visibility} use crate::bindings::{{helper{}}};",
                    if renamed { " as alias" } else { "" }
                );
                repo.write("cases/layout/bridge.rs", &format!("{hop}\n"));
                let import = format!(
                    "{}use crate::bridge::{binding};",
                    if source_public { "pub(crate) " } else { "" }
                );
                let selected = format!("fn selected() -> u8 {{ {binding}() + {binding}() }}");
                repo.write("cases/layout/source.rs", &format!("{import}\n{selected}\n"));
                compile(&repo);
                let args = request(&repo, "cases/layout/source.rs", &selected);
                let result = run(&repo, args.clone());
                let copy = apply(&repo, &result);
                compile(&copy);
                let expected = format!(
                    "use crate::bindings::helper{};",
                    if renamed { " as alias" } else { "" }
                );
                assert!(
                    fs::read_to_string(copy.0.join("cases/layout/target.rs"))
                        .unwrap()
                        .contains(&expected)
                );
                let rewrites = written(&result);
                assert_eq!(
                    rewrites.len(),
                    1,
                    "repeated references deduplicate one import"
                );
                assert_eq!(rewrites[0]["kind"], "import_insert");
                assert_eq!(rewrites[0]["after_text"], expected);
                assert_anchor(&repo, rewrites[0], "cases/layout/bridge.rs", &hop);
                assert_anchor(&repo, rewrites[0], "cases/layout/bindings.rs", declaration);
                assert_anchor(&repo, rewrites[0], "cases/layout/source.rs", &import);
                assert_eq!(result["plan"]["integrity"]["semantic"], "not_performed");
                assert_eq!(result["plan"], run(&repo, args.clone())["plan"]);
                let mut retained = args.clone();
                retained["rewrite_overrides"] =
                    json!([{"target":rewrites[0]["target"],"action":"retain"}]);
                withheld(&run(&repo, retained));
                let mut replaced = args;
                replaced["rewrite_overrides"] = json!([{"target":rewrites[0]["target"],"action":"replace","replacement_text":expected.replace("crate::", "super::")}]);
                let replaced = run(&repo, replaced);
                compile(&apply(&repo, &replaced));
                assert_eq!(written(&replaced)[0]["origin"], "caller_override");
            }
        }
    }
}

#[test]
fn relative_transitive_renames_follow_exactly_eight_hops_and_refuse_nine() {
    for hops in [2, 8, 9] {
        let repo = Fixture::generate();
        let mut root = "mod source;\nmod bindings;\n".to_owned();
        let declaration = "pub(crate) fn helper() {}";
        repo.write("cases/layout/bindings.rs", &format!("{declaration}\n"));
        let mut hop_anchors = Vec::new();
        for index in 0..hops {
            root.push_str(&format!("mod bridge{index};\n"));
            let next = if index + 1 == hops {
                "bindings::helper".to_owned()
            } else {
                format!("bridge{}::alias", index + 1)
            };
            let hop = format!("pub(crate) use super::{next} as alias;");
            let path = format!("cases/layout/bridge{index}.rs");
            repo.write(&path, &format!("{hop}\n"));
            hop_anchors.push((path, hop));
        }
        repo.write("cases/layout/lib.rs", &root);
        let selected = "fn selected() { alias(); }";
        repo.write(
            "cases/layout/source.rs",
            &format!("use crate::bridge0::alias;\n{selected}\n"),
        );
        let result = run(&repo, request(&repo, "cases/layout/source.rs", selected));
        if hops == 9 {
            withheld(&result);
            assert!(written(&result).is_empty());
            assert!(
                result["plan"]["decisions"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|d| d["reason"] == "external_or_missing_binding"
                        && d["unresolved_consequence"]
                            .as_str()
                            .unwrap()
                            .contains("eight-hop")),
                "{result}"
            );
        } else {
            compile(&apply(&repo, &result));
            let rewrites = written(&result);
            assert_eq!(
                rewrites[0]["after_text"],
                "use crate::bindings::helper as alias;"
            );
            for (path, hop) in hop_anchors {
                assert_anchor(&repo, rewrites[0], &path, &hop);
            }
            assert_anchor(&repo, rewrites[0], "cases/layout/bindings.rs", declaration);
        }
    }
}

#[test]
fn same_module_reexport_aliases_and_scoped_consumer_imports_are_canonical() {
    let repo = Fixture::generate();
    repo.write(
        "cases/layout/lib.rs",
        "mod source;\nmod bindings;\nmod bridge;\n",
    );
    let declaration = "pub fn helper() {}";
    repo.write("cases/layout/bindings.rs", &format!("{declaration}\n"));
    let first = "pub use crate::bindings::helper as first;";
    let second = "pub use first as alias;";
    repo.write("cases/layout/bridge.rs", &format!("{first}\n{second}\n"));
    for selected in [
        "fn selected() { use crate::bridge::{alias}; alias(); }",
        "fn selected() { crate::bridge::alias(); }",
    ] {
        repo.write("cases/layout/source.rs", &format!("{selected}\n"));
        compile(&repo);
        let result = run(&repo, request(&repo, "cases/layout/source.rs", selected));
        compile(&apply(&repo, &result));
        assert!(!written(&result).is_empty());
        for rewrite in written(&result) {
            assert_anchor(&repo, rewrite, "cases/layout/bridge.rs", first);
            assert_anchor(&repo, rewrite, "cases/layout/bridge.rs", second);
            assert_anchor(&repo, rewrite, "cases/layout/bindings.rs", declaration);
        }
    }
}

#[test]
fn reexport_evidence_does_not_discharge_terminal_attribute_or_constructor_vetoes() {
    for (declaration, selected, reason) in [
        (
            "#[cfg(all())]\npub enum Value { Pass }",
            "fn selected(value: Value) { let _ = value; }",
            "conditional_or_inherited_context",
        ),
        (
            "pub struct Value(u8);",
            "fn selected() { let _ = Value(1); }",
            "member_or_constructor_unproved",
        ),
    ] {
        let repo = Fixture::generate();
        repo.write(
            "cases/layout/lib.rs",
            "mod source;\nmod bridge;\nmod bindings;\n",
        );
        repo.write(
            "cases/layout/bridge.rs",
            "pub use crate::bindings::Value;\n",
        );
        repo.write("cases/layout/bindings.rs", &format!("{declaration}\n"));
        repo.write(
            "cases/layout/source.rs",
            &format!("use crate::bridge::Value;\n{selected}\n"),
        );
        let result = run(&repo, request(&repo, "cases/layout/source.rs", selected));
        withheld(&result);
        assert!(
            result["plan"]["decisions"]
                .as_array()
                .unwrap()
                .iter()
                .any(|d| d["reason"] == reason),
            "{result}"
        );
    }
}

#[test]
fn relative_named_reexport_in_a_declaring_parent_matches_the_probe_shape() {
    for outside_parent in [false, true] {
        let repo = Fixture::generate();
        repo.write("cases/layout/lib.rs", "mod scheduler;\nmod target;\n");
        repo.write("cases/layout/target.rs", "");
        let hop = "pub use claim_review::{ClaimReviewOutcome};";
        repo.write(
            "cases/layout/scheduler/mod.rs",
            &format!("mod claim_review;\nmod operations;\n{hop}\n"),
        );
        let declaration = "pub enum ClaimReviewOutcome { Pass, Fail }";
        repo.write(
            "cases/layout/scheduler/claim_review.rs",
            &format!("{declaration}\n"),
        );
        let selected =
            "fn claim_review_status(outcome: ClaimReviewOutcome) -> ClaimReviewOutcome { outcome }";
        repo.write(
            "cases/layout/scheduler/operations.rs",
            &format!("use super::ClaimReviewOutcome;\n{selected}\n"),
        );
        compile(&repo);
        let mut args = request(&repo, "cases/layout/scheduler/operations.rs", selected);
        args["moves"][0]["destination"] = if outside_parent {
            json!({"kind":"existing","path":"cases/layout/target.rs"})
        } else {
            json!({"kind":"new_sibling","path":"cases/layout/scheduler/projections.rs","parent_path":"cases/layout/scheduler/mod.rs"})
        };
        let result = run(&repo, args);
        compile(&apply(&repo, &result));
        let rewrites = written(&result);
        // Private children are visible to descendants of their declaring parent.
        // Moving outside that parent requires its public re-export instead.
        assert_eq!(
            rewrites[0]["after_text"],
            if outside_parent {
                "use crate::scheduler::ClaimReviewOutcome;"
            } else {
                "use crate::scheduler::claim_review::ClaimReviewOutcome;"
            }
        );
        assert_anchor(&repo, rewrites[0], "cases/layout/scheduler/mod.rs", hop);
        assert_anchor(
            &repo,
            rewrites[0],
            "cases/layout/scheduler/claim_review.rs",
            declaration,
        );
        if outside_parent {
            assert_anchor(
                &repo,
                rewrites[0],
                "cases/layout/scheduler/mod.rs",
                "mod claim_review;",
            );
            assert!(
                rewrites[0]["rationale"]
                    .as_str()
                    .unwrap()
                    .contains("inaccessible_route:claim_review")
            );
        }
        assert!(
            !result["plan"]["decisions"]
                .as_array()
                .unwrap()
                .iter()
                .any(|d| d["reason"] == "public_path_change")
        );
        assert!(
            !result["plan"]["rewrites"]
                .as_array()
                .unwrap()
                .iter()
                .any(|r| r["kind"] == "visibility")
        );
    }
}

#[test]
fn direct_canonical_import_uses_public_fallback_without_widening_hidden_modules() {
    for canonical_visible in [false, true] {
        let repo = Fixture::generate();
        repo.write("cases/layout/lib.rs", "mod source;\nmod target;\n");
        repo.write("cases/layout/target.rs", "");
        let module = if canonical_visible {
            "pub(crate) mod private;"
        } else {
            "mod private;"
        };
        let hop = "pub use private::Value;";
        repo.write(
            "cases/layout/source.rs",
            &format!("{module}\nmod operations;\n{hop}\n"),
        );
        let declaration = "pub enum Value { Pass }";
        repo.write(
            "cases/layout/source/private.rs",
            &format!("{declaration}\n"),
        );
        let source = "cases/layout/source/operations.rs";
        let import = "use super::private::Value;";
        let selected = "fn selected(value: Value) -> Value { value }";
        repo.write(source, &format!("{import}\n{selected}\n"));
        compile(&repo);
        let mut args = request(&repo, source, selected);
        args["moves"][0]["destination"] =
            json!({"kind":"existing","path":"cases/layout/target.rs"});
        let result = run(&repo, args);
        let expected = if canonical_visible {
            "use crate::source::private::Value;"
        } else {
            "use crate::source::Value;"
        };
        let rewrites = result["plan"]["rewrites"].as_array().unwrap();
        let rewrite = rewrites
            .iter()
            .find(|r| r["kind"] == "import_insert")
            .unwrap();
        assert_eq!(rewrite["after_text"], expected, "{result}");
        assert!(
            !rewrites.iter().any(|r| r["kind"] == "visibility"),
            "direct imports must not widen canonical module visibility: {result}"
        );
        if !canonical_visible {
            assert_eq!(written(&result).len(), 1, "{result}");
            assert_anchor(&repo, rewrite, source, import);
            assert_anchor(&repo, rewrite, "cases/layout/lib.rs", "mod source;");
            assert_anchor(&repo, rewrite, "cases/layout/source.rs", module);
            assert_anchor(&repo, rewrite, "cases/layout/source.rs", hop);
            assert_anchor(
                &repo,
                rewrite,
                "cases/layout/source/private.rs",
                declaration,
            );
            assert!(
                rewrite["rationale"]
                    .as_str()
                    .unwrap()
                    .contains("written_reexport_route_fallback"),
                "{rewrite}"
            );
        }
        let copy = apply(&repo, &result);
        compile(&copy);
        assert_eq!(
            fs::read_to_string(copy.0.join("cases/layout/source.rs")).unwrap(),
            fs::read_to_string(repo.0.join("cases/layout/source.rs")).unwrap()
        );
        assert!(
            fs::read_to_string(copy.0.join("cases/layout/target.rs"))
                .unwrap()
                .contains(expected)
        );
        assert_eq!(result["plan"]["integrity"]["semantic"], "not_performed");
    }
}

#[test]
fn direct_canonical_import_without_public_route_refuses_instead_of_widening() {
    let repo = Fixture::generate();
    repo.write("cases/layout/lib.rs", "mod source;\nmod target;\n");
    repo.write("cases/layout/target.rs", "");
    repo.write("cases/layout/source.rs", "mod private;\nmod operations;\n");
    repo.write(
        "cases/layout/source/private.rs",
        "pub enum Value { Pass }\n",
    );
    let source = "cases/layout/source/operations.rs";
    let selected = "fn selected(value: Value) -> Value { value }";
    repo.write(source, &format!("use super::private::Value;\n{selected}\n"));
    compile(&repo);
    let mut args = request(&repo, source, selected);
    args["moves"][0]["destination"] = json!({"kind":"existing","path":"cases/layout/target.rs"});
    let result = run(&repo, args);
    withheld(&result);
    let expected = anchor(&repo, "cases/layout/source.rs", "mod private;");
    assert!(
        result["plan"]["decisions"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|d| d["refusal_basis"].as_array().into_iter().flatten())
            .any(|b| b["class"] == "inaccessible_route:private"
                && b["anchor"]["path"] == expected["path"]
                && b["anchor"]["range"] == expected["range"]),
        "{result}"
    );
    assert!(
        !result["plan"]["rewrites"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["kind"] == "visibility"),
        "{result}"
    );
}

#[test]
fn canonical_reexport_route_checks_visibility_or_uses_the_public_fallback() {
    for (visibility, accessible) in [
        ("", false),
        ("pub ", true),
        ("pub(crate) ", true),
        ("pub(super) ", true),
        ("pub(self) ", false),
        ("pub(in crate) ", true),
        ("pub(in crate::scheduler) ", true),
        ("pub(in crate::scheduler::persistence) ", false),
        ("pub(in super) ", true),
        ("pub(in super::super) ", true),
        ("pub(in self) ", false),
        ("pub ( /* visibility trivia */ super ) ", true),
    ] {
        for form in ["bare", "path", "scoped", "grouped"] {
            // Exercise each rewrite form at both access outcomes; the shared check
            // handles the full visibility matrix for bare imports.
            if form != "bare" && !matches!(visibility, "" | "pub(crate) ") {
                continue;
            }
            for existing in [false, true] {
                let repo = Fixture::generate();
                repo.write("cases/layout/lib.rs", "mod scheduler;\n");
                repo.write(
                    "cases/layout/scheduler/mod.rs",
                    "mod persistence;\nmod runtime;\n",
                );
                let module = format!("{visibility}mod foundation_types;");
                let hop = "pub use foundation_types::VerificationStatus;";
                repo.write(
                    "cases/layout/scheduler/persistence/mod.rs",
                    &format!("{module}\n{hop}\n"),
                );
                repo.write(
                    "cases/layout/scheduler/persistence/foundation_types.rs",
                    "pub enum VerificationStatus { Passed }\n",
                );
                repo.write(
                    "cases/layout/scheduler/runtime/mod.rs",
                    if existing {
                        "mod operations;\nmod projections;\n"
                    } else {
                        "mod operations;\n"
                    },
                );
                if existing {
                    repo.write("cases/layout/scheduler/runtime/projections.rs", "");
                }
                let selected = match form {
                    "bare" => {
                        "fn selected(value: VerificationStatus) -> VerificationStatus { value }"
                    }
                    "path" => {
                        "fn selected(value: crate::scheduler::persistence::VerificationStatus) {}"
                    }
                    "scoped" => {
                        "fn selected() { use crate::scheduler::persistence::VerificationStatus; }"
                    }
                    _ => {
                        "fn selected() { use crate::scheduler::persistence::{VerificationStatus}; }"
                    }
                };
                let source = "cases/layout/scheduler/runtime/operations.rs";
                repo.write(
                    source,
                    &format!(
                        "use crate::scheduler::persistence::VerificationStatus;\n{selected}\n"
                    ),
                );
                compile(&repo);
                let mut args = request(&repo, source, selected);
                args["moves"][0]["destination"] = if existing {
                    json!({"kind":"existing","path":"cases/layout/scheduler/runtime/projections.rs"})
                } else {
                    json!({"kind":"new_sibling","path":"cases/layout/scheduler/runtime/projections.rs","parent_path":"cases/layout/scheduler/runtime/mod.rs"})
                };
                let result = run(&repo, args);
                compile(&apply(&repo, &result));
                if accessible {
                    assert!(!written(&result).is_empty(), "{result}");
                    assert!(
                        written(&result).iter().any(|r| r["after_text"]
                            .as_str()
                            .unwrap()
                            .contains("foundation_types")),
                        "{result}"
                    );
                } else {
                    // The original public route already works at the destination.
                    // Scoped/path uses may need no textual repair at all.
                    assert!(
                        !result["plan"]["rewrites"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .any(|r| r["after_text"]
                                .as_str()
                                .unwrap()
                                .contains("foundation_types")),
                        "{result}"
                    );
                    for rewrite in written(&result) {
                        assert_anchor(
                            &repo,
                            rewrite,
                            "cases/layout/scheduler/persistence/mod.rs",
                            &module,
                        );
                        assert_anchor(
                            &repo,
                            rewrite,
                            "cases/layout/scheduler/persistence/mod.rs",
                            hop,
                        );
                        assert_anchor(
                            &repo,
                            rewrite,
                            "cases/layout/scheduler/persistence/foundation_types.rs",
                            "pub enum VerificationStatus { Passed }",
                        );
                        assert!(
                            rewrite["rationale"]
                                .as_str()
                                .unwrap()
                                .contains("written_reexport_route_fallback"),
                            "{rewrite}"
                        );
                        assert!(
                            rewrite["rationale"]
                                .as_str()
                                .unwrap()
                                .contains("inaccessible_route:foundation_types"),
                            "{rewrite}"
                        );
                    }
                    if form == "bare" {
                        assert_eq!(
                            written(&result)[0]["after_text"],
                            "use crate::scheduler::persistence::VerificationStatus;"
                        );
                    }
                }
                assert!(
                    !result["plan"]["rewrites"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|r| r["kind"] == "visibility"),
                    "route checking must not widen module visibility: {result}"
                );
                assert_eq!(result["plan"]["integrity"]["semantic"], "not_performed");
            }
        }
    }
}

#[test]
fn canonical_route_checks_every_edge_and_private_parent_descendants() {
    for (visibility, destination, accessible) in [
        ("", "runtime", true),
        ("", "store", true),
        ("pub(in crate::unrelated) ", "store", false),
    ] {
        let repo = Fixture::generate();
        repo.write("cases/layout/lib.rs", "mod scheduler;\n");
        repo.write(
            "cases/layout/scheduler/mod.rs",
            "mod store;\nmod runtime;\n",
        );
        let module = format!("{visibility}mod persistence;");
        repo.write(
            "cases/layout/scheduler/store/mod.rs",
            &format!("{module}\npub use persistence::VerificationStatus;\nmod projections;\n"),
        );
        repo.write(
            "cases/layout/scheduler/store/persistence/mod.rs",
            "pub(crate) mod foundation_types;\npub use foundation_types::VerificationStatus;\n",
        );
        repo.write(
            "cases/layout/scheduler/store/persistence/foundation_types.rs",
            "pub enum VerificationStatus { Passed }\n",
        );
        repo.write(
            "cases/layout/scheduler/runtime/mod.rs",
            "mod operations;\nmod projections;\n",
        );
        repo.write("cases/layout/scheduler/store/projections.rs", "");
        repo.write("cases/layout/scheduler/runtime/projections.rs", "");
        let selected = "fn selected(value: VerificationStatus) -> VerificationStatus { value }";
        let source = "cases/layout/scheduler/runtime/operations.rs";
        repo.write(
            source,
            &format!("use crate::scheduler::store::VerificationStatus;\n{selected}\n"),
        );
        if visibility.is_empty() {
            compile(&repo);
        }
        // The non-ancestor restriction is syntax-valid but deliberately not
        // compiler-valid; it exercises conservative refusal of unproved scope.
        let mut args = request(&repo, source, selected);
        args["moves"][0]["destination"] = json!({"kind":"existing","path":format!("cases/layout/scheduler/{destination}/projections.rs")});
        let result = run(&repo, args);
        if accessible {
            compile(&apply(&repo, &result));
            assert!(!written(&result).is_empty(), "{result}");
        } else {
            withheld(&result);
            assert!(written(&result).is_empty(), "{result}");
            let expected = anchor(&repo, "cases/layout/scheduler/store/mod.rs", &module);
            assert!(
                result["plan"]["decisions"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .flat_map(|d| d["refusal_basis"].as_array().into_iter().flatten())
                    .any(|b| {
                        b["class"] == "inaccessible_route:persistence"
                            && b["anchor"]["path"] == expected["path"]
                            && b["anchor"]["range"] == expected["range"]
                    }),
                "{result}"
            );
        }
    }
}

#[test]
fn fallback_route_does_not_discharge_terminal_attribute_veto() {
    let repo = Fixture::generate();
    repo.write("cases/layout/lib.rs", "mod source;\nmod persistence;\n");
    repo.write(
        "cases/layout/persistence/mod.rs",
        "mod foundation_types;\npub use foundation_types::VerificationStatus;\n",
    );
    repo.write(
        "cases/layout/persistence/foundation_types.rs",
        "#[derive(Clone, Copy, Debug, Eq, PartialEq)]\npub enum VerificationStatus { Passed }\n",
    );
    let selected = "fn selected(value: VerificationStatus) -> VerificationStatus { value }";
    repo.write(
        "cases/layout/source.rs",
        &format!("use crate::persistence::VerificationStatus;\n{selected}\n"),
    );
    compile(&repo);
    let result = run(&repo, request(&repo, "cases/layout/source.rs", selected));
    withheld(&result);
    assert_eq!(
        written(&result)[0]["after_text"],
        "use crate::persistence::VerificationStatus;"
    );
    assert!(
        !result["plan"]["rewrites"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["kind"] == "visibility")
    );
    assert!(
        result["plan"]["decisions"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|d| d["refusal_basis"].as_array().into_iter().flatten())
            .any(|b| b["class"] == "derive_veto"),
        "{result}"
    );
}

#[test]
fn inaccessible_facade_with_no_public_route_keeps_anchored_refusal() {
    for terminal_visibility in ["pub ", ""] {
        let repo = Fixture::generate();
        repo.write("cases/layout/lib.rs", "mod scheduler;\n");
        repo.write(
            "cases/layout/scheduler/mod.rs",
            "mod store;\nmod runtime;\n",
        );
        repo.write("cases/layout/scheduler/store/mod.rs", "mod private;\n");
        repo.write(
            "cases/layout/scheduler/store/private/mod.rs",
            "mod inner;\npub use inner::Value;\nmod operations;\n",
        );
        repo.write(
            "cases/layout/scheduler/store/private/inner.rs",
            &format!("{terminal_visibility}enum Value {{ Pass }}\n"),
        );
        repo.write(
            "cases/layout/scheduler/runtime/mod.rs",
            "mod projections;\n",
        );
        repo.write("cases/layout/scheduler/runtime/projections.rs", "");
        let source = "cases/layout/scheduler/store/private/operations.rs";
        let selected = "fn selected(value: Value) -> Value { value }";
        repo.write(source, &format!("use super::Value;\n{selected}\n"));
        if !terminal_visibility.is_empty() {
            compile(&repo);
        }
        let mut args = request(&repo, source, selected);
        args["moves"][0]["destination"] =
            json!({"kind":"existing","path":"cases/layout/scheduler/runtime/projections.rs"});
        let result = run(&repo, args);
        withheld(&result);
        assert!(written(&result).is_empty(), "{result}");
        let expected = anchor(&repo, "cases/layout/scheduler/store/mod.rs", "mod private;");
        assert!(
            result["plan"]["decisions"]
                .as_array()
                .unwrap()
                .iter()
                .flat_map(|d| d["refusal_basis"].as_array().into_iter().flatten())
                .any(|b| b["class"] == "inaccessible_route:private"
                    && b["anchor"]["path"] == expected["path"]
                    && b["anchor"]["range"] == expected["range"]),
            "{result}"
        );
        assert!(
            !result["plan"]["rewrites"]
                .as_array()
                .unwrap()
                .iter()
                .any(|r| r["kind"] == "visibility")
        );
    }
}

#[test]
fn fallback_route_choice_is_shortest_then_lexicographic_with_all_anchors() {
    for reverse in [false, true] {
        let repo = Fixture::generate();
        repo.write(
            "cases/layout/lib.rs",
            "mod source;\nmod facade;\nmod a_long;\n",
        );
        let terminal = "pub enum Value { Pass }";
        repo.write("cases/layout/facade/inner.rs", &format!("{terminal}\n"));
        // The implementation is hidden from the consumer, but visible to its
        // exporting parent. A lexically earlier two-hop route must lose.
        let facade = if reverse {
            "mod inner;\npub use inner::Value as Z;\npub use inner::Value as B;\npub use inner::Value as Original;\n"
        } else {
            "mod inner;\npub use inner::Value as Original;\npub use inner::Value as B;\npub use inner::Value as Z;\n"
        };
        repo.write("cases/layout/facade/mod.rs", facade);
        repo.write(
            "cases/layout/a_long.rs",
            "pub use crate::facade::Original as A;\n",
        );
        let selected = "fn selected(value: Alias) -> Alias { value }";
        repo.write(
            "cases/layout/source.rs",
            &format!("use crate::a_long::A as Alias;\n{selected}\n"),
        );
        compile(&repo);
        let args = request(&repo, "cases/layout/source.rs", selected);
        let result = run(&repo, args.clone());
        compile(&apply(&repo, &result));
        let rewrites = written(&result);
        assert_eq!(rewrites.len(), 1, "{result}");
        let rewrite = rewrites[0];
        assert_eq!(rewrite["after_text"], "use crate::facade::B as Alias;");
        assert_anchor(&repo, rewrite, "cases/layout/facade/mod.rs", "mod inner;");
        assert_anchor(
            &repo,
            rewrite,
            "cases/layout/facade/mod.rs",
            "pub use inner::Value as B;",
        );
        assert_anchor(
            &repo,
            rewrite,
            "cases/layout/facade/mod.rs",
            "pub use inner::Value as Original;",
        );
        assert_anchor(
            &repo,
            rewrite,
            "cases/layout/a_long.rs",
            "pub use crate::facade::Original as A;",
        );
        assert_anchor(&repo, rewrite, "cases/layout/facade/inner.rs", terminal);
        let rationale = rewrite["rationale"].as_str().unwrap();
        assert!(
            rationale.contains("from 4 accessible written routes"),
            "{rewrite}"
        );
        assert!(
            rationale
                .contains("shortest re-export hops, then lexicographic absolute path (1 hops)"),
            "{rewrite}"
        );
        assert_eq!(result["plan"], run(&repo, args)["plan"]);
        assert!(
            !result["plan"]["rewrites"]
                .as_array()
                .unwrap()
                .iter()
                .any(|r| r["kind"] == "visibility")
        );
    }
}

#[test]
fn unsupported_reexport_routes_remain_blocked() {
    for (bridge, second, bindings, reason) in [
        (
            "pub(crate) use crate::bindings::*;",
            "",
            "pub(crate) fn helper() {}",
            "external_or_missing_binding",
        ),
        (
            "#[cfg_attr(all(), cfg(all()))]\npub(crate) use crate::bindings::helper;",
            "",
            "pub(crate) fn helper() {}",
            "conditional_or_inherited_context",
        ),
        (
            "#[cfg(all())]\n// still attached\npub(crate) use crate::bindings::helper;",
            "",
            "pub(crate) fn helper() {}",
            "conditional_or_inherited_context",
        ),
        (
            "#[cfg(all())]\npub(crate) use crate::bindings::helper;",
            "",
            "pub(crate) fn helper() {}",
            "conditional_or_inherited_context",
        ),
        (
            "pub(crate) use crate::second::helper;",
            "pub(crate) use crate::bridge::helper;",
            "",
            "external_or_missing_binding",
        ),
        (
            "pub(crate) use crate::bindings::helper;",
            "",
            "",
            "external_or_missing_binding",
        ),
        (
            "pub(crate) use crate::bindings::helper;\npub(crate) use crate::second::helper;",
            "pub(crate) fn helper() {}",
            "pub(crate) fn helper() {}",
            "external_or_missing_binding",
        ),
        (
            "pub(super) use crate::bindings::helper;",
            "",
            "pub(crate) fn helper() {}",
            "external_or_missing_binding",
        ),
    ] {
        let repo = Fixture::generate();
        repo.write(
            "cases/layout/lib.rs",
            "mod source;\nmod bridge;\nmod second;\nmod bindings;\n",
        );
        repo.write("cases/layout/bridge.rs", &format!("{bridge}\n"));
        repo.write("cases/layout/second.rs", &format!("{second}\n"));
        repo.write("cases/layout/bindings.rs", &format!("{bindings}\n"));
        for selected in [
            "fn selected() { helper(); }",
            "fn selected() { crate::bridge::helper(); }",
        ] {
            repo.write(
                "cases/layout/source.rs",
                &format!("use crate::bridge::helper;\n{selected}\n"),
            );
            let result = run(&repo, request(&repo, "cases/layout/source.rs", selected));
            withheld(&result);
            assert!(written(&result).is_empty(), "{result}");
            assert!(
                result["plan"]["decisions"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|d| d["reason"] == reason),
                "{result}"
            );
        }
    }
}

#[test]
fn moving_the_reexported_declaration_still_requires_an_api_decision() {
    for hop in [
        "pub use crate::bindings::helper;",
        "pub(crate) use crate::bindings::helper as alias;",
    ] {
        let repo = Fixture::generate();
        repo.write("cases/layout/lib.rs", "mod bindings;\nmod bridge;\n");
        repo.write("cases/layout/bridge.rs", &format!("{hop}\n"));
        let selected = "pub fn helper() {}";
        repo.write("cases/layout/bindings.rs", &format!("{selected}\n"));
        let result = run(&repo, request(&repo, "cases/layout/bindings.rs", selected));
        withheld(&result);
        assert!(
            result["plan"]["decisions"]
                .as_array()
                .unwrap()
                .iter()
                .any(|d| d["reason"] == "public_path_change"
                    && d["category"] == "reexport_dependency"
                    && d["anchors"].as_array().unwrap().contains(&anchor(
                        &repo,
                        "cases/layout/bridge.rs",
                        hop
                    ))),
            "{result}"
        );
    }
}
