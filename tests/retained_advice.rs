#[path = "support/stdio_client.rs"]
mod stdio_client;
use serde_json::{Value, json};
use std::{fs, path::PathBuf, process::Command};
use stdio_client::Client;

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "rust-sitter-retention-stdio-{}",
            std::process::id()
        ));
        fs::create_dir(&root).unwrap();
        assert!(
            Command::new("git")
                .args(["init", "--quiet", "--template="])
                .arg(&root)
                .status()
                .unwrap()
                .success()
        );
        fs::write(root.join("lib.rs"), "mod worker;\n").unwrap();
        fs::write(root.join("worker.rs"), "struct Worker;\r\nimpl Worker {\r\n fn first() {\r\n // λ\r\n Self::second();\r\n }\r\n fn second() { Self::first(); }\r\n}\r\nfn alpha() { beta(); }\r\nfn beta() { alpha(); }\r\nfn risky(x: u8) { x.unknown(); } // λ").unwrap();
        Self(root)
    }
    fn request(&self) -> Value {
        json!({"repo_path":self.0,"crate_root":"lib.rs","source_path":"worker.rs","retain_snapshot":true,"response_mode":"compact","limits":{"response_bytes":262144,"diagnostic_count":0,"text_bytes":0}})
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}
fn detail(manifest: &Value, selector: Value) -> Value {
    json!({"analysis_handle":manifest["retention"]["analysis_handle"],"analysis_id":manifest["retention"]["analysis_id"],"snapshot_id":manifest["snapshot_id"],"scope_input_digest":manifest["retention"]["scope_input_digest"],"selector":selector,"limits":{"response_bytes":65536}})
}
fn failed(client: &mut Client, args: Value, code: &str) {
    let response = client.rpc(
        "tools/call",
        json!({"name":"get_split_detail","arguments":args}),
    );
    assert_eq!(response["isError"], true, "{response}");
    let envelope = &response["structuredContent"];
    let fallback: Value =
        serde_json::from_str(response["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(*envelope, fallback);
    assert_eq!(envelope["error"]["code"], code);
    assert_eq!(envelope["returned_page_complete"], false);
    assert!(envelope["records"].as_array().unwrap().is_empty());
}

#[test]
fn real_stdio_retention_navigation_release_and_restart_are_historical_only() {
    let repo = Fixture::new();
    let initial_bytes = fs::read(repo.0.join("worker.rs")).unwrap();
    let mut client = Client::new();
    let tools = client.rpc("tools/list", json!({}));
    let suggest = tools["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == "suggest_split")
        .unwrap();
    assert_eq!(
        suggest["inputSchema"]["properties"]["retain_snapshot"]["type"],
        "boolean"
    );
    assert_eq!(
        suggest["inputSchema"]["properties"]["retain_snapshot"]["default"],
        false
    );
    assert!(suggest["inputSchema"]["properties"]["response_mode"].is_object());
    assert!(suggest["outputSchema"]["anyOf"].is_array());
    let manifest = client.call("suggest_split", repo.request());
    assert_eq!(manifest["envelope_kind"], "split_manifest");
    assert_eq!(manifest["analysis_status"], "complete");
    assert_eq!(manifest["manifest_complete"], true);
    assert_eq!(manifest["retention"]["state"], "retained");
    assert_eq!(
        manifest["retention"]["limits"],
        json!({"records": 1, "aggregate_bytes": 134_217_728, "ttl_seconds": 900})
    );
    assert!(
        manifest["retention"]["accounted_bytes"].as_u64().unwrap() > initial_bytes.len() as u64
    );
    assert_eq!(fs::read(repo.0.join("worker.rs")).unwrap(), initial_bytes);
    let first_request = detail(
        &manifest,
        json!({"kind":"page","collection":"inventory","page_size":1}),
    );
    let first = client.call("get_split_detail", first_request.clone());
    assert_eq!(
        first,
        client.call("get_split_detail", first_request.clone())
    );
    assert_eq!(first["historical"], true);
    assert_eq!(first["live_freshness"], "not_checked");
    let mut continuation = first_request.clone();
    continuation["selector"]["page_token"] = first["next_page_token"].clone();
    let mut wrong = continuation.clone();
    wrong["selector"]["page_size"] = json!(2);
    failed(&mut client, wrong, "INVALID_ADVICE_PAGE");
    let mut wrong = first_request.clone();
    wrong["scope_input_digest"] = json!("wrong");
    failed(&mut client, wrong, "ADVICE_SNAPSHOT_MISMATCH");
    let mut records = first["records"].as_array().unwrap().clone();
    loop {
        let page = client.call("get_split_detail", continuation.clone());
        records.extend(page["records"].as_array().unwrap().clone());
        if page["next_page_token"].is_null() {
            assert_eq!(page["collection_exhausted"], true);
            break;
        }
        continuation["selector"]["page_token"] = page["next_page_token"].clone();
    }
    assert_eq!(
        records.len() as u64,
        manifest["totals"]["inventory"].as_u64().unwrap()
    );
    let item = records.iter().find(|i| i["name"] == "first").unwrap();
    let units = detail(&manifest, json!({"kind":"units","item_ids":[item["id"]]}));
    let original = client.call("get_split_detail", units.clone());
    let unit = &original["records"][0];
    let range = &unit["item"]["range"];
    assert_eq!(
        unit["item"]["expected_text"].as_str().unwrap().as_bytes(),
        &initial_bytes[range["start_byte"].as_u64().unwrap() as usize
            ..range["end_byte"].as_u64().unwrap() as usize]
    );
    assert_eq!(unit["enclosing_impl"]["expected_text"], "impl Worker ");
    assert_eq!(original["integrity"]["semantic"], "not_performed");
    for forbidden in ["request", "patch", "edits", "applicable", "created_files"] {
        assert!(original.get(forbidden).is_none());
    }
    fs::write(repo.0.join("worker.rs"), "fn replaced() {}\n").unwrap();
    fs::write(repo.0.join(".gitignore"), "worker.rs\n").unwrap();
    assert_eq!(original, client.call("get_split_detail", units));
    assert_eq!(first, client.call("get_split_detail", first_request));
    let occupied = client.call("suggest_split", json!({"repo_path":repo.0,"crate_root":"lib.rs","source_path":"lib.rs","retain_snapshot":true,"response_mode":"compact"}));
    assert_eq!(occupied["analysis_status"], "complete");
    assert_eq!(occupied["retention"]["reason"], "capacity");
    let release_request = detail(&manifest, json!({"kind":"release"}));
    let mut restart = Client::new();
    failed(
        &mut restart,
        release_request.clone(),
        "ADVICE_SNAPSHOT_UNKNOWN",
    );
    let released = client.call("get_split_detail", release_request.clone());
    assert_eq!(released["released"], true);
    failed(&mut client, release_request, "ADVICE_SNAPSHOT_UNKNOWN");
    for selector in [
        json!({"kind":"release","extra":true}),
        json!({"kind":"page","collection":"inventory","filter":{"unknown":"value"}}),
        json!({"kind":"units","item_ids":[]}),
    ] {
        failed(&mut client, detail(&manifest, selector), "INVALID_PARAMS");
    }
    let mut oversized = detail(&manifest, json!({"kind":"release"}));
    oversized["analysis_handle"] = json!("h".repeat(8 * 1024 * 1024));
    failed(&mut client, oversized, "INVALID_PARAMS");
    let next = client.call("suggest_split", json!({"repo_path":repo.0,"crate_root":"lib.rs","source_path":"lib.rs","retain_snapshot":true,"response_mode":"compact"}));
    assert_eq!(next["retention"]["state"], "retained");
    assert_ne!(
        next["retention"]["analysis_handle"],
        manifest["retention"]["analysis_handle"]
    );
    assert_eq!(
        fs::read_to_string(repo.0.join("worker.rs")).unwrap(),
        "fn replaced() {}\n"
    );
}
