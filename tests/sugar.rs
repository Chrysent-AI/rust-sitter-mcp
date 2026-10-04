use rust_sitter_mcp::{
    engine::Engine,
    result::{PatternRequest, SearchEnvelope, SearchRequest},
};
use serde_json::json;
use std::{
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
};

static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Repo(PathBuf);
impl Repo {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "rust-sitter-sugar-{}-{}",
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
    fn write(&self, path: &str, source: &str) {
        let path = self.0.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, source).unwrap();
    }
    fn engine(&self) -> Engine {
        Engine::new(self.0.clone()).unwrap()
    }
    fn request(&self, pattern: &str) -> PatternRequest {
        serde_json::from_value(json!({"repo_path":self.0,"pattern":pattern})).unwrap()
    }
    fn search(&self, pattern: &str) -> SearchEnvelope {
        let result = self
            .engine()
            .search_pattern(self.request(pattern), &AtomicBool::new(false));
        assert!(result.error.is_none(), "{pattern}: {:?}", result.error);
        result
    }
}
impl Drop for Repo {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}
fn texts(result: &SearchEnvelope) -> Vec<&str> {
    result
        .matches
        .iter()
        .map(|m| m.span.text.as_deref().unwrap())
        .collect()
}
const UNWRAP_QUERY: &str = "((call_expression function: (field_expression value: (_expression) @a field: (field_identifier) @method) arguments: (arguments) @args) @match (#eq? @method \"unwrap\") (#rust-arity? @args \"0\"))";

#[test]
fn minimum_patterns_unification_capture_ranges_and_raw_equivalence() {
    let repo = Repo::new();
    repo.write("a.rs", "fn f(){ thing.unwrap(/* comment */); other.unwrap(x); thing.expect(\"why\"); other.expect(x,y); pair(x,x); pair(x,y); pair(1,1); pair((x + y),(x+y)); pair((x /*c*/),(x /*c*/)); pair((x /*c*/),(x /*d*/)); pair(x,x,); }");
    repo.write("sub/b.rs", "fn f(){ é.unwrap(); pair(1,2); let s = \"fake.unwrap()\"; // fake.unwrap()\n mac!(hidden.unwrap()); }");
    let sugar = repo.search("$a.unwrap()");
    assert_eq!(texts(&sugar), ["thing.unwrap(/* comment */)", "é.unwrap()"]);
    assert_eq!(sugar.tool, "search");
    assert_eq!(sugar.counts.total_matches, Some(2));
    let raw: SearchRequest =
        serde_json::from_value(json!({"repo_path":repo.0,"query":UNWRAP_QUERY})).unwrap();
    let raw = repo.engine().search(raw, &AtomicBool::new(false));
    assert!(raw.error.is_none());
    for (s, r) in sugar.matches.iter().zip(&raw.matches) {
        assert_eq!(s.path, r.path);
        assert_eq!(s.span.range, r.span.range);
        assert_eq!(s.span.text, r.span.text);
        assert_eq!(s.captures.len(), 1);
        let source = std::fs::read_to_string(repo.0.join(&s.path)).unwrap();
        let a = &s.captures["a"].as_ref().unwrap()[0];
        assert_eq!(
            a.text.as_deref(),
            Some(&source[a.range.start_byte..a.range.end_byte])
        );
        assert_eq!(a.range.end_byte, s.span.range.start_byte + a.text_bytes);
    }
    let expect = repo.search("$a.expect($b)");
    assert_eq!(texts(&expect), ["thing.expect(\"why\")"]);
    assert_eq!(
        expect.matches[0].captures["b"].as_ref().unwrap()[0]
            .text
            .as_deref(),
        Some("\"why\"")
    );
    let unified = repo.search("pair($a, $a)");
    assert_eq!(
        texts(&unified),
        ["pair(x,x)", "pair(1,1)", "pair((x /*c*/),(x /*c*/))"]
    );
    for m in &unified.matches {
        let a = m.captures["a"].as_ref().unwrap();
        assert_eq!(a.len(), 2);
        assert_eq!(a[0].text, a[1].text);
        assert!(a[0].range.end_byte <= a[1].range.start_byte);
    }
    let independent = repo.search("pair($a,$b)");
    assert_eq!(independent.matches.len(), 7);
    assert!(texts(&independent).contains(&"pair(1,1)"));
    assert!(texts(&independent).contains(&"pair(1,2)"));
}

#[test]
fn accepted_expression_roots_and_compositions_are_live_parsed_and_matched() {
    let repo = Repo::new();
    // Every supported root/composition, including byte/C-prefixed literal kinds.
    let examples = [
        ("$a", "value"),
        ("$_", "value"),
        ("$a1", "value"),
        ("value", "value"),
        ("self", "self"),
        ("std::mem::drop", "std::mem::drop"),
        ("::path::value", "::path::value"),
        ("12u32", "12u32"),
        ("1.25f64", "1.25f64"),
        ("true", "true"),
        ("'é'", "'é'"),
        ("b'x'", "b'x'"),
        ("\"$a\"", "\"$a\""),
        ("b\"$a\"", "b\"$a\""),
        ("c\"$a\"", "c\"$a\""),
        ("r##\"$a\"##", "r##\"$a\"##"),
        ("br#\"$a\"#", "br#\"$a\"#"),
        ("cr#\"$a\"#", "cr#\"$a\"#"),
        ("$f($a)", "function(value)"),
        ("$a.field", "value.field"),
        ("$a.0", "value.0"),
        ("($a)", "(value)"),
        ("()", "()"),
        ("($a, $b)", "(value, other)"),
        ("($a,)", "(value,)"),
        ("[$a, $b]", "[value, other]"),
        ("[$a; $b]", "[value; count]"),
        ("[]", "[]"),
        ("-$a", "-value"),
        ("!$a", "!value"),
        ("*$a", "*value"),
        ("&$a", "&value"),
        ("&mut $a", "&mut value"),
        ("&raw const $a", "&raw const value"),
        ("&raw mut $a", "&raw mut value"),
        ("$a?", "value?"),
        ("$a.await", "value.await"),
        ("$a[$b]", "value[index]"),
        ("$a + $b", "value + other"),
        ("$a << $b", "value << other"),
        ("$a && $b", "value && other"),
        ("$a = $b", "value = other"),
        ("$a += $b", "value += other"),
        ("$a..$b", "value..other"),
        ("$a..=$b", "value..=other"),
        ("..$a", "..value"),
        ("$a..", "value.."),
        ("..", ".."),
    ];
    for (pattern, candidate) in examples {
        repo.write("x.rs", &format!("fn f() {{ let result = {candidate}; }}"));
        let result = repo.search(pattern);
        assert_eq!(texts(&result), [candidate], "pattern: {pattern}");
        assert!(
            !result.matches[0].syntax.file_has_recovery,
            "candidate: {candidate}"
        );
    }
}

#[test]
fn exact_significant_structure_includes_punctuation_and_optional_endpoints() {
    let repo = Repo::new();
    repo.write("x.rs", "fn f(){ f(a); f(a,b); f(a,); f::<T>(a); x.field; x.other; x.0; x.1; let a=[1,2]; let b=[1;2]; let c=[1,2,]; let d=1..2; let e=1..=2; let f=1..; let g=..2; let h=1+2; let i=1-2; }");
    for (pattern, expected) in [
        ("f($a)", "f(a)"),
        ("f($a,)", "f(a,)"),
        ("$a.field", "x.field"),
        ("$a.0", "x.0"),
        ("[$a,$b]", "[1,2]"),
        ("[$a;$b]", "[1;2]"),
        ("[$a,$b,]", "[1,2,]"),
        ("$a..$b", "1..2"),
        ("$a..=$b", "1..=2"),
        ("$a..", "1.."),
        ("..$a", "..2"),
        ("$a+$b", "1+2"),
    ] {
        assert_eq!(texts(&repo.search(pattern)), [expected], "{pattern}");
    }
}

#[test]
fn comments_are_extras_but_literal_bytes_and_captured_bytes_are_exact() {
    let repo = Repo::new();
    repo.write("x.rs", "fn f(){ pair(/* nested /* inner */ */x, // line\nx); pair(\"a b\",\"a b\"); pair(\"a b\",\"ab\"); pair((x + y),(x+y)); f(r#\"$a\"#); f(r##\"$a\"##); f(\"a\\nb\"); f(\"a\nb\"); }");
    assert_eq!(
        texts(&repo.search("pair($a, /* $... */ $a) // $not_a_reference\n")),
        [
            "pair(/* nested /* inner */ */x, // line\nx)",
            "pair(\"a b\",\"a b\")"
        ]
    );
    let literal = repo.search("f(r#\"$a\"#)");
    assert_eq!(texts(&literal), ["f(r#\"$a\"#)"]);
    assert!(literal.matches[0].captures.is_empty());
    assert_eq!(texts(&repo.search("f(\"a\\nb\")")), ["f(\"a\\nb\")"]);
}

#[test]
fn metavariables_bind_whole_broader_expressions_but_authored_forms_are_rejected() {
    let repo = Repo::new();
    repo.write("x.rs", "fn f(){ use_it({ value }); use_it(|x| x); use_it(make::<T>()); use_it(mac!(hidden.unwrap())); use_it(value as Type); }");
    let result = repo.search("use_it($a)");
    assert_eq!(result.matches.len(), 5);
    assert_eq!(
        result.matches[3].captures["a"].as_ref().unwrap()[0]
            .text
            .as_deref(),
        Some("mac!(hidden.unwrap())")
    );
    assert!(repo.search("$a.unwrap()").matches.is_empty());
    for pattern in [
        "{value}",
        "if x { y } else { z }",
        "|x| x",
        "S { field: x }",
        "x as Type",
        "f::<T>()",
        "mac!(x)",
        "return x",
        "loop {}",
        "async { x }",
        "const { x }",
        "<T as Trait>::value",
    ] {
        let result = repo
            .engine()
            .search_pattern(repo.request(pattern), &AtomicBool::new(false));
        assert_eq!(
            result.error.as_ref().unwrap().code,
            "UNSUPPORTED_PATTERN_FORM",
            "{pattern}: {:?}",
            result.error
        );
        assert!(result.error.unwrap().byte_offset.is_some());
        assert_eq!(result.counts.scanned_files, 0);
    }
}

#[test]
fn invalid_names_positions_sequences_annotations_and_unbalanced_syntax_have_offsets() {
    let repo = Repo::new();
    let engine = repo.engine();
    for (pattern, code) in [
        ("$", "INVALID_PATTERN"),
        ("${name}", "INVALID_PATTERN"),
        ("$...", "INVALID_PATTERN"),
        ("pair($a...)", "INVALID_PATTERN"),
        ("pair($a*)", "INVALID_PATTERN"),
        ("$$a", "INVALID_PATTERN"),
        ("$__ssr_private", "INVALID_PATTERN"),
        ("$aé", "INVALID_PATTERN"),
        ("$9", "INVALID_PATTERN"),
        ("prefix_$a", "INVALID_PATTERN"),
        ("f($a", "INVALID_PATTERN"),
        ("x; y", "INVALID_PATTERN"),
        ("x;", "INVALID_PATTERN"),
        ("x @name", "INVALID_PATTERN"),
        ("prefix$a", "INVALID_PATTERN"),
        ("$a.$b", "UNSUPPORTED_PLACEHOLDER_POSITION"),
        ("$a::$b", "UNSUPPORTED_PLACEHOLDER_POSITION"),
        ("f::<$a>()", "UNSUPPORTED_PLACEHOLDER_POSITION"),
        ("|$a| x", "UNSUPPORTED_PLACEHOLDER_POSITION"),
        ("mac!($a)", "UNSUPPORTED_PLACEHOLDER_POSITION"),
    ] {
        let result = engine.search_pattern(repo.request(pattern), &AtomicBool::new(false));
        let error = result.error.expect(pattern);
        assert_eq!(error.code, code, "{pattern}: {error:?}");
        assert_eq!(error.field.as_deref(), Some("pattern"));
        assert!(error.byte_offset.unwrap() <= pattern.len());
        assert_eq!(result.tool, "search");
        assert!(result.root.is_none());
    }
}

#[test]
fn shared_discovery_pagination_scope_and_display_limits_do_not_change_unification() {
    let repo = Repo::new();
    repo.write(".gitignore", "ignored.rs\n");
    for p in ["a.rs", "sub/b.rs", "ignored.rs", "target/no.rs"] {
        repo.write(p, "fn f(){ pair(x,x); pair(x,y); pair(1,1); }");
    }
    let engine = repo.engine();
    let mut request = repo.request("pair($a,$a)");
    request.page_size = 1;
    request.limits.text_bytes = 0;
    let mut ranges = Vec::new();
    loop {
        let result = engine.search_pattern(request.clone(), &AtomicBool::new(false));
        assert!(result.error.is_none(), "{:?}", result.error);
        assert_eq!(result.matches.len(), 1);
        let m = &result.matches[0];
        assert!(m.span.text_omitted);
        assert_eq!(m.captures["a"].as_ref().unwrap().len(), 2);
        ranges.push((m.path.clone(), m.span.range.clone()));
        if result.next_cursor.is_none() {
            assert!(!result.counts.total_is_exact);
            break;
        }
        request.cursor = result.next_cursor;
    }
    assert_eq!(ranges.len(), 4);
    let mut unique = ranges.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(unique.len(), 4);
    request.cursor = None;
    let first = engine.search_pattern(request.clone(), &AtomicBool::new(false));
    let token = first.next_cursor.unwrap();
    request.cursor = Some(token.clone());
    request.pattern = "pair($a,$b)".into();
    assert_eq!(
        engine
            .search_pattern(request.clone(), &AtomicBool::new(false))
            .error
            .unwrap()
            .code,
        "CURSOR_SCOPE_MISMATCH"
    );
    let mut raw: SearchRequest = serde_json::from_value(json!({"repo_path":repo.0,"query":"pair($a,$a)","page_size":1,"limits":{"text_bytes":0},"cursor":token})).unwrap();
    raw.query = "(call_expression) @match".into();
    assert_eq!(
        engine
            .search(raw, &AtomicBool::new(false))
            .error
            .unwrap()
            .code,
        "CURSOR_SCOPE_MISMATCH"
    );
    request.pattern = "pair($a,$a)".into();
    repo.write("a.rs", "fn f(){ pair(y,y); }");
    assert_eq!(
        engine
            .search_pattern(request.clone(), &AtomicBool::new(false))
            .error
            .unwrap()
            .code,
        "STALE_CURSOR"
    );
    request.cursor = None;
    request.paths = Some(vec!["sub".into()]);
    request.globs = Some(vec!["sub/**".into()]);
    assert_eq!(
        engine
            .search_pattern(request, &AtomicBool::new(false))
            .matches[0]
            .path,
        "sub/b.rs"
    );
}

#[test]
fn concrete_roots_only_match_expression_positions_not_bindings_or_macro_tokens() {
    let repo = Repo::new();
    repo.write("x.rs", "fn value(){ let value = value; value(value); use path::value; mac!(value, 1, \"value\"); let n = 1; let s = \"value\"; }");
    assert_eq!(texts(&repo.search("value")), ["value", "value", "value"]);
    assert_eq!(texts(&repo.search("1")), ["1"]);
    assert_eq!(texts(&repo.search("\"value\"")), ["\"value\""]);
}

#[test]
fn whole_expression_metavariables_do_not_reinterpret_macro_tokens() {
    let repo = Repo::new();
    repo.write("x.rs", "fn f(){ mac!(hidden.unwrap()); }");
    let result = repo.search("$a");
    assert_eq!(texts(&result), ["mac!(hidden.unwrap())"]);
}

#[test]
fn recovery_cancellation_and_input_guards_use_shared_envelopes() {
    let repo = Repo::new();
    repo.write("x.rs", "fn f(){ @ x.unwrap(); }");
    let result = repo.search("$a.unwrap()");
    assert_eq!(result.matches.len(), 1);
    assert!(result.matches[0].syntax.file_has_recovery);
    assert!(result.matches[0].syntax.enclosing_has_recovery);
    let cancelled = repo
        .engine()
        .search_pattern(repo.request("$a.unwrap()"), &AtomicBool::new(true));
    assert_eq!(cancelled.error.unwrap().code, "CANCELLED");
    for pattern in [
        "x".repeat(64 * 1024 + 1),
        format!("f({})", vec!["$a"; 65].join(",")),
        format!("[{}]", vec!["1"; 2100].join(",")),
    ] {
        let result = repo
            .engine()
            .search_pattern(repo.request(&pattern), &AtomicBool::new(false));
        assert_eq!(result.error.unwrap().code, "INVALID_PATTERN");
        assert_eq!(result.counts.scanned_files, 0);
    }
}

fn observation(root: &Path) -> Vec<(PathBuf, Vec<u8>, u32, i64, i64)> {
    use std::os::unix::fs::MetadataExt;
    fn walk(root: &Path, path: &Path, out: &mut Vec<(PathBuf, Vec<u8>, u32, i64, i64)>) {
        let m = std::fs::symlink_metadata(path).unwrap();
        out.push((
            path.strip_prefix(root).unwrap().to_owned(),
            if m.is_file() {
                std::fs::read(path).unwrap()
            } else {
                Vec::new()
            },
            m.mode(),
            m.mtime(),
            m.mtime_nsec(),
        ));
        if m.is_dir() {
            let mut children: Vec<_> = std::fs::read_dir(path)
                .unwrap()
                .map(|e| e.unwrap().path())
                .collect();
            children.sort();
            for child in children {
                walk(root, &child, out);
            }
        }
    }
    let mut out = Vec::new();
    walk(root, root, &mut out);
    out
}
#[test]
fn sugar_calls_are_read_only_for_success_empty_invalid_and_partial_results() {
    let repo = Repo::new();
    repo.write("x.rs", "fn f(){ x.unwrap(); y.unwrap(); }");
    let before = observation(&repo.0);
    repo.search("$a.unwrap()");
    repo.search("$a.expect($b)");
    assert!(
        repo.engine()
            .search_pattern(repo.request("f("), &AtomicBool::new(false))
            .error
            .is_some()
    );
    let mut request = repo.request("$a.unwrap()");
    request.page_size = 1;
    assert!(
        repo.engine()
            .search_pattern(request, &AtomicBool::new(false))
            .next_cursor
            .is_some()
    );
    assert_eq!(observation(&repo.0), before);
}
