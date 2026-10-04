use rust_sitter_mcp::{
    engine::Engine,
    result::{SearchEnvelope, SearchRequest},
};
use serde_json::json;
use std::os::unix::{
    ffi::OsStringExt,
    fs::{MetadataExt, PermissionsExt, symlink},
};
use std::{
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
};
static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Repo(PathBuf);
impl Repo {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "rust-sitter-search-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        git(&path, &["init", "--quiet"]);
        Self(path)
    }
    fn write(&self, path: &str, bytes: impl AsRef<[u8]>) {
        let path = self.0.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }
    fn request(&self, query: &str) -> SearchRequest {
        serde_json::from_value(json!({"repo_path":self.0,"query":query})).unwrap()
    }
    fn engine(&self) -> Engine {
        Engine::new(self.0.clone()).unwrap()
    }
}
impl Drop for Repo {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}
fn git(root: &Path, args: &[&str]) {
    let status = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .status()
        .unwrap();
    assert!(status.success());
}
fn run(engine: &Engine, request: SearchRequest) -> SearchEnvelope {
    engine.search(request, &AtomicBool::new(false))
}
fn ok(result: &SearchEnvelope) {
    assert!(result.error.is_none(), "{:?}", result.error);
}
fn code(result: SearchEnvelope, expected: &str) {
    assert_eq!(result.error.unwrap().code, expected);
}
const UNWRAP: &str = "((call_expression function: (field_expression value: (_expression) @a field: (field_identifier) @method) arguments: (arguments) @args) @match (#eq? @method \"unwrap\") (#rust-arity? @args \"0\"))";

#[test]
fn rooted_queries_exact_ranges_captures_context_and_written_syntax() {
    let repo = Repo::new();
    let source = "// before\r\nfn f() {\r\n\tlet é = thing.unwrap(/* comment */);\r\n other.unwrap(x);\r\n let s = \"fake.unwrap()\"; // fake.unwrap()\r\n mac!(hidden.unwrap());\r\n}\r\n";
    repo.write("src/a.rs", source);
    let result = run(&repo.engine(), repo.request(UNWRAP));
    ok(&result);
    assert_eq!(result.status, "complete");
    assert!(result.coverage.scope_exhaustive);
    assert_eq!(result.counts.total_matches, Some(1));
    let m = &result.matches[0];
    assert_eq!(m.path, "src/a.rs");
    assert_eq!(m.span.text.as_deref(), Some("thing.unwrap(/* comment */)"));
    assert_eq!(
        &source[m.span.range.start_byte..m.span.range.end_byte],
        m.span.text.as_ref().unwrap()
    );
    assert_eq!(m.span.start.line, 3);
    assert_eq!(m.span.start.byte_column, 10);
    assert_eq!(
        m.captures["a"].as_ref().unwrap()[0].text.as_deref(),
        Some("thing")
    );
    assert_eq!(m.context.before[0].text.as_deref(), Some("// before\r\n"));
    assert_eq!(
        m.context.after[0].text.as_deref(),
        Some(" other.unwrap(x);\r\n")
    );
    assert!(!m.syntax.file_has_recovery);
}
#[test]
fn descendant_result_preserves_captures_outside_root_and_repeated_names() {
    let repo = Repo::new();
    repo.write("x.rs", "fn f(){ pair(x,y); }");
    let result = run(&repo.engine(),repo.request("(call_expression function: (identifier) @callee arguments: (arguments (identifier) @match (identifier) @value))"));
    ok(&result);
    assert_eq!(result.matches[0].span.text.as_deref(), Some("x"));
    assert_eq!(
        result.matches[0].captures["callee"].as_ref().unwrap()[0]
            .text
            .as_deref(),
        Some("pair")
    );
    let result = run(
        &repo.engine(),
        repo.request(
            "(call_expression arguments: (arguments (identifier) @v (identifier) @v)) @match",
        ),
    );
    ok(&result);
    let values = result.matches[0].captures["v"].as_ref().unwrap();
    assert_eq!(values.len(), 2);
    assert_eq!(values[0].text.as_deref(), Some("x"));
    assert_eq!(values[1].text.as_deref(), Some("y"));
}
#[test]
fn ignore_hard_boundaries_and_positive_filter_intersection() {
    let repo = Repo::new();
    repo.write(
        ".gitignore",
        "ignored.rs\nsub/*.rs\n!sub/keep.rs\n!target/**\n",
    );
    repo.write(".ignore", "root.rs\n");
    for p in [
        "root.rs",
        "ignored.rs",
        "sub/no.rs",
        "sub/keep.rs",
        "other/a.rs",
        "target/a.rs",
        "sub/target/a.rs",
        "nested/a.rs",
        "hidden/.x.rs",
    ] {
        repo.write(p, "fn f(){x();}");
    }
    git(&repo.0.join("nested"), &["init", "--quiet"]);
    symlink(repo.0.join("root.rs"), repo.0.join("link.rs")).unwrap();
    symlink(repo.0.join("other"), repo.0.join("linked")).unwrap();
    let engine = repo.engine();
    let result = run(&engine, repo.request("(call_expression) @match"));
    ok(&result);
    let paths: Vec<_> = result.matches.iter().map(|m| m.path.as_str()).collect();
    assert_eq!(
        paths,
        ["hidden/.x.rs", "other/a.rs", "root.rs", "sub/keep.rs"]
    );
    assert_eq!(result.skipped["nested_repository"].count, 1);
    assert_eq!(result.skipped["symlink"].count, 2);
    assert!(result.coverage.scope_exhaustive);
    let mut request = repo.request("(call_expression) @match");
    request.paths = Some(vec!["sub/./".into(), "root.rs".into()]);
    request.globs = Some(vec!["sub/**".into()]);
    let result = run(&engine, request);
    ok(&result);
    assert_eq!(result.matches.len(), 1);
    assert_eq!(result.matches[0].path, "sub/keep.rs");
    let mut request = repo.request("(call_expression) @match");
    request.globs = Some(vec!["*.rs".into()]);
    assert_eq!(run(&engine, request).matches.len(), 1);
}
#[test]
fn eligibility_precedence_sniff_boundaries_caps_and_bounded_examples() {
    let repo = Repo::new();
    repo.write("good.rs", "fn f(){}\n");
    for i in 0..5 {
        repo.write(&format!("binary{i}.rs"), b"\0fn f(){}");
    }
    repo.write("bad.rs", [0xff]);
    repo.write("oversize.rs", vec![0; 2 * 1024 * 1024 + 1]);
    let mut inside = vec![b' '; 8193];
    inside[8191] = 0;
    repo.write("inside.rs", inside);
    let mut outside = vec![b' '; 8193];
    outside[8192] = 0;
    repo.write("outside.rs", outside);
    let invalid_name = std::ffi::OsString::from_vec(b"invalid-\xff.rs".to_vec());
    let non_utf8_path_created = match std::fs::write(repo.0.join(invalid_name), b"fn f(){}") {
        Ok(()) => true,
        Err(error) if cfg!(target_os = "macos") && error.raw_os_error() == Some(92) => false,
        Err(error) => panic!("non-UTF-8 fixture failed: {error}"),
    };
    let result = run(&repo.engine(), repo.request("(function_item) @match"));
    ok(&result);
    assert_eq!(result.skipped["oversized"].count, 1);
    assert_eq!(result.skipped["binary"].count, 6);
    assert_eq!(result.skipped["non_utf8"].count, 1);
    assert_eq!(result.skipped["binary"].examples.len(), 3);
    assert_eq!(result.skipped["binary"].examples_omitted, 3);
    assert_eq!(
        result.skipped["non_utf8_path"].count,
        u64::from(non_utf8_path_created)
    );
    if non_utf8_path_created {
        assert!(result.skipped["non_utf8_path"].examples[0].path.is_none());
        assert!(
            result.skipped["non_utf8_path"].examples[0]
                .path_bytes_hex
                .is_some()
        );
    }
    assert_eq!(result.counts.eligible_files, 2);
    assert_eq!(result.status, "partial");
    assert!(!result.coverage.scope_exhaustive);
    let mut request = repo.request("(function_item) @match");
    request.limits.max_file_bytes = 9;
    request.paths = Some(vec!["good.rs".into()]);
    let result = run(&repo.engine(), request.clone());
    ok(&result);
    assert_eq!(result.skipped["oversized"].count, 0);
    request.limits.max_file_bytes = 8;
    assert_eq!(run(&repo.engine(), request).skipped["oversized"].count, 1);
}
#[test]
fn malformed_query_and_scope_errors_are_distinct_before_discovery() {
    let repo = Repo::new();
    repo.write("a.rs", "fn f(){}");
    let engine = repo.engine();
    for (query, error) in [
        ("(", "INVALID_QUERY"),
        ("(identifier)", "INVALID_QUERY_ROOT"),
        ("((identifier) @match (identifier))", "INVALID_QUERY_ROOT"),
        ("(identifier) @match?", "INVALID_QUERY_ROOT"),
        (
            "((identifier) @match (#set! foo \"bar\"))",
            "UNSUPPORTED_QUERY_OPERATION",
        ),
        (
            "((identifier) @match (#is? local))",
            "UNSUPPORTED_QUERY_OPERATION",
        ),
        (
            "((identifier) @match (#offset! @match \"1\"))",
            "UNSUPPORTED_QUERY_OPERATION",
        ),
        (
            "((arguments) @match (#rust-arity? @match \"-1\"))",
            "INVALID_QUERY",
        ),
        (
            "((arguments) @match (#rust-arity? @match \"4294967296\"))",
            "INVALID_QUERY",
        ),
        (
            "((identifier) @match (#rust-arity? @match \"0\"))",
            "INVALID_QUERY_ROOT",
        ),
    ] {
        code(run(&engine, repo.request(query)), error);
    }
    for path in ["../", "/tmp", "target", "link.rs"] {
        if path == "target" {
            repo.write("target/x.rs", "");
        }
        if path == "link.rs" {
            symlink(repo.0.join("a.rs"), repo.0.join(path)).unwrap();
        }
        let mut req = repo.request("(function_item) @match");
        req.paths = Some(vec![path.into()]);
        assert!(run(&engine, req).error.is_some());
    }
    for glob in ["!*.rs", "/src/**", "../*", "src/", "{a,b}.rs", "#comment"] {
        let mut req = repo.request("(function_item) @match");
        req.globs = Some(vec![glob.into()]);
        code(run(&engine, req), "INVALID_GLOB");
    }
    let mut req = repo.request("(function_item) @match");
    req.paths = Some(vec![]);
    code(run(&engine, req), "INVALID_PARAMS");
    let mut req = repo.request("(function_item) @match");
    req.repo_path = repo.0.join("missing").to_str().unwrap().into();
    code(run(&engine, req), "PATH_NOT_FOUND");
    let mut req = repo.request("(function_item) @match");
    req.repo_path = std::env::temp_dir().to_str().unwrap().into();
    code(run(&engine, req), "NOT_GIT_WORKTREE");
}
#[test]
fn recovery_missing_tokens_and_diagnostic_omissions_remain_searchable() {
    let repo = Repo::new();
    repo.write("bad.rs", "fn f() { @ x.unwrap(); }\n");
    repo.write(
        "missing.rs",
        "({ \"session_id\": session_id, \"output\": output })",
    );
    let mut request = repo.request("(call_expression) @match");
    request.limits.diagnostic_count = 0;
    let result = run(&repo.engine(), request);
    ok(&result);
    assert_eq!(result.matches.len(), 1);
    assert!(result.matches[0].syntax.file_has_recovery);
    assert!(result.matches[0].syntax.enclosing_has_recovery);
    assert!(result.diagnostics.is_empty());
    assert!(result.diagnostics_omitted > 0);
    assert!(!result.coverage.scope_exhaustive);
    assert_eq!(result.status, "partial");
    let result = run(&repo.engine(), repo.request("(MISSING) @match"));
    ok(&result);
    assert!(!result.matches.is_empty());
    assert!(
        result
            .matches
            .iter()
            .all(|m| m.span.range.start_byte == m.span.range.end_byte)
    );
    assert!(result.diagnostics.iter().any(|d| d.code == "MISSING"));
}
#[test]
fn continued_pages_are_deterministic_without_omissions_and_terminal_totals_are_honest() {
    let repo = Repo::new();
    repo.write("a.rs", format!("fn f(){{ {} }}", "x();".repeat(70)));
    repo.write("b.rs", "fn f(){y();}");
    let engine = repo.engine();
    let mut request = repo.request("(call_expression) @match");
    request.page_size = 2;
    let mut ids = Vec::new();
    let first = run(&engine, request.clone());
    ok(&first);
    let first_token = first.next_cursor.clone().unwrap();
    let mut page = first;
    loop {
        ids.extend(page.matches.iter().map(|m| m.id.clone()));
        if let Some(token) = page.next_cursor.take() {
            request.cursor = Some(token);
            page = run(&engine, request.clone());
            ok(&page);
        } else {
            break;
        }
    }
    assert_eq!(ids.len(), 71);
    let mut unique = ids.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(unique.len(), 71);
    assert_eq!(page.counts.total_matches, None);
    assert!(!page.counts.total_is_exact);
    assert_eq!(page.has_more, Some(false));
    request.cursor = Some(first_token);
    let retry = run(&engine, request.clone());
    ok(&retry);
    assert_eq!(retry.matches[0].id, ids[2]);
    request.query = "(function_item) @match".into();
    code(run(&engine, request.clone()), "CURSOR_SCOPE_MISMATCH");
    request.query = "(call_expression) @match".into();
    repo.write("a.rs", "fn f(){changed();}");
    code(run(&engine, request.clone()), "STALE_CURSOR");
    request.cursor = Some("v1/1/999/0/bad".into());
    code(run(&engine, request), "INVALID_CURSOR");
}
#[test]
fn page_boundary_lookahead_capacity_and_discovery_stoppage() {
    let repo = Repo::new();
    repo.write("a.rs", "fn f(){x();x();}");
    let engine = repo.engine();
    let mut request = repo.request("(call_expression) @match");
    request.page_size = 2;
    let result = run(&engine, request.clone());
    ok(&result);
    assert!(result.next_cursor.is_none());
    assert_eq!(result.counts.total_matches, Some(2));
    request.page_size = 1;
    for _ in 0..32 {
        assert!(run(&engine, request.clone()).next_cursor.is_some());
    }
    code(run(&engine, request.clone()), "CURSOR_CAPACITY");
    repo.write("b.rs", "fn f(){x();}");
    request.limits.max_files = 1;
    let result = run(&repo.engine(), request);
    ok(&result);
    assert!(!result.coverage.scan_exhausted);
    assert!(result.next_cursor.is_none());
    assert_eq!(result.has_more, None);
    assert_eq!(result.counts.total_matches, None);
}
#[test]
fn wire_and_text_bounds_use_full_original_ranges_and_descriptors() {
    let repo = Repo::new();
    let long = "q".repeat(80_000);
    repo.write("x.rs", format!("fn f(){{ call(\"{long}\"); }}"));
    let engine = repo.engine();
    let mut request = repo.request("(call_expression) @match");
    request.limits.response_bytes = 64 * 1024;
    request.limits.text_bytes = 64 * 1024;
    let result = run(&engine, request);
    ok(&result);
    assert!(result.wire_bytes() <= 64 * 1024);
    assert!(result.matches[0].span.text.is_none());
    assert_eq!(result.matches[0].span.text_bytes, 80_008);
    let mut request = repo.request("(call_expression) @match");
    request.limits.text_bytes = 0;
    assert!(run(&engine, request).matches[0].span.text_omitted);
    repo.write(
        "x.rs",
        format!("fn f(){{ {} }}", "call(\"a\\\\b\\\"c\");".repeat(300)),
    );
    let mut request = repo.request("(call_expression) @match");
    request.limits.response_bytes = 64 * 1024;
    request.page_size = 1000;
    let result = run(&engine, request);
    ok(&result);
    assert!(result.wire_bytes() <= 64 * 1024);
    assert!(result.next_cursor.is_some());
    assert!(result.matches.len() < 300);
}
#[test]
fn linked_worktree_resolution_uses_the_worktree_not_main() {
    let repo = Repo::new();
    repo.write("src/a.rs", "fn f(){x();}");
    git(&repo.0, &["add", "src/a.rs"]);
    git(
        &repo.0,
        &[
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "commit",
            "--quiet",
            "-m",
            "fixture",
        ],
    );
    let linked = repo.0.join("linked");
    git(
        &repo.0,
        &[
            "worktree",
            "add",
            "--quiet",
            "--detach",
            linked.to_str().unwrap(),
        ],
    );
    let mut request = repo.request("(call_expression) @match");
    request.repo_path = linked.join("src/a.rs").to_str().unwrap().into();
    let result = run(&repo.engine(), request);
    ok(&result);
    assert_eq!(
        result.root.as_deref(),
        linked.canonicalize().unwrap().to_str()
    );
    assert_eq!(result.matches.len(), 1);
    git(&repo.0, &["worktree", "remove", linked.to_str().unwrap()]);
}
fn observation(root: &Path) -> Vec<(PathBuf, Vec<u8>, u32, i64, i64)> {
    fn walk(root: &Path, path: &Path, result: &mut Vec<(PathBuf, Vec<u8>, u32, i64, i64)>) {
        let metadata = std::fs::symlink_metadata(path).unwrap();
        let bytes = if metadata.is_file() {
            std::fs::read(path).unwrap()
        } else {
            Vec::new()
        };
        result.push((
            path.strip_prefix(root).unwrap().to_owned(),
            bytes,
            metadata.mode(),
            metadata.mtime(),
            metadata.mtime_nsec(),
        ));
        if metadata.is_dir() {
            let mut children: Vec<_> = std::fs::read_dir(path)
                .unwrap()
                .map(|e| e.unwrap().path())
                .collect();
            children.sort();
            for child in children {
                walk(root, &child, result);
            }
        }
    }
    let mut result = Vec::new();
    walk(root, root, &mut result);
    result
}
#[test]
fn successful_empty_failed_and_incomplete_calls_do_not_mutate_source_or_git() {
    let repo = Repo::new();
    repo.write("x.rs", "fn f(){x();x();}");
    let engine = repo.engine();
    let before = observation(&repo.0);
    ok(&run(&engine, repo.request("(call_expression) @match")));
    let empty = run(&engine, repo.request("(struct_item) @match"));
    ok(&empty);
    assert!(empty.matches.is_empty());
    assert_eq!(empty.status, "complete");
    assert_eq!(empty.counts.total_matches, Some(0));
    code(run(&engine, repo.request("(")), "INVALID_QUERY");
    let mut request = repo.request("(call_expression) @match");
    request.page_size = 1;
    ok(&run(&engine, request));
    assert_eq!(observation(&repo.0), before);
    let flag = AtomicBool::new(true);
    code(
        engine.search(repo.request("(call_expression) @match"), &flag),
        "CANCELLED",
    );
}
#[test]
fn continuation_does_not_resume_when_snapshot_cannot_be_validated() {
    let repo = Repo::new();
    repo.write("a.rs", "fn f(){x();x();}");
    repo.write("b.rs", "fn f(){y();}");
    let engine = repo.engine();
    let mut request = repo.request("(call_expression) @match");
    request.page_size = 1;
    request.cursor = run(&engine, request.clone()).next_cursor;
    assert!(request.cursor.is_some());
    std::fs::set_permissions(repo.0.join("b.rs"), std::fs::Permissions::from_mode(0o0)).unwrap();
    let result = run(&engine, request);
    std::fs::set_permissions(repo.0.join("b.rs"), std::fs::Permissions::from_mode(0o644)).unwrap();
    ok(&result);
    assert_eq!(result.status, "partial");
    assert!(result.matches.is_empty());
    assert!(result.next_cursor.is_none());
    assert_eq!(result.has_more, None);
    assert_eq!(result.counts.total_matches, None);
}

#[test]
fn execution_guard_discards_unfinished_file_but_keeps_observed_count() {
    let repo = Repo::new();
    repo.write(
        "many.rs",
        format!("fn f() {{ {} }}", "x();".repeat(100_001)),
    );
    let result = run(&repo.engine(), repo.request("(call_expression) @match"));
    ok(&result);
    assert_eq!(result.status, "partial");
    assert!(result.matches.is_empty());
    assert_eq!(result.counts.observed_matches, 100_000);
    assert_eq!(result.counts.total_matches, None);
    assert!(result.next_cursor.is_some());
    assert_eq!(result.has_more, None);
    assert!(
        result
            .truncation_reasons
            .iter()
            .any(|r| r == "candidate_match_limit")
    );
}

#[test]
fn unreadable_and_aggregate_source_bounds_are_not_empty_successes() {
    let repo = Repo::new();
    repo.write("x.rs", "fn f(){x();}");
    std::fs::set_permissions(repo.0.join("x.rs"), std::fs::Permissions::from_mode(0o0)).unwrap();
    let result = run(&repo.engine(), repo.request("(call_expression) @match"));
    ok(&result);
    assert_eq!(result.skipped["unreadable"].count, 1);
    assert_eq!(result.status, "partial");
    std::fs::set_permissions(repo.0.join("x.rs"), std::fs::Permissions::from_mode(0o644)).unwrap();
    let mut request = repo.request("(call_expression) @match");
    request.limits.max_source_bytes = 1;
    let result = run(&repo.engine(), request);
    ok(&result);
    assert_eq!(result.status, "partial");
    assert!(result.next_cursor.is_none());
    assert_eq!(result.counts.total_matches, None);
}
