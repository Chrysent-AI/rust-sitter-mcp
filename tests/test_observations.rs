#[allow(dead_code)]
#[path = "support/fixture_gen.rs"]
mod fixture_gen;
#[path = "support/stdio_client.rs"]
mod stdio_client;
use fixture_gen::{Fixture, observe};
use serde_json::{Value, json};
use stdio_client::Client;

fn advice(client: &mut Client, repo: &Fixture, source: &str, paths: Value) -> Value {
    client.call("suggest_split", json!({"repo_path":repo.0,"crate_root":"cases/coupling/lib.rs","source_path":source,"paths":paths,"limits":{"diagnostic_count":100000,"response_bytes":16777216}}))
}
fn records(value: &Value) -> &[Value] {
    value["test_observations"]["records"].as_array().unwrap()
}
fn has(value: &Value, text: &str, channel: &str) -> bool {
    records(value)
        .iter()
        .any(|r| r["anchor"]["span"]["text"] == text && r["channel"] == channel)
}
fn assert_anchors(repo: &Fixture, value: &Value) {
    for record in records(value) {
        for anchor in
            std::iter::once(&record["anchor"]).chain(record["route_evidence"].as_array().unwrap())
        {
            let source =
                std::fs::read_to_string(repo.0.join(anchor["path"].as_str().unwrap())).unwrap();
            let range = &anchor["span"]["range"];
            assert_eq!(
                anchor["span"]["text"],
                source[range["start_byte"].as_u64().unwrap() as usize
                    ..range["end_byte"].as_u64().unwrap() as usize]
            );
        }
    }
}
#[test]
fn conventional_conditional_tests_both_layouts_are_scoped_not_acknowledgeable() {
    for (source, tests) in [
        ("cases/coupling/worker.rs", "cases/coupling/worker/tests.rs"),
        (
            "cases/coupling/worker/mod.rs",
            "cases/coupling/worker/tests/mod.rs",
        ),
    ] {
        let repo = Fixture::generate();
        repo.write("cases/coupling/lib.rs", "mod worker;\n");
        repo.write(source, "struct State { secret: u8 }\nimpl State { fn inspect(&self) {} }\nconst LIMIT: u8 = 4;\n#[cfg(test)] mod tests;\n");
        repo.write(tests, "use super::*;\n#[test] fn inspect() { let state: State = State { secret: 1 }; let _ = state\n .secret; let _ = LIMIT; assert!(state.secret > 0); }\n");
        let before = observe(&repo.0);
        let mut client = Client::new();
        let value = advice(&mut client, &repo, source, json!(["cases/coupling"]));
        assert_eq!(value["status"], "complete", "{value}");
        assert_eq!(value["integrity"]["semantic"], "not_performed");
        let projection = &value["test_observations"];
        assert_eq!(projection["coverage"]["completed"], true);
        let route = &projection["routes"][0];
        assert_eq!(route["status"], "conditional_written_route");
        assert_eq!(route["conditional"], true);
        assert_eq!(route["traversed_path"], tests);
        assert_eq!(route["attributes"][0]["span"]["text"], "#[cfg(test)]");
        assert!(has(&value, "state\n .secret", "cst"), "{projection}");
        let field = records(&value)
            .iter()
            .find(|r| r["access_kind"] == "field_access")
            .unwrap();
        assert!(
            !field["target_item_ids"].as_array().unwrap().is_empty(),
            "{field}"
        );
        assert!(
            field["labels"]
                .as_array()
                .unwrap()
                .contains(&json!("private_state")),
            "{field}"
        );
        assert_eq!(field["possible_companion"], true);
        assert!(
            field["uncertainty"]
                .as_array()
                .unwrap()
                .contains(&json!("glob_binding_candidate"))
        );
        assert!(has(&value, "state.secret", "macro_token_candidate"));
        assert_anchors(&repo, &value);
        // The broader advice never broadens the execution opt-in. The glob
        // consumer in another file remains a blocker with either flag value.
        let start = std::fs::read_to_string(repo.0.join(source))
            .unwrap()
            .find("const LIMIT")
            .unwrap();
        let (destination, parent) = if source.ends_with("/mod.rs") {
            ("cases/coupling/worker/moved.rs", source)
        } else {
            ("cases/coupling/moved.rs", "cases/coupling/lib.rs")
        };
        for flag in [false, true] {
            let moved = client.call("move_item", json!({"repo_path":repo.0,"crate_root":"cases/coupling/lib.rs","paths":["cases/coupling"],"acknowledge_test_consumers":flag,"moves":[{"item":{"path":source,"range":{"start_byte":start,"end_byte":start+"const LIMIT: u8 = 4;".len()},"expected_text":"const LIMIT: u8 = 4;"},"destination":{"kind":"new_sibling","path":destination,"parent_path":parent}}],"limits":{"diagnostic_count":100000}}));
            assert_eq!(moved["plan"]["applicable"], false, "{moved}");
            assert!(
                moved["plan"]["decisions"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|d| d["reason"] == "glob_consumer_unrepaired"),
                "{moved}"
            );
            assert!(
                !moved["plan"]["decisions"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|d| d["reason"] == "test_consumer_acknowledged")
            );
        }
        assert_eq!(observe(&repo.0), before);
    }
}
#[test]
fn missing_excluded_competing_and_remapped_test_routes_disclose_limits() {
    let repo = Fixture::generate();
    repo.write(
        "cases/coupling/lib.rs",
        "const LIMIT: u8 = 4;\n#[cfg(test)] mod tests;\n",
    );
    let mut client = Client::new();
    let missing = advice(
        &mut client,
        &repo,
        "cases/coupling/lib.rs",
        json!(["cases/coupling"]),
    );
    assert_eq!(
        missing["test_observations"]["routes"][0]["status"],
        "missing_test_file"
    );
    assert!(
        !missing["test_observations"]["limitations"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    repo.write(
        "cases/coupling/tests.rs",
        "use super::*; #[test] fn checks() { let _ = LIMIT; }",
    );
    let excluded = advice(
        &mut client,
        &repo,
        "cases/coupling/lib.rs",
        json!(["cases/coupling/lib.rs"]),
    );
    assert_eq!(
        excluded["test_observations"]["routes"][0]["status"],
        "unadmitted_test_file"
    );
    assert!(records(&excluded).is_empty());
    assert!(
        excluded["test_observations"]["coverage"]["note"]
            .as_str()
            .unwrap()
            .contains("never means tests unaffected")
    );
    repo.write("cases/coupling/tests/mod.rs", "#[test] fn checks() {}");
    let competing = advice(
        &mut client,
        &repo,
        "cases/coupling/lib.rs",
        json!(["cases/coupling"]),
    );
    assert_eq!(
        competing["test_observations"]["routes"][0]["status"],
        "competing_layouts"
    );
    repo.write(
        "cases/coupling/lib.rs",
        "const LIMIT: u8 = 4;\n#[cfg(test)] #[path = \"custom.rs\"] mod tests;\n",
    );
    let remapped = advice(
        &mut client,
        &repo,
        "cases/coupling/lib.rs",
        json!(["cases/coupling"]),
    );
    assert_eq!(
        remapped["test_observations"]["routes"][0]["status"],
        "unsupported_attributes_or_remapping"
    );
}
#[test]
fn nominal_fields_multiline_macros_and_unrelated_receivers_stay_honest() {
    let repo = Fixture::generate();
    let source = "pub struct State { secret: u8 }\nimpl State { fn inspect(&self) {} }\npub fn facade() {}\nconst LIMIT: u8 = 4;\n#[cfg(test)] mod tests { use super::State; use super::facade; struct Other { secret: u8 } #[test] fn checks(state: State, other: Other) { let _ = state.secret; let _ = state\n .secret; let _ = state.child.secret; let _ = other.secret; facade(); } #[test] fn tokens(state: State) { assert!(state.secret > 0, \"other.secret\"); custom!(state.secret /* other.secret */); assert!(super::LIMIT > 0); } }\n";
    repo.write("cases/coupling/lib.rs", source);
    let mut client = Client::new();
    let value = advice(
        &mut client,
        &repo,
        "cases/coupling/lib.rs",
        json!(["cases/coupling"]),
    );
    assert_eq!(value["status"], "complete", "{value}");
    let field = |text: &str| {
        records(&value)
            .iter()
            .find(|r| r["anchor"]["span"]["text"] == text && r["access_kind"] == "field_access")
            .unwrap()
    };
    assert_eq!(
        field("state.secret")["target_item_ids"],
        field("state\n .secret")["target_item_ids"]
    );
    assert!(
        !field("state.secret")["target_item_ids"]
            .as_array()
            .unwrap()
            .is_empty(),
        "{value}"
    );
    assert_eq!(
        field("state.secret")["attribution"],
        "written_nominal_or_path_route"
    );
    assert!(
        field("state.secret")["labels"]
            .as_array()
            .unwrap()
            .contains(&json!("mixed"))
    );
    assert_eq!(field("other.secret")["target_item_ids"], json!([]));
    assert_eq!(field("state.child.secret")["target_item_ids"], json!([]));
    assert_eq!(
        field("state.child.secret")["attribution"],
        "candidate_or_unresolved"
    );
    let macro_fields: Vec<_> = records(&value)
        .iter()
        .filter(|r| r["channel"] == "macro_token_candidate" && r["access_kind"] == "field_tokens")
        .collect();
    assert_eq!(macro_fields.len(), 2, "{value}");
    assert!(macro_fields.iter().all(|r| {
        r["uncertainty"]
            .as_array()
            .unwrap()
            .contains(&json!("macro_tokens_not_expanded_or_bound"))
    }));
    assert!(has(&value, "super::LIMIT", "macro_token_candidate"));
    assert_anchors(&repo, &value);
}
#[test]
fn external_test_markers_never_infer_target_or_library_alias() {
    let repo = Fixture::generate();
    repo.write(
        "cases/coupling/lib.rs",
        "pub struct State { secret: u8 }\nimpl State { fn inspect(&self) {} }\n",
    );
    repo.write(
        "cases/coupling/external.rs",
        "use library::State; #[test] fn inspect(value: State) { let _ = value.secret; }",
    );
    let mut client = Client::new();
    let value = advice(
        &mut client,
        &repo,
        "cases/coupling/lib.rs",
        json!(["cases/coupling"]),
    );
    assert_eq!(
        value["test_observations"]["coverage"]["unlinked_test_roots"],
        json!(["cases/coupling/external.rs"])
    );
    let field = records(&value)
        .iter()
        .find(|r| r["access_kind"] == "field_access")
        .unwrap();
    assert_eq!(field["target_item_ids"], json!([]));
    assert_eq!(field["module_segments"], Value::Null);
    assert!(
        field["uncertainty"]
            .as_array()
            .unwrap()
            .contains(&json!("unlinked_test_root"))
    );
}

#[test]
fn cfg_mentions_disclose_limits_without_unlinked_test_roots() {
    for attribute in [
        "#[cfg(not(test))]",
        "#[cfg_attr(test, allow(dead_code))]",
        "#[cfg(any(test, feature = \"other\"))]",
        "#[cfg(all(test, feature = \"other\"))]",
    ] {
        let repo = Fixture::generate();
        repo.write("cases/coupling/lib.rs", "const LIMIT: u8 = 4;\n");
        repo.write(
            "cases/coupling/external.rs",
            &format!("{attribute} fn helper() {{ let _ = crate::LIMIT; }}\n"),
        );
        let mut client = Client::new();
        let value = advice(
            &mut client,
            &repo,
            "cases/coupling/lib.rs",
            json!(["cases/coupling"]),
        );
        assert_eq!(value["status"], "complete", "{value}");
        let projection = &value["test_observations"];
        assert_eq!(projection["coverage"]["completed"], true);
        assert_eq!(projection["coverage"]["unlinked_test_roots"], json!([]));
        assert_eq!(projection["routes"], json!([]));
        assert!(records(&value).is_empty(), "{projection}");
        let limitations = projection["limitations"].as_array().unwrap();
        assert_eq!(limitations.len(), 1, "{projection}");
        assert_eq!(limitations[0]["reason"], "unsupported_test_cfg");
        assert_eq!(limitations[0]["anchor"]["span"]["text"], attribute);
        assert_eq!(
            limitations[0]["paths"],
            json!(["cases/coupling/external.rs"])
        );
        assert!(
            limitations[0]["note"]
                .as_str()
                .unwrap()
                .contains("does not establish a test root or coupling")
        );
    }
}

#[test]
fn cfg_mentions_do_not_seed_inline_test_routes() {
    for attribute in ["#[cfg(not(test))]", "#[cfg_attr(test, allow(dead_code))]"] {
        let repo = Fixture::generate();
        repo.write(
            "cases/coupling/lib.rs",
            &format!("const LIMIT: u8 = 4;\n{attribute} mod helpers {{ fn helper() {{ let _ = super::LIMIT; }} }}\n"),
        );
        let mut client = Client::new();
        let value = advice(
            &mut client,
            &repo,
            "cases/coupling/lib.rs",
            json!(["cases/coupling"]),
        );
        assert_eq!(value["status"], "complete", "{value}");
        let projection = &value["test_observations"];
        assert_eq!(projection["routes"], json!([]));
        assert_eq!(projection["coverage"]["unlinked_test_roots"], json!([]));
        assert!(records(&value).is_empty(), "{projection}");
        let limitation = &projection["limitations"][0];
        assert_eq!(limitation["reason"], "unsupported_test_cfg");
        assert_eq!(limitation["anchor"]["span"]["text"], attribute);
        assert_eq!(limitation["paths"], json!(["cases/coupling/lib.rs"]));
    }
}

#[test]
fn positive_test_cfg_markers_preserve_linked_and_unlinked_routes() {
    for attribute in ["#[cfg(test)]", "#[ cfg ( test ) ]"] {
        let repo = Fixture::generate();
        repo.write(
            "cases/coupling/lib.rs",
            &format!("const LIMIT: u8 = 4;\n{attribute} mod checks {{ fn helper() {{ let _ = super::LIMIT; }} }}\n"),
        );
        repo.write(
            "cases/coupling/external.rs",
            &format!("{attribute} mod checks {{ fn helper() {{ let _ = crate::LIMIT; }} }}\n"),
        );
        let mut client = Client::new();
        let value = advice(
            &mut client,
            &repo,
            "cases/coupling/lib.rs",
            json!(["cases/coupling"]),
        );
        assert_eq!(value["status"], "complete", "{value}");
        let projection = &value["test_observations"];
        assert_eq!(
            projection["coverage"]["unlinked_test_roots"],
            json!(["cases/coupling/external.rs"])
        );
        assert_eq!(projection["routes"][0]["status"], "inline_written");
        assert_eq!(projection["routes"][1]["status"], "inline_uncertain");
        for route in projection["routes"].as_array().unwrap() {
            assert_eq!(route["conditional"], true);
            assert_eq!(route["attributes"][0]["span"]["text"], attribute);
        }
        assert!(has(&value, "super::LIMIT", "cst"), "{projection}");
        assert!(has(&value, "crate::LIMIT", "cst"), "{projection}");
        assert!(
            projection["limitations"]
                .as_array()
                .unwrap()
                .iter()
                .all(|limitation| limitation["reason"] != "unsupported_test_cfg")
        );
        assert_anchors(&repo, &value);
    }
}

#[test]
fn diagnostic_caps_preserve_projection_and_output_fitting_discloses_omissions() {
    let repo = Fixture::generate();
    let source = format!(
        "struct State {{ secret: u8 }}\nimpl State {{ fn inspect(&self) {{}} }}\n#[cfg(test)] mod tests {{ use super::State; #[test] fn inspect(value: State) {{ {} }} }}\n",
        "let _ = value.secret;".repeat(80)
    );
    repo.write("cases/coupling/lib.rs", &source);
    let mut client = Client::new();
    let args = json!({"repo_path":repo.0,"crate_root":"cases/coupling/lib.rs","source_path":"cases/coupling/lib.rs","paths":["cases/coupling"],"limits":{"diagnostic_count":100000,"response_bytes":16777216}});
    let full = client.call("suggest_split", args.clone());
    let mut capped = args.clone();
    capped["limits"]["diagnostic_count"] = json!(0);
    let capped = client.call("suggest_split", capped);
    assert_eq!(capped["test_observations"], full["test_observations"]);
    let mut tight = args;
    tight["limits"] = json!({"response_bytes":65536,"text_bytes":0});
    let tight = client.call("suggest_split", tight);
    assert_eq!(
        tight["test_observations"]["coverage"]["completed"], false,
        "{tight}"
    );
    assert_eq!(
        tight["counts"]["test_observations"],
        full["counts"]["test_observations"]
    );
    assert_eq!(
        tight["counts"]["omissions"]["test_observations"],
        full["counts"]["test_observations"]
    );
    assert!(
        tight["counts"]["omissions"]["test_routes"]
            .as_u64()
            .unwrap()
            > 0
    );
    let wire = json!({"content":[{"type":"text","text":tight.to_string()}],"structuredContent":tight,"isError":false}).to_string().len()+4096;
    assert!(wire <= 65536);
}
