use super::*;
use std::time::Duration;

fn context(paths: &[&str], unresolved: bool) -> ModuleEvidence {
    ModuleEvidence {
        crate_root: "src/lib.rs".into(),
        module_segments: vec![],
        declaration_anchors: vec![],
        filesystem_paths: paths.iter().map(|p| (*p).into()).collect(),
        assumptions: vec![],
        unresolved: if unresolved {
            vec!["uncertain".into()]
        } else {
            vec![]
        },
    }
}
fn failure(at: &str, prefix: &[&str], candidates: &[&str], reason: ChainReason) -> ChainDiagnostic {
    let mut diagnostic = ChainDiagnostic::boundary("src/lib.rs", "", ChainRole::Source, reason);
    diagnostic.at_file_path = at.into();
    diagnostic.evidenced_prefix_paths = prefix.iter().map(|p| (*p).into()).collect();
    diagnostic.candidate_paths = candidates.iter().map(|p| (*p).into()).collect();
    diagnostic
}

#[test]
fn projection_uses_longest_evidenced_prefix_without_proving_untraversed_descendants() {
    let cancelled = AtomicBool::new(false);
    let controls = (Instant::now() + Duration::from_secs(30), &cancelled);
    let mut analysis = ModuleAnalysis::default();
    analysis
        .contexts
        .insert("src/lib.rs".into(), context(&["src/lib.rs"], false));
    analysis.failures = vec![
        failure(
            "src/lib.rs",
            &["src/lib.rs"],
            &["src/branch.rs", "src/branch/mod.rs"],
            ChainReason::ConditionalDeclaration,
        ),
        failure(
            "src/branch.rs",
            &["src/lib.rs", "src/branch.rs"],
            &["src/branch/lost.rs", "src/branch/lost/mod.rs"],
            ChainReason::ChainFileMissing,
        ),
        failure(
            "src/unrelated.rs",
            &["src/lib.rs", "src/unrelated.rs"],
            &["src/unrelated/leaf.rs"],
            ChainReason::PathAttribute,
        ),
    ];
    let projected = analysis
        .project(
            "src/lib.rs",
            "src/branch/lost/leaf.rs",
            ChainRole::Destination,
            controls,
        )
        .unwrap();
    assert_eq!(projected.len(), 1);
    assert_eq!(projected[0].reason, ChainReason::ChainFileMissing);
    assert_eq!(projected[0].at_file_path, "src/branch.rs");
    assert_eq!(projected[0].relation, ChainRelation::PossibleAncestor);
    assert_eq!(projected[0].role, ChainRole::Destination);
    assert_eq!(
        projected[0].evidenced_prefix_paths,
        ["src/lib.rs", "src/branch.rs"]
    );
    let outside = analysis
        .project(
            "src/lib.rs",
            "src/branchish/leaf.rs",
            ChainRole::Source,
            controls,
        )
        .unwrap();
    assert_eq!(outside[0].reason, ChainReason::SourceNotInRootChain);
    assert_eq!(outside[0].relation, ChainRelation::RootSearchExhausted);
    assert_eq!(outside[0].at_file_path, "src/lib.rs");
    assert_eq!(outside[0].evidenced_prefix_paths, ["src/lib.rs"]);
}

#[test]
fn competing_inclusion_origins_remain_at_the_origin_for_traversed_descendants() {
    let cancelled = AtomicBool::new(false);
    let controls = (Instant::now() + Duration::from_secs(30), &cancelled);
    let mut analysis = ModuleAnalysis::default();
    let mut origin = failure(
        "src/branch.rs",
        &["src/lib.rs", "src/branch.rs"],
        &["src/branch.rs"],
        ChainReason::InheritedUncertainty,
    );
    origin
        .origin_reasons
        .push(ChainOrigin::MultipleInclusionContexts);
    origin.declaration = Some(ChainLocation {
        path: "src/lib.rs".into(),
        range: ByteRange {
            start_byte: 0,
            end_byte: 11,
        },
    });
    analysis.failures.push(origin.clone());
    analysis.contexts.insert(
        "src/branch/leaf.rs".into(),
        context(&["src/lib.rs", "src/branch.rs", "src/branch/leaf.rs"], true),
    );
    let projected = analysis
        .project(
            "src/lib.rs",
            "src/branch/leaf.rs",
            ChainRole::Source,
            controls,
        )
        .unwrap();
    assert_eq!(projected[0].relation, ChainRelation::Direct);
    assert_eq!(projected[0].at_file_path, origin.at_file_path);
    assert_eq!(projected[0].declaration, origin.declaration);
    assert_eq!(
        projected[0].origin_reasons,
        [ChainOrigin::MultipleInclusionContexts]
    );
    // A clean known chain cannot be vetoed by unrelated stored obstructions.
    analysis.contexts.insert(
        "src/good.rs".into(),
        context(&["src/lib.rs", "src/good.rs"], false),
    );
    assert!(
        analysis
            .project("src/lib.rs", "src/good.rs", ChainRole::Source, controls)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn deterministic_ids_descriptor_caps_and_cancellation_are_not_silent() {
    let a = failure(
        "src/lib.rs",
        &["src/lib.rs"],
        &["src/a.rs"],
        ChainReason::PathAttribute,
    );
    let mut b = a.clone();
    b.reason = ChainReason::ConditionalDeclaration;
    let mut forward = vec![a.clone(), b.clone()];
    let mut reversed = vec![b, a];
    for (i, diagnostic) in forward.iter_mut().chain(&mut reversed).enumerate() {
        diagnostic.id = format!("temporary/{i}");
    }
    finalize_chain(&mut forward);
    finalize_chain(&mut reversed);
    assert_eq!(forward, reversed);
    assert_eq!(forward[0].id, "chain/0");
    assert_eq!(forward[1].id, "chain/1");
    let mut analysis = ModuleAnalysis::default();
    let mut bytes = 128 * 1024 * 1024;
    assert_eq!(
        analysis
            .record(forward[0].clone(), &mut bytes)
            .unwrap_err()
            .code,
        "analysis_descriptor_bytes"
    );
    assert!(analysis.failures.is_empty());
    analysis.failures = forward;
    let cancelled = AtomicBool::new(true);
    assert_eq!(
        analysis
            .project(
                "src/lib.rs",
                "src/a.rs",
                ChainRole::Source,
                (Instant::now() + Duration::from_secs(30), &cancelled)
            )
            .unwrap_err()
            .code,
        "CANCELLED"
    );
}
