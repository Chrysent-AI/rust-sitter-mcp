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
            "use unknown as Debug;\n#[derive(Debug)] struct Other;\n",
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
            1,
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
