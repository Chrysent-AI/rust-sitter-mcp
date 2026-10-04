use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Command, Stdio};

#[test]
fn real_stdio_query() {
    let root = std::env::temp_dir().join(format!("rust-sitter-stdio-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    assert!(
        Command::new("git")
            .args(["init", "--quiet"])
            .arg(&root)
            .status()
            .unwrap()
            .success()
    );
    std::fs::write(root.join("sample.rs"), "fn f() { thing.unwrap(); }\n").unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_rust-sitter-mcp"))
        .env("RUST_LOG", "info")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    let mut output = BufReader::new(child.stdout.take().unwrap());
    let mut exchange = |request: Value| -> Value {
        writeln!(input, "{request}").unwrap();
        input.flush().unwrap();
        let mut line = String::new();
        output.read_line(&mut line).unwrap();
        serde_json::from_str(&line).unwrap()
    };
    let initialized = exchange(
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"fixture","version":"1"}}}),
    );
    assert!(
        initialized["result"]["serverInfo"]["version"]
            .as_str()
            .unwrap()
            .contains("0.1.0 (")
    );
    let tools = exchange(json!({"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}));
    assert_eq!(tools["result"]["tools"][0]["name"], "search_query");
    assert!(tools["result"]["tools"][0]["outputSchema"].is_object());
    let result = exchange(
        json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"search_query","arguments":{"repo_path":root,"query":"(call_expression) @match"}}}),
    );
    assert_eq!(
        result["result"]["structuredContent"]["matches"][0]["span"]["text"],
        "thing.unwrap()"
    );
    let fallback: Value =
        serde_json::from_str(result["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(fallback, result["result"]["structuredContent"]);
    let invalid = exchange(
        json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"search_query","arguments":{"repo_path":root,"query":"(identifier) @match","unknown":true}}}),
    );
    assert_eq!(invalid["result"]["isError"], true);
    assert_eq!(
        invalid["result"]["structuredContent"]["error"]["code"],
        "INVALID_PARAMS"
    );
    assert_eq!(invalid["result"]["structuredContent"]["status"], "failed");
    drop(input);
    assert!(child.wait().unwrap().success());
    let mut diagnostics = String::new();
    child
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut diagnostics)
        .unwrap();
    assert!(diagnostics.contains("search_call"));
    assert!(diagnostics.contains("paths_count"));
    assert!(!diagnostics.contains("thing.unwrap()"));
    std::fs::remove_dir_all(root).unwrap();
}
