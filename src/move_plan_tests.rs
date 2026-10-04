use super::*;
#[path = "../tests/support/fixture_gen.rs"]
mod fixture_gen;
#[path = "../tests/support/move_artifacts.rs"]
mod move_artifacts;
use fixture_gen::{Fixture, observe};
use serde_json::json;
use std::{fs, os::unix::fs::PermissionsExt};

#[test]
fn final_recheck_detects_creation_layout_source_ignore_and_mode_races() {
    for kind in ["creation", "layout", "source", "parent", "ignore", "mode"] {
        let repo = Fixture::generate();
        let request: MoveRequest = serde_json::from_value(json!({"repo_path":repo.0,"crate_root":"cases/layout/lib.rs","paths":["cases/layout"],"moves":[{"item":move_artifacts::anchor(&repo,"cases/layout/source.rs","fn clean() {}"),"destination":{"kind":"new_sibling","path":"cases/layout/new_file.rs","parent_path":"cases/layout/lib.rs"}}]})).unwrap();
        let before = observe(&repo.0);
        let base = run(&repo.0, request.clone(), &AtomicBool::new(false));
        move_artifacts::apply(&repo, &serde_json::to_value(base).unwrap());
        assert_eq!(observe(&repo.0), before);
        let result = run_with_recheck(&repo.0, request, &AtomicBool::new(false), || match kind {
            "creation" => repo.write("cases/layout/new_file.rs", "fn competing() {}"),
            "layout" => repo.write("cases/layout/new_file/mod.rs", "fn competing() {}"),
            "source" => repo.write("cases/layout/source.rs", "fn changed() {}"),
            "parent" => repo.write("cases/layout/lib.rs", "mod source;\n"),
            "ignore" => repo.write("cases/layout/.gitignore", "new_file.rs\n"),
            "mode" => fs::set_permissions(
                repo.0.join("cases/layout/source.rs"),
                fs::Permissions::from_mode(0o755),
            )
            .unwrap(),
            _ => unreachable!(),
        });
        let code = if kind == "creation" || kind == "layout" {
            "CREATION_RACE"
        } else {
            "SOURCE_CHANGED"
        };
        assert_eq!(result.plan.state, "incomplete", "{result:?}");
        assert!(
            result.plan.blockers.iter().any(|b| b.code == code),
            "{result:?}"
        );
        assert!(
            result.plan.edits.is_none()
                && result.plan.created_files.is_none()
                && result.plan.patch.is_none()
        );
    }
}

#[test]
fn whole_call_descriptor_accounting_and_real_inventory_reference_guards() {
    let mut envelope = MoveEnvelope::empty(Limits::default());
    let origin = MoveOrigin {
        id: "o/0".into(),
        source_path: "a.rs".into(),
        source_range: range(0, 10),
        output_path: "b.rs".into(),
        output_range: range(0, 10),
        role: "item".into(),
    };
    let bytes = descriptor_bytes(&vec![origin]).unwrap();
    assert!(bytes > 100);
    envelope.account(bytes).unwrap();
    envelope.effective_work_limits.analysis_descriptor_bytes = bytes;
    assert_eq!(
        envelope.account(1).unwrap_err().code,
        "analysis_descriptor_bytes"
    );
    envelope.incomplete("analysis_descriptor_bytes");
    assert!(
        envelope.plan.edits.is_none()
            && envelope.plan.created_files.is_none()
            && envelope.plan.patch.is_none()
    );
    let repo = Fixture::generate();
    for (source, expected) in [
        ("fn f(){}".repeat(100001), "inventory_work_limit"),
        (
            format!("fn selected(){{{}}}", "missing();".repeat(100001)),
            "reference_work_limit",
        ),
    ] {
        repo.write("cases/layout/source.rs", &source);
        let text = if expected == "inventory_work_limit" {
            "fn f(){}"
        } else {
            &source
        };
        let request: MoveRequest = serde_json::from_value(json!({"repo_path":repo.0,"crate_root":"cases/layout/lib.rs","paths":["cases/layout"],"moves":[{"item":move_artifacts::anchor(&repo,"cases/layout/source.rs",text),"destination":{"kind":"new_sibling","path":"cases/layout/new_file.rs","parent_path":"cases/layout/lib.rs"}}],"limits":{"text_bytes":0}})).unwrap();
        let result = run(&repo.0, request, &AtomicBool::new(false));
        assert_eq!(result.plan.state, "incomplete", "{result:?}");
        assert!(
            result.plan.blockers.iter().any(|b| b.code == expected),
            "{result:?}"
        );
        assert!(
            result.plan.edits.is_none()
                && result.plan.created_files.is_none()
                && result.plan.patch.is_none()
        );
    }
}
