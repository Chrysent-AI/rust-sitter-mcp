use super::tests::{fixture_gen, move_artifacts};
use super::*;
use fixture_gen::Fixture;
use serde_json::json;
use std::{fs, os::unix::fs::PermissionsExt, sync::atomic::Ordering};
fn fixture(existing: bool) -> (Fixture, MoveRequest) {
    let repo = Fixture::generate();
    repo.write("cases/child/lib.rs", "mod worker;\n");
    repo.write(
        "cases/child/worker.rs",
        "fn selected() {}\nfn retained() {}\n",
    );
    if existing {
        fs::create_dir(repo.0.join("cases/child/worker")).unwrap();
    }
    let request = serde_json::from_value(json!({"repo_path":repo.0,"crate_root":"cases/child/lib.rs","paths":["cases/child"],"moves":[{"item":move_artifacts::anchor(&repo,"cases/child/worker.rs","fn selected() {}"),"destination":{"kind":"new_child","parent_path":"cases/child/worker.rs","path":"cases/child/worker/detail.rs"}}]})).unwrap();
    (repo, request)
}
fn withheld(result: &MoveEnvelope) {
    assert_eq!(result.schema_version, 3, "{result:?}");
    assert!(!result.plan.applicable, "{result:?}");
    assert!(
        result.plan.edits.is_none()
            && result.plan.created_files.is_none()
            && result.plan.patch.is_none()
    );
    assert!(
        matches!(result.plan.directory_preconditions, Some(None)),
        "{result:?}"
    );
}
#[test]
fn child_final_recheck_withholds_all_artifacts_for_directory_file_layout_and_input_races() {
    for existing in [false, true] {
        for kind in [
            "directory",
            "file",
            "layout",
            "ignore",
            "ignore_addition",
            "parent_bytes",
            "root_bytes",
            "parent_mode",
            "root_mode",
            "ancestor_mode",
            "nested_boundary",
        ] {
            if existing && kind == "directory" {
                continue;
            }
            let (repo, request) = fixture(existing);
            let initial = run(&repo.0, request.clone(), &AtomicBool::new(false));
            assert!(initial.plan.applicable, "{initial:?}");
            let before = fixture_gen::observe(&repo.0);
            move_artifacts::apply(&repo, &serde_json::to_value(initial).unwrap());
            assert_eq!(fixture_gen::observe(&repo.0), before);
            let raced = run_with_recheck(&repo.0, request, &AtomicBool::new(false), || {
                match kind {
                "directory" => fs::create_dir(repo.0.join("cases/child/worker")).unwrap(),
                "file" => repo.write("cases/child/worker/detail.rs","fn appeared() {}"),
                "layout" => repo.write("cases/child/worker/detail/mod.rs","fn appeared() {}"),
                "ignore" => repo.write(".gitignore","target/\n# policy bytes changed, same admitted corpus\ncases/paths/ignored.rs\ncases/paths/nested/\n"),
                "ignore_addition" => repo.write("cases/child/.gitignore","# same admission\n"),
                "parent_bytes" => repo.write("cases/child/worker.rs","fn selected() {}\nfn retained() { /* changed */ }\n"),
                "root_bytes" => repo.write("cases/child/lib.rs","mod worker; // changed\n"),
                "parent_mode" => fs::set_permissions(repo.0.join("cases/child/worker.rs"),fs::Permissions::from_mode(0o755)).unwrap(),
                "root_mode" => fs::set_permissions(repo.0.join("cases/child/lib.rs"),fs::Permissions::from_mode(0o755)).unwrap(),
                "ancestor_mode" => fs::set_permissions(repo.0.join("cases/child"),fs::Permissions::from_mode(0o700)).unwrap(),
                "nested_boundary" => {if !existing { fs::create_dir(repo.0.join("cases/child/worker")).unwrap(); } fs::create_dir(repo.0.join("cases/child/worker/.git")).unwrap();},
                _ => unreachable!(),
            }
            });
            withheld(&raced);
            let creation_race = !existing
                && ["directory", "file", "layout", "nested_boundary"].contains(&kind)
                || existing && ["file", "layout"].contains(&kind);
            assert_eq!(raced.plan.state, "incomplete", "{kind}: {raced:?}");
            assert!(
                raced.truncation_reasons.iter().any(|s| s
                    == if creation_race {
                        "CREATION_RACE"
                    } else {
                        "SOURCE_CHANGED"
                    }),
                "{kind}: {raced:?}"
            );
            if creation_race {
                let diagnostic = &raced.plan.directory_diagnostics.as_ref().unwrap()[0];
                assert!(!diagnostic.path.is_empty());
                assert_eq!(diagnostic.parent_path, "cases/child/worker.rs");
                assert!(
                    serde_json::to_value(diagnostic)
                        .unwrap()
                        .get("range")
                        .is_none()
                );
            }
        }
    }
}
#[test]
fn existing_child_directory_identity_replacement_is_a_distinguishable_race() {
    let (repo, request) = fixture(true);
    let result = run_with_recheck(&repo.0, request, &AtomicBool::new(false), || {
        fs::rename(
            repo.0.join("cases/child/worker"),
            repo.0.join("cases/child/old_directory"),
        )
        .unwrap();
        fs::create_dir(repo.0.join("cases/child/worker")).unwrap();
    });
    withheld(&result);
    assert_eq!(
        result.plan.directory_diagnostics.as_ref().unwrap()[0].reason,
        "directory_creation_race"
    );
}
#[test]
fn child_cancellation_work_and_output_stoppage_never_publish_artifact_subsets() {
    let (repo, mut request) = fixture(false);
    repo.write("cases/child/lib.rs", "mod worker; mod other;\n");
    repo.write("cases/child/other.rs", "fn another() {}\n");
    request.moves.push(Move {
        item: serde_json::from_value(move_artifacts::anchor(
            &repo,
            "cases/child/other.rs",
            "fn another() {}",
        ))
        .unwrap(),
        enclosing_impl: None,
        destination: Destination::NewSibling {
            path: "cases/child/sibling.rs".into(),
            parent_path: "cases/child/lib.rs".into(),
        },
    });
    let baseline = run(&repo.0, request.clone(), &AtomicBool::new(false));
    assert!(baseline.plan.applicable, "{baseline:?}");
    withheld(&run(&repo.0, request.clone(), &AtomicBool::new(true)));
    let cancelled = AtomicBool::new(false);
    let result = run_with_recheck(&repo.0, request.clone(), &cancelled, || {
        cancelled.store(true, Ordering::Relaxed)
    });
    withheld(&result);
    assert_eq!(result.error.unwrap().code, "CANCELLED");
    let mut work = request.clone();
    work.moves.push(Move {
        item: serde_json::from_value(move_artifacts::anchor(
            &repo,
            "cases/child/worker.rs",
            "fn retained() {}",
        ))
        .unwrap(),
        enclosing_impl: None,
        destination: work.moves[0].destination.clone(),
    });
    work.max_moves = 1;
    let result = run(&repo.0, work, &AtomicBool::new(false));
    withheld(&result);
    assert!(result.truncation_reasons.contains(&"max_moves".into()));
    let text = format!("fn selected() {{ let x = \"{}\"; }}", "é".repeat(30000));
    repo.write("cases/child/worker.rs", &text);
    let mut large = request;
    large.moves[0].item = serde_json::from_value(move_artifacts::anchor(
        &repo,
        "cases/child/worker.rs",
        &text,
    ))
    .unwrap();
    large.limits.response_bytes = 65536;
    let result = run(&repo.0, large, &AtomicBool::new(false));
    withheld(&result);
    assert!(
        result.truncation_reasons.contains(&"response_bytes".into()),
        "{result:?}"
    );
}
