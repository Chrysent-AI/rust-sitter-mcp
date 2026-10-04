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
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let fixture = Fixture(
        std::env::temp_dir().join(format!("rust-sitter-eof-{}-{unique}", std::process::id())),
    );
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
    let source = line.repeat(1024 * 1024 / line.len());
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
    writeln!(input, "{}", json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"search_query","arguments":{"repo_path":fixture.0,"query":"(call_expression) @match","page_size":1,"limits":{"time_budget_ms":300000}}}})).unwrap();
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
        log.contains("search finished") && log.contains("CANCELLED"),
        "EOF did not cancel active engine work: {log}"
    );
    for response in responses {
        if response["id"] == 2 {
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
