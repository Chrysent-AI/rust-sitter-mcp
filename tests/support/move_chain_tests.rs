use super::*;

#[test]
fn wrong_binary_root_blocks_both_destination_forms_and_library_root_moves() {
    use chain_fixture::*;
    let repo = Fixture::generate();
    install(&repo);
    let mut client = stdio_client::Client::new();
    for destination in [
        json!({"kind":"existing","path":DESTINATION}),
        json!({"kind":"new_sibling","path":"cases/chain/src/scheduler/dangling.rs","parent_path":PARENT}),
    ] {
        let mut request = json!({"repo_path":repo.0,"crate_root":BIN_ROOT,"paths":["cases/chain"],"moves":[{"item":anchor(&repo,SOURCE,SELECTED),"destination":destination}],"limits":{"text_bytes":0,"diagnostic_count":0}});
        let before = observe(&repo.0);
        let wrong = client.call("move_item", request.clone());
        code(&wrong, "CRATE_IDENTITY_UNCERTAIN");
        let diagnostics = wrong["plan"]["chain_diagnostics"].as_array().unwrap();
        assert_eq!(diagnostics.len(), 2);
        for diagnostic in diagnostics {
            assert_eq!(diagnostic["reason"], "source_not_in_root_chain");
            assert_eq!(diagnostic["crate_root"], BIN_ROOT);
            assert_eq!(diagnostic["at_file_path"], BIN_ROOT);
            assert_eq!(diagnostic["evidenced_prefix_paths"], json!([BIN_ROOT]));
            assert_eq!(diagnostic["relation"], "root_search_exhausted");
            assert!(diagnostic["declaration"].is_null());
        }
        assert!(
            diagnostics
                .iter()
                .any(|d| d["role"] == "source" && d["requested_path"] == SOURCE)
        );
        let (role, path) = if destination["kind"] == "existing" {
            ("destination", DESTINATION)
        } else {
            ("declaration_parent", PARENT)
        };
        assert!(
            diagnostics
                .iter()
                .any(|d| d["role"] == role && d["requested_path"] == path)
        );
        linked(
            &wrong["plan"]["chain_diagnostics"],
            &wrong["plan"]["decisions"],
        );
        assert_eq!(wrong, client.call("move_item", request.clone()));
        request["crate_root"] = json!(LIB_ROOT);
        let correct = client.call("move_item", request);
        assert_eq!(correct["schema_version"], 1);
        assert_eq!(correct["plan"]["applicable"], true, "{correct}");
        assert!(
            correct["plan"]["chain_diagnostics"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        let copy = apply(&repo, &correct);
        assert!(
            fs::read_to_string(copy.0.join(destination["path"].as_str().unwrap()))
                .unwrap()
                .contains(SELECTED)
        );
        if role == "declaration_parent" {
            assert_eq!(
                correct["plan"]["created_files"][0]["declaration_link"]["kind"],
                "reused"
            );
        }
        assert_eq!(observe(&repo.0), before);
    }
}

#[test]
fn chain_failure_decisions_survive_capped_blockers_and_preserve_error_boundaries() {
    let repo = fixture("fn selected() {}\nfn other() {}\n");
    let root = "cases/layout/lib.rs";
    let target = "cases/layout/target.rs";
    let make = || {
        request(
            &repo,
            json!([entry(&repo, "fn selected() {}", new(target))]),
        )
    };
    for (written, reason) in [
        ("#[cfg(any())] mod target;", "conditional_declaration"),
        ("#[path = \"elsewhere.rs\"] mod target;", "path_attribute"),
        ("mod target; mod target;", "competing_declarations"),
        ("mod target {}", "inline_module_layout"),
        (
            "#[allow(dead_code)] mod target;",
            "unexamined_declaration_attributes",
        ),
    ] {
        let text = format!("mod source;\nmod destination;\n{written}\n");
        repo.write(root, &text);
        let result = run(&repo, make());
        code(&result, "MODULE_DECLARATION_CONFLICT");
        assert_eq!(result["error"]["field"], "moves.destination");
        let diagnostic = result["plan"]["chain_diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .find(|d| d["reason"] == reason)
            .unwrap();
        assert_eq!(diagnostic["crate_root"], root);
        assert_eq!(diagnostic["at_file_path"], root);
        assert_eq!(diagnostic["role"], "declaration_parent");
        assert_eq!(diagnostic["requested_path"], target);
        let range = &diagnostic["declaration"]["range"];
        let bytes = &text[range["start_byte"].as_u64().unwrap() as usize
            ..range["end_byte"].as_u64().unwrap() as usize];
        assert!(bytes.starts_with("mod target"));
        chain_fixture::linked(
            &result["plan"]["chain_diagnostics"],
            &result["plan"]["decisions"],
        );
    }
    repo.write(root, "mod source;\nmod destination;\n");
    for (parent, reason) in [
        ("cases/layout/source.rs", "ordinary_layout_mismatch"),
        ("cases/layout/unadmitted.rs", "chain_file_unadmitted"),
    ] {
        let mut request = make();
        request["moves"][0]["destination"]["parent_path"] = json!(parent);
        let result = run(&repo, request);
        code(&result, "INVALID_DECLARATION_PARENT");
        assert_eq!(result["error"]["field"], "moves[0].destination.parent_path");
        assert_eq!(result["plan"]["chain_diagnostics"][0]["reason"], reason);
        assert_eq!(
            result["plan"]["chain_diagnostics"][0]["at_file_path"],
            parent
        );
        chain_fixture::linked(
            &result["plan"]["chain_diagnostics"],
            &result["plan"]["decisions"],
        );
    }
    repo.write("cases/layout/target/mod.rs", "fn competing() {}\n");
    let competing = run(&repo, make());
    code(&competing, "MODULE_DECLARATION_CONFLICT");
    assert_eq!(
        competing["plan"]["chain_diagnostics"][0]["reason"],
        "competing_file_layout"
    );
    // Distinct chain decisions cannot be recovered from a one-entry blocker preview.
    repo.write(
        root,
        "#[cfg(any())] mod source;\n#[path = \"elsewhere.rs\"] mod destination;\n",
    );
    let mut request = request(
        &repo,
        json!([entry(
            &repo,
            "fn selected() {}",
            json!({"kind":"existing","path":"cases/layout/destination.rs"})
        )]),
    );
    request["limits"] = json!({"diagnostic_count":1});
    let result = run(&repo, request.clone());
    assert!(result["plan"]["blockers"].as_array().unwrap().len() <= 1);
    let diagnostics = result["plan"]["chain_diagnostics"].as_array().unwrap();
    assert!(
        diagnostics
            .iter()
            .any(|d| d["reason"] == "conditional_declaration" && d["role"] == "source")
    );
    assert!(
        diagnostics
            .iter()
            .any(|d| d["reason"] == "path_attribute" && d["role"] == "destination")
    );
    chain_fixture::linked(
        &result["plan"]["chain_diagnostics"],
        &result["plan"]["decisions"],
    );
    request["globs"] = json!(["cases/layout/source.rs", "cases/layout/destination.rs"]);
    let root_error = run(&repo, request);
    code(&root_error, "STALE_SELECTION");
    assert_eq!(root_error["error"]["field"], "crate_root");
    assert_eq!(
        root_error["plan"]["chain_diagnostics"][0]["crate_root"],
        root
    );
    assert_eq!(
        root_error["plan"]["chain_diagnostics"][0]["reason"],
        "chain_file_unadmitted"
    );
}

#[test]
fn missing_unadmitted_and_inherited_source_hops_do_not_blame_a_clean_destination() {
    let repo = Fixture::generate();
    let root = "cases/hops/lib.rs";
    let branch = "cases/hops/branch.rs";
    let source = "cases/hops/branch/leaf.rs";
    let destination = "cases/hops/destination.rs";
    repo.write(root, "mod branch;\nmod destination;\n");
    repo.write(source, "fn selected() {}\n");
    repo.write(destination, "fn retained() {}\n");
    let mut args = json!({"repo_path":repo.0,"crate_root":root,"paths":["cases/hops"],"moves":[{"item":anchor(&repo,source,"fn selected() {}"),"destination":{"kind":"existing","path":destination}}],"limits":{"text_bytes":0,"diagnostic_count":1}});
    for (written, reason, relation, at) in [
        (None, "chain_file_missing", "possible_ancestor", root),
        (
            Some("mod leaf;\n"),
            "chain_file_unadmitted",
            "possible_ancestor",
            root,
        ),
        (
            Some("#![no_implicit_prelude]\nmod leaf;\n"),
            "inherited_uncertainty",
            "direct",
            branch,
        ),
        (
            Some("@\nmod leaf;\n"),
            "inherited_uncertainty",
            "direct",
            branch,
        ),
    ] {
        if let Some(written) = written {
            repo.write(branch, written);
        }
        if reason == "chain_file_unadmitted" {
            args["globs"] = json!([root, source, destination]);
        } else {
            args.as_object_mut().unwrap().remove("globs");
        }
        let result = run(&repo, args.clone());
        code(&result, "CRATE_IDENTITY_UNCERTAIN");
        let diagnostics = result["plan"]["chain_diagnostics"].as_array().unwrap();
        assert!(diagnostics.iter().all(|d| d["role"] == "source"));
        let diagnostic = diagnostics.iter().find(|d| d["reason"] == reason).unwrap();
        assert_eq!(diagnostic["crate_root"], root);
        assert_eq!(diagnostic["requested_path"], source);
        assert_eq!(diagnostic["at_file_path"], at);
        assert_eq!(diagnostic["relation"], relation);
        chain_fixture::linked(
            &result["plan"]["chain_diagnostics"],
            &result["plan"]["decisions"],
        );
    }
    repo.write(branch, "mod leaf;\n");
    let good = run(&repo, args);
    assert_eq!(good["plan"]["applicable"], true);
    assert!(
        good["plan"]["chain_diagnostics"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn chain_diagnostic_links_are_omitted_together_on_move_overflow() {
    use chain_fixture::*;
    let repo = Fixture::generate();
    install(&repo);
    repo.write(
        SOURCE,
        &format!(
            "fn selected() {{ let _payload = \"{}\"; }}\n",
            "x".repeat(24000)
        ),
    );
    let text = fs::read_to_string(repo.0.join(SOURCE)).unwrap();
    let args = json!({"repo_path":repo.0,"crate_root":BIN_ROOT,"paths":["cases/chain"],"moves":[{"item":anchor(&repo,SOURCE,text.trim()),"destination":{"kind":"existing","path":DESTINATION}}],"limits":{"text_bytes":0,"response_bytes":65536}});
    let result = run(&repo, args);
    code(&result, "response_bytes");
    assert_eq!(result["status"], "partial");
    assert!(
        result["plan"]["chain_diagnostics"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(result["plan"]["decisions"].as_array().unwrap().is_empty());
    assert!(
        result["counts"]["omissions"]["chain_diagnostics"]
            .as_u64()
            .unwrap()
            > 0
    );
    assert!(
        result["counts"]["omissions"]["chain_diagnostic_references"]
            .as_u64()
            .unwrap()
            > 0
    );
}
