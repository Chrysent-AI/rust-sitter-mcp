use super::*;

fn enabled(mut args: Value) -> Value {
    args["assume_standard_prelude"] = json!(true);
    args["limits"]["diagnostic_count"] = json!(100_000);
    args
}
fn proof_count(result: &Value, name: &str) -> usize {
    result["plan"]["binding_proofs"]
        .as_array()
        .map(|proofs| {
            proofs
                .iter()
                .filter(|p| p["anchor"]["expected_text"] == name)
                .count()
        })
        .unwrap_or(0)
}
// Disclosure changes metadata sizes, never the prior refusal/patch/proof fields.
fn refusal_behavior(mut value: Value) -> Vec<u8> {
    for decision in value["plan"]["decisions"].as_array_mut().unwrap() {
        decision.as_object_mut().unwrap().remove("refusal_basis");
    }
    value["plan"]
        .as_object_mut()
        .unwrap()
        .remove("blocked_summary");
    value["counts"]
        .as_object_mut()
        .unwrap()
        .remove("blocked_summary");
    value["counts"]
        .as_object_mut()
        .unwrap()
        .remove("analysis_descriptor_bytes");
    serde_json::to_vec(&value).unwrap()
}
fn existing() -> Value {
    json!({"kind":"existing","path":"cases/layout/destination.rs"})
}
fn has_reason(result: &Value, reason: &str) -> bool {
    result["plan"]["decisions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|d| d["reason"] == reason)
}

#[test]
fn prelude_only_batch_opt_in_is_lossless_labeled_and_counted() {
    let record =
        "struct Record { option: Option<String>, result: Result<Box<String>, Vec<String>> }";
    let function = "fn selected(input: Vec<String>) -> Vec<String> { input }";
    let repo = fixture(&format!("{record}\n{function}\n"));
    for destination in [existing(), new("cases/layout/prelude.rs")] {
        let args = request(
            &repo,
            json!([
                entry(&repo, record, destination.clone()),
                entry(&repo, function, destination.clone()),
            ]),
        );
        let default = run(&repo, args.clone());
        let mut off = args.clone();
        off["assume_standard_prelude"] = json!(false);
        assert_eq!(
            serde_json::to_vec(&default).unwrap(),
            serde_json::to_vec(&run(&repo, off)).unwrap()
        );
        withheld(&default);
        assert!(has_reason(&default, "external_or_missing_binding"));
        assert!(default["plan"].get("binding_proofs").is_none());
        assert!(default["coverage"].get("standard_prelude").is_none());
        let result = run(&repo, enabled(args));
        assert_eq!(result["schema_version"], 2);
        assert_eq!(result["coverage"]["standard_prelude"], 11);
        let proofs = result["plan"]["binding_proofs"].as_array().unwrap();
        assert_eq!(proofs.len(), 11);
        for (name, count) in [
            ("Option", 1),
            ("Result", 1),
            ("Box", 1),
            ("Vec", 3),
            ("String", 5),
        ] {
            assert_eq!(proof_count(&result, name), count);
        }
        let source = fs::read_to_string(repo.0.join("cases/layout/source.rs")).unwrap();
        let mut occurrences = std::collections::BTreeSet::new();
        for proof in proofs {
            assert_eq!(proof["class"], "standard_prelude");
            assert_eq!(proof["destination_path"], destination["path"]);
            assert!(
                proof["basis"]
                    .as_str()
                    .unwrap()
                    .contains("caller enabled assume_standard_prelude")
            );
            assert!(
                proof["standard_path"]
                    .as_str()
                    .unwrap()
                    .starts_with("std::")
            );
            assert_eq!(proof["item_ids"].as_array().unwrap().len(), 1);
            let anchor = &proof["anchor"];
            let start = anchor["range"]["start_byte"].as_u64().unwrap() as usize;
            let end = anchor["range"]["end_byte"].as_u64().unwrap() as usize;
            assert_eq!(
                &source[start..end],
                anchor["expected_text"].as_str().unwrap()
            );
            assert!(occurrences.insert((start, end)));
        }
        assert!(
            !result["plan"]["rewrites"]
                .as_array()
                .unwrap()
                .iter()
                .any(|r| r["kind"].as_str().unwrap().starts_with("import"))
        );
        let copy = apply(&repo, &result);
        let moved = fs::read_to_string(copy.0.join(destination["path"].as_str().unwrap())).unwrap();
        assert!(moved.contains(record) && moved.contains(function));
        assert!(!moved.contains("use std::"));
    }
}

#[test]
fn prelude_discharge_is_independent_of_unrelated_conditional_derives() {
    // A wildcard keeps the independent final named-binder gate out of this test.
    let selected = "fn selected(_: Option<Result<Box<Vec<String>>, u8>>) -> Option<u8> { if true { Some(1) } else { None } }";
    for path in ["source.rs", "destination.rs"] {
        for prefix in [
            "#[derive(Debug, Clone, JsonSchema)] #[schemars(crate = \"rmcp::schemars\")] struct Other;",
            "#[derive(Debug, Clone, Serialize)] #[serde(rename_all = \"snake_case\")] struct Other;",
            "#[derive(Debug, Clone, Deserialize)] #[serde(default)] struct Other;",
            "#[derive(Debug)] #[doc(hidden)] struct Other;",
            "#[derive(Debug, Serialize)] #[cfg_attr(feature = \"unknown\", serde(default))] struct Other;",
            "#[derive(Debug)] #[cfg_attr(feature = \"unknown\", derive(Clone))] struct Other;",
            "#[derive(Debug)] #[cfg_attr(feature = \"unknown\", cfg_attr(test, derive(Clone)))] struct Other;",
        ] {
            let repo = fixture(selected);
            let file = format!("cases/layout/{path}");
            let old = fs::read_to_string(repo.0.join(&file)).unwrap();
            repo.write(&file, &format!("{prefix}\n{old}"));
            let args = request(&repo, json!([entry(&repo, selected, existing())]));
            let default = run(&repo, args.clone());
            let mut off = args.clone();
            off["assume_standard_prelude"] = json!(false);
            assert_eq!(
                serde_json::to_vec(&default).unwrap(),
                serde_json::to_vec(&run(&repo, off)).unwrap()
            );
            withheld(&default);
            let result = run(&repo, enabled(args));
            assert_eq!(
                result["plan"]["applicable"], true,
                "{path}: {prefix}: {result}"
            );
            assert_eq!(result["coverage"]["standard_prelude"], 6);
            assert_eq!(result["coverage"]["standard_prelude_constructor"], 2);
            for (name, count) in [
                ("Option", 2),
                ("Result", 1),
                ("Box", 1),
                ("Vec", 1),
                ("String", 1),
                ("Some", 1),
                ("None", 1),
            ] {
                assert_eq!(proof_count(&result, name), count, "{result}");
            }
            assert_eq!(result["plan"]["integrity"]["semantic"], "not_performed");
            let copy = apply(&repo, &result);
            assert!(
                fs::read_to_string(copy.0.join("cases/layout/destination.rs"))
                    .unwrap()
                    .contains(selected)
            );
        }
    }
}

#[test]
fn qualified_prelude_is_independent_of_unrelated_conditional_companions() {
    let selected = "fn selected() -> std::result::Result<u8, u8> { loop {} }";
    for path in ["source.rs", "destination.rs"] {
        for prefix in [
            "#[derive(Debug, Serialize)] #[serde(default)] struct Other;",
            "#[derive(Debug, Deserialize)] #[serde(default)] struct Other;",
            "#[derive(Debug, JsonSchema)] #[schemars(crate = \"rmcp::schemars\")] struct Other;",
        ] {
            let repo = fixture(selected);
            let file = format!("cases/layout/{path}");
            let old = fs::read_to_string(repo.0.join(&file)).unwrap();
            repo.write(&file, &format!("{prefix}\n{old}"));
            let result = run(
                &repo,
                enabled(request(&repo, json!([entry(&repo, selected, existing())]))),
            );
            assert_eq!(
                result["plan"]["applicable"], true,
                "{path}: {prefix}: {result}"
            );
            assert_eq!(proof_count(&result, "std::result::Result"), 1, "{result}");
            assert_eq!(proof_count(&result, "Debug"), 0, "{result}");
            apply(&repo, &result);
        }
    }
}

#[test]
fn shadowed_builtin_derive_providers_refuse_prelude_types_and_constructors() {
    let selected = "fn selected(_: Option<Result<Box<Vec<String>>, u8>>) -> Option<u8> { if true { Some(1) } else { None } }";
    for path in ["source.rs", "destination.rs"] {
        for (prefix, witness) in [
            (
                "use unknown as Debug; #[derive(Debug)] struct Other;",
                "unknown as Debug",
            ),
            (
                "use custom::Debug; #[derive(Debug)] struct Other;",
                "custom::Debug",
            ),
            (
                "macro_rules! Debug { () => {} } #[derive(Debug)] struct Other;",
                "Debug",
            ),
        ] {
            let repo = fixture(selected);
            let file = format!("cases/layout/{path}");
            let old = fs::read_to_string(repo.0.join(&file)).unwrap();
            repo.write(&file, &format!("{prefix}\n{old}"));
            let args = request(&repo, json!([entry(&repo, selected, existing())]));
            let default = run(&repo, args.clone());
            let mut off = args.clone();
            off["assume_standard_prelude"] = json!(false);
            assert_eq!(
                serde_json::to_vec(&default).unwrap(),
                serde_json::to_vec(&run(&repo, off)).unwrap()
            );
            let result = run(&repo, enabled(args));
            withheld(&result);
            for name in [
                "Option", "Result", "Box", "Vec", "String", "Some", "None", "Debug",
            ] {
                assert_eq!(proof_count(&result, name), 0, "{path}: {prefix}: {result}");
            }
            for decision in result["plan"]["decisions"].as_array().unwrap() {
                if !["Option", "Result", "Box", "Vec", "String", "Some", "None"]
                    .iter()
                    .any(|name| decision["anchors"][0]["expected_text"] == *name)
                {
                    continue;
                }
                let bases = decision["refusal_basis"].as_array().unwrap();
                assert!(
                    bases.iter().any(|basis| {
                        basis["class"] == "shadow"
                            && basis["name"] == "Debug"
                            && basis["anchor"]["path"] == file
                            && basis["anchor"]["range"]["start_byte"]
                                == prefix.find(witness).unwrap()
                            && basis["anchor"]["range"]["end_byte"]
                                == prefix.find(witness).unwrap() + witness.len()
                    }),
                    "{decision}"
                );
                assert!(
                    bases
                        .iter()
                        .all(|basis| basis["class"] != "conditional_context"),
                    "{decision}"
                );
            }
        }
    }
}

#[test]
fn custom_debug_derive_emitting_option_cannot_receive_a_prelude_proof() {
    let selected = "fn selected(_: Option<u8>) -> Option<u8> { if true { Some(1) } else { None } }";
    for path in ["source.rs", "destination.rs"] {
        let repo = fixture(selected);
        repo.write("cases/layout/Cargo.toml", "[package]\nname = \"derive-binding-fixture\"\nversion = \"0.0.0\"\nedition = \"2024\"\n[lib]\npath = \"lib.rs\"\n[dependencies]\ninjector = { path = \"injector\" }\n[workspace]\nmembers = [\"injector\"]\n");
        repo.write("cases/layout/injector/Cargo.toml", "[package]\nname = \"injector\"\nversion = \"0.0.0\"\nedition = \"2024\"\n[lib]\nproc-macro = true\n");
        repo.write("cases/layout/injector/src/lib.rs", "extern crate proc_macro;\n#[proc_macro_derive(Debug)]\npub fn debug(_: proc_macro::TokenStream) -> proc_macro::TokenStream { \"pub enum Option<T> { Some(T), None } use self::Option::{Some, None};\".parse().unwrap() }\n");
        let file = format!("cases/layout/{path}");
        let old = fs::read_to_string(repo.0.join(&file)).unwrap();
        repo.write(
            &file,
            &format!("use injector::Debug; #[derive(Debug)] struct Other;\n{old}"),
        );
        let generated = path.strip_suffix(".rs").unwrap();
        repo.write("cases/layout/lib.rs", &format!("#![allow(dead_code, unused_imports)]\nmod source; mod destination;\nfn verify_generated_identity() {{ let _: {generated}::Option<u8> = {generated}::Option::Some(1); }}\n"));
        let check = std::process::Command::new("cargo")
            .args(["check", "--workspace", "--offline", "--quiet"])
            .current_dir(repo.0.join("cases/layout"))
            .output()
            .unwrap();
        assert!(
            check.status.success(),
            "{}",
            String::from_utf8_lossy(&check.stderr)
        );
        let result = run(
            &repo,
            enabled(request(&repo, json!([entry(&repo, selected, existing())]))),
        );
        withheld(&result);
        for name in ["Option", "Some", "None", "Debug"] {
            assert_eq!(proof_count(&result, name), 0, "{path}: {result}");
        }
    }
}

#[test]
fn direct_custom_derive_emitting_option_refuses_at_the_exact_attribute() {
    let selected = "fn selected(_: Option<u8>) -> Option<u8> { if true { Some(1) } else { None } }";
    for path in ["source.rs", "destination.rs"] {
        for (attribute, derive_name) in [
            ("#[derive(Inject)]", "Inject"),
            ("#[derive(Debug, Inject)]", "Inject"),
            ("#[derive(injector::Inject)]", "injector::Inject"),
        ] {
            let repo = fixture(selected);
            repo.write("cases/layout/Cargo.toml", "[package]\nname = \"direct-derive-binding-fixture\"\nversion = \"0.0.0\"\nedition = \"2024\"\n[lib]\npath = \"lib.rs\"\n[dependencies]\ninjector = { path = \"injector\" }\n[workspace]\nmembers = [\"injector\"]\n");
            repo.write("cases/layout/injector/Cargo.toml", "[package]\nname = \"injector\"\nversion = \"0.0.0\"\nedition = \"2024\"\n[lib]\nproc-macro = true\n");
            repo.write("cases/layout/injector/src/lib.rs", "extern crate proc_macro;\n#[proc_macro_derive(Inject)]\npub fn inject(_: proc_macro::TokenStream) -> proc_macro::TokenStream { \"pub enum Option<T> { Some(T), None } use self::Option::{Some, None};\".parse().unwrap() }\n");
            let file = format!("cases/layout/{path}");
            let old = fs::read_to_string(repo.0.join(&file)).unwrap();
            let source = format!(
                "// original é coordinates\nuse injector::Inject;\n{attribute}\nstruct Other;\n{old}"
            );
            repo.write(&file, &source);
            let generated = path.strip_suffix(".rs").unwrap();
            repo.write("cases/layout/lib.rs", &format!("#![allow(dead_code, unused_imports)]\nmod source; mod destination;\nfn verify_generated_identity() {{ let _: {generated}::Option<u8> = {generated}::Option::Some(1); }}\n"));
            let check = std::process::Command::new("cargo")
                .args(["check", "--workspace", "--offline", "--quiet"])
                .current_dir(repo.0.join("cases/layout"))
                .output()
                .unwrap();
            assert!(
                check.status.success(),
                "{path}: {attribute}: {}",
                String::from_utf8_lossy(&check.stderr)
            );
            for move_owner in [false, true] {
                if move_owner && path != "source.rs" {
                    continue;
                }
                let mut moves = vec![entry(&repo, selected, existing())];
                if move_owner {
                    moves.push(entry(&repo, "struct Other;", existing()));
                }
                let args = request(&repo, json!(moves));
                let default = run(&repo, args.clone());
                let mut off = args.clone();
                off["assume_standard_prelude"] = json!(false);
                assert_eq!(
                    serde_json::to_vec(&default).unwrap(),
                    serde_json::to_vec(&run(&repo, off)).unwrap()
                );
                let result = run(&repo, enabled(args));
                withheld(&result);
                for name in ["Option", "Some", "None", "Debug"] {
                    assert_eq!(
                        proof_count(&result, name),
                        0,
                        "{path}: {attribute}: {result}"
                    );
                }
                let attribute_start = source.find(attribute).unwrap();
                for name in ["Option", "Some", "None"] {
                    let decisions: Vec<_> = result["plan"]["decisions"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .filter(|d| d["anchors"][0]["expected_text"] == name)
                        .collect();
                    assert!(!decisions.is_empty(), "{name}: {result}");
                    for decision in decisions {
                        assert!(
                            decision["refusal_basis"]
                                .as_array()
                                .unwrap()
                                .iter()
                                .any(|basis| {
                                    basis["class"] == "conditional_context"
                                        && basis.get("name").is_none()
                                        && basis["anchor"]["path"] == file
                                        && basis["anchor"]["range"]["start_byte"] == attribute_start
                                        && basis["anchor"]["range"]["end_byte"]
                                            == attribute_start + attribute.len()
                                }),
                            "{decision}"
                        );
                    }
                }
                if move_owner {
                    let name_start = attribute_start + attribute.find(derive_name).unwrap();
                    assert!(
                        result["plan"]["decisions"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .any(|decision| {
                                decision["refusal_basis"].as_array().is_some_and(|bases| {
                                    bases.iter().any(|basis| {
                                        basis["class"] == "derive_veto"
                                            && basis["anchor"]["path"] == file
                                            && basis["anchor"]["range"]["start_byte"] == name_start
                                            && basis["anchor"]["range"]["end_byte"]
                                                == name_start + derive_name.len()
                                    })
                                })
                            }),
                        "{result}"
                    );
                }
            }
        }
    }
}

#[test]
fn cfg_attr_custom_derive_emitting_option_refuses_at_the_exact_attribute() {
    let selected = "fn selected(_: Option<u8>) -> Option<u8> { if true { Some(1) } else { None } }";
    for path in ["source.rs", "destination.rs"] {
        for attribute in [
            "#[cfg_attr(feature = \"inject\", derive(Inject))]",
            "#[cfg_attr(feature = \"inject\", r#derive(Inject))]",
            "#[r#cfg_attr(feature = \"inject\", derive(Inject))]",
            "#[cfg_attr(feature = \"inject\", r#cfg_attr(feature = \"inject\", derive(Inject)))]",
            "#[cfg_attr(feature = \"inject\", doc(hidden), cfg_attr(feature = \"inject\", derive(Inject)))]",
        ] {
            let repo = fixture(selected);
            repo.write("cases/layout/Cargo.toml", "[package]\nname = \"conditional-derive-binding-fixture\"\nversion = \"0.0.0\"\nedition = \"2024\"\n[lib]\npath = \"lib.rs\"\n[features]\ninject = []\n[dependencies]\ninjector = { path = \"injector\" }\n[workspace]\nmembers = [\"injector\"]\n");
            repo.write("cases/layout/injector/Cargo.toml", "[package]\nname = \"injector\"\nversion = \"0.0.0\"\nedition = \"2024\"\n[lib]\nproc-macro = true\n");
            repo.write("cases/layout/injector/src/lib.rs", "extern crate proc_macro;\n#[proc_macro_derive(Inject)]\npub fn inject(_: proc_macro::TokenStream) -> proc_macro::TokenStream { \"pub enum Option<T> { Some(T), None } use self::Option::{Some, None};\".parse().unwrap() }\n");
            let file = format!("cases/layout/{path}");
            let old = fs::read_to_string(repo.0.join(&file)).unwrap();
            let source = format!(
                "use injector::Inject;\n#[derive(Debug)]\n{attribute}\nstruct Other;\n{old}"
            );
            repo.write(&file, &source);
            let generated = path.strip_suffix(".rs").unwrap();
            repo.write("cases/layout/lib.rs", &format!("#![allow(dead_code, unused_imports)]\nmod source; mod destination;\nfn verify_generated_identity() {{ let _: {generated}::Option<u8> = {generated}::Option::Some(1); }}\n"));
            let check = std::process::Command::new("cargo")
                .args([
                    "check",
                    "--workspace",
                    "--offline",
                    "--quiet",
                    "--features",
                    "inject",
                ])
                .current_dir(repo.0.join("cases/layout"))
                .output()
                .unwrap();
            assert!(
                check.status.success(),
                "{}",
                String::from_utf8_lossy(&check.stderr)
            );
            let args = request(&repo, json!([entry(&repo, selected, existing())]));
            let default = run(&repo, args.clone());
            let mut off = args.clone();
            off["assume_standard_prelude"] = json!(false);
            assert_eq!(
                serde_json::to_vec(&default).unwrap(),
                serde_json::to_vec(&run(&repo, off)).unwrap()
            );
            for configured in [false, true] {
                let mut args = enabled(args.clone());
                if configured {
                    args["semantic_configuration"] = json!({"crates":[{"name":"fixture","root_file":"cases/layout/lib.rs","edition":"2024","features":["inject"],"cfg":[],"dependencies":[]}]});
                }
                let result = run(&repo, args);
                withheld(&result);
                assert_eq!(result["plan"]["integrity"]["semantic"], "not_performed");
                for name in ["Option", "Some", "None"] {
                    assert_eq!(
                        proof_count(&result, name),
                        0,
                        "{path}: {attribute}: {result}"
                    );
                    let decisions: Vec<_> = result["plan"]["decisions"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .filter(|d| d["anchors"][0]["expected_text"] == name)
                        .collect();
                    assert!(!decisions.is_empty(), "{name}: {result}");
                    for decision in decisions {
                        assert!(
                            decision["refusal_basis"]
                                .as_array()
                                .unwrap()
                                .iter()
                                .any(|basis| {
                                    basis["class"] == "conditional_context"
                                        && basis["name"].is_null()
                                        && basis["anchor"]["path"] == file
                                        && basis["anchor"]["range"]["start_byte"]
                                            == source.find(attribute).unwrap()
                                        && basis["anchor"]["range"]["end_byte"]
                                            == source.find(attribute).unwrap() + attribute.len()
                                }),
                            "{decision}"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn proc_attribute_companions_that_emit_module_items_refuse_at_the_exact_outer_witness() {
    let selected = "fn selected(_: Option<u8>) -> Option<u8> { if true { Some(1) } else { None } }";
    for path in ["source.rs", "destination.rs"] {
        for (attribute, feature) in [
            ("#[injector::inject]", None),
            (
                "#[cfg_attr(feature = \"inject\", injector::inject)]",
                Some("inject"),
            ),
        ] {
            let repo = fixture(selected);
            repo.write("cases/layout/Cargo.toml", "[package]\nname = \"proc-attribute-binding-fixture\"\nversion = \"0.0.0\"\nedition = \"2024\"\n[lib]\npath = \"lib.rs\"\n[features]\ninject = []\n[dependencies]\ninjector = { path = \"injector\" }\n[workspace]\nmembers = [\"injector\"]\n");
            repo.write("cases/layout/injector/Cargo.toml", "[package]\nname = \"injector\"\nversion = \"0.0.0\"\nedition = \"2024\"\n[lib]\nproc-macro = true\n");
            repo.write("cases/layout/injector/src/lib.rs", "extern crate proc_macro;\n#[proc_macro_attribute]\npub fn inject(_: proc_macro::TokenStream, item: proc_macro::TokenStream) -> proc_macro::TokenStream { let mut out = item; out.extend(\"pub enum Option<T> { Some(T), None } use self::Option::{Some, None};\".parse::<proc_macro::TokenStream>().unwrap()); out }\n");
            let file = format!("cases/layout/{path}");
            let old = fs::read_to_string(repo.0.join(&file)).unwrap();
            let source = format!("#[derive(Debug)]\n{attribute}\nstruct Other;\n{old}");
            repo.write(&file, &source);
            let generated = path.strip_suffix(".rs").unwrap();
            repo.write("cases/layout/lib.rs", &format!("#![allow(dead_code, unused_imports)]\nmod source; mod destination;\nfn verify_generated_identity() {{ let _: {generated}::Option<u8> = {generated}::Option::Some(1); }}\n"));
            let mut command = std::process::Command::new("cargo");
            command.args(["check", "--workspace", "--offline", "--quiet"]);
            if let Some(feature) = feature {
                command.args(["--features", feature]);
            }
            let check = command
                .current_dir(repo.0.join("cases/layout"))
                .output()
                .unwrap();
            assert!(
                check.status.success(),
                "{}",
                String::from_utf8_lossy(&check.stderr)
            );
            let result = run(
                &repo,
                enabled(request(&repo, json!([entry(&repo, selected, existing())]))),
            );
            withheld(&result);
            assert_eq!(result["plan"]["integrity"]["semantic"], "not_performed");
            for name in ["Option", "Some", "None"] {
                assert_eq!(
                    proof_count(&result, name),
                    0,
                    "{path}: {attribute}: {result}"
                );
                let decisions: Vec<_> = result["plan"]["decisions"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter(|d| d["anchors"][0]["expected_text"] == name)
                    .collect();
                assert!(!decisions.is_empty(), "{name}: {result}");
                for decision in decisions {
                    assert!(
                        decision["refusal_basis"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .any(|basis| {
                                basis["class"] == "conditional_context"
                                    && basis["name"].is_null()
                                    && basis["anchor"]["path"] == file
                                    && basis["anchor"]["range"]["start_byte"]
                                        == source.find(attribute).unwrap()
                                    && basis["anchor"]["range"]["end_byte"]
                                        == source.find(attribute).unwrap() + attribute.len()
                            }),
                        "{decision}"
                    );
                }
            }
        }
    }
}

#[test]
fn cfg_attr_builtin_spelling_with_a_custom_provider_keeps_the_wrapper_witness() {
    let selected = "fn selected(_: Option<u8>) -> Option<u8> { if true { Some(1) } else { None } }";
    let attribute = "#[cfg_attr(feature = \"inject\", derive(Debug))]";
    for path in ["source.rs", "destination.rs"] {
        let repo = fixture(selected);
        let file = format!("cases/layout/{path}");
        let old = fs::read_to_string(repo.0.join(&file)).unwrap();
        let source = format!("use custom::Debug;\n{attribute}\nstruct Other;\n{old}");
        repo.write(&file, &source);
        let result = run(
            &repo,
            enabled(request(&repo, json!([entry(&repo, selected, existing())]))),
        );
        withheld(&result);
        for name in ["Option", "Some", "None", "Debug"] {
            assert_eq!(proof_count(&result, name), 0, "{result}");
        }
        for decision in result["plan"]["decisions"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|d| {
                ["Option", "Some", "None"]
                    .iter()
                    .any(|name| d["anchors"][0]["expected_text"] == *name)
            })
        {
            assert!(
                decision["refusal_basis"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|basis| {
                        basis["class"] == "conditional_context"
                            && basis["name"].is_null()
                            && basis["anchor"]["path"] == file
                            && basis["anchor"]["range"]["start_byte"]
                                == source.find(attribute).unwrap()
                            && basis["anchor"]["range"]["end_byte"]
                                == source.find(attribute).unwrap() + attribute.len()
                    }),
                "{decision}"
            );
        }
    }
}

#[test]
fn cfg_attr_custom_derives_known_off_do_not_veto_unrelated_prelude_types() {
    let selected = "fn selected(_: Option<std::string::String>) {}";
    for path in ["source.rs", "destination.rs"] {
        for attribute in [
            "#[cfg_attr(not(feature = \"inject\"), derive(Inject))]",
            "#[cfg_attr(feature = \"unknown\", cfg_attr(false, derive(Inject)))]",
        ] {
            let repo = fixture(selected);
            let file = format!("cases/layout/{path}");
            let old = fs::read_to_string(repo.0.join(&file)).unwrap();
            repo.write(
                &file,
                &format!("#[derive(Debug)]\n{attribute}\nstruct Other;\n{old}"),
            );
            let mut args = enabled(request(&repo, json!([entry(&repo, selected, existing())])));
            args["semantic_configuration"] = json!({"crates":[{"name":"fixture","root_file":"cases/layout/lib.rs","edition":"2024","features":["inject"],"cfg":[],"dependencies":[]}]});
            let result = run(&repo, args);
            assert_eq!(
                result["plan"]["applicable"], true,
                "{path}: {attribute}: {result}"
            );
            assert_eq!(proof_count(&result, "Option"), 1, "{result}");
            assert_eq!(proof_count(&result, "std::string::String"), 1, "{result}");
            assert_eq!(result["plan"]["integrity"]["semantic"], "not_performed");
        }
    }
}

#[test]
fn qualified_prelude_refuses_shadowed_builtin_derive_providers() {
    let selected = "fn selected() -> std::result::Result<u8, u8> { loop {} }";
    for path in ["source.rs", "destination.rs"] {
        let repo = fixture(selected);
        let file = format!("cases/layout/{path}");
        let old = fs::read_to_string(repo.0.join(&file)).unwrap();
        repo.write(
            &file,
            &format!("use unknown as Debug; #[derive(Debug)] struct Other;\n{old}"),
        );
        let result = run(
            &repo,
            enabled(request(&repo, json!([entry(&repo, selected, existing())]))),
        );
        withheld(&result);
        assert_eq!(proof_count(&result, "std::result::Result"), 0, "{result}");
    }
}

#[test]
fn unrelated_derive_bases_do_not_attach_to_refused_prelude_types() {
    let selected = "fn selected() -> Option<u8> { loop {} }";
    for (gate, class) in [
        ("struct Option;", "shadow"),
        ("introduce!();", "chain_macro_statement"),
        ("#![no_implicit_prelude]", "prelude_disabled"),
    ] {
        let repo = fixture(&format!(
            "#[derive(Debug, Clone)] #[serde(default)] struct Other;\n{selected}"
        ));
        repo.write("cases/layout/destination.rs", gate);
        let result = run(
            &repo,
            enabled(request(&repo, json!([entry(&repo, selected, existing())]))),
        );
        withheld(&result);
        assert_eq!(proof_count(&result, "Option"), 0, "{result}");
        let decision = result["plan"]["decisions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|d| d["anchors"][0]["expected_text"] == "Option")
            .unwrap();
        let bases = decision["refusal_basis"].as_array().unwrap();
        assert!(bases.iter().any(|b| b["class"] == class), "{decision}");
        assert!(
            bases.iter().all(|b| b["class"] != "derive_veto"
                && b["name"] != "Debug"
                && b["name"] != "Clone"),
            "{decision}"
        );
    }
}

#[test]
fn attributable_helper_companions_do_not_veto_the_owning_declaration_types() {
    let selected = "struct Record { value: Option<String> }";
    for (declares, companion) in [
        (
            "Debug, Clone, JsonSchema",
            "#[schemars(crate = \"rmcp::schemars\")]",
        ),
        (
            "Debug, Clone, Serialize",
            "#[serde(rename_all = \"snake_case\")]",
        ),
        ("Debug, Clone, Deserialize", "#[serde(default)]"),
    ] {
        let original = format!("#[derive({declares})]\n{companion}\n{selected}");
        let repo = fixture(&original);
        let result = run(
            &repo,
            enabled(request(&repo, json!([entry(&repo, selected, existing())]))),
        );
        withheld(&result);
        assert_eq!(proof_count(&result, "Option"), 1, "{result}");
        assert_eq!(proof_count(&result, "String"), 1, "{result}");
        assert_eq!(proof_count(&result, "Debug"), 0, "{result}");
        assert_eq!(proof_count(&result, "Clone"), 0, "{result}");
        // The registered helper conditions the built-in derive identity only.
        assert!(
            result["plan"]["decisions"]
                .as_array()
                .unwrap()
                .iter()
                .any(|d| {
                    d["reason"] == "conditional_or_inherited_context"
                        && d["refusal_basis"].as_array().is_some_and(|bases| {
                            bases.iter().any(|b| {
                                b["class"] == "conditional_context" && b["name"] == "Debug"
                            })
                        })
                }),
            "{result}"
        );
    }
}

#[test]
fn unclassified_companion_on_the_owning_declaration_vetoes_its_types() {
    let selected = "struct Record { value: Option<String> }";
    let companion = "#[unknown_provider]";
    let original = format!("#[derive(Debug, Clone)]\n{companion}\n{selected}");
    let repo = fixture(&original);
    let companion_start = original.find(companion).unwrap();
    let result = run(
        &repo,
        enabled(request(&repo, json!([entry(&repo, selected, existing())]))),
    );
    withheld(&result);
    assert_eq!(proof_count(&result, "Option"), 0, "{result}");
    assert_eq!(proof_count(&result, "String"), 0, "{result}");
    assert_eq!(proof_count(&result, "Debug"), 0, "{result}");
    assert_eq!(proof_count(&result, "Clone"), 0, "{result}");
    // The owning declaration still records its exact written-attribute witness.
    assert!(
        result["plan"]["decisions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["refusal_basis"].as_array().is_some_and(|bases| {
                bases.iter().any(|b| {
                    b["class"] == "conditional_context"
                        && b["name"].is_null()
                        && b["anchor"]["path"] == "cases/layout/source.rs"
                        && b["anchor"]["range"]["start_byte"] == companion_start
                        && b["anchor"]["range"]["end_byte"] == companion_start + companion.len()
                })
            })),
        "{result}"
    );
}

#[test]
fn qualified_prelude_signatures_are_opt_in_lossless_and_nonsemantic() {
    let selected = "fn selected(input: std::vec::Vec<std::string::String>) -> std::result::Result<std::option::Option<std::boxed::Box<u8>>, u8> { loop {} }";
    let repo = fixture(selected);
    for destination in [existing(), new("cases/layout/qualified.rs")] {
        let args = request(&repo, json!([entry(&repo, selected, destination.clone())]));
        let default = run(&repo, args.clone());
        withheld(&default);
        let mut off = args.clone();
        off["assume_standard_prelude"] = json!(false);
        assert_eq!(default, run(&repo, off));
        let result = run(&repo, enabled(args));
        assert_eq!(result["plan"]["applicable"], true, "{result}");
        assert_eq!(result["plan"]["integrity"]["semantic"], "not_performed");
        assert_eq!(result["coverage"]["standard_prelude"], 5);
        let proofs = result["plan"]["binding_proofs"].as_array().unwrap();
        assert_eq!(proofs.len(), 5);
        for path in [
            "std::option::Option",
            "std::result::Result",
            "std::boxed::Box",
            "std::vec::Vec",
            "std::string::String",
        ] {
            assert_eq!(proof_count(&result, path), 1, "{result}");
            let proof = proofs.iter().find(|p| p["standard_path"] == path).unwrap();
            assert_eq!(proof["class"], "standard_prelude");
            let anchor = &proof["anchor"];
            let start = anchor["range"]["start_byte"].as_u64().unwrap() as usize;
            let end = anchor["range"]["end_byte"].as_u64().unwrap() as usize;
            assert_eq!(&selected[start..end], path);
            assert_eq!(anchor["expected_text"], path);
            let basis = proof["basis"].as_str().unwrap();
            assert!(basis.contains("caller enabled assume_standard_prelude"));
            assert!(basis.contains(&format!("fully-qualified type-position {path}")));
            assert!(basis.contains("no macro hygiene claim"));
            assert!(basis.contains("semantic checking not performed"));
        }
        assert!(
            result["plan"]["rewrites"]
                .as_array()
                .unwrap()
                .iter()
                .all(|r| { !r["kind"].as_str().unwrap().starts_with("import") })
        );
        let copy = apply(&repo, &result);
        assert!(
            fs::read_to_string(copy.0.join(destination["path"].as_str().unwrap()))
                .unwrap()
                .contains(selected)
        );
    }
}

#[test]
fn qualified_prelude_ignores_terminal_imports_and_declarations() {
    let selected = "fn selected() -> std::result::Result<u8, u8> { loop {} }";
    for path in ["source.rs", "destination.rs"] {
        for prefix in [
            "use unknown as Result;\n",
            "use std::result::Result;\n",
            "struct Result;\n",
        ] {
            let repo = fixture(selected);
            let file = format!("cases/layout/{path}");
            let old = fs::read_to_string(repo.0.join(&file)).unwrap();
            repo.write(&file, &format!("{prefix}{old}"));
            let result = run(
                &repo,
                enabled(request(&repo, json!([entry(&repo, selected, existing())]))),
            );
            assert_eq!(
                result["plan"]["applicable"], true,
                "{path}: {prefix}: {result}"
            );
            assert_eq!(proof_count(&result, "std::result::Result"), 1, "{result}");
            assert_eq!(result["plan"]["integrity"]["semantic"], "not_performed");
            apply(&repo, &result);
        }
    }
    for selected in [
        "fn selected() { use unknown as Result; let value: std::result::Result<u8, u8>; }",
        "fn selected() { use std::result::Result; let value: std::result::Result<u8, u8>; }",
    ] {
        let repo = fixture(selected);
        let result = run(
            &repo,
            enabled(request(&repo, json!([entry(&repo, selected, existing())]))),
        );
        assert_eq!(proof_count(&result, "std::result::Result"), 1, "{result}");
    }
}

#[test]
fn qualified_prelude_extern_crates_audit_effective_bindings() {
    let selected = "fn selected() -> std::result::Result<u8, u8> { loop {} }";
    for path in ["source.rs", "destination.rs", "lib.rs"] {
        for (declaration, allowed) in [
            ("extern crate std as something;", true),
            ("extern crate external as std;", false),
            ("extern crate std;", false),
        ] {
            let repo = fixture(selected);
            let file = format!("cases/layout/{path}");
            let old = fs::read_to_string(repo.0.join(&file)).unwrap();
            let source = format!("{declaration}\n{old}");
            repo.write(&file, &source);
            let result = run(
                &repo,
                enabled(request(&repo, json!([entry(&repo, selected, existing())]))),
            );
            assert_eq!(
                proof_count(&result, "std::result::Result"),
                usize::from(allowed),
                "{path}: {declaration}: {result}"
            );
            assert_eq!(result["plan"]["applicable"], allowed, "{result}");
            assert_eq!(result["plan"]["integrity"]["semantic"], "not_performed");
            if allowed {
                assert_eq!(
                    result["plan"]["binding_proofs"][0]["class"],
                    "standard_prelude"
                );
                apply(&repo, &result);
            } else {
                withheld(&result);
                assert!(
                    result["plan"]["decisions"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|decision| {
                            decision["refusal_basis"].as_array().is_some_and(|bases| {
                                bases.iter().any(|basis| {
                                    basis["class"] == "shadow"
                                        && basis["name"] == "std"
                                        && basis["anchor"]["path"] == file
                                        && basis["anchor"]["range"]["start_byte"]
                                            == declaration.rfind("std").unwrap()
                                        && basis["anchor"]["range"]["end_byte"]
                                            == declaration.len() - 1
                                })
                            })
                        }),
                    "{result}"
                );
            }
        }
    }
}

#[test]
fn qualified_prelude_keeps_both_overlay_root_and_context_vetoes() {
    let selected = "fn selected() -> std::result::Result<u8, u8> { loop {} }";
    for path in ["source.rs", "destination.rs"] {
        for prefix in [
            "mod std { pub mod result { pub struct Result; } }\n",
            "extern crate external as std;\n",
            "use unknown as std;\n",
            "use unknown::*;\n",
            "#![no_implicit_prelude]\n",
            "introduce!();\n",
        ] {
            let repo = fixture(selected);
            let file = format!("cases/layout/{path}");
            let old = fs::read_to_string(repo.0.join(&file)).unwrap();
            repo.write(&file, &format!("{prefix}{old}"));
            let mut args = enabled(request(&repo, json!([entry(&repo, selected, existing())])));
            let result = run(&repo, args.clone());
            withheld(&result);
            assert_eq!(
                proof_count(&result, "std::result::Result"),
                0,
                "{path}: {prefix}: {result}"
            );
            args["assume_standard_prelude"] = json!(false);
            assert_eq!(refusal_behavior(result), refusal_behavior(run(&repo, args)));
        }
    }
    for selected in [
        "fn selected() { mod std { pub mod result { pub struct Result; } } let value: std::result::Result<u8, u8>; }",
        "fn selected() { let value: std::result::Result<u8, u8>; mod std { pub mod result { pub struct Result; } } }",
        "fn selected() { mod std {} { let value: std::result::Result<u8, u8>; } }",
        "fn selected() { extern crate external as std; let value: std::result::Result<u8, u8>; }",
        "fn selected() { let value: std::result::Result<u8, u8>; extern crate external as std; }",
        "fn selected() { mod enclosing { mod std {} mod inner { struct Record { value: std::result::Result<u8, u8> } } } }",
        "fn selected() { introduce!(); let value: std::result::Result<u8, u8>; }",
        "fn selected() { let value: std::result::Result<u8, u8>; introduce!(); }",
        "fn selected() { let value: std::result::Result<u8, u8>; #[introduce] struct Other; }",
    ] {
        let repo = fixture(selected);
        let result = run(
            &repo,
            enabled(request(&repo, json!([entry(&repo, selected, existing())]))),
        );
        withheld(&result);
        assert_eq!(proof_count(&result, "std::result::Result"), 0, "{result}");
    }
}

#[test]
fn qualified_prelude_root_scope_chain_and_disjoint_scopes() {
    let selected = "fn selected() -> std::result::Result<u8, u8> { loop {} }";
    for root in ["mod std {}", "extern crate external as std;"] {
        let repo = fixture(selected);
        repo.write(
            "cases/layout/lib.rs",
            &format!("{root}\nmod source;\nmod destination;\n"),
        );
        let result = run(
            &repo,
            enabled(request(&repo, json!([entry(&repo, selected, existing())]))),
        );
        withheld(&result);
        assert_eq!(proof_count(&result, "std::result::Result"), 0, "{result}");
        assert!(
            result["plan"]["decisions"]
                .as_array()
                .unwrap()
                .iter()
                .any(|decision| {
                    decision["refusal_basis"].as_array().is_some_and(|bases| {
                        bases.iter().any(|basis| {
                            basis["class"] == "shadow"
                                && basis["name"] == "std"
                                && basis["anchor"]["path"] == "cases/layout/lib.rs"
                        })
                    })
                }),
            "{result}"
        );
    }
    for selected in [
        "fn selected() -> std::result::Result<u8, u8> { mod std {} loop {} }",
        "fn selected() { { mod std {} } let value: std::result::Result<u8, u8>; }",
        "fn selected() { mod enclosing { struct Record { value: std::result::Result<u8, u8> } mod sibling { mod std {} } } }",
    ] {
        let repo = fixture(selected);
        let result = run(
            &repo,
            enabled(request(&repo, json!([entry(&repo, selected, existing())]))),
        );
        assert_eq!(proof_count(&result, "std::result::Result"), 1, "{result}");
    }
}

#[test]
fn qualified_prelude_final_batch_terminal_arrivals_and_planned_imports_do_not_veto() {
    let selected = "fn selected() -> std::result::Result<u8, u8> { loop {} }";
    for other in ["struct Result;", "struct Other { value: Result<u8, u8> }"] {
        let repo = fixture(selected);
        repo.write(
            "cases/layout/other.rs",
            &format!("use std::result::Result;\n{other}\n"),
        );
        repo.write(
            "cases/layout/lib.rs",
            "mod source;\nmod destination;\nmod other;\n",
        );
        let result = run(
            &repo,
            enabled(request(
                &repo,
                json!([
                    entry(&repo, selected, existing()),
                    {"item":anchor(&repo, "cases/layout/other.rs", other), "destination":existing()},
                ]),
            )),
        );
        assert_eq!(proof_count(&result, "std::result::Result"), 1, "{result}");
        assert_eq!(result["plan"]["integrity"]["semantic"], "not_performed");
    }
}

#[test]
fn qualified_prelude_does_not_admit_other_paths_or_value_positions() {
    for selected in [
        "fn selected() -> std::collections::HashMap<u8, u8> { loop {} }",
        "fn selected() -> core::result::Result<u8, u8> { loop {} }",
        "fn selected() -> result::Result<u8, u8> { loop {} }",
        "fn selected() -> std::result::Other<u8, u8> { loop {} }",
        "fn selected() -> std::Result<u8, u8> { loop {} }",
        "fn selected() -> crate::std::result::Result<u8, u8> { loop {} }",
        "fn selected() -> ::std::result::Result<u8, u8> { loop {} }",
        "fn selected() -> Other<u8, u8> { loop {} }",
        "fn selected() { let value = std::vec::Vec::new(); }",
        "fn selected() { let value = std::option::Option::Some(1); }",
        "fn selected() { let value = std::string::String; }",
    ] {
        let repo = fixture(selected);
        let mut args = enabled(request(&repo, json!([entry(&repo, selected, existing())])));
        let result = run(&repo, args.clone());
        withheld(&result);
        assert!(
            result["coverage"].get("standard_prelude").is_none(),
            "{result}"
        );
        args["assume_standard_prelude"] = json!(false);
        assert_eq!(run(&repo, args), result, "{selected}");
    }
}

#[test]
fn prelude_shadow_refusal_at_source_destination_and_parent() {
    let selected = "struct Record { option: Option<u8> }";
    for (path, prefix) in [
        ("source.rs", "use unknown::*;\n"),
        ("destination.rs", "use unknown::*;\n"),
        ("destination.rs", "use unknown as Option;\n"),
        ("destination.rs", "use std::option::Option;\n"),
        ("destination.rs", "struct Option;\n"),
        ("destination.rs", "extern crate external as Option;\n"),
        (
            "source.rs",
            "#[cfg_attr(test, derive(Clone))] struct Option;\n",
        ),
        ("source.rs", "#![no_implicit_prelude]\n"),
        ("destination.rs", "#![no_implicit_prelude]\n"),
        ("lib.rs", "#![no_implicit_prelude]\n"),
        ("source.rs", "#![no_std]\n"),
        ("destination.rs", "#![no_core]\n"),
        ("destination.rs", "#[cfg(feature=\"x\")] struct Option;\n"),
        ("lib.rs", "#![prelude_import]\n"),
        ("lib.rs", "#[introduce] mod unrelated {}\n"),
        ("lib.rs", "#[cfg(] mod unrelated;\n"),
        ("source.rs", "introduce!();\n"),
        ("destination.rs", "introduce!();\n"),
        ("lib.rs", "introduce!();\n"),
        ("destination.rs", "fn broken( {\n"),
    ] {
        let repo = fixture(selected);
        let file = format!("cases/layout/{path}");
        let old = fs::read_to_string(repo.0.join(&file)).unwrap();
        repo.write(&file, &format!("{prefix}{old}"));
        let args = enabled(request(&repo, json!([entry(&repo, selected, existing())])));
        let on = run(&repo, args.clone());
        withheld(&on);
        assert_eq!(proof_count(&on, "Option"), 0, "{path}: {prefix}");
        assert!(
            has_reason(&on, "external_or_missing_binding")
                || has_reason(&on, "glob_binding_unproved")
                || has_reason(&on, "conditional_or_inherited_context")
                || has_reason(&on, "module_chain_failure"),
            "{on}"
        );
        let mut off = args;
        off["assume_standard_prelude"] = json!(false);
        assert_eq!(
            refusal_behavior(run(&repo, off)),
            refusal_behavior(on),
            "fallback changed refused response for {path}: {prefix}"
        );
    }
}

#[test]
fn prelude_child_module_shadows_do_not_escape_at_either_end() {
    let selected = "struct Record { option: Option<u8> }";
    for path in ["source.rs", "destination.rs", "lib.rs"] {
        for child in [
            "mod child { use unknown::*; }",
            "#[cfg(test)] mod tests { use super::*; }",
            "mod child { struct Option; }",
            "mod sibling { use std::option::Option; }",
        ] {
            let repo = fixture(selected);
            let file = format!("cases/layout/{path}");
            let old = fs::read_to_string(repo.0.join(&file)).unwrap();
            repo.write(&file, &format!("{old}\n{child}\n"));
            let result = run(
                &repo,
                enabled(request(&repo, json!([entry(&repo, selected, existing())]))),
            );
            assert_eq!(
                proof_count(&result, "Option"),
                1,
                "{path}: {child}: {result}"
            );
            assert_eq!(result["coverage"]["standard_prelude"], 1);
            assert_eq!(result["plan"]["applicable"], true, "{result}");
            apply(&repo, &result);
        }
    }
}

#[test]
fn prelude_inline_module_needs_use_only_their_enclosing_scopes() {
    for disjoint in [
        "use unknown::*;",
        "struct Option;",
        "use std::option::Option;",
    ] {
        let selected = format!(
            "fn selected() {{ mod enclosing {{ struct Record {{ option: Option<u8> }} mod sibling {{ {disjoint} }} }} }}"
        );
        let repo = fixture(&selected);
        let result = run(
            &repo,
            enabled(request(&repo, json!([entry(&repo, &selected, existing())]))),
        );
        assert_eq!(proof_count(&result, "Option"), 1, "{result}");
        assert_eq!(result["coverage"]["standard_prelude"], 1);
    }
    for shadow in [
        "use unknown::*;",
        "struct Option;",
        "use std::option::Option;",
    ] {
        for ancestor in [false, true] {
            let body = if ancestor {
                format!("{shadow} mod inner {{ struct Record {{ option: Option<u8> }} }}")
            } else {
                format!("{shadow} struct Record {{ option: Option<u8> }}")
            };
            let selected = format!("fn selected() {{ mod enclosing {{ {body} }} }}");
            let repo = fixture(&selected);
            let args = enabled(request(&repo, json!([entry(&repo, &selected, existing())])));
            let result = run(&repo, args.clone());
            assert_eq!(proof_count(&result, "Option"), 0, "{result}");
            let mut off = args;
            off["assume_standard_prelude"] = json!(false);
            assert_eq!(
                refusal_behavior(run(&repo, off)),
                refusal_behavior(result),
                "{selected}"
            );
        }
    }
}

#[test]
fn prelude_batch_child_scopes_stay_disjoint_and_own_contexts_refuse() {
    let record = "struct Record { option: Option<u8> }";
    let carrier = "fn carrier() { mod child { use std::option::Option; } }";
    let repo = fixture(&format!("{record}\n{carrier}\n"));
    for destination in [existing(), new("cases/layout/arrivals.rs")] {
        let result = run(
            &repo,
            enabled(request(
                &repo,
                json!([
                    entry(&repo, record, destination.clone()),
                    entry(&repo, carrier, destination),
                ]),
            )),
        );
        assert_eq!(proof_count(&result, "Option"), 1, "{result}");
        assert_eq!(result["plan"]["applicable"], true, "{result}");
        apply(&repo, &result);
    }
    for context in [
        "#[cfg(feature=\"x\")] mod enclosing { struct Record { option: Option<u8> } }",
        "mod enclosing { #![no_implicit_prelude] struct Record { option: Option<u8> } }",
        "mod enclosing { introduce!(); struct Record { option: Option<u8> } }",
        "#[cfg(feature=\"x\")] mod enclosing { mod inner { struct Record { option: Option<u8> } } }",
    ] {
        let selected = format!("fn selected() {{ {context} }}");
        let repo = fixture(&selected);
        let args = enabled(request(&repo, json!([entry(&repo, &selected, existing())])));
        let result = run(&repo, args.clone());
        assert_eq!(proof_count(&result, "Option"), 0, "{result}");
        let mut off = args;
        off["assume_standard_prelude"] = json!(false);
        assert_eq!(
            refusal_behavior(run(&repo, off)),
            refusal_behavior(result),
            "{selected}"
        );
    }
}

#[test]
fn prelude_written_imports_and_declarations_keep_existing_repairs() {
    let selected = "struct Record { option: Option<u8> }";
    for prefix in [
        "use std::option::Option;\n",
        "struct Option;\n",
        "use unknown as Option;\n",
    ] {
        let repo = fixture(&format!("{prefix}{selected}\n"));
        let args = enabled(request(&repo, json!([entry(&repo, selected, existing())])));
        let on = run(&repo, args.clone());
        assert_eq!(proof_count(&on, "Option"), 0);
        assert_eq!(on["plan"]["applicable"], true, "{on}");
        assert!(
            on["plan"]["rewrites"]
                .as_array()
                .unwrap()
                .iter()
                .any(|r| r["kind"] == "import_insert")
        );
        let mut off = args;
        off["assume_standard_prelude"] = json!(false);
        assert_eq!(run(&repo, off), on);
        apply(&repo, &on);
    }
}

#[test]
fn prelude_generic_local_pattern_and_batch_shadows_never_become_assumptions() {
    for selected in [
        "fn selected<Option>(input: Option) -> Option { input }",
        "fn selected(Option: u8) { let value: Option<u8>; }",
        "fn selected() { let Option = 1; let value: Option<u8>; }",
        "fn selected() { let (Option, other) = (1, 2); let value: Option<u8>; }",
        "fn selected() { struct Option; let value: Option; }",
        "fn selected() { introduce!(); let value: Option<u8>; }",
    ] {
        let repo = fixture(selected);
        let args = enabled(request(&repo, json!([entry(&repo, selected, existing())])));
        let on = run(&repo, args.clone());
        assert_eq!(proof_count(&on, "Option"), 0, "{on}");
        let mut off = args;
        off["assume_standard_prelude"] = json!(false);
        assert_eq!(
            refusal_behavior(run(&repo, off)),
            refusal_behavior(on),
            "{selected}"
        );
    }
    let selected = "struct Record { option: Option<u8> }";
    let repo = fixture(selected);
    repo.write("cases/layout/other.rs", "struct Option;\n");
    repo.write(
        "cases/layout/lib.rs",
        "mod source;\nmod destination;\nmod other;\n",
    );
    let args = request(
        &repo,
        json!([
            entry(&repo, selected, existing()),
            {"item":anchor(&repo,"cases/layout/other.rs","struct Option;"),"destination":existing()},
        ]),
    );
    let result = run(&repo, enabled(args));
    withheld(&result);
    assert!(has_reason(&result, "external_or_missing_binding"));
    assert_eq!(proof_count(&result, "Option"), 0);
}

#[test]
fn prelude_planned_import_shadows_and_incomplete_chains_refuse() {
    let selected = "struct Record { option: Option<u8> }";
    let repo = fixture(selected);
    let other = "struct Other { option: Option<u8> }";
    repo.write(
        "cases/layout/other.rs",
        &format!("use std::option::Option;\n{other}\n"),
    );
    repo.write(
        "cases/layout/lib.rs",
        "mod source;\nmod destination;\nmod other;\n",
    );
    let result = run(
        &repo,
        enabled(request(
            &repo,
            json!([
                entry(&repo, selected, existing()),
                {"item":anchor(&repo,"cases/layout/other.rs",other),"destination":existing()},
            ]),
        )),
    );
    withheld(&result);
    assert_eq!(proof_count(&result, "Option"), 0);
    assert!(has_reason(&result, "external_or_missing_binding"));
    repo.write("cases/layout/lib.rs", "mod source;\n");
    let result = run(
        &repo,
        enabled(request(&repo, json!([entry(&repo, selected, existing())]))),
    );
    withheld(&result);
    assert_eq!(proof_count(&result, "Option"), 0);
    assert!(has_reason(&result, "module_chain_failure"));
}

#[test]
fn prelude_batch_parent_arrivals_and_new_module_names_do_not_leak() {
    let selected = "struct Record { option: Option<u8> }";
    let repo = fixture(selected);
    let result = run(
        &repo,
        enabled(request(
            &repo,
            json!([entry(&repo, selected, new("cases/layout/Option.rs")),]),
        )),
    );
    assert_eq!(result["plan"]["applicable"], true, "{result}");
    assert_eq!(proof_count(&result, "Option"), 1);
    apply(&repo, &result);
    repo.write("cases/layout/other.rs", "struct Option;\n");
    repo.write(
        "cases/layout/lib.rs",
        "mod source;\nmod destination;\nmod other;\n",
    );
    let result = run(
        &repo,
        enabled(request(
            &repo,
            json!([
                entry(&repo, selected, existing()),
                {"item":anchor(&repo,"cases/layout/other.rs","struct Option;"),
                 "destination":{"kind":"existing","path":"cases/layout/lib.rs"}},
            ]),
        )),
    );
    assert_eq!(result["plan"]["applicable"], true, "{result}");
    assert_eq!(proof_count(&result, "Option"), 1);
    apply(&repo, &result);
}

#[test]
fn prelude_does_not_bridge_constructors_associated_calls_methods_macros_or_custom_derives() {
    for (selected, prefix, reason, proofs) in [
        (
            "fn selected(input: Vec<u8>) -> usize { input.len() }",
            "",
            "member_or_constructor_unproved",
            1,
        ),
        (
            "fn selected() { let value = Vec::new(); }",
            "",
            "external_or_missing_binding",
            0,
        ),
        (
            "fn selected() { let value = Option(1); }",
            "",
            "external_or_missing_binding",
            0,
        ),
        (
            "fn selected() { let value = Box { field: 1 }; }",
            "",
            "external_or_missing_binding",
            0,
        ),
        (
            "fn selected(input: Option<u8>) { invoke!(); }",
            "",
            "macro_context_unexamined",
            1,
        ),
        (
            "struct Record { value: Option<u8> }",
            "#[derive(Args)]\n",
            "conditional_or_inherited_context",
            0,
        ),
    ] {
        let repo = fixture(&format!("{prefix}{selected}"));
        let result = run(
            &repo,
            enabled(request(&repo, json!([entry(&repo, selected, existing())]))),
        );
        withheld(&result);
        assert!(has_reason(&result, reason), "{result}");
        assert_eq!(
            result["plan"]["binding_proofs"]
                .as_array()
                .map(Vec::len)
                .unwrap_or(0),
            proofs,
            "{result}"
        );
    }
}

#[test]
fn prelude_proof_response_limit_withholds_artifacts_and_discloses_omissions() {
    let fields = (0..100)
        .map(|i| format!("field_{i}: Option<String>"))
        .collect::<Vec<_>>()
        .join(",");
    let selected = format!("struct Record {{ {fields} }}");
    let repo = fixture(&selected);
    let mut args = enabled(request(&repo, json!([entry(&repo, &selected, existing())])));
    args["limits"]["response_bytes"] = json!(65_536);
    let result = run(&repo, args);
    withheld(&result);
    assert_eq!(result["status"], "partial");
    assert_eq!(result["coverage"]["standard_prelude"], 200);
    assert_eq!(result["counts"]["omissions"]["binding_proofs"], 200);
}

#[test]
fn prelude_stdio_flag_and_proof_schema_are_additive_and_move_only() {
    let repo = fixture("struct Record { value: Option<u8> }");
    let mut client = stdio_client::Client::new();
    let tools = client.rpc("tools/list", json!({}));
    let tools = tools["tools"].as_array().unwrap();
    let move_tool = tools.iter().find(|t| t["name"] == "move_item").unwrap();
    assert_eq!(
        move_tool["inputSchema"]["properties"]["assume_standard_prelude"]["type"],
        "boolean"
    );
    assert_eq!(
        move_tool["inputSchema"]["properties"]["assume_standard_prelude"]["default"],
        false
    );
    assert!(
        move_tool["description"]
            .as_str()
            .unwrap()
            .contains("class standard_prelude")
    );
    assert!(
        move_tool["description"]
            .as_str()
            .unwrap()
            .contains("class standard_builtin_derive")
    );
    let advice = tools.iter().find(|t| t["name"] == "suggest_split").unwrap();
    assert!(
        advice["inputSchema"]["properties"]
            .get("assume_standard_prelude")
            .is_none()
    );
    let result = client.call(
        "move_item",
        enabled(request(
            &repo,
            json!([entry(
                &repo,
                "struct Record { value: Option<u8> }",
                existing()
            ),]),
        )),
    );
    assert_eq!(result["coverage"]["standard_prelude"], 1);
    apply(&repo, &result);
    repo.write(
        "cases/layout/source.rs",
        "#[derive(Debug, Clone)]\nstruct Record { value: Option<u8> }\n",
    );
    let result = client.call(
        "move_item",
        enabled(request(
            &repo,
            json!([entry(
                &repo,
                "struct Record { value: Option<u8> }",
                existing()
            )]),
        )),
    );
    assert_eq!(result["coverage"]["standard_builtin_derive"], 2);
    assert_eq!(result["coverage"]["standard_prelude"], 1);
    apply(&repo, &result);
}
