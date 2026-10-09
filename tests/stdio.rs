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
        "fn f() { thing.unwrap(); wrong.unwrap(x); thing.expect(\"why\"); pair(x,x); pair(x,y); }\n",
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
            .contains(&format!("{} (", env!("CARGO_PKG_VERSION")))
    );
    exchange(json!({"jsonrpc":"2.0","method":"notifications/initialized"}));
    let tools = exchange(json!({"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}));
    let advertised = tools["result"]["tools"].as_array().unwrap();
    assert_eq!(advertised.len(), 7);
    let export_tool = advertised
        .iter()
        .find(|t| t["name"] == "export_move_request")
        .unwrap();
    assert!(export_tool["outputSchema"].is_object());
    assert_eq!(export_tool["annotations"]["readOnlyHint"], true);
    assert_eq!(export_tool["inputSchema"]["additionalProperties"], false);
    assert!(
        export_tool["inputSchema"]["required"]
            .as_array()
            .unwrap()
            .contains(&json!("selection"))
    );
    assert!(
        export_tool["description"]
            .as_str()
            .unwrap()
            .contains("submitted:false")
    );
    let detail_tool = advertised
        .iter()
        .find(|t| t["name"] == "get_split_detail")
        .unwrap();
    assert!(detail_tool["outputSchema"].is_object());
    assert_eq!(detail_tool["annotations"]["readOnlyHint"], true);
    assert!(
        detail_tool["inputSchema"]["required"]
            .as_array()
            .unwrap()
            .contains(&json!("analysis_handle"))
    );
    assert!(
        detail_tool["description"]
            .as_str()
            .unwrap()
            .contains("live_freshness:not_checked")
    );
    let unknown = exchange(
        json!({"jsonrpc":"2.0","id":20,"method":"tools/call","params":{"name":"get_split_detail","arguments":{"analysis_handle":"unknown","snapshot_id":"sha1:unknown","selector":{"kind":"release"}}}}),
    );
    assert_eq!(unknown["result"]["isError"], true);
    assert_eq!(
        unknown["result"]["structuredContent"]["error"]["code"],
        "ADVICE_SNAPSHOT_UNKNOWN"
    );
    for name in [
        "search",
        "search_query",
        "replace",
        "move_item",
        "suggest_split",
    ] {
        let tool = advertised.iter().find(|t| t["name"] == name).unwrap();
        assert!(tool["outputSchema"].is_object());
        assert_eq!(tool["annotations"]["readOnlyHint"], true);
        let description = tool["description"].as_str().unwrap();
        for phrase in [
            "Use for:",
            "Does NOT:",
            "Example arguments",
            "repo_path: the Git repository directory",
            "paths: optional scope narrowing",
            "Workflow:",
            "Safety:",
            "Advanced details:",
        ] {
            assert!(
                description.contains(phrase),
                "{name} missing {phrase}: {description}"
            );
        }
        let examples: Vec<_> = description
            .lines()
            .filter(|line| line.starts_with('{'))
            .collect();
        assert_eq!(examples.len(), 1, "{name} needs one complete JSON example");
        let example: Value = serde_json::from_str(examples[0]).unwrap();
        assert_eq!(example["repo_path"], ".");
        assert!(example["paths"].is_array());
        let phrases: &[&str] = match name {
            "search" => &[
                "collect::<Vec<_>>()",
                "use search_query instead",
                "send it as cursor with every other argument unchanged",
                "replace's selection parameter",
                "Empty results do not prove absence in generated code",
            ],
            "search_query" => &[
                "(function_item) @match",
                "reusing a name does not require equal source text",
                "#rust-arity?",
                "No property predicates or directives",
                "An empty search does not prove absence in generated code",
            ],
            "replace" => &[
                "First call search to find matches",
                "Omitted or null selection means all matches, [] means none",
                "caps selected matches",
                "does not count unselected scope matches",
                "exact scope totals after complete bounded matching",
                "withhold ALL artifacts",
                "semantic checking is not_performed",
                "verify unchanged base bytes",
            ],
            "move_item" => &[
                "not a Cargo.toml",
                "inline-module destination",
                "full current bytes",
                "plan.applicable:true",
            ],
            "suggest_split" => &[
                "not a Cargo.toml",
                "source_path: the Git-root-relative existing .rs file to analyze",
                "submit chosen items as move_item anchors",
                "no patch, edit, creation content, execution handle, stored plan",
                "Banner adjacency never assigns ownership",
                "semantic checking is not_performed",
            ],
            _ => unreachable!(),
        };
        for phrase in phrases {
            assert!(
                description.contains(phrase),
                "{name} missing {phrase}: {description}"
            );
        }
    }
    let move_tool = advertised
        .iter()
        .find(|t| t["name"] == "move_item")
        .unwrap();
    assert!(
        move_tool["inputSchema"]["required"]
            .as_array()
            .unwrap()
            .contains(&json!("moves"))
    );
    assert!(
        move_tool["inputSchema"]["required"]
            .as_array()
            .unwrap()
            .contains(&json!("crate_root"))
    );
    assert_eq!(
        move_tool["inputSchema"]["properties"]["resolve_semantic"]["type"],
        "boolean"
    );
    assert!(
        !move_tool["inputSchema"]["required"]
            .as_array()
            .unwrap()
            .contains(&json!("resolve_semantic"))
    );
    assert_eq!(
        move_tool["inputSchema"]["properties"]["acknowledge_test_consumers"]["type"],
        "boolean"
    );
    assert!(
        !move_tool["inputSchema"]["required"]
            .as_array()
            .unwrap()
            .contains(&json!("acknowledge_test_consumers"))
    );
    assert!(move_tool["inputSchema"]["properties"]["semantic_configuration"].is_object());
    let description = move_tool["description"].as_str().unwrap();
    for phrase in [
        "Read-only simultaneous plan",
        "Unique ordinary module and written-binding evidence",
        "itemized import/path repairs",
        "necessary pub(crate) access repairs",
        "published full target in rewrite_overrides",
        "crate_root: a Rust source file path like 'src/lib.rs' or 'src/main.rs'",
        "The server does not infer a root from Cargo",
        "unsupported or uncertain dependencies block the entire batch",
        "Blocked previews are not applicable artifacts",
        "published together or withheld",
        "Default semantic checking is not_performed",
        "resolve_semantic defaults to false",
        "acknowledge_test_consumers defaults to false",
        "test_consumer_acknowledged",
        "caller's test run as the validator",
        "class ra_resolved",
        "compilation/equivalence not performed",
        "No Cargo/project discovery",
        "actual final virtual batch",
        "never writes, formats, compiles or applies changes",
    ] {
        assert!(
            description.contains(phrase),
            "missing {phrase}: {description}"
        );
    }
    assert!(!description.contains("dependency-free"));
    assert!(!description.contains("repair needs are blockers"));
    let invalid_move = exchange(
        json!({"jsonrpc":"2.0","id":19,"method":"tools/call","params":{"name":"move_item","arguments":{"repo_path":root,"crate_root":"sample.rs","moves":[],"apply":true}}}),
    );
    assert_eq!(invalid_move["result"]["isError"], true);
    assert_eq!(
        invalid_move["result"]["structuredContent"]["error"]["code"],
        "INVALID_PARAMS"
    );
    assert_eq!(
        invalid_move["result"]["structuredContent"]["plan"]["integrity"]["semantic"],
        "not_performed"
    );
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
    std::fs::write(root.join("other.rs"), "fn g(){ pair(1,1); pair(1,2); }\n").unwrap();
    for (id, pattern, count) in [
        (21, "$a.expect($b)", 1),
        (22, "pair($a,$a)", 2),
        (23, "pair($a,$b)", 4),
    ] {
        let result = exchange(
            json!({"jsonrpc":"2.0","id":id,"method":"tools/call","params":{"name":"search","arguments":{"repo_path":root,"pattern":pattern}}}),
        );
        let envelope = &result["result"]["structuredContent"];
        assert_eq!(envelope["tool"], "search");
        assert_eq!(envelope["status"], "complete");
        assert_eq!(envelope["matches"].as_array().unwrap().len(), count);
        if pattern == "pair($a,$a)" {
            assert_eq!(envelope["matches"][0]["path"], "other.rs");
            assert_eq!(envelope["matches"][0]["captures"]["a"][0]["text"], "1");
            assert_eq!(envelope["matches"][0]["captures"]["a"][1]["text"], "1");
        }
    }
    let malformed = exchange(
        json!({"jsonrpc":"2.0","id":24,"method":"tools/call","params":{"name":"search","arguments":{"repo_path":root,"pattern":"$a.$b"}}}),
    );
    assert_eq!(malformed["result"]["isError"], true);
    assert_eq!(malformed["result"]["structuredContent"]["tool"], "search");
    assert_eq!(
        malformed["result"]["structuredContent"]["error"]["code"],
        "UNSUPPORTED_PLACEHOLDER_POSITION"
    );
    assert_eq!(
        malformed["result"]["structuredContent"]["error"]["byte_offset"],
        3
    );
    let unknown = exchange(
        json!({"jsonrpc":"2.0","id":25,"method":"tools/call","params":{"name":"search","arguments":{"repo_path":root,"pattern":"$a","query":"ignored"}}}),
    );
    assert_eq!(unknown["result"]["isError"], true);
    assert_eq!(unknown["result"]["structuredContent"]["tool"], "search");
    assert_eq!(
        unknown["result"]["structuredContent"]["error"]["code"],
        "INVALID_PARAMS"
    );
    let result = exchange(
        json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"search_query","arguments":{"repo_path":root,"query":"(call_expression) @match","paths":["sample.rs"]}}}),
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
    let replacement = exchange(
        json!({"jsonrpc":"2.0","id":30,"method":"tools/call","params":{"name":"replace","arguments":{"repo_path":root,"pattern":"$a.unwrap()","replacement":"$a.expect(\"reason\")"}}}),
    );
    let plan = &replacement["result"]["structuredContent"]["plan"];
    assert_eq!(plan["state"], "applicable", "{replacement}");
    let patch = plan["patch"].as_str().unwrap();
    let copy = root.with_extension("apply");
    std::fs::create_dir(&copy).unwrap();
    for path in ["sample.rs", "other.rs"] {
        std::fs::copy(root.join(path), copy.join(path)).unwrap();
    }
    let mut check = Command::new("git")
        .arg("-C")
        .arg(&copy)
        .args(["apply", "--check", "-"])
        .stdin(Stdio::piped())
        .spawn()
        .unwrap();
    check
        .stdin
        .take()
        .unwrap()
        .write_all(patch.as_bytes())
        .unwrap();
    assert!(check.wait().unwrap().success());
    assert!(
        std::fs::read_to_string(root.join("sample.rs"))
            .unwrap()
            .contains("thing.unwrap()")
    );
    std::fs::remove_dir_all(copy).unwrap();
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
