use rust_sitter_mcp::{
    edit::{Edit, reconstruct},
    engine::Engine,
    plan::{PlanEnvelope, ReplaceRequest},
};
use serde_json::json;
use std::{
    collections::BTreeMap,
    io::Write,
    os::unix::fs::MetadataExt,
    path::PathBuf,
    process::{Command, Stdio},
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
};
static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Repo(PathBuf);
impl Repo {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "rust-sitter-replace-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).unwrap();
        assert!(
            Command::new("git")
                .args(["init", "--quiet"])
                .arg(&root)
                .status()
                .unwrap()
                .success()
        );
        Self(root)
    }
    fn write(&self, path: &str, text: &str) {
        let p = self.0.join(path);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }
    fn request(&self, pattern: &str, replacement: &str) -> ReplaceRequest {
        serde_json::from_value(
            json!({"repo_path":self.0,"pattern":pattern,"replacement":replacement}),
        )
        .unwrap()
    }
    fn run(&self, request: ReplaceRequest) -> PlanEnvelope {
        Engine::new(self.0.clone())
            .unwrap()
            .replace(request, &AtomicBool::new(false))
    }
    fn plan(&self, pattern: &str, replacement: &str) -> PlanEnvelope {
        self.run(self.request(pattern, replacement))
    }
    fn apply(&self, result: &PlanEnvelope) -> BTreeMap<String, String> {
        assert_eq!(result.plan.state, "applicable", "{result:?}");
        let mut rebuilt = BTreeMap::new();
        let copy = Repo::new();
        for base in &result.plan.base_files {
            let source = std::fs::read_to_string(self.0.join(&base.path)).unwrap();
            copy.write(&base.path, &source);
            let metadata = std::fs::metadata(self.0.join(&base.path)).unwrap();
            std::fs::set_permissions(copy.0.join(&base.path), metadata.permissions()).unwrap();
            let edits: Vec<Edit> = result
                .plan
                .edits
                .as_ref()
                .unwrap()
                .iter()
                .filter(|e| e.path == base.path)
                .cloned()
                .collect();
            rebuilt.insert(base.path.clone(), reconstruct(&source, &edits).unwrap());
        }
        let patch = result.plan.patch.as_ref().unwrap();
        if !patch.is_empty() {
            for args in [vec!["apply", "--check", "-"], vec!["apply", "-"]] {
                let mut child = Command::new("git")
                    .arg("-C")
                    .arg(&copy.0)
                    .args(args)
                    .stdin(Stdio::piped())
                    .stderr(Stdio::piped())
                    .spawn()
                    .unwrap();
                child
                    .stdin
                    .take()
                    .unwrap()
                    .write_all(patch.as_bytes())
                    .unwrap();
                let output = child.wait_with_output().unwrap();
                assert!(
                    output.status.success(),
                    "{}\n{patch}",
                    String::from_utf8_lossy(&output.stderr)
                );
            }
        }
        for (path, text) in &rebuilt {
            assert_eq!(std::fs::read_to_string(copy.0.join(path)).unwrap(), *text);
            assert_eq!(
                std::fs::metadata(copy.0.join(path)).unwrap().mode(),
                std::fs::metadata(self.0.join(path)).unwrap().mode()
            );
        }
        rebuilt
    }
}
impl Drop for Repo {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}
fn anchor(path: &str, source: &str, text: &str) -> serde_json::Value {
    let start = source.find(text).unwrap();
    json!({"path":path,"range":{"start_byte":start,"end_byte":start+text.len()},"expected_text":text})
}
fn blocked(result: &PlanEnvelope, code: &str) {
    assert!(!result.plan.applicable, "{result:?}");
    assert!(result.plan.patch.is_none());
    assert!(result.plan.edits.is_none());
    assert!(
        result.plan.blockers.iter().any(|b| b.code == code),
        "{result:?}"
    );
    assert!(result.plan.proposed_edits.iter().all(|p| !p.applicable));
}
#[test]
fn multi_file_hunks_path_quotes_newlines_and_json_equivalence() {
    let repo = Repo::new();
    for (path, source) in [
        ("a.rs", "fn f(){ x.unwrap(); y.unwrap(); }\n"),
        ("spaces tabs\t\"é\\.rs", "fn f(){\r\n x.unwrap();\r\n}\r\n"),
        ("none.rs", "fn f(){ é.unwrap(); }"),
        (
            "mixed.rs",
            "fn f(){\r\n x.unwrap();\n\n\n\n\n\n\n\n y.unwrap();\r\n}",
        ),
    ] {
        repo.write(path, source);
    }
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(repo.0.join("a.rs"), std::fs::Permissions::from_mode(0o755)).unwrap();
    let result = repo.plan("$a.unwrap()", "$a.expect(\"why\")");
    let rebuilt = repo.apply(&result);
    assert_eq!(result.plan.selected_count, Some(6));
    for (path, new) in rebuilt {
        let original = std::fs::read_to_string(repo.0.join(path)).unwrap();
        assert_eq!(new, original.replace(".unwrap()", ".expect(\"why\")"));
    }
    assert!(
        result
            .plan
            .patch
            .unwrap()
            .contains("\\ No newline at end of file")
    );
}
#[test]
fn selection_noops_and_staleness() {
    let repo = Repo::new();
    let source = "fn f(){ x.unwrap(); y.unwrap(); }";
    repo.write("a.rs", source);
    let mut request = repo.request("$a.unwrap()", "$a.expect(\"why\")");
    request.selection = Some(vec![]);
    let result = repo.run(request.clone());
    assert_eq!(result.plan.selected_count, Some(0));
    assert_eq!(result.plan.patch.as_deref(), Some(""));
    assert!(result.plan.edits.unwrap().is_empty());
    request.selection =
        Some(serde_json::from_value(json!([anchor("a.rs", source, "x.unwrap()")])).unwrap());
    let result = repo.run(request.clone());
    assert_eq!(result.plan.selected_count, Some(1));
    assert!(repo.apply(&result)["a.rs"].contains("y.unwrap()"));
    request.selection.as_mut().unwrap()[0].expected_text = "changed".into();
    assert_eq!(repo.run(request).error.unwrap().code, "STALE_SELECTION");
}
#[test]
fn explicit_selection_on_thousand_match_scope_is_admitted() {
    let repo = Repo::new();
    let source = format!("fn f(){{ {} }}", "x.unwrap();".repeat(1000));
    repo.write("a.rs", &source);
    let mut request = repo.request("$a.unwrap()", "$a.expect(\"why\")");
    request.selection =
        Some(serde_json::from_value(json!([anchor("a.rs", &source, "x.unwrap()")])).unwrap());
    for cap in [500, 1] {
        request.max_matches = cap;
        let result = repo.run(request.clone());
        assert_eq!(result.status, "complete", "{result:?}");
        assert!(result.coverage.scope_exhaustive);
        assert_eq!(result.counts.total_matches, Some(1000));
        assert!(result.counts.total_is_exact);
        assert_eq!(result.counts.observed_matches, 1000);
        assert_eq!(result.counts.returned_matches, 1);
        assert_eq!(result.plan.selected_count, Some(1));
        assert_eq!(result.plan.matches.len(), 1);
        assert_eq!(result.plan.edits.as_ref().unwrap().len(), 1);
        assert_eq!(result.plan.integrity.semantic, "not_performed");
        assert_eq!(
            repo.apply(&result)["a.rs"],
            source.replacen(".unwrap()", ".expect(\"why\")", 1)
        );
        assert_eq!(
            std::fs::read_to_string(repo.0.join("a.rs")).unwrap(),
            source
        );
    }
}
#[test]
fn large_scope_selection_still_reports_typed_trivia_blocker() {
    let repo = Repo::new();
    let source = format!(
        "fn f(){{ x.unwrap(/* keep */); {} }}",
        "y.unwrap();".repeat(999)
    );
    repo.write("a.rs", &source);
    let mut request = repo.request("$a.unwrap()", "$a.expect(\"why\")");
    request.selection = Some(
        serde_json::from_value(json!([anchor("a.rs", &source, "x.unwrap(/* keep */)")])).unwrap(),
    );
    request.max_matches = 1;
    let result = repo.run(request);
    blocked(&result, "UNRETAINED_TRIVIA");
    assert_eq!(result.plan.state, "blocked");
    assert_eq!(result.counts.total_matches, Some(1000));
    assert!(result.counts.total_is_exact);
    assert_eq!(result.plan.selected_count, Some(1));
    assert_eq!(result.plan.integrity.semantic, "not_performed");
}
#[test]
fn no_selection_and_oversized_explicit_selection_still_honor_cap() {
    let repo = Repo::new();
    let source = format!("fn f(){{ {} }}", "x.unwrap();".repeat(1000));
    repo.write("a.rs", &source);
    let mut request = repo.request("$a.unwrap()", "$a.expect(\"why\")");
    let result = repo.run(request.clone());
    blocked(&result, "max_matches");
    assert_eq!(result.status, "partial");
    assert_eq!(result.plan.state, "incomplete");
    assert_eq!(result.counts.observed_matches, 1000);
    assert_eq!(result.counts.total_matches, None);
    assert!(!result.counts.total_is_exact);
    assert!(result.plan.matches.len() <= request.max_matches);

    request.max_matches = 1000;
    request.limits.text_bytes = 0;
    request.context.before_lines = 0;
    request.context.after_lines = 0;
    let result = repo.run(request.clone());
    assert!(result.plan.applicable, "{result:?}");
    assert_eq!(result.plan.selected_count, Some(1000));
    assert_eq!(result.counts.total_matches, Some(1000));
    assert!(result.counts.total_is_exact);

    let first = anchor("a.rs", &source, "x.unwrap()");
    let start = source.rfind("x.unwrap()").unwrap();
    let last = json!({"path":"a.rs","range":{"start_byte":start,"end_byte":start+"x.unwrap()".len()},"expected_text":"x.unwrap()"});
    request.selection = Some(serde_json::from_value(json!([first, last])).unwrap());
    request.max_matches = 1;
    let result = repo.run(request.clone());
    blocked(&result, "max_matches");
    assert_eq!(result.plan.state, "incomplete");
    assert!(result.plan.matches.len() <= 1);
    request.max_matches = 2;
    let result = repo.run(request);
    assert!(result.plan.applicable, "{result:?}");
    assert_eq!(result.plan.selected_count, Some(2));
    assert_eq!(result.counts.returned_matches, 2);
    assert_eq!(result.counts.total_matches, Some(1000));
    assert!(result.counts.total_is_exact);
}
#[test]
fn selection_counts_unselected_files_and_preserves_fail_closed_checks() {
    let repo = Repo::new();
    let source = format!("fn f(){{ {} }}", "x.unwrap();".repeat(600));
    repo.write("a.rs", &source);
    let other_source = format!("fn f(){{ {} }}", "y.unwrap();".repeat(400));
    repo.write("b.rs", &other_source);
    let mut request = repo.request("$a.unwrap()", "$a.expect(\"why\")");
    request.selection =
        Some(serde_json::from_value(json!([anchor("a.rs", &source, "x.unwrap()")])).unwrap());
    let result = repo.run(request.clone());
    assert!(result.plan.applicable, "{result:?}");
    assert_eq!(result.counts.matched_files, 2);
    assert_eq!(result.counts.total_matches, Some(1000));
    assert!(result.counts.total_is_exact);
    assert_eq!(result.plan.selected_count, Some(1));

    let mut over_cap = request.clone();
    over_cap
        .selection
        .as_mut()
        .unwrap()
        .push(serde_json::from_value(anchor("b.rs", &other_source, "y.unwrap()")).unwrap());
    over_cap.max_matches = 1;
    let result = repo.run(over_cap);
    blocked(&result, "max_matches");
    assert_eq!(result.counts.returned_matches, 1);

    request.selection.as_mut().unwrap()[0].expected_text = "changed".into();
    let result = repo.run(request.clone());
    assert_eq!(result.error.unwrap().code, "STALE_SELECTION");
    assert!(result.plan.edits.is_none() && result.plan.patch.is_none());
    assert_eq!(result.counts.total_matches, Some(1000));
    assert!(result.counts.total_is_exact);

    request.selection = Some(vec![]);
    let result = repo.run(request.clone());
    assert!(result.plan.applicable);
    assert_eq!(result.plan.selected_count, Some(0));
    assert_eq!(result.plan.patch.as_deref(), Some(""));
    assert!(result.plan.edits.unwrap().is_empty());
    assert_eq!(result.counts.total_matches, Some(1000));
    assert!(result.counts.total_is_exact);

    request.limits.max_files = 1;
    let result = repo.run(request);
    blocked(&result, "scan_incomplete");
    assert_eq!(result.counts.total_matches, None);
    assert!(!result.counts.total_is_exact);

    let result = repo.plan("$a.unwrap()", "$a.expect(\"why\")");
    blocked(&result, "max_matches");
    assert_eq!(result.counts.observed_matches, 600);
    assert_eq!(result.counts.matched_files, 1);
    assert_eq!(result.counts.total_matches, None);
    assert!(!result.counts.total_is_exact);
}
#[test]
fn syntax_refusal_nested_conflicts_templates_and_dollars() {
    let repo = Repo::new();
    repo.write("a.rs", "fn f(){ @ x.unwrap(); }");
    blocked(
        &repo.plan("$a.unwrap()", "$a.expect(\"why\")"),
        "PREEXISTING_SYNTAX_ERROR",
    );
    repo.write("a.rs", "fn f(){ x.unwrap().unwrap(); }");
    blocked(
        &repo.plan("$a.unwrap()", "$a.expect(\"why\")"),
        "OVERLAPPING_EDITS",
    );
    for (template, code) in [
        ("$missing", "UNBOUND_METAVARIABLE"),
        ("${a}", "INVALID_TEMPLATE"),
        ("$$a", "INVALID_TEMPLATE"),
        ("$a;", "INVALID_TEMPLATE"),
        ("f::<T>($a)", "INVALID_TEMPLATE"),
    ] {
        let result = repo.plan("$a.unwrap()", template);
        let error = result.error.unwrap();
        assert_eq!(error.code, code);
        assert_eq!(error.field.as_deref(), Some("replacement"));
        assert!(error.byte_offset.is_some());
    }
    repo.write("a.rs", "fn f(){ x.unwrap(); }");
    let result = repo.plan("$a.unwrap()", "$a.expect(r#\"$literal\"#)");
    assert!(repo.apply(&result)["a.rs"].contains("r#\"$literal\"#"));
    repo.write("a.rs", "fn f(){ pair(x,x); }");
    let result = repo.plan("pair($a,$a)", "pair($a,$a)");
    assert!(result.plan.applicable);
    assert_eq!(result.plan.patch.as_deref(), Some(""));
}
#[test]
fn required_trivia_survives_and_internal_loss_is_not_text_equality() {
    let repo = Repo::new();
    let source = "//! inner\n#[cfg(any())]\n/// docs\nfn f(){ x.unwrap(); // trailing\n use_it({\n/// local docs\nfn inner(){}\ninner()\n}); }\n";
    repo.write("a.rs", source);
    let result = repo.plan("$a.unwrap()", "$a.expect(\"why\")");
    let new = repo.apply(&result);
    assert_eq!(new["a.rs"], source.replace(".unwrap()", ".expect(\"why\")"));
    let result = repo.plan("use_it($a)", "other($a)");
    assert!(repo.apply(&result)["a.rs"].contains("/// local docs"));
    repo.write("a.rs", "fn f(){ x.unwrap(/* nested /* inner */ */); }");
    blocked(
        &repo.plan("$a.unwrap()", "$a.expect(\"why\")"),
        "UNRETAINED_TRIVIA",
    );
    blocked(
        &repo.plan("$a.unwrap()", "$a.expect(/* nested /* inner */ */\"why\")"),
        "UNRETAINED_TRIVIA",
    );
    repo.write(
        "a.rs",
        "fn f(){ pair((x /* distinct */),(x /* distinct */)); }",
    );
    blocked(&repo.plan("pair($a,$a)", "$a"), "UNRETAINED_TRIVIA");
}
#[test]
fn banner_default_and_preserving_external_internal_overrides() {
    let repo = Repo::new();
    let source = "fn f(){\n// --- banner\n\nx.unwrap(/* inside */);\n}";
    repo.write("a.rs", source);
    let mut request = repo.request("$a.unwrap()", "$a.expect(\"why\")");
    let target = anchor("a.rs", source, "x.unwrap(/* inside */)");
    request.trivia_overrides=Some(serde_json::from_value(json!([{ "trivia":anchor("a.rs",source,"/* inside */"),"disposition":"before_match","target_match":target }])).unwrap());
    let result = repo.run(request.clone());
    let new = repo.apply(&result);
    assert!(new["a.rs"].contains("/* inside */ x.expect"));
    assert!(
        result
            .plan
            .trivia_decisions
            .iter()
            .any(|d| d.reason == "banner" && d.selected_disposition == "keep_in_place")
    );
    request.trivia_overrides.as_mut().unwrap().push(serde_json::from_value(json!({"trivia":anchor("a.rs",source,"// --- banner"),"disposition":"after_match","target_match":target})).unwrap());
    let result = repo.run(request);
    let new = repo.apply(&result);
    assert_eq!(new["a.rs"].matches("// --- banner").count(), 1);
    assert!(new["a.rs"].contains("x.expect(\"why\")\n// --- banner\n;"));
}
#[test]
fn invalid_overrides_and_attachment_unknowns_fail_closed() {
    let repo = Repo::new();
    let source = "/// docs\nfn f(){ x.unwrap(); }";
    repo.write("a.rs", source);
    let mut request = repo.request("$a.unwrap()", "$a.expect(\"why\")");
    request.trivia_overrides=Some(serde_json::from_value(json!([{ "trivia":anchor("a.rs",source,"/// docs\n"),"disposition":"before_match","target_match":anchor("a.rs",source,"x.unwrap()") }])).unwrap());
    blocked(&repo.run(request.clone()), "UNSUPPORTED_TRIVIA_DISPOSITION");
    let duplicate = request.trivia_overrides.as_ref().unwrap()[0].clone();
    request.trivia_overrides.as_mut().unwrap().push(duplicate);
    assert_eq!(
        repo.run(request).error.unwrap().code,
        "INVALID_TRIVIA_OVERRIDE"
    );
}
#[test]
fn calls_leave_source_and_git_metadata_unchanged() {
    fn observe(path: &std::path::Path, output: &mut Vec<(PathBuf, Vec<u8>, u32, i64, i64)>) {
        let meta = std::fs::metadata(path).unwrap();
        output.push((
            path.into(),
            if meta.is_file() {
                std::fs::read(path).unwrap()
            } else {
                vec![]
            },
            meta.mode(),
            meta.mtime(),
            meta.mtime_nsec(),
        ));
        if meta.is_dir() {
            let mut children: Vec<_> = std::fs::read_dir(path)
                .unwrap()
                .map(|e| e.unwrap().path())
                .collect();
            children.sort();
            for child in children {
                observe(&child, output);
            }
        }
    }
    let repo = Repo::new();
    repo.write("a.rs", "fn f(){x.unwrap();}");
    let mut before = vec![];
    observe(&repo.0, &mut before);
    assert!(
        repo.plan("$a.unwrap()", "$a.expect(\"why\")")
            .plan
            .applicable
    );
    let mut request = repo.request("$a.unwrap()", "$a");
    request.selection = Some(vec![]);
    assert!(repo.run(request).plan.applicable);
    assert!(repo.plan("$a.unwrap()", "$unknown").error.is_some());
    let mut after = vec![];
    observe(&repo.0, &mut after);
    assert_eq!(before, after);
    repo.write("a.rs", "fn f(){ x.unwrap(); }");
    blocked(
        &repo.plan("$a.unwrap()", "$a // authored"),
        "NEW_SYNTAX_ERROR",
    );
}
#[test]
fn wire_limits_skip_incompleteness_and_original_capture_bytes() {
    let repo = Repo::new();
    repo.write("a.rs", "fn f(){ (x /*c*/ + y).unwrap(); }");
    let mut request = repo.request("$a.unwrap()", "$a.expect(\"why\")");
    request.limits.text_bytes = 0;
    let result = repo.run(request);
    assert!(repo.apply(&result)["a.rs"].contains("(x /*c*/ + y).expect"));
    assert!(result.plan.matches[0].span.text.is_none());
    let source = format!("fn f(){{ {} }}", vec!["x.unwrap();"; 400].join(" "));
    repo.write("a.rs", &source);
    let mut request = repo.request("$a.unwrap()", "$a.expect(\"why\")");
    request.limits.response_bytes = 65536;
    let result = repo.run(request);
    blocked(&result, "response_bytes");
    assert!(result.wire_bytes() <= 65536);
    assert!(result.plan.matches_omitted > 0);
    repo.write("binary.rs", "\0");
    blocked(
        &repo.plan("$a.unwrap()", "$a.expect(\"why\")"),
        "scan_incomplete",
    );
}
