use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Read, Write},
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

struct Fixture(PathBuf);
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
struct Service(Child);
impl Drop for Service {
    fn drop(&mut self) {
        // A failing regression must not leave the server or its hasher running.
        if self.0.try_wait().ok().flatten().is_none() {
            if let Some(pid) = owned_hasher(self.0.id()) {
                let _ = Command::new("kill")
                    .args(["-KILL", &pid.to_string()])
                    .status();
            }
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}

fn owned_hasher(server_pid: u32) -> Option<u32> {
    // Inspect names first, then only this test server's Git child's arguments.
    let processes = Command::new("ps")
        .args(["-axo", "pid=,ppid=,comm="])
        .output()
        .unwrap();
    assert!(processes.status.success());
    for line in String::from_utf8(processes.stdout).unwrap().lines() {
        let mut fields = line.split_whitespace();
        let pid = fields.next().unwrap().parse::<u32>().unwrap();
        let parent = fields.next().unwrap().parse::<u32>().unwrap();
        let name = fields.next().unwrap();
        if parent != server_pid || name.rsplit('/').next() != Some("git") {
            continue;
        }
        let args = Command::new("ps")
            .args(["-o", "args=", "-p", &pid.to_string()])
            .output()
            .unwrap();
        if String::from_utf8(args.stdout)
            .unwrap()
            .contains("hash-object --stdin --no-filters")
        {
            return Some(pid);
        }
    }
    None
}

fn process_exists(pid: u32) -> bool {
    let output = Command::new("ps")
        .args(["-o", "pid=", "-p", &pid.to_string()])
        .output()
        .unwrap();
    !output.stdout.is_empty()
}

#[test]
fn eof_cancels_active_work_and_reaps_owned_git_child() {
    eof_flow("search_query");
}
#[test]
fn move_shares_admission_and_eof_reaps_owned_git_child() {
    eof_flow("move_item");
}
#[test]
fn advice_shares_admission_and_eof_reaps_owned_git_child() {
    eof_flow("suggest_split");
}
#[test]
fn export_shares_admission_and_eof_reaps_owned_git_child() {
    eof_flow("export_move_request");
}
fn eof_flow(tool: &str) {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let fixture = Fixture(std::env::temp_dir().join(format!(
        "rust-sitter-eof-{}-{tool}-{unique}",
        std::process::id()
    )));
    std::fs::create_dir(&fixture.0).unwrap();
    assert!(
        Command::new("git")
            .args(["init", "--quiet"])
            .arg(&fixture.0)
            .status()
            .unwrap()
            .success()
    );
    // Same substantial corpus as the lifecycle reproduction: keep the hasher
    // observable without a fake Git executable or a product-side test hook.
    let line = "fn f(){foo();}\n";
    let source = if tool == "export_move_request" {
        format!("fn f() {{}}\n/*{}*/\n", " ".repeat(1024 * 1024))
    } else {
        line.repeat(1024 * 1024 / line.len())
    };
    for index in 0..120 {
        std::fs::write(fixture.0.join(format!("stress-{index:03}.rs")), &source).unwrap();
    }
    let mut service = Service(
        Command::new(env!("CARGO_BIN_EXE_rust-sitter-mcp"))
            .env("RUST_LOG", "info")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let mut input = service.0.stdin.take().unwrap();
    let output = service.0.stdout.take().unwrap();
    let mut diagnostics = service.0.stderr.take().unwrap();
    let (tx, rx) = mpsc::channel();
    let output_reader = thread::spawn(move || {
        for line in BufReader::new(output).lines() {
            let message: Value = serde_json::from_str(&line.unwrap()).unwrap();
            if tx.send(message).is_err() {
                break;
            }
        }
    });
    let diagnostic_reader = thread::spawn(move || {
        let mut log = String::new();
        diagnostics.read_to_string(&mut log).unwrap();
        log
    });
    writeln!(input, "{}", json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"eof-fixture","version":"1"}}})).unwrap();
    input.flush().unwrap();
    let initialized = rx.recv_timeout(Duration::from_secs(10)).unwrap();
    assert!(initialized["result"]["serverInfo"].is_object());
    writeln!(
        input,
        "{}",
        json!({"jsonrpc":"2.0","method":"notifications/initialized"})
    )
    .unwrap();
    let args = if tool == "export_move_request" {
        writeln!(input, "{}", json!({"jsonrpc":"2.0","id":99,"method":"tools/call","params":{"name":"suggest_split","arguments":{
            "repo_path":fixture.0,"crate_root":"stress-000.rs","source_path":"stress-000.rs","retain_snapshot":true,"limits":{"time_budget_ms":300000}
        }}})).unwrap();
        input.flush().unwrap();
        let retained = rx.recv_timeout(Duration::from_secs(120)).unwrap();
        let advice = &retained["result"]["structuredContent"];
        assert_eq!(advice["retention"]["state"], "retained", "{retained}");
        json!({"analysis_handle":advice["retention"]["analysis_handle"],"snapshot_id":advice["snapshot_id"],"selection":[{
            "unit_ref":{"analysis_id":advice["retention"]["analysis_id"],"item_id":advice["inventory"][0]["id"]},
            "destination":{"kind":"new_sibling","path":"helper.rs","parent_path":"stress-000.rs"}}],"limits":{"time_budget_ms":300000}})
    } else if tool == "suggest_split" {
        json!({"repo_path":fixture.0,"crate_root":"stress-000.rs","source_path":"stress-000.rs","retain_snapshot":true,"limits":{"time_budget_ms":300000}})
    } else if tool == "move_item" {
        json!({"repo_path":fixture.0,"crate_root":"stress-000.rs","moves":[],"limits":{"time_budget_ms":300000}})
    } else {
        json!({"repo_path":fixture.0,"query":"(call_expression) @match","page_size":1,"limits":{"time_budget_ms":300000}})
    };
    writeln!(input, "{}", json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":tool,"arguments":args}})).unwrap();
    input.flush().unwrap();

    let deadline = Instant::now() + Duration::from_secs(20);
    let git_pid = loop {
        if let Some(pid) = owned_hasher(service.0.id()) {
            break pid;
        }
        assert!(
            rx.try_recv().is_err(),
            "search finished before observing hasher"
        );
        assert!(Instant::now() < deadline, "did not observe an owned hasher");
        thread::sleep(Duration::from_millis(10));
    };
    if tool == "move_item" || tool == "suggest_split" || tool == "export_move_request" {
        // Freeze only this test's hasher so the competing request observes a held permit.
        assert!(
            Command::new("kill")
                .args(["-STOP", &git_pid.to_string()])
                .status()
                .unwrap()
                .success()
        );
        let competing_calls = [
            (
                "export_move_request",
                json!({"analysis_handle":"unknown","snapshot_id":"sha1:unknown","selection":[{"unit_ref":{"analysis_id":"unknown","item_id":"unknown"},"destination":{"kind":"existing","path":"other.rs"}}]}),
            ),
            ("search", json!({"repo_path":fixture.0,"pattern":"foo()"})),
            (
                "search_query",
                json!({"repo_path":fixture.0,"query":"(call_expression) @match"}),
            ),
            (
                "replace",
                json!({"repo_path":fixture.0,"pattern":"foo()","replacement":"foo()"}),
            ),
            (
                "move_item",
                json!({"repo_path":fixture.0,"crate_root":"stress-000.rs","moves":[]}),
            ),
            (
                "suggest_split",
                json!({"repo_path":fixture.0,"crate_root":"stress-000.rs","source_path":"stress-000.rs"}),
            ),
            (
                "get_split_detail",
                json!({"analysis_handle":"unknown","snapshot_id":"sha1:unknown","selector":{"kind":"page","collection":"inventory"}}),
            ),
            (
                "get_split_detail",
                json!({"analysis_handle":"unknown","snapshot_id":"sha1:unknown","selector":{"kind":"release"}}),
            ),
        ];
        for (index, (busy_tool, arguments)) in competing_calls.into_iter().enumerate() {
            let id = index + 3;
            writeln!(input, "{}", json!({"jsonrpc":"2.0","id":id,"method":"tools/call","params":{"name":busy_tool,"arguments":arguments}})).unwrap();
            input.flush().unwrap();
            let busy = rx.recv_timeout(Duration::from_secs(10)).unwrap();
            assert_eq!(busy["id"], id);
            assert_eq!(busy["result"]["isError"], true);
            let result = &busy["result"]["structuredContent"];
            assert_eq!(result["tool"], busy_tool);
            if busy_tool == "get_split_detail" {
                assert_eq!(result["returned_page_complete"], false);
                assert_eq!(result["historical"], true);
                assert_eq!(result["live_freshness"], "not_checked");
            } else if busy_tool == "export_move_request" {
                assert_eq!(result["submitted"], false);
                assert_eq!(result["applicability"], "not_assessed");
                assert!(result["request"].is_null());
            } else {
                assert_eq!(result["status"], "failed");
            }
            assert_eq!(
                result["error"],
                json!({
                    "code":"BUSY",
                    "message":concat!(
                        "the analysis slot is occupied; this rejected call started no analysis and was not queued; ",
                        "wait for the active call to finish or cancellation to settle, then retry serially"
                    )
                })
            );
            assert!(result["root"].is_null() && result["snapshot_id"].is_null());
            if busy_tool == "replace" || busy_tool == "move_item" {
                let plan = &result["plan"];
                assert_eq!(plan["applicable"], false);
                assert!(plan["edits"].is_null() && plan["patch"].is_null());
                if busy_tool == "move_item" {
                    assert!(plan["created_files"].is_null());
                }
            } else if busy_tool == "suggest_split" {
                assert!(result["drafts"].as_array().unwrap().is_empty());
            } else if busy_tool == "get_split_detail" {
                assert!(result["records"].as_array().unwrap().is_empty());
                assert!(result["released"].is_null());
            } else if busy_tool == "export_move_request" {
                assert!(result["request"].is_null());
            } else {
                assert!(result["matches"].as_array().unwrap().is_empty());
            }
        }
        assert!(
            Command::new("kill")
                .args(["-CONT", &git_pid.to_string()])
                .status()
                .unwrap()
                .success()
        );
    }
    let eof = Instant::now();
    drop(input);
    let status = loop {
        if let Some(status) = service.0.try_wait().unwrap() {
            break status;
        }
        assert!(eof.elapsed() < Duration::from_secs(10), "EOF shutdown hung");
        thread::sleep(Duration::from_millis(10));
    };
    let elapsed = eof.elapsed();
    output_reader.join().unwrap();
    let log = diagnostic_reader.join().unwrap();
    let responses: Vec<Value> = rx.try_iter().collect();
    eprintln!("EOF exit after {elapsed:?}; Git PID {git_pid}; diagnostics:\n{log}");
    assert!(status.success(), "service exit: {status}");
    assert!(!process_exists(git_pid), "owned Git child was not reaped");
    assert!(
        !process_exists(service.0.id()),
        "server process still exists"
    );
    assert!(
        log.contains(match tool {
            "move_item" => "move plan finished",
            "suggest_split" => "split advice finished",
            "export_move_request" => "move request export finished",
            _ => "search finished",
        }) && log.contains("CANCELLED"),
        "EOF did not cancel active engine work: {log}"
    );
    for response in responses {
        assert_eq!(response["id"], 2, "rejected calls must not run later");
        if response["id"] == 2 {
            if tool == "suggest_split" {
                let advice = &response["result"]["structuredContent"];
                assert_eq!(advice["integrity"]["semantic"], "not_performed");
                assert!(advice["drafts"].as_array().unwrap().is_empty());
                assert!(advice.get("plan").is_none() && advice.get("patch").is_none());
                assert!(advice["retention"]["analysis_handle"].is_null());
            }
            if tool == "move_item" {
                let plan = &response["result"]["structuredContent"]["plan"];
                assert_eq!(plan["integrity"]["semantic"], "not_performed");
                assert!(
                    plan["edits"].is_null()
                        && plan["created_files"].is_null()
                        && plan["patch"].is_null()
                );
            }
            assert_eq!(
                response["result"]["structuredContent"]["error"]["code"],
                "CANCELLED"
            );
        }
    }
    assert!(
        elapsed < Duration::from_secs(2),
        "EOF waited for normal work: {elapsed:?}"
    );
}
