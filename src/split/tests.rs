use super::*;
use std::{
    fs,
    path::PathBuf,
    process::Command,
    sync::atomic::{AtomicUsize, Ordering},
};

static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "rust-sitter-advice-unit-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        assert!(
            Command::new("git")
                .args(["init", "--quiet", "--template="])
                .arg(&root)
                .status()
                .unwrap()
                .success()
        );
        fs::write(
            root.join("lib.rs"),
            "fn alpha_read() { alpha_parse(); }\nfn alpha_parse() {}\nfn retained() {}\n",
        )
        .unwrap();
        Self(root)
    }
    fn request(&self) -> SuggestSplitRequest {
        serde_json::from_value(serde_json::json!({"repo_path":self.0,"crate_root":"lib.rs","source_path":"lib.rs","limits":{"text_bytes":0}})).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}
#[test]
fn final_recheck_change_withholds_every_complete_draft() {
    let repo = Fixture::new();
    let result = run_with_recheck(&repo.0, repo.request(), &AtomicBool::new(false), || {
        fs::write(repo.0.join("lib.rs"), "fn changed() {}\n").unwrap();
    });
    assert_eq!(result.status, "partial");
    assert!(result.drafts.is_empty());
    assert!(
        result
            .truncation_reasons
            .iter()
            .any(|r| r == "SOURCE_CHANGED")
    );
    assert!(result.counts.omissions["drafts"] > 0);
    assert!(result.signals.is_empty() && result.decisions.is_empty());
}
#[test]
fn creation_and_ignore_changes_are_not_execution_authority() {
    for ignore in [false, true] {
        let repo = Fixture::new();
        let result = run_with_recheck(&repo.0, repo.request(), &AtomicBool::new(false), || {
            if ignore {
                fs::write(repo.0.join(".gitignore"), "alpha.rs\n").unwrap();
            } else {
                fs::write(repo.0.join("alpha.rs"), "fn competing() {}\n").unwrap();
            }
        });
        assert!(result.drafts.is_empty());
        assert_ne!(result.status, "complete");
        assert_eq!(result.integrity.semantic, "not_performed");
    }
}
#[test]
fn cancellation_after_analysis_keeps_a_typed_failed_envelope() {
    let repo = Fixture::new();
    let cancelled = AtomicBool::new(false);
    let result = run_with_recheck(&repo.0, repo.request(), &cancelled, || {
        cancelled.store(true, Ordering::Relaxed);
    });
    assert_eq!(result.status, "failed");
    assert_eq!(result.error.unwrap().code, "CANCELLED");
    assert!(result.drafts.is_empty());
}
#[test]
fn stopped_chain_analysis_omits_provisional_diagnostics_and_links_together() {
    for cancel in [false, true] {
        let repo = Fixture::new();
        fs::write(repo.0.join("lib.rs"), "use fixture_library::scheduler;\n").unwrap();
        fs::write(repo.0.join("leaf.rs"), "fn one() {}\nfn two() {}\n").unwrap();
        let mut request = repo.request();
        request.source_path = "leaf.rs".into();
        let cancelled = AtomicBool::new(false);
        let result = run_with_recheck(&repo.0, request, &cancelled, || {
            if cancel {
                cancelled.store(true, Ordering::Relaxed);
            } else {
                fs::write(repo.0.join("leaf.rs"), "fn changed() {}\n").unwrap();
            }
        });
        if cancel {
            assert_eq!(result.error.as_ref().unwrap().code, "CANCELLED");
        } else {
            assert!(
                result
                    .truncation_reasons
                    .iter()
                    .any(|r| r == "SOURCE_CHANGED")
            );
        }
        assert!(result.chain_diagnostics.is_empty());
        assert!(result.decisions.is_empty());
        assert!(result.drafts.is_empty());
        assert_eq!(result.counts.omissions["chain_diagnostics"], 1);
        assert_eq!(result.counts.omissions["chain_diagnostic_references"], 1);
    }
}

#[test]
fn injected_small_candidate_cap_stops_at_its_first_excess() {
    let repo = Fixture::new();
    fs::write(
        repo.0.join("lib.rs"),
        "fn target() {}\nfn caller() { target(); target(); }\n",
    )
    .unwrap();
    let request = repo.request();
    let cancelled = AtomicBool::new(false);
    let controls = Controls {
        deadline: Instant::now() + Duration::from_secs(30),
        cancelled: &cancelled,
    };
    let mut result = SuggestSplitEnvelope::empty(request.limits.clone());
    result.effective_work_limits.reference_candidates = 1;
    let error = build(&repo.0, &request, controls, || {}, &mut result).unwrap_err();
    assert_eq!(error.code, "reference_work_limit");
    assert_eq!(result.counts.reference_candidates, 2);
    assert!(result.drafts.is_empty());
}
#[test]
fn final_fit_tier_accounts_membership_without_losing_root_or_snapshot() {
    let repo = Fixture::new();
    let source = (0..1500)
        .map(|i| format!("fn unit_{i}() {{}}\n"))
        .collect::<String>();
    fs::write(repo.0.join("lib.rs"), source).unwrap();
    let mut request = repo.request();
    request.max_items = 1500;
    request.limits.response_bytes = 16 * 1024 * 1024;
    let mut result = run(&repo.0, request, &AtomicBool::new(false));
    assert_eq!(result.status, "complete");
    let root = result.root.clone();
    let snapshot = result.snapshot_id.clone();
    let drafts = result.drafts.len();
    assert!(drafts > 0);
    result.limits.response_bytes = 65536;
    result
        .fit(Controls {
            deadline: Instant::now() + Duration::from_secs(30),
            cancelled: &AtomicBool::new(false),
        })
        .unwrap();
    assert_eq!(result.root, root);
    assert_eq!(result.snapshot_id, snapshot);
    assert_eq!(result.status, "partial");
    assert!(result.drafts.is_empty());
    assert_eq!(result.counts.omissions["draft_summaries"], drafts);
    assert_eq!(
        result.counts.omissions["draft_summary_membership_references"],
        drafts * 1500
    );
    assert!(result.wire_bytes() <= result.limits.response_bytes);
}

#[test]
fn descriptor_and_expired_work_checks_are_not_passing_evidence() {
    let mut result = SuggestSplitEnvelope::empty(Limits::default());
    assert!(result.account(128 * 1024 * 1024 + 1).is_err());
    let cancelled = AtomicBool::new(false);
    let error = result
        .fit(Controls {
            deadline: Instant::now(),
            cancelled: &cancelled,
        })
        .unwrap_err();
    assert_eq!(error.code, "planning_deadline");
}
