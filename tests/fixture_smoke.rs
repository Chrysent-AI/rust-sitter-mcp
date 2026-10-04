#[path = "support/fixture_gen.rs"]
mod fixture_gen;
use fixture_gen::{Fixture, observe, tree};
use rust_sitter_mcp::{engine::Engine, plan::ReplaceRequest, result::SearchRequest};
use serde_json::{Value, json};
use std::{
    fs,
    io::{BufRead, BufReader, Write},
    path::Path,
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
    sync::atomic::AtomicBool,
};

const UNWRAP_QUERY: &str = "((call_expression function: (field_expression value: (_expression) @a field: (field_identifier) @method) arguments: (arguments) @args) @match (#eq? @method \"unwrap\") (#rust-arity? @args \"0\"))";

// Integration map (not acceptance evidence for unavailable tools):
// Pending move integration (move_item tool): single/batch to existing/new siblings; inventory every
// kind, full anchors; carried trivia/ordinary carry override/protected refusal;
// parent synthesis + reused declaration (cases/layout/lib.rs), legacy mod.rs,
// source also destination/retained empty source, quoted bytes; mixed patch/JSON modes.
// New paths: occupied.rs, occupied_dir.rs, link.rs, competing.rs vs competing/mod.rs,
// bad-name.rs, fn.rs, mismatched parent=source.rs, ../escape.rs, ignored.rs,
// filtered.rs (globs exclude it), nested/new.rs, linked_dir/new.rs, clash.rs vs Clash.rs.
// Inject creation at final absence-recheck seam, NOT before request admission.
// Pending import/visibility rewrite integration: src/rewrite.rs clean repairs/reuse/dedup/private descendant/batch;
// each isolated cases/ambiguity area; reject required rewrite, anchored alternative,
// stale source/destination/override, duplicate/conflicting batch, no partial artifacts.
// Pending split advice integration: rich/weak/single/empty/recovered/unsupported-layout advice;
// exact-once deterministic membership, same-response decision IDs, bounded no-drafts;
// no patch/edits/creates, retained anonymous/context units, text/work/output caps.
// Pending edited-split batch integration: caller edits rich.rs membership AND filenames into two new siblings,
// submits full explicit anchors through move_item, no execute-draft shortcut.
// Join all above here using the same read-only snapshot and external applicator;
// cancellation/deadline/work/output/failed scan/no-op/busy must withhold every artifact.

struct Client {
    child: Child,
    input: Option<ChildStdin>,
    output: BufReader<ChildStdout>,
    id: u64,
}
impl Client {
    fn new() -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_rust-sitter-mcp"))
            .env("RUST_LOG", "error")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let input = Some(child.stdin.take().unwrap());
        let output = BufReader::new(child.stdout.take().unwrap());
        let mut client = Self {
            child,
            input,
            output,
            id: 0,
        };
        client.rpc("initialize", json!({"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"fixture-smoke","version":"1"}}));
        writeln!(
            client.input.as_mut().unwrap(),
            "{}",
            json!({"jsonrpc":"2.0","method":"notifications/initialized"})
        )
        .unwrap();
        client
    }
    fn rpc(&mut self, method: &str, params: Value) -> Value {
        self.id += 1;
        let input = self.input.as_mut().unwrap();
        writeln!(
            input,
            "{}",
            json!({"jsonrpc":"2.0","id":self.id,"method":method,"params":params})
        )
        .unwrap();
        input.flush().unwrap();
        let mut line = String::new();
        assert!(self.output.read_line(&mut line).unwrap() > 0, "server EOF");
        let response: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(response["id"], self.id);
        assert!(response.get("error").is_none(), "{response}");
        response["result"].clone()
    }
    fn call(&mut self, name: &str, args: Value) -> Value {
        let result = self.rpc("tools/call", json!({"name":name,"arguments":args}));
        let fallback: Value =
            serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(fallback, result["structuredContent"]);
        assert_eq!(result["isError"], false, "{result}");
        result["structuredContent"].clone()
    }
}
impl Drop for Client {
    fn drop(&mut self) {
        drop(self.input.take());
        if std::thread::panicking() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        } else {
            assert!(self.child.wait().unwrap().success());
        }
    }
}

// Test-only compilation, on a COPY so Cargo cannot alter the observed caller tree.
// --offline and a std-only manifest mean no network/dependency resolution is needed.
fn cargo_check(repo: &Fixture) {
    for tool in ["cargo", "rustc"] {
        let output = match Command::new(tool).arg("--version").output() {
            Ok(output) => output,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                eprintln!("SKIP fixture cargo check: {tool} executable absent");
                return;
            }
            Err(error) => panic!("failed to probe {tool} --version: {error}"),
        };
        assert!(
            output.status.success(),
            "{tool} --version failed ({}): {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let output = Command::new("cargo")
        .args(["check", "--offline", "--quiet"])
        .current_dir(&repo.0)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "fixture cargo check failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    eprintln!("fixture cargo check passed (test-only)");
}

fn apply_and_compare(repo: &Fixture, envelope: &Value) -> Fixture {
    let plan = &envelope["plan"];
    assert_eq!(envelope["status"], "complete");
    assert_eq!(plan["state"], "applicable");
    assert_eq!(plan["applicable"], true);
    assert_eq!(plan["integrity"]["syntax"], "checked");
    assert_eq!(plan["integrity"]["semantic"], "not_performed");
    assert!(plan["blockers"].as_array().unwrap().is_empty());
    let patch = plan["patch"].as_str().unwrap();
    assert!(!patch.is_empty());
    for forbidden in [
        "new file mode",
        "deleted file mode",
        "old mode",
        "new mode",
        "rename from",
    ] {
        assert!(!patch.contains(forbidden)); // v1 replace never creates/deletes/changes modes
    }
    let mut expected = tree(&repo.0);
    let edits = plan["edits"].as_array().unwrap();
    assert_eq!(edits.len(), 4);
    assert_eq!(plan["base_files"].as_array().unwrap().len(), 4);
    for base in plan["base_files"].as_array().unwrap() {
        let path = base["path"].as_str().unwrap();
        let entry = expected.get_mut(Path::new(path)).unwrap();
        assert_eq!(base["original_length"], entry.bytes.len());
        assert_eq!(
            base["mode"],
            if entry.mode & 0o111 != 0 {
                "100755"
            } else {
                "100644"
            }
        );
        let original = String::from_utf8(entry.bytes.clone()).unwrap();
        let mut rebuilt = original.clone();
        let relevant: Vec<_> = edits.iter().filter(|e| e["path"] == path).collect();
        let mut end = 0;
        for e in &relevant {
            let start = e["range"]["start_byte"].as_u64().unwrap() as usize;
            let stop = e["range"]["end_byte"].as_u64().unwrap() as usize;
            assert!(start >= end && stop >= start);
            assert_eq!(&original[start..stop], e["original_text"].as_str().unwrap());
            end = stop;
        }
        for e in relevant.into_iter().rev() {
            let start = e["range"]["start_byte"].as_u64().unwrap() as usize;
            let stop = e["range"]["end_byte"].as_u64().unwrap() as usize;
            rebuilt.replace_range(start..stop, e["replacement_text"].as_str().unwrap());
        }
        assert_eq!(
            rebuilt,
            original.replace(".unwrap()", ".expect(\"fixture value\")")
        );
        entry.bytes = rebuilt.into_bytes();
    }
    let copy = repo.copy();
    for args in [
        ["apply", "--check", "-"].as_slice(),
        ["apply", "-"].as_slice(),
    ] {
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
            "git {args:?}: {}\n{patch}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    assert_eq!(tree(&copy.0), expected); // ALL paths/bytes/link targets/raw modes, not just edited files
    copy
}

#[test]
fn deterministic_generations_and_available_tool_flow() {
    let first = Fixture::generate();
    let second = Fixture::generate();
    assert_eq!(tree(&first.0), tree(&second.0));
    let mut artifacts = None;
    // Verify the generators really committed, rather than merely running git init.
    for repo in [&first, &second] {
        repo.git(&["rev-parse", "--verify", "HEAD"]);
        let before = observe(&repo.0);
        cargo_check(&repo.copy());
        let mut client = Client::new();
        let tools = client.rpc("tools/list", json!({}));
        for name in ["search", "search_query", "replace"] {
            let tool = tools["tools"]
                .as_array()
                .unwrap()
                .iter()
                .find(|t| t["name"] == name)
                .unwrap();
            assert_eq!(tool["annotations"]["readOnlyHint"], true);
            assert!(tool["outputSchema"].is_object());
        }
        let sugar = client.call(
            "search",
            json!({"repo_path":repo.0,"paths":["src"],"pattern":"$a.unwrap()"}),
        );
        let raw = client.call(
            "search_query",
            json!({"repo_path":repo.0,"paths":["src"],"query":UNWRAP_QUERY}),
        );
        assert_eq!(sugar["tool"], "search");
        assert_eq!(raw["tool"], "search_query");
        for result in [&sugar, &raw] {
            assert_eq!(result["status"], "complete");
            assert_eq!(result["coverage"]["scope_exhaustive"], true);
            assert_eq!(result["counts"]["total_matches"], 4);
        }
        for (s, r) in sugar["matches"]
            .as_array()
            .unwrap()
            .iter()
            .zip(raw["matches"].as_array().unwrap())
        {
            assert_eq!(s["path"], r["path"]);
            assert_eq!(s["span"], r["span"]);
            let source = fs::read_to_string(repo.0.join(s["path"].as_str().unwrap())).unwrap();
            let range = &s["span"]["range"];
            assert_eq!(
                &source[range["start_byte"].as_u64().unwrap() as usize
                    ..range["end_byte"].as_u64().unwrap() as usize],
                s["span"]["text"].as_str().unwrap()
            );
            assert_eq!(s["captures"]["a"], r["captures"]["a"]);
        }
        let inventory = client.call(
            "search_query",
            json!({"repo_path":repo.0,"paths":["src/inventory.rs"],"query":"(impl_item) @match"}),
        );
        assert_eq!(inventory["counts"]["total_matches"], 3);
        let replacement = client.call("replace", json!({"repo_path":repo.0,"paths":["src"],"pattern":"$a.unwrap()","replacement":"$a.expect(\"fixture value\")"}));
        assert_eq!(replacement["plan"]["selected_count"], 4);
        let current = json!({"edits":replacement["plan"]["edits"],"base_files":replacement["plan"]["base_files"],"patch":replacement["plan"]["patch"]});
        if let Some(previous) = &artifacts {
            assert_eq!(&current, previous);
        }
        artifacts = Some(current);
        assert!(
            replacement["plan"]["patch"]
                .as_str()
                .unwrap()
                .contains("\\ No newline at end of file")
        );
        cargo_check(&apply_and_compare(repo, &replacement));
        assert_eq!(observe(&repo.0), before);
        eprintln!(
            "fresh fixture: search/raw/replace, Git apply + independent JSON, source/Git bytes/modes/mtimes passed"
        );
    }
}

#[test]
fn blocking_and_lifecycle_calls_leave_fixture_unchanged() {
    let repo = Fixture::generate();
    let engine = Engine::new(repo.0.clone()).unwrap();
    let before = observe(&repo.0);
    let make = |paths: &[&str]| -> ReplaceRequest {
        serde_json::from_value(json!({"repo_path":repo.0,"paths":paths,"pattern":"$a.unwrap()","replacement":"$a.expect(\"fixture value\")"})).unwrap()
    };
    let mut noop = make(&["src"]);
    noop.selection = Some(vec![]);
    let noop = engine.replace(noop, &AtomicBool::new(false));
    assert!(noop.plan.applicable);
    assert_eq!(noop.plan.integrity.semantic, "not_performed");
    assert_eq!(noop.plan.selected_count, Some(0));
    assert_eq!(noop.plan.patch.as_deref(), Some(""));
    assert!(noop.plan.edits.unwrap().is_empty());
    for (path, code) in [
        ("cases/limits/recovered.rs", "PREEXISTING_SYNTAX_ERROR"),
        ("cases/limits/overlap.rs", "OVERLAPPING_EDITS"),
        ("cases/limits/trivia_loss.rs", "UNRETAINED_TRIVIA"),
        ("cases/limits/many.rs", "max_matches"),
        ("cases/limits/binary.rs", "scan_incomplete"),
    ] {
        let result = engine.replace(make(&[path]), &AtomicBool::new(false));
        assert!(!result.plan.applicable, "{result:?}");
        assert!(result.plan.patch.is_none() && result.plan.edits.is_none());
        assert_eq!(result.plan.integrity.semantic, "not_performed");
        assert!(
            result.plan.blockers.iter().any(|b| b.code == code),
            "{result:?}"
        );
    }
    let mut capped = make(&["cases/limits/output.rs"]);
    capped.limits.response_bytes = 65536;
    let capped = engine.replace(capped, &AtomicBool::new(false));
    assert_eq!(capped.plan.state, "incomplete");
    assert!(capped.plan.patch.is_none() && capped.plan.edits.is_none());
    assert!(capped.wire_bytes() <= 65536);
    let mut stale = make(&["src/inventory.rs"]);
    stale.selection = Some(serde_json::from_value(json!([{"path":"src/inventory.rs","range":{"start_byte":0,"end_byte":5},"expected_text":"stale"}])).unwrap());
    assert_eq!(
        engine
            .replace(stale, &AtomicBool::new(false))
            .error
            .unwrap()
            .code,
        "STALE_SELECTION"
    );
    let cancelled = engine.replace(make(&["src"]), &AtomicBool::new(true));
    assert_eq!(cancelled.error.unwrap().code, "CANCELLED");
    assert!(cancelled.plan.patch.is_none() && cancelled.plan.edits.is_none());
    let mut page: SearchRequest = serde_json::from_value(
        json!({"repo_path":repo.0,"paths":["src"],"query":UNWRAP_QUERY,"page_size":1}),
    )
    .unwrap();
    let first = engine.search(page.clone(), &AtomicBool::new(false));
    assert!(first.next_cursor.is_some());
    page.cursor = first.next_cursor;
    let continued = engine.search(page, &AtomicBool::new(false));
    assert!(continued.error.is_none());
    assert_ne!(continued.matches[0].id, first.matches[0].id);
    assert_eq!(observe(&repo.0), before);
}
