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
    std::fs::write(
        root.join("sample.rs"),
        "fn f() { thing.unwrap(); wrong.unwrap(x); }\n",
    )
    .unwrap();
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
        if request.get("id").is_none() {
            return Value::Null;
        }
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
    exchange(json!({"jsonrpc":"2.0","method":"notifications/initialized"}));
    let tools = exchange(json!({"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}));
    let advertised = tools["result"]["tools"].as_array().unwrap();
    assert_eq!(advertised.len(), 2);
    for name in ["search", "search_query"] {
        let tool = advertised.iter().find(|t| t["name"] == name).unwrap();
        assert!(tool["outputSchema"].is_object());
        assert_eq!(tool["annotations"]["readOnlyHint"], true);
    }
    let sugar = exchange(
        json!({"jsonrpc":"2.0","id":20,"method":"tools/call","params":{"name":"search","arguments":{"repo_path":root,"pattern":"$a.unwrap()"}}}),
    );
    let envelope = &sugar["result"]["structuredContent"];
    assert_eq!(envelope["tool"], "search");
    assert_eq!(envelope["status"], "complete");
    assert_eq!(envelope["matches"].as_array().unwrap().len(), 1);
    assert_eq!(envelope["matches"][0]["span"]["text"], "thing.unwrap()");
    assert_eq!(envelope["matches"][0]["captures"]["a"][0]["text"], "thing");
    assert_eq!(
        envelope["matches"][0]["span"]["range"],
        json!({"start_byte":9,"end_byte":23})
    );
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
