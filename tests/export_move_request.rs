#[path = "support/stdio_client.rs"]
mod stdio_client;
use serde_json::{Value, json};
use std::{fs, path::PathBuf, process::Command};
use stdio_client::Client;

const SOURCE: &str = "fn alpha() {}\nfn beta() { alpha(); }\nfn risky(x: u8) { x.one(); x.two(); x.three(); x.four(); x.five(); x.six(); }\n";
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let root =
            std::env::temp_dir().join(format!("rust-sitter-export-stdio-{}", std::process::id()));
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
        fs::write(root.join("worker.rs"), SOURCE).unwrap();
        Self(root)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}
fn failed(client: &mut Client, args: Value, code: &str) {
    let response = client.rpc(
        "tools/call",
        json!({"name":"export_move_request","arguments":args}),
    );
    assert_eq!(response["isError"], true, "{response}");
    let envelope = &response["structuredContent"];
    assert_eq!(envelope["error"]["code"], code);
    assert!(envelope["request"].is_null());
    let text: Value =
        serde_json::from_str(response["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(*envelope, text);
}
#[test]
fn real_stdio_inactive_export_separate_hand_anchor_move_and_later_stale_check() {
    let repo = Fixture::new();
    let before = fs::read(repo.0.join("worker.rs")).unwrap();
    let root_before = fs::read(repo.0.join("lib.rs")).unwrap();
    let mut client = Client::new();
    let tools = client.rpc("tools/list", json!({}));
    assert_eq!(tools["tools"].as_array().unwrap().len(), 7);
    let advice = client.call("suggest_split", json!({"repo_path":repo.0,"crate_root":"lib.rs","source_path":"worker.rs","paths":["."],"retain_snapshot":true,"limits":{"text_bytes":0}}));
    assert_eq!(advice["retention"]["state"], "retained");
    let item = advice["inventory"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["name"] == "alpha")
        .unwrap();
    assert!(item["span"]["text"].is_null());
    let export = json!({"analysis_handle":advice["retention"]["analysis_handle"],"snapshot_id":advice["snapshot_id"],
        "selection":[{"unit_ref":{"analysis_id":advice["retention"]["analysis_id"],"item_id":item["id"]},
        "destination":{"kind":"new_sibling","path":"helpers.rs","parent_path":"lib.rs"}}]});
    let result = client.call("export_move_request", export.clone());
    assert!(result["error"].is_null(), "{result}");
    assert_eq!(result["envelope_kind"], "move_request_scaffold");
    assert_eq!(result["scaffold"], true);
    assert_eq!(result["submitted"], false);
    assert_eq!(result["applicability"], "not_assessed");
    assert_eq!(result["source_freshness"], "checked_at_export");
    assert_eq!(result["integrity"]["semantic"], "not_performed");
    let hand = json!({"repo_path":repo.0,"crate_root":"lib.rs","paths":["."],"moves":[{
        "item":{"path":"worker.rs","range":{"start_byte":0,"end_byte":13},"expected_text":"fn alpha() {}"},
        "destination":{"kind":"new_sibling","path":"helpers.rs","parent_path":"lib.rs"}}]});
    assert_eq!(result["request"]["moves"], hand["moves"]);
    let from_export = client.call("move_item", result["request"].clone());
    let from_hand = client.call("move_item", hand);
    assert_eq!(from_export, from_hand);
    assert_eq!(from_export["plan"]["state"], "applicable", "{from_export}");
    assert!(
        result["request"]["limits"]
            .get("diagnostic_count")
            .is_none()
    );
    let risky = advice["inventory"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["name"] == "risky")
        .unwrap();
    let mut risk_export = export.clone();
    risk_export["selection"][0]["unit_ref"]["item_id"] = risky["id"].clone();
    let scaffold = client.call("export_move_request", risk_export);
    let text = "fn risky(x: u8) { x.one(); x.two(); x.three(); x.four(); x.five(); x.six(); }";
    let start = SOURCE.find(text).unwrap();
    let hand = json!({"repo_path":repo.0,"crate_root":"lib.rs","paths":["."],"moves":[{
        "item":{"path":"worker.rs","range":{"start_byte":start,"end_byte":start+text.len()},"expected_text":text},
        "destination":{"kind":"new_sibling","path":"helpers.rs","parent_path":"lib.rs"}}]});
    let blocked_export = client.call("move_item", scaffold["request"].clone());
    let blocked_hand = client.call("move_item", hand);
    assert_eq!(blocked_export["plan"]["state"], "blocked");
    assert_eq!(
        blocked_export, blocked_hand,
        "export must not turn omitted diagnostics into expanded blocked previews"
    );
    assert!(!repo.0.join("helpers.rs").exists());
    assert_eq!(fs::read(repo.0.join("worker.rs")).unwrap(), before);
    assert_eq!(fs::read(repo.0.join("lib.rs")).unwrap(), root_before);
    assert!(fs::read_dir(&repo.0).unwrap().all(|p| matches!(
        p.unwrap().file_name().to_str(),
        Some(".git" | "lib.rs" | "worker.rs")
    )));
    let mut bad = export.clone();
    bad["selection"][0]["unit_ref"]["analysis_id"] = json!("wrong");
    failed(&mut client, bad, "INVALID_SCAFFOLD_SELECTION");
    let mut bad = export.clone();
    bad["analysis_handle"] = json!("unknown");
    failed(&mut client, bad, "ADVICE_SNAPSHOT_UNKNOWN");
    for field in ["destination", "unit_ref"] {
        let mut bad = export.clone();
        bad["selection"][0].as_object_mut().unwrap().remove(field);
        failed(&mut client, bad, "INVALID_PARAMS");
    }
    let mut bad = export.clone();
    bad["move_options"] = json!({"assume_standard_prelude":true});
    failed(&mut client, bad, "INVALID_PARAMS");
    fs::write(
        repo.0.join("worker.rs"),
        "fn alpha() { /* changed */ }\nfn beta() { alpha(); }\n",
    )
    .unwrap();
    failed(&mut client, export, "SOURCE_CHANGED");
    let later = client.rpc(
        "tools/call",
        json!({"name":"move_item","arguments":result["request"]}),
    );
    assert_eq!(later["isError"], true);
    assert_eq!(
        later["structuredContent"]["error"]["code"],
        "STALE_SELECTION"
    );
    assert!(later["structuredContent"]["plan"]["patch"].is_null());
}
