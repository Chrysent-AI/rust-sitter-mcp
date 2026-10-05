#[path = "support/advice_flow.rs"]
mod advice_flow;
#[path = "support/fixture_gen.rs"]
mod fixture_gen;
#[path = "support/move_artifacts.rs"]
mod move_artifacts;
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
// Move execution below joins real stdio planning to the same external applicator.
// Focused move tests cover trivia, paths and injected final-recheck races.
// Pending import/visibility rewrite integration: src/rewrite.rs clean repairs/reuse/dedup/private descendant/batch;
// each isolated cases/ambiguity area; reject required rewrite, anchored alternative,
// stale source/destination/override, duplicate/conflicting batch, no partial artifacts.
// Advice and caller-edited two-sibling execution below exercise complete membership,
// linked decisions, no execution artifacts, external application and test-only compilation.
// Focused advice tests additionally cover bounds, recovery, layout and fresh-process determinism.
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
        for name in [
            "search",
            "search_query",
            "replace",
            "move_item",
            "suggest_split",
        ] {
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
fn stdio_advice_and_caller_edited_split() {
    let repo = Fixture::generate();
    let before = observe(&repo.0);
    let mut client = Client::new();
    let make = |path| json!({"repo_path":repo.0,"crate_root":"src/lib.rs","source_path":path,"paths":["src"],"limits":{"text_bytes":0}});
    let advice = client.call("suggest_split", make("src/rich.rs"));
    advice_flow::complete(&advice);
    assert!(!advice["drafts"].as_array().unwrap().is_empty(), "{advice}");
    assert_eq!(advice, client.call("suggest_split", make("src/rich.rs")));
    let batch = advice_flow::edited_batch(&repo, &advice);
    let moved = client.call("move_item", batch.clone());
    let plan = &moved["plan"];
    let files = plan["created_files"].as_array().unwrap();
    let edits = plan["edits"].as_array().unwrap();
    let rewrites = plan["rewrites"].as_array().unwrap();
    assert_eq!(plan["selected_count"], 4);
    assert_eq!(files.len(), 2);
    // Both absent siblings require synthesized declarations in the same parent.
    for (path, declaration) in [
        ("src/edited_a.rs", "mod edited_a;"),
        ("src/edited_b.rs", "mod edited_b;"),
    ] {
        let file = files.iter().find(|f| f["path"] == path).unwrap();
        assert_eq!(file["parent_path"], "src/lib.rs");
        assert_eq!(file["declaration_link"]["kind"], "synthesized");
        let rewrite = rewrites
            .iter()
            .find(|r| r["id"] == file["declaration_link"]["rewrite_id"])
            .unwrap();
        assert_eq!(rewrite["kind"], "module_declaration");
        assert_eq!(rewrite["after_text"], declaration);
        assert_eq!(rewrite["artifact_links"].as_array().unwrap().len(), 1);
        let link = &rewrite["artifact_links"][0];
        assert_eq!(link["kind"], "edit");
        let edit = &edits[link["index"].as_u64().unwrap() as usize];
        assert_eq!(edit["path"], "src/lib.rs");
        assert!(
            edit["rewrite_ids"]
                .as_array()
                .unwrap()
                .contains(&rewrite["id"])
        );
    }
    // The edited crossing beta_write -> beta_flush, the retained const, and the
    // imported helper require these repairs, not merely any rewrites that exist.
    for (kind, text, output_path) in [
        (
            "import_insert",
            "use crate::rewrite::helper;",
            "src/edited_a.rs",
        ),
        (
            "import_insert",
            "use crate::edited_b::beta_flush;",
            "src/edited_a.rs",
        ),
        (
            "import_insert",
            "use crate::rich::RETAIN;",
            "src/edited_b.rs",
        ),
        ("visibility", "pub(crate) ", "src/edited_b.rs"),
        ("visibility", "pub(crate) ", "src/rich.rs"),
    ] {
        let rewrite = rewrites
            .iter()
            .find(|r| {
                if r["kind"] != kind || r["after_text"] != text {
                    return false;
                }
                let link = &r["artifact_links"][0];
                if link["kind"] == "edit" {
                    edits[link["index"].as_u64().unwrap() as usize]["path"] == output_path
                } else {
                    files
                        .iter()
                        .any(|f| f["id"] == link["id"] && f["path"] == output_path)
                }
            })
            .unwrap_or_else(|| panic!("missing {kind} {text:?} in {output_path}: {plan}"));
        assert_eq!(rewrite["artifact_links"].as_array().unwrap().len(), 1);
        let link = &rewrite["artifact_links"][0];
        let artifact = if link["kind"] == "edit" {
            &edits[link["index"].as_u64().unwrap() as usize]
        } else {
            assert_eq!(link["kind"], "created_file");
            files.iter().find(|f| f["id"] == link["id"]).unwrap()
        };
        assert!(
            artifact["rewrite_ids"]
                .as_array()
                .unwrap()
                .contains(&rewrite["id"])
        );
    }
    // The existing applicator also verifies every linked range against its bytes.
    let copy = move_artifacts::apply(&repo, &moved);
    cargo_check(&copy);
    assert!(
        fs::read_to_string(copy.0.join("src/edited_a.rs"))
            .unwrap()
            .contains("fn beta_write")
    );
    assert!(
        !fs::read_to_string(copy.0.join("src/edited_b.rs"))
            .unwrap()
            .contains("fn beta_write")
    );
    assert_eq!(observe(&repo.0), before);
    // Only the final member (the second sibling) is stale; earlier valid members
    // must not leak a partial first sibling, parent edits, or a patch.
    let mut stale = batch;
    stale["moves"][3]["item"]["expected_text"] = json!("fn beta_flush() -> u8 { 0 }");
    let failed = client.rpc("tools/call", json!({"name":"move_item","arguments":stale}));
    assert_eq!(failed["isError"], true, "{failed}");
    let blocked = &failed["structuredContent"];
    let fallback: Value =
        serde_json::from_str(failed["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(&fallback, blocked);
    assert_eq!(blocked["status"], "failed");
    assert_eq!(blocked["error"]["code"], "STALE_SELECTION");
    assert_eq!(blocked["error"]["field"], "moves[3].item");
    assert_eq!(blocked["plan"]["state"], "blocked");
    assert_eq!(blocked["plan"]["applicable"], false);
    assert_eq!(blocked["plan"]["integrity"]["semantic"], "not_performed");
    for field in ["edits", "created_files", "patch"] {
        assert_eq!(
            blocked["plan"].get(field),
            Some(&Value::Null),
            "partial or missing {field}: {blocked}"
        );
    }
    assert_eq!(observe(&repo.0), before);
    for path in [
        "src/inventory.rs",
        "src/weak.rs",
        "src/single.rs",
        "src/empty.rs",
    ] {
        let result = client.call("suggest_split", make(path));
        advice_flow::complete(&result);
        if path == "src/inventory.rs" {
            assert_eq!(
                result["inventory"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter(|i| i["kind"] == "impl_item")
                    .count(),
                3
            );
        }
        if path == "src/empty.rs" || path == "src/single.rs" {
            assert!(result["drafts"].as_array().unwrap().is_empty());
            assert!(
                !result["draft_eligibility"]["reasons"]
                    .as_array()
                    .unwrap()
                    .is_empty()
            );
        }
    }
    assert_eq!(observe(&repo.0), before);
    eprintln!(
        "stdio advice + edited batch: every unit accounted once, complete decision closure, no execution artifact; caller-edited membership/filenames; both declaration links and required import/visibility audit links; stale final member withholds ALL artifacts; Git/JSON/modes, compile-on-copy and read-only observations passed"
    );
}

#[test]
fn stdio_move_existing_synthesis_reuse_and_batch() {
    let repo = Fixture::generate();
    let before = observe(&repo.0);
    let mut client = Client::new();
    let item = move_artifacts::anchor(&repo, "src/weak.rs", "fn apple() {}");
    let make = |destination: Value| json!({"repo_path":repo.0,"crate_root":"src/lib.rs","paths":["src"],"moves":[{"item":item,"destination":destination}]});
    for destination in [
        json!({"kind":"existing","path":"src/empty.rs"}),
        json!({"kind":"new_sibling","path":"src/moved.rs","parent_path":"src/lib.rs"}),
    ] {
        let args = make(destination);
        let first = client.call("move_item", args.clone());
        let again = client.call("move_item", args);
        assert_eq!(first["plan"], again["plan"]);
        cargo_check(&move_artifacts::apply(&repo, &first));
        assert_eq!(observe(&repo.0), before);
    }
    let reuse = client.call("move_item", json!({"repo_path":repo.0,"crate_root":"cases/layout/lib.rs","paths":["cases/layout"],"moves":[{"item":move_artifacts::anchor(&repo,"cases/layout/source.rs","fn clean() {}"),"destination":{"kind":"new_sibling","path":"cases/layout/reused.rs","parent_path":"cases/layout/lib.rs"}}]}));
    assert_eq!(
        reuse["plan"]["created_files"][0]["declaration_link"]["kind"],
        "reused"
    );
    assert!(
        !reuse["plan"]["base_files"]
            .as_array()
            .unwrap()
            .iter()
            .any(|b| b["path"] == "cases/layout/lib.rs")
    );
    move_artifacts::apply(&repo, &reuse);
    let batch = client.call("move_item", json!({"repo_path":repo.0,"crate_root":"src/lib.rs","paths":["src"],"moves":[{"item":move_artifacts::anchor(&repo,"src/weak.rs","fn zebra() {}"),"destination":{"kind":"new_sibling","path":"src/first.rs","parent_path":"src/lib.rs"}},{"item":move_artifacts::anchor(&repo,"src/weak.rs","fn mountain() {}"),"destination":{"kind":"new_sibling","path":"src/second.rs","parent_path":"src/lib.rs"}}]}));
    assert_eq!(batch["plan"]["created_files"].as_array().unwrap().len(), 2);
    cargo_check(&move_artifacts::apply(&repo, &batch));
    assert_eq!(observe(&repo.0), before);
    eprintln!(
        "stdio move: existing + synthesis + reuse + batch; external Git/JSON/modes and unchanged caller passed"
    );
}

#[test]
fn stdio_rewrite_smoke_with_choices_and_grouped_imports() {
    let repo = Fixture::generate();
    let before = observe(&repo.0);
    let mut client = Client::new();
    let private = "fn private() -> u8 { helper() }";
    let args = json!({"repo_path":repo.0,"crate_root":"src/lib.rs","paths":["src"],"moves":[{"item":move_artifacts::anchor(&repo,"src/rewrite.rs",private),"destination":{"kind":"new_sibling","path":"src/moved_private.rs","parent_path":"src/lib.rs"}}]});
    let default = client.call("move_item", args.clone());
    let copy = move_artifacts::apply(&repo, &default);
    cargo_check(&copy);
    assert!(
        fs::read_to_string(copy.0.join("src/moved_private.rs"))
            .unwrap()
            .contains("pub(crate) fn private")
    );
    for kind in ["import_insert", "path", "visibility"] {
        assert!(
            default["plan"]["rewrites"]
                .as_array()
                .unwrap()
                .iter()
                .any(|r| r["kind"] == kind)
        );
    }
    let repair = default["plan"]["rewrites"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["kind"] == "path")
        .unwrap();
    let mut rejected = args.clone();
    rejected["rewrite_overrides"] = json!([{"target":repair["target"],"action":"retain"}]);
    let blocked = client.call("move_item", rejected);
    assert_eq!(blocked["plan"]["applicable"], false);
    for field in ["edits", "created_files", "patch"] {
        assert!(blocked["plan"][field].is_null());
    }
    assert!(
        blocked["plan"]["decisions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["selected_choice"] == "retain" && d["blocks_applicability"] == true)
    );
    let import = default["plan"]["rewrites"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["kind"] == "import_insert" && r["target"]["path"] == "src/moved_private.rs")
        .unwrap();
    let mut alternative = args;
    alternative["rewrite_overrides"] = json!([{"target":import["target"],"action":"replace","replacement_text":"use super::rewrite::helper as dependency;"}]);
    let chosen = client.call("move_item", alternative.clone());
    cargo_check(&move_artifacts::apply(&repo, &chosen));
    assert_eq!(
        chosen["plan"],
        client.call("move_item", alternative)["plan"]
    );
    assert!(
        chosen["plan"]["rewrites"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["after_text"] == "dependency" && r["origin"] == "caller_override")
    );
    let grouped = client.call("move_item", json!({"repo_path":repo.0,"crate_root":"src/lib.rs","paths":["src"],"moves":[{"item":move_artifacts::anchor(&repo,"src/inventory.rs","pub const DEFAULT: u8 = 1;"),"destination":{"kind":"new_sibling","path":"src/moved_constant.rs","parent_path":"src/lib.rs"}}]}));
    cargo_check(&move_artifacts::apply(&repo, &grouped));
    assert!(
        grouped["plan"]["rewrites"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["kind"] == "import_leaf_extract")
    );
    assert_eq!(observe(&repo.0), before);
    eprintln!(
        "rewrite smoke: private remaining caller, inline consumer, needed import/visibility, alias alternative, grouped extraction, reject-all, Git/JSON/modes, test-only cargo checks and read-only observations passed"
    );
}
#[test]
#[ignore = "release-build ten-run move workload; not part of the routine gate"]
#[allow(clippy::assertions_on_constants)] // Refuse debug-build timing evidence at runtime.
fn release_move_workload_ten_runs() {
    assert!(!cfg!(debug_assertions), "run with --release");
    let repo = Fixture::generate();
    let large = format!(
        "fn large() {{ let _payload = \"{}\"; }}",
        "x".repeat(48 * 1024)
    );
    let small: Vec<_> = (0..100)
        .map(|i| {
            format!(
                "fn item_{i:03}() {{ let _payload = \"{}\"; }}",
                "x".repeat(128)
            )
        })
        .collect();
    let mut corpus = Vec::new();
    corpus.push((
        "corpus/lib.rs".to_owned(),
        (0..99)
            .map(|i| format!("mod file_{i:03};\n"))
            .collect::<String>(),
    ));
    for i in 0..99 {
        corpus.push((
            format!("corpus/file_{i:03}.rs"),
            if i == 0 {
                format!("{large}\n{}\n", small.join("\n"))
            } else {
                String::new()
            },
        ));
    }
    let mut remaining = 10 * 1024 * 1024;
    for (i, (path, source)) in corpus.iter().enumerate() {
        let length = if i == 99 {
            remaining
        } else {
            (100 * 1024).max(source.len() + 16)
        };
        remaining -= length;
        let padded = format!(
            "{source}\n\n/*{}*/\n",
            "p".repeat(length - source.len() - 7)
        );
        assert_eq!(padded.len(), length);
        repo.write(path, &padded);
    }
    assert_eq!(remaining, 0);
    let before = observe(&repo.0);
    let destination =
        |path: &str| json!({"kind":"new_sibling","path":path,"parent_path":"corpus/lib.rs"});
    let single = json!([{"item":move_artifacts::anchor(&repo,"corpus/file_000.rs",&large),"destination":destination("corpus/single_move.rs")}]);
    let batch = json!(small.iter().enumerate().map(|(i,text)| json!({"item":move_artifacts::anchor(&repo,"corpus/file_000.rs",text),"destination":destination(if i < 50 {"corpus/batch_a.rs"} else {"corpus/batch_b.rs"})})).collect::<Vec<_>>());
    for (name, moves, selected_bytes, target) in [
        ("single", single, large.len(), 5000u128),
        ("batch", batch, small.iter().map(String::len).sum(), 10000),
    ] {
        let mut successes = 0;
        for repetition in 0..10 {
            for (path, _) in &corpus {
                fs::read(repo.0.join(path)).unwrap();
            }
            let mut client = Client::new(); // fresh process: no retained parse state
            let started = std::time::Instant::now();
            let result = client.call("move_item",json!({"repo_path":repo.0,"crate_root":"corpus/lib.rs","paths":["corpus"],"moves":moves,"limits":{"text_bytes":0}}));
            let elapsed = started.elapsed().as_millis();
            assert_eq!(observe(&repo.0), before);
            assert_eq!(result["plan"]["applicable"], true, "{result}");
            assert_eq!(result["counts"]["eligible_files"], 100);
            if elapsed <= target {
                successes += 1;
            }
            eprintln!("move workload {name} run={} ms={elapsed} selected_bytes={selected_bytes} candidates={} rewrites={} structured_bytes={} duplicated_wire_bytes={}",repetition+1,result["counts"]["reference_candidates"],result["counts"]["rewrites"],result.to_string().len(),json!({"content":[{"type":"text","text":result.to_string()}],"structuredContent":result,"isError":false}).to_string().len()+4096);
            if repetition == 0 {
                move_artifacts::apply(&repo, &result);
            }
        }
        assert!(
            successes >= 9,
            "{name}: only {successes}/10 within {target} ms"
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
