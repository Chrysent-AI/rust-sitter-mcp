use super::*;

#[test]
fn workspace_member_chains_admit_metadata_and_declared_cfg_without_guessing() {
    let repo = Fixture::generate();
    let root = "cases/workspace/member/src/lib.rs";
    let parent = "cases/workspace/member/src/branch/mod.rs";
    let source = "cases/workspace/member/src/branch/leaf.rs";
    repo.write(
        "cases/workspace/Cargo.toml",
        "[workspace]\nmembers = [\"member\"]\n",
    );
    repo.write(root, "#![doc = include_str!(\"missing.md\")]\n#![cfg_attr(docsrs, feature(doc_cfg))]\n#[cfg(feature = \"enabled\")]\nmod branch;\n");
    repo.write(parent, "#![forbid(unsafe_code)]\n#[cfg_attr(docsrs, doc(cfg(feature = \"enabled\")))]\nmod leaf;\n");
    repo.write(source, "fn selected() {}\nfn retained() {}\n");
    let mut args = json!({"repo_path":repo.0,"crate_root":root,"paths":["cases/workspace/member/src"],"resolve_semantic":true,"semantic_configuration":{"crates":[{"name":"member","root_file":root,"edition":"2021","features":["enabled"],"cfg":[],"dependencies":[]}]},"moves":[{"item":anchor(&repo,source,"fn selected() {}"),"destination":{"kind":"new_sibling","path":"cases/workspace/member/src/branch/moved.rs","parent_path":parent}}]});
    let before = observe(&repo.0);
    let good = run(&repo, args.clone());
    assert_eq!(good["plan"]["applicable"], true, "{good}");
    assert_eq!(good["plan"]["chain_diagnostics"], json!([]));
    assert_eq!(observe(&repo.0), before);
    args["resolve_semantic"] = json!(false);
    let written = run(&repo, args.clone());
    assert_eq!(written["plan"]["applicable"], true, "{written}");
    assert_eq!(written["plan"]["integrity"]["semantic"], "not_performed");
    let mut without_configuration = args.clone();
    without_configuration["semantic_configuration"] = Value::Null;
    let default = run(&repo, without_configuration);
    code(&default, "CRATE_IDENTITY_UNCERTAIN");
    assert!(
        default["plan"]["chain_diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["reason"] == "conditional_declaration" && d["at_file_path"] == root)
    );
    args["resolve_semantic"] = json!(true);
    let configuration = args["semantic_configuration"].clone();
    args["semantic_configuration"]["crates"][0]["edition"] = json!("invalid");
    code(&run(&repo, args.clone()), "CRATE_IDENTITY_UNCERTAIN");
    args["semantic_configuration"] = configuration.clone();
    let duplicate = args["semantic_configuration"]["crates"][0].clone();
    args["semantic_configuration"]["crates"]
        .as_array_mut()
        .unwrap()
        .push(duplicate);
    code(&run(&repo, args.clone()), "CRATE_IDENTITY_UNCERTAIN");
    args["semantic_configuration"] = configuration;
    for (written, reason) in [
        (
            "#[cfg(any(feature = \"enabled\", missing))] mod branch;\n",
            "conditional_declaration",
        ),
        (
            "#[cfg(not(feature = \"enabled\"))] mod branch;\n",
            "conditional_declaration",
        ),
        (
            "#[cfg(feature = \"enabled\")] mod branch;\nmod branch;\n",
            "competing_declarations",
        ),
        ("#[path = \"other.rs\"] mod branch;\n", "path_attribute"),
        ("@\nmod branch;\n", "inherited_uncertainty"),
    ] {
        repo.write(root, written);
        let blocked = run(&repo, args.clone());
        withheld(&blocked);
        code(&blocked, "CRATE_IDENTITY_UNCERTAIN");
        assert!(
            blocked["plan"]["chain_diagnostics"]
                .as_array()
                .unwrap()
                .iter()
                .any(|d| d["reason"] == reason && d["at_file_path"] == root),
            "{blocked}"
        );
        linked_move(&blocked);
    }
    repo.write(root, "mod branch;\n");
    args["globs"] = json!([root, source]);
    args["moves"][0]["destination"] = json!({"kind":"existing","path":root});
    let unadmitted = run(&repo, args);
    code(&unadmitted, "CRATE_IDENTITY_UNCERTAIN");
    assert!(
        unadmitted["plan"]["chain_diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["reason"] == "chain_file_unadmitted"
                && d["candidate_paths"]
                    .as_array()
                    .unwrap()
                    .contains(&json!(parent)))
    );
}

#[test]
fn workspace_member_layouts_and_metadata_bearing_dangling_edges_are_proved() {
    for (member, branch, leaf, root_metadata) in [
        (
            "web_member",
            "routing",
            "method_routing",
            "#![doc = include_str!(\"missing.md\")]\n#![cfg_attr(docsrs, feature(doc_cfg))]\n#![cfg_attr(test, allow(clippy::float_cmp))]\n#![cfg_attr(not(test), warn(clippy::print_stdout))]\n",
        ),
        (
            "cli_member",
            "builder",
            "command",
            "#![doc = include_str!(\"../README.md\")]\n#![cfg_attr(docsrs, feature(doc_cfg))]\n#![forbid(unsafe_code)]\n#![warn(missing_docs)]\n",
        ),
    ] {
        let repo = Fixture::generate();
        repo.write(
            "cases/members/Cargo.toml",
            "[workspace]\nmembers = [\"web_member\", \"cli_member\"]\n",
        );
        let base = format!("cases/members/{member}/src");
        let root = format!("{base}/lib.rs");
        let parent = format!("{base}/{branch}/mod.rs");
        let source = format!("{base}/{branch}/{leaf}.rs");
        let destination = format!("{base}/{branch}/moved.rs");
        repo.write(&root, &format!("{root_metadata}pub mod {branch};\n"));
        repo.write(
            &parent,
            &format!("#[doc(hidden)]\nmod {leaf};\n#[doc(hidden)]\nmod moved;\n"),
        );
        repo.write(&source, "fn selected() {}\nfn retained() {}\n");
        let args = json!({"repo_path":repo.0,"crate_root":root,"paths":[base],"moves":[{"item":anchor(&repo,&source,"fn selected() {}"),"destination":{"kind":"new_sibling","path":destination,"parent_path":parent}}]});
        let result = run(&repo, args);
        assert_eq!(result["status"], "complete", "{result}");
        assert_eq!(result["plan"]["applicable"], true, "{result}");
        assert_eq!(result["plan"]["chain_diagnostics"], json!([]));
        assert_eq!(result["plan"]["integrity"]["semantic"], "not_performed");
        let copy = apply(&repo, &result);
        assert_eq!(
            fs::read_to_string(copy.0.join(destination)).unwrap(),
            "fn selected() {}\n"
        );
        assert_eq!(
            fs::read(copy.0.join(&parent)).unwrap(),
            fs::read(repo.0.join(&parent)).unwrap()
        );
        assert_eq!(
            fs::read(copy.0.join(&root)).unwrap(),
            fs::read(repo.0.join(&root)).unwrap()
        );
    }
}

#[test]
fn root_and_module_metadata_preserve_identity_without_executing_doc_payloads() {
    let repo = Fixture::generate();
    let root = "cases/metadata/lib.rs";
    let parent = "cases/metadata/branch.rs";
    let source = "cases/metadata/branch/leaf.rs";
    repo.write(source, "fn selected() {}\nfn retained() {}\n");
    let args = json!({"repo_path":repo.0,"crate_root":root,"paths":["cases/metadata"],"moves":[{"item":anchor(&repo,source,"fn selected() {}"),"destination":{"kind":"new_sibling","path":"cases/metadata/branch/moved.rs","parent_path":parent}}]});
    for attribute in [
        "#![doc = include_str!(\"missing.md\")]",
        "#![doc = \"plain documentation\"]",
        "#![allow(dead_code)]",
        "#![deny(warnings)]",
        "#![warn(missing_docs)]",
        "#![forbid(unsafe_code)]",
    ] {
        for at in [root, parent] {
            repo.write(root, "mod branch;\n");
            repo.write(parent, "mod leaf;\n");
            let declaration = if at == root { "branch" } else { "leaf" };
            repo.write(at, &format!("{attribute}\nmod {declaration};\n"));
            let before = observe(&repo.0);
            let result = run(&repo, args.clone());
            assert_eq!(
                result["plan"]["applicable"], true,
                "{attribute} at {at}: {result}"
            );
            assert_eq!(result["plan"]["chain_diagnostics"], json!([]));
            assert_eq!(result["plan"]["integrity"]["semantic"], "not_performed");
            assert_eq!(observe(&repo.0), before);
            let copy = apply(&repo, &result);
            // The parent can gain a sibling declaration; the scope prologue
            // itself must remain byte-identical at its original location.
            assert!(
                fs::read(copy.0.join(at))
                    .unwrap()
                    .starts_with(format!("{attribute}\n").as_bytes())
            );
        }
    }
}

#[test]
fn root_and_module_controls_and_attribute_providers_keep_anchored_chain_refusals() {
    let repo = Fixture::generate();
    let root = "cases/metadata/lib.rs";
    let parent = "cases/metadata/branch.rs";
    let source = "cases/metadata/branch/leaf.rs";
    repo.write(source, "fn selected() {}\nfn retained() {}\n");
    let args = json!({"repo_path":repo.0,"crate_root":root,"paths":["cases/metadata"],"moves":[{"item":anchor(&repo,source,"fn selected() {}"),"destination":{"kind":"new_sibling","path":"cases/metadata/branch/moved.rs","parent_path":parent}}],"limits":{"diagnostic_count":1000}});
    for attribute in [
        "#![no_std]",
        "#![no_core]",
        "#![feature(arbitrary_gate)]",
        "#![recursion_limit = \"256\"]",
        "#![macro_use]",
        "#![provider(doc = \"not builtin\")]",
        "#![custom::doc(include_str!(\"missing.md\"))]",
    ] {
        for at in [root, parent] {
            repo.write(root, "mod branch;\n");
            repo.write(parent, "mod leaf;\n");
            let declaration = if at == root { "branch" } else { "leaf" };
            repo.write(at, &format!("{attribute}\nmod {declaration};\n"));
            let before = observe(&repo.0);
            let result = run(&repo, args.clone());
            withheld(&result);
            code(&result, "CRATE_IDENTITY_UNCERTAIN");
            let diagnostic = result["plan"]["chain_diagnostics"]
                .as_array()
                .unwrap()
                .iter()
                .find(|d| {
                    d["reason"] == "root_attribute_chain_uncertainty" && d["at_file_path"] == at
                })
                .unwrap_or_else(|| panic!("{attribute} at {at}: {result}"));
            assert_eq!(diagnostic["declaration"]["path"], at);
            assert_eq!(
                diagnostic["declaration"]["range"],
                json!({"start_byte":0,"end_byte":attribute.len()})
            );
            assert!(
                result["plan"]["chain_diagnostics"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|d| d["reason"] == "inherited_uncertainty")
            );
            linked_move(&result);
            assert_eq!(observe(&repo.0), before);
        }
    }
}

#[test]
fn macro_root_move_refuses_with_named_actionable_cause() {
    let repo = Fixture::generate();
    let root = "cases/refusals/lib.rs";
    let parent = "cases/refusals/branch.rs";
    let source = "cases/refusals/branch/leaf.rs";
    repo.write(parent, "mod leaf;\n");
    repo.write(source, "fn selected() {}\nfn retained() {}\n");
    let args = json!({"repo_path":repo.0,"crate_root":root,"paths":["cases/refusals"],"moves":[{"item":anchor(&repo,source,"fn selected() {}"),"destination":{"kind":"new_sibling","path":"cases/refusals/branch/moved.rs","parent_path":parent}}],"limits":{"diagnostic_count":1000}});
    for written in [
        chain_fixture::MACRO_ROOT,
        "emit_modules!(mod /* tokens */ branch;);\n",
        chain_fixture::MACRO_NESTED_INPUT,
        chain_fixture::MACRO_NESTED_OUTPUT,
    ] {
        repo.write(root, written);
        let result = run(&repo, args.clone());
        withheld(&result);
        chain_fixture::named_refusal(
            &result["plan"]["chain_diagnostics"],
            &result["plan"]["decisions"],
            "macro_generated_module_tree",
            "macro token tree",
        );
        assert_eq!(
            result["plan"]["chain_diagnostics"][0]["relation"],
            "possible_ancestor"
        );
        assert_eq!(result, run(&repo, args.clone()));
    }
    // An unrelated macro declaration cannot change a proven ordinary chain.
    repo.write(
        root,
        "mod branch;\nmacro_rules! unrelated { () => { mod other; }; }\n",
    );
    let clean = run(&repo, args);
    assert!(
        clean["plan"]["chain_diagnostics"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn inner_attribute_move_adds_named_cause_without_changing_inherited_veto() {
    let repo = Fixture::generate();
    let root = "cases/refusals/lib.rs";
    let parent = "cases/refusals/branch.rs";
    let source = "cases/refusals/branch/leaf.rs";
    repo.write(parent, "mod leaf;\n");
    for (written, leaf) in [
        (
            chain_fixture::INNER_ATTR_ROOT,
            "fn selected() {}\nfn retained() {}\n",
        ),
        ("mod branch;\n", chain_fixture::INNER_ATTR_NESTED),
    ] {
        repo.write(root, written);
        repo.write(source, leaf);
        let args = json!({"repo_path":repo.0,"crate_root":root,"paths":["cases/refusals"],"moves":[{"item":anchor(&repo,source,"fn selected() {}"),"destination":{"kind":"new_sibling","path":"cases/refusals/branch/moved.rs","parent_path":parent}}],"limits":{"diagnostic_count":1000}});
        let result = run(&repo, args);
        withheld(&result);
        let diagnostics = &result["plan"]["chain_diagnostics"];
        chain_fixture::named_refusal(
            diagnostics,
            &result["plan"]["decisions"],
            "root_attribute_chain_uncertainty",
            "non-allowlisted inner attribute",
        );
        assert!(
            diagnostics
                .as_array()
                .unwrap()
                .iter()
                .any(|d| d["reason"] == "inherited_uncertainty")
        );
    }
}

fn linked_move(result: &Value) {
    let diagnostics = &result["plan"]["chain_diagnostics"];
    let decisions = &result["plan"]["decisions"];
    if result["counts"]["omissions"]["decisions"]
        .as_u64()
        .unwrap_or(0)
        == 0
    {
        chain_fixture::linked(diagnostics, decisions);
        return;
    }
    for decision in decisions.as_array().unwrap() {
        for id in decision["chain_diagnostic_ids"].as_array().unwrap() {
            assert!(
                diagnostics
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|d| d["id"] == *id)
            );
        }
    }
    let groups: Vec<_> = result["plan"]["decision_groups"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|g| g["reason"] == "module_chain_failure")
        .collect();
    assert_eq!(
        groups
            .iter()
            .map(|g| g["count"].as_u64().unwrap() as usize)
            .sum::<usize>(),
        diagnostics.as_array().unwrap().len()
    );
    assert!(
        groups
            .iter()
            .all(|g| !g["actions"].as_array().unwrap().is_empty())
    );
}

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
        linked_move(&wrong);
        assert_eq!(wrong["plan"]["decisions"], json!([]));
        assert_eq!(wrong["counts"]["omissions"]["decisions"], 2);
        let actions = wrong["plan"]["decision_groups"][0]["actions"]
            .as_array()
            .unwrap();
        assert!(
            actions
                .iter()
                .any(|a| a["field"] == "crate_root" && a["purpose"] == "submit_for_analysis")
        );
        assert_eq!(wrong, client.call("move_item", request.clone()));
        request["crate_root"] = json!(LIB_ROOT);
        let correct = client.call("move_item", request);
        assert_eq!(correct["schema_version"], 2);
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
            "#[unexamined] mod target;",
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
    linked_move(&result);
    assert_eq!(result["plan"]["decisions"].as_array().unwrap().len(), 1);
    let actions: Vec<_> = result["plan"]["decision_groups"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|group| group["actions"].as_array().unwrap())
        .collect();
    for construct in ["conditional_declaration", "path_attribute"] {
        assert!(actions.iter().any(|a| a["construct"] == construct));
    }
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
        linked_move(&result);
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
fn move_overflow_retains_chain_summary_when_anchored_detail_cannot_fit() {
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
        !result["plan"]["chain_diagnostics"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(result["plan"]["decisions"].as_array().unwrap().is_empty());
    assert!(result["counts"]["omissions"]["chain_diagnostics"].is_null());
    assert_eq!(result["plan"]["decision_groups"][0]["count"], 1);
    assert!(result["root"].is_string() && result["snapshot_id"].is_string());
    assert!(
        result["counts"]["omissions"]["chain_diagnostic_references"]
            .as_u64()
            .unwrap()
            > 0
    );
}
