use super::*;
#[path = "../tests/support/fixture_gen.rs"]
mod fixture_gen;
#[path = "../tests/support/move_artifacts.rs"]
mod move_artifacts;
use fixture_gen::{Fixture, observe};
use serde_json::json;
use std::{fs, os::unix::fs::PermissionsExt, sync::atomic::Ordering};

#[test]
fn final_recheck_detects_creation_layout_source_ignore_and_mode_races() {
    for kind in ["creation", "layout", "source", "parent", "ignore", "mode"] {
        let repo = Fixture::generate();
        let mut request: MoveRequest = serde_json::from_value(json!({"repo_path":repo.0,"crate_root":"cases/layout/lib.rs","paths":["cases/layout"],"moves":[{"item":move_artifacts::anchor(&repo,"cases/layout/source.rs","fn clean() {}"),"destination":{"kind":"new_sibling","path":"cases/layout/new_file.rs","parent_path":"cases/layout/lib.rs"}}]})).unwrap();
        let before = observe(&repo.0);
        let base = run(&repo.0, request.clone(), &AtomicBool::new(false));
        let rewrite_count = base.counts.rewrites;
        assert!(rewrite_count > 0);
        move_artifacts::apply(&repo, &serde_json::to_value(base).unwrap());
        request.limits.diagnostic_count = 0;
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
        assert!(result.plan.rewrites.is_empty());
        assert_eq!(result.counts.rewrites, rewrite_count);
        assert_eq!(result.counts.omissions["rewrites"], rewrite_count);
        assert!(
            result
                .truncation_reasons
                .iter()
                .any(|r| r == "diagnostic_count")
        );
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
fn stopped_decision_grouping_clears_decisions_and_links_with_accounted_omissions() {
    let repo = Fixture::generate();
    repo.write("cases/layout/lib.rs", "mod source;\nmod destination;\n");
    repo.write(
        "cases/layout/source.rs",
        "fn keep() {}\n// --- first banner ---\n\nfn first() {}\n// --- second banner ---\n\nfn second() {}\n",
    );
    repo.write("cases/layout/destination.rs", "");
    let mut request: MoveRequest = serde_json::from_value(json!({
        "repo_path": repo.0,
        "crate_root": "cases/layout/lib.rs",
        "paths": ["cases/layout"],
        "moves": (["fn first() {}", "fn second() {}"].map(|text| json!({
            "item": move_artifacts::anchor(&repo, "cases/layout/source.rs", text),
            "destination": {"kind": "existing", "path": "cases/layout/destination.rs"}
        })))
    }))
    .unwrap();
    let before = observe(&repo.0);
    let baseline = run(&repo.0, request.clone(), &AtomicBool::new(false));
    assert_eq!(baseline.plan.decisions.len(), 2);
    request.trivia_overrides = Some(
        baseline
            .plan
            .decisions
            .iter()
            .enumerate()
            .map(|(index, d)| MoveTriviaOverride {
                trivia: d.anchors[0].clone(),
                disposition: MoveDisposition::CarryWithItem,
                target_item: Some(request.moves[index].item.clone()),
            })
            .collect(),
    );
    for code in [
        "CANCELLED",
        "planning_deadline",
        "analysis_descriptor_bytes",
    ] {
        let cancelled = AtomicBool::new(false);
        let mut result = MoveEnvelope::empty(request.limits.clone().into());
        build(&repo.0, &request, &cancelled, || {}, &mut result).unwrap();
        assert!(result.plan.applicable, "{result:?}");
        assert!(result.plan.edits.is_some() && result.plan.patch.is_some());
        let decision_count = result.plan.decisions.len();
        assert_eq!(decision_count, 2, "{result:?}");
        let move_links: usize = result.plan.moves.iter().map(|m| m.decision_ids.len()).sum();
        assert!(move_links > 0);
        let deadline = if code == "planning_deadline" {
            Instant::now()
        } else {
            Instant::now() + Duration::from_secs(30)
        };
        if code == "analysis_descriptor_bytes" {
            result.counts.analysis_descriptor_bytes = 128 * 1024 * 1024;
        }
        let bytes_before_grouping = result.counts.analysis_descriptor_bytes;
        let mut visited = 0;
        let groups = decision_groups(
            result.plan.decisions.iter().enumerate().map(|(index, d)| {
                visited += 1;
                // Cancel only after one group record has been assembled.
                if code == "CANCELLED" && index == 1 {
                    cancelled.store(true, Ordering::Relaxed);
                }
                (
                    d.category.as_str(),
                    d.reason,
                    &d.action,
                    d.blocks_applicability,
                    d.id.as_str(),
                )
            }),
            &mut result.counts.analysis_descriptor_bytes,
            (deadline, &cancelled),
        );
        assert_eq!(groups.as_ref().unwrap_err().code, code);
        if code == "CANCELLED" {
            assert_eq!(visited, 2);
            assert!(result.counts.analysis_descriptor_bytes > bytes_before_grouping);
        }
        result.finish_decision_groups(groups);
        result.fit();
        if code == "CANCELLED" {
            assert_eq!(result.status, "failed");
            assert_eq!(result.error.as_ref().unwrap().code, code);
            assert_eq!(result.plan.state, "blocked");
        } else {
            assert_eq!(result.status, "partial");
            assert_eq!(result.plan.state, "incomplete");
            assert!(result.truncation_reasons.iter().any(|r| r == code));
        }
        assert!(!result.plan.applicable);
        assert!(result.plan.decisions.is_empty() && result.plan.decision_groups.is_empty());
        assert_eq!(result.counts.omissions["decisions"], decision_count);
        assert_eq!(result.counts.omissions["decision_groups"], 0);
        assert_eq!(result.counts.omissions["decision_group_references"], 0);
        assert_eq!(
            result.counts.omissions["move_decision_references"],
            move_links
        );
        assert!(result.plan.moves.iter().all(|m| m.decision_ids.is_empty()));
        assert!(
            result
                .plan
                .rewrites
                .iter()
                .all(|r| r.decision_ids.is_empty())
        );
        assert!(
            result.plan.edits.is_none()
                && result.plan.created_files.is_none()
                && result.plan.patch.is_none()
        );
        let wire = serde_json::to_value(&result).unwrap();
        assert_eq!(wire["plan"]["decisions"], json!([]));
        assert_eq!(wire["plan"]["decision_groups"], json!([]));
        assert_eq!(wire["plan"]["edits"], json!(null));
        assert_eq!(wire["plan"]["created_files"], json!(null));
        assert_eq!(wire["plan"]["patch"], json!(null));
    }
    assert_eq!(observe(&repo.0), before);
}

#[test]
fn blocked_multi_thousand_line_batch_caps_details_and_reports_scoped_omissions() {
    let repo = Fixture::generate();
    repo.write("cases/layout/lib.rs", "mod source;\nmod destination;\n");
    repo.write("cases/layout/destination.rs", "fn keep() {}\n");
    let selected: Vec<_> = (0..30)
        .map(|i| {
            format!(
                "fn selected_{i}(x: u8) {{\n{}}}",
                "    x.foo();\n".repeat(12)
            )
        })
        .collect();
    let mut source = format!("{}\n", selected.join("\n"));
    for i in 0..300 {
        source.push_str(&format!(
            "#[allow(dead_code)]\nfn unselected_{i}() {{\n{}}}\n\n",
            "\n".repeat(16)
        ));
    }
    source.push_str(&"\n".repeat(6535 - source.lines().count()));
    assert_eq!(source.lines().count(), 6535);
    repo.write("cases/layout/source.rs", &source);
    let request: MoveRequest = serde_json::from_value(json!({
        "repo_path": repo.0,
        "crate_root": "cases/layout/lib.rs",
        "paths": ["cases/layout"],
        "moves": selected.iter().map(|text| json!({
            "item": move_artifacts::anchor(&repo, "cases/layout/source.rs", text),
            "destination": {"kind": "existing", "path": "cases/layout/destination.rs"}
        })).collect::<Vec<_>>()
    }))
    .unwrap();
    let cancelled = AtomicBool::new(false);
    let before = observe(&repo.0);
    let mut result = MoveEnvelope::empty(request.limits.clone().into());
    build(&repo.0, &request, &cancelled, || {}, &mut result).unwrap();
    let groups = decision_groups(
        result.plan.decisions.iter().map(|d| {
            (
                d.category.as_str(),
                d.reason,
                &d.action,
                d.blocks_applicability,
                d.id.as_str(),
            )
        }),
        &mut result.counts.analysis_descriptor_bytes,
        (Instant::now() + Duration::from_secs(30), &cancelled),
    );
    result.finish_decision_groups(groups);
    assert!(!result.plan.applicable);
    assert_eq!(result.plan.decisions.len(), 360);
    assert_eq!(result.plan.trivia_decisions.len(), 300);
    let legacy_bytes = result.wire_bytes();
    result.shape_diagnostics(false, request.limits.rewrite_preview_count());
    let lean_bytes = result.wire_bytes();
    eprintln!(
        "30 moves / 6535 lines: legacy wire {legacy_bytes} bytes; lean wire {lean_bytes} bytes"
    );
    assert!(
        lean_bytes < legacy_bytes,
        "legacy={legacy_bytes}, lean={lean_bytes}"
    );
    assert_eq!(result.plan.decisions.len(), 64);
    assert_eq!(result.counts.omissions["decisions"], 296);
    assert_eq!(result.counts.omissions["trivia_decisions"], 300);
    assert_eq!(result.plan.decision_groups[0].count, 360);
    result.fit();
    assert_eq!(result.status, "complete");
    assert_eq!(observe(&repo.0), before);
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
