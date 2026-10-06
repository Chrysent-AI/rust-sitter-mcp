use super::*;

#[test]
fn zero_and_ambiguous_parent_outcomes_preserve_all_candidates_and_links() {
    let request: SuggestSplitRequest = serde_json::from_value(serde_json::json!({
        "repo_path":".", "crate_root":"src/lib.rs", "source_path":"src/branch/source.rs",
        "limits":{"text_bytes":0}
    }))
    .unwrap();
    let search = serde_json::from_value(serde_json::json!({"repo_path":".","query":""})).unwrap();
    let scope = Scope::new(Path::new(".").to_owned(), &search).unwrap();
    let cancelled = AtomicBool::new(false);
    let controls = Controls {
        deadline: Instant::now() + std::time::Duration::from_secs(30),
        cancelled: &cancelled,
    };
    let files: BTreeMap<String, FileSnapshot> =
        ["src/branch/source.rs", "src/branch.rs", "src/branch/mod.rs"]
            .into_iter()
            .map(|path| {
                (
                    path.into(),
                    FileSnapshot {
                        path: path.into(),
                        source: "fn one() {}\nfn two() {}\n".into(),
                        mode: 0o644,
                    },
                )
            })
            .collect();
    let mut parsed = BTreeMap::new();
    let mut observed = 0;
    for (path, file) in &files {
        parsed.insert(
            path.clone(),
            items::parse(file, 0, controls.deadline, &cancelled, &mut observed).unwrap(),
        );
    }
    let evidence = items::ModuleEvidence {
        crate_root: request.crate_root.clone(),
        module_segments: vec!["branch".into(), "source".into()],
        declaration_anchors: vec![],
        filesystem_paths: vec![
            "src/lib.rs".into(),
            "src/branch/mod.rs".into(),
            request.source_path.clone(),
        ],
        assumptions: vec![],
        unresolved: vec![],
    };
    // Synthetic analyses isolate defensive parent outcomes. Ordinary traversal itself
    // rejects a simultaneous flat/legacy inclusion rather than creating these contexts.
    for (parents, expected) in [
        (vec![], items::ChainReason::NoOrdinarySiblingParent),
        (
            vec!["src/branch.rs", "src/branch/mod.rs"],
            items::ChainReason::AmbiguousParent,
        ),
    ] {
        let mut analysis = items::ModuleAnalysis::default();
        analysis
            .contexts
            .insert(request.source_path.clone(), evidence.clone());
        for parent in &parents {
            analysis.contexts.insert((*parent).into(), evidence.clone());
        }
        let mut result = SuggestSplitEnvelope::empty(request.limits.clone().into());
        result.inventory = parsed[&request.source_path].items.clone();
        build(
            &scope,
            &request,
            &files,
            &parsed,
            &analysis,
            controls,
            &mut result,
        )
        .unwrap();
        finalize(&mut result, controls).unwrap();
        assert!(result.drafts.is_empty());
        assert_eq!(result.chain_diagnostics.len(), 1);
        let diagnostic = &result.chain_diagnostics[0];
        assert_eq!(diagnostic.reason, expected);
        assert_eq!(diagnostic.role, items::ChainRole::DeclarationParent);
        assert_eq!(diagnostic.parent_candidates, parents);
        assert_eq!(diagnostic.crate_root, request.crate_root);
        assert_eq!(diagnostic.at_file_path, request.source_path);
        assert_eq!(
            result.decisions[0].chain_diagnostic_ids,
            vec![diagnostic.id.clone()]
        );
        assert_eq!(
            result.decisions[0].anchors[0].span.range,
            result.inventory[0].span.range
        );
        assert!(
            result
                .draft_eligibility
                .reasons
                .iter()
                .any(|r| r == "unsupported_or_uncertain_ordinary_layout")
        );
    }
    let mut analysis = items::ModuleAnalysis::default();
    analysis
        .contexts
        .insert(request.source_path.clone(), evidence.clone());
    analysis
        .contexts
        .insert("src/branch/mod.rs".into(), evidence);
    let layout = Layout {
        scope: &scope,
        request: &request,
        files: &files,
        parsed: &parsed,
        contexts: &analysis.contexts,
        analysis: &analysis,
        controls,
    };
    assert!(matches!(
        supported_parent(&layout).unwrap(),
        ParentOutcome::Supported("src/branch/mod.rs")
    ));
}
