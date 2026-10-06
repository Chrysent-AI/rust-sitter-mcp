use super::*;

fn acknowledged(value: &Value) -> Vec<&Value> {
    value["plan"]["decisions"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|d| d["reason"] == "test_consumer_acknowledged")
        .collect()
}

#[test]
fn test_consumer_acknowledgment_unlocks_const_fieldless_batch_and_patch() {
    let repo = fixture(
        "const LIMIT: usize = 4;\nstruct Marker;\n#[cfg(test)]\nmod tests {\n    #[test]\n    fn checks_limit() { assert!(super::LIMIT > 0); }\n}\n",
    );
    let args = request(
        &repo,
        json!([
            entry(
                &repo,
                "const LIMIT: usize = 4;",
                new("cases/layout/moved.rs")
            ),
            entry(&repo, "struct Marker;", new("cases/layout/moved.rs"))
        ]),
    );
    let off = run(&repo, args.clone());
    code(&off, "MACRO_DEPENDENCY");
    assert!(off["coverage"].get("test_consumers_acknowledged").is_none());
    assert!(off["plan"].get("test_consumer_disclosure").is_none());
    let mut on = args;
    on["acknowledge_test_consumers"] = json!(true);
    let value = run(&repo, on);
    assert_eq!(value["plan"]["applicable"], true, "{value}");
    assert_eq!(value["coverage"]["test_consumers_acknowledged"], 1);
    let risks = acknowledged(&value);
    assert_eq!(risks.len(), 1);
    let risk = risks[0];
    assert_eq!(risk["blocks_applicability"], false);
    assert_eq!(risk["resolution"], "risk_acknowledged");
    assert_eq!(risk["selected_choice"], "acknowledge_test_consumers");
    assert_eq!(
        risk["anchors"][0],
        anchor(&repo, "cases/layout/source.rs", "assert!(super::LIMIT > 0)")
    );
    assert_eq!(risk["item_ids"].as_array().unwrap().len(), 1);
    assert_eq!(risk["action"]["field"], "acknowledge_test_consumers");
    assert!(
        risk["next_action"]
            .as_str()
            .unwrap()
            .contains("caller's test run")
    );
    assert_eq!(
        value["plan"]["test_consumer_disclosure"],
        "1 consumers under cfg(test) acknowledged by the caller; behavior under test is validated by the caller's test run, not by this engine"
    );
    assert!(
        value["plan"]["decision_groups"]
            .as_array()
            .unwrap()
            .iter()
            .any(|g| {
                g["reason"] == "test_consumer_acknowledged"
                    && g["count"] == 1
                    && g["blocks_applicability"] == false
            })
    );
    let copy = apply(&repo, &value); // Disposable copy, git apply --check, then independent byte comparison.
    assert_eq!(
        fs::read_to_string(copy.0.join("cases/layout/moved.rs")).unwrap(),
        "const LIMIT: usize = 4;\nstruct Marker;\n"
    );
    assert!(
        fs::read_to_string(copy.0.join("cases/layout/source.rs"))
            .unwrap()
            .contains("assert!(super::LIMIT > 0)")
    );
}

#[test]
fn test_consumer_super_glob_alias_and_ordinary_path_risks_stay_visible() {
    for body in [
        "use super::*; #[test] fn check() { assert_eq!(LIMIT, 4); }",
        "use super::LIMIT as L; #[test] fn check() { assert_eq!(L, 4); }",
        "#[test] fn check() { let n = super::LIMIT; }",
        "use super::*; #[test] fn check() { let n = LIMIT; }",
    ] {
        let repo = fixture(&format!(
            "const LIMIT: usize = 4;\n#[ cfg ( test ) ]\nmod tests {{ {body} }}\n"
        ));
        let mut args = request(
            &repo,
            json!([entry(
                &repo,
                "const LIMIT: usize = 4;",
                new("cases/layout/moved.rs")
            )]),
        );
        withheld(&run(&repo, args.clone()));
        args["acknowledge_test_consumers"] = json!(true);
        let value = run(&repo, args);
        assert_eq!(value["plan"]["applicable"], true, "{body}: {value}");
        assert!(!acknowledged(&value).is_empty());
        assert_eq!(
            value["coverage"]["test_consumers_acknowledged"],
            acknowledged(&value).len()
        );
        for risk in acknowledged(&value) {
            assert_eq!(risk["blocks_applicability"], false);
            assert!(
                !risk["anchors"][0]["expected_text"]
                    .as_str()
                    .unwrap()
                    .is_empty()
            );
        }
        apply(&repo, &value);
    }
}

#[test]
fn test_consumer_acknowledgment_keeps_non_test_conditional_and_macro_blockers() {
    for remainder in [
        "#[cfg(feature = \"extra\")] mod consumers { fn check() { assert!(super::LIMIT > 0); } }",
        "#[cfg(unix)] mod consumers { fn check() { assert!(super::LIMIT > 0); } }",
        "fn check() { assert!(LIMIT > 0); }",
        "#[cfg(test)] fn check() { assert!(LIMIT > 0); }",
        "#[cfg(test)] #[cfg(unix)] mod tests { fn check() { assert!(super::LIMIT > 0); } }",
        "#[cfg(test)] mod tests { #[cfg(feature = \"extra\")] fn check() { assert!(super::LIMIT > 0); } }",
        "#[cfg(test)] mod tests { mod nested { fn check() { assert!(super::super::LIMIT > 0); } } }",
        "#[cfg(all(test))] mod tests { fn check() { assert!(super::LIMIT > 0); } }",
        "#[cfg(test)] mod tests { #![cfg(unix)] fn check() { assert!(super::LIMIT > 0); } }",
        "#[cfg(test)] mod tests { fn check() { assert!(external::LIMIT > 0); } }",
        "#[cfg(test)] mod tests { fn check() { assert!(super::LIMIT > external::LIMIT); } }",
    ] {
        let repo = fixture(&format!("const LIMIT: usize = 4;\n{remainder}\n"));
        let mut args = request(
            &repo,
            json!([entry(
                &repo,
                "const LIMIT: usize = 4;",
                new("cases/layout/moved.rs")
            )]),
        );
        args["acknowledge_test_consumers"] = json!(true);
        let value = run(&repo, args);
        code(&value, "MACRO_DEPENDENCY");
        assert!(acknowledged(&value).is_empty(), "{remainder}: {value}");
    }
}

#[test]
fn test_consumer_mixed_batch_preserves_risks_and_non_test_residue_under_caps() {
    let repo = fixture(
        "const LIMIT: usize = 4;\nstruct Marker;\n#[cfg(test)] mod tests { #[test] fn check() { assert!(super::LIMIT > 0); assert_eq!(super::LIMIT, 4); } }\nfn outside() { assert!(LIMIT > 0); }\n",
    );
    let mut args = request(
        &repo,
        json!([
            entry(
                &repo,
                "const LIMIT: usize = 4;",
                new("cases/layout/moved.rs")
            ),
            entry(&repo, "struct Marker;", new("cases/layout/moved.rs"))
        ]),
    );
    args["acknowledge_test_consumers"] = json!(true);
    for limits in [
        json!({}),
        json!({"diagnostic_count":0}),
        json!({"diagnostic_count":100}),
    ] {
        args["limits"] = limits;
        let value = run(&repo, args.clone());
        withheld(&value);
        assert_eq!(value["coverage"]["test_consumers_acknowledged"], 2);
        assert_eq!(acknowledged(&value).len(), 2, "{value}");
        assert!(
            value["plan"]["decision_groups"]
                .as_array()
                .unwrap()
                .iter()
                .any(|g| {
                    g["reason"] == "macro_context_unexamined" && g["blocks_applicability"] == true
                })
        );
        assert!(
            value["plan"]["test_consumer_disclosure"]
                .as_str()
                .unwrap()
                .contains("2 consumers")
        );
    }
}

#[test]
fn test_consumer_acknowledgment_does_not_discharge_selected_attributes_or_root_chain() {
    for prefix in ["#[cfg(feature = \"extra\")]\n", "#![cfg(unix)]\n"] {
        let repo = fixture(&format!(
            "{prefix}const LIMIT: usize = 4;\n#[cfg(test)] mod tests {{ fn check() {{ assert!(super::LIMIT > 0); }} }}\n"
        ));
        let mut args = request(
            &repo,
            json!([entry(
                &repo,
                "const LIMIT: usize = 4;",
                new("cases/layout/moved.rs")
            )]),
        );
        args["acknowledge_test_consumers"] = json!(true);
        let value = run(&repo, args);
        withheld(&value);
        assert!(
            value["plan"]["decision_groups"]
                .as_array()
                .unwrap()
                .iter()
                .any(|g| {
                    g["blocks_applicability"] == true
                        && (g["reason"] == "conditional_or_inherited_context"
                            || g["reason"] == "module_chain_failure")
                }),
            "{value}"
        );
    }
}

#[test]
fn probe_b_hoist_uncertainty_and_macro_consumers_are_acknowledged() {
    let source = include_str!("../fixtures/test_consumers/probe_b.rs");
    let repo = fixture(source);
    let texts = [
        "const MAX_PROMPT_BYTES: usize = 4;",
        "const MAX_MODEL_BYTES: usize = 8;",
        "const MAX_EVIDENCE: usize = 16;",
        "const MAX_KIND_BYTES: usize = 32;",
    ];
    let moves: Vec<_> = texts
        .iter()
        .map(|text| entry(&repo, text, new("cases/layout/limits.rs")))
        .collect();
    let mut args = request(&repo, json!(moves));
    args["limits"] = json!({"diagnostic_count":100});
    let off = run(&repo, args.clone());
    code(&off, "MODULE_CONTEXT");
    assert!(acknowledged(&off).is_empty());
    let off_lexical: Vec<_> = off["plan"]["decisions"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|d| !d["lexical_uncertainty"].is_null())
        .collect();
    assert_eq!(off_lexical.len(), 4);
    assert!(
        off_lexical
            .iter()
            .all(|d| d["blocks_applicability"] == true)
    );
    args["acknowledge_test_consumers"] = json!(true);
    args["limits"] = json!({"diagnostic_count":0, "text_bytes":0});
    let on = run(&repo, args);
    assert_eq!(on["plan"]["applicable"], true, "{on}");
    assert_eq!(on["plan"]["integrity"]["semantic"], "not_performed");
    let risks = acknowledged(&on);
    assert_eq!(risks.len(), 8, "{on}");
    assert_eq!(on["coverage"]["test_consumers_acknowledged"], risks.len());
    let lexical: Vec<_> = risks
        .iter()
        .filter(|d| !d["lexical_uncertainty"].is_null())
        .collect();
    assert_eq!(lexical.len(), 4);
    for risk in lexical {
        assert_eq!(risk["blocks_applicability"], false);
        assert_eq!(risk["resolution"], "risk_acknowledged");
        assert_eq!(risk["action"]["field"], "acknowledge_test_consumers");
        let witness = &risk["lexical_uncertainty"];
        assert_eq!(witness["witness_relation"], "hoist_possibility");
        assert_eq!(witness["reason"], "unsupported_pattern");
        let at = risk["anchors"][0]["range"]["end_byte"].as_u64().unwrap();
        let start = witness["pattern"]["range"]["start_byte"].as_u64().unwrap();
        assert!(start > at, "{risk}");
        let end = witness["pattern"]["range"]["end_byte"].as_u64().unwrap() as usize;
        assert!(source[start as usize..end].starts_with("assert_eq!(oversized.len(),"));
    }
    assert!(risks.iter().any(|d| d["category"] == "macro_dependency"));
    let copy = apply(&repo, &on);
    let after = fs::read_to_string(copy.0.join("cases/layout/source.rs")).unwrap();
    assert!(after.contains(&source[source.find("#[cfg(test)]").unwrap()..]));
    assert_eq!(
        fs::read_to_string(copy.0.join("cases/layout/limits.rs")).unwrap(),
        format!("pub(crate) {}\n", texts.join("\npub(crate) "))
    );
    let compiled = std::process::Command::new("rustc")
        .current_dir(&copy.0)
        .args([
            "--edition=2024",
            "--test",
            "cases/layout/lib.rs",
            "-o",
            "probe-tests",
        ])
        .output()
        .unwrap();
    assert!(
        compiled.status.success(),
        "{}",
        String::from_utf8_lossy(&compiled.stderr)
    );
    let tested = std::process::Command::new(copy.0.join("probe-tests"))
        .output()
        .unwrap();
    assert!(
        tested.status.success(),
        "{}",
        String::from_utf8_lossy(&tested.stdout)
    );
    assert!(String::from_utf8_lossy(&tested.stdout).contains("4 passed"));
}

#[test]
fn test_consumer_lexical_acknowledgment_keeps_written_conflicts_blocking() {
    for body in [
        "let (LIMIT,) = pair; let n = LIMIT;",
        "let LIMIT = 1; let _: LIMIT = 2;",
        "let LIMIT = 1; let _: LIMIT = 2; assert!(true);",
        "let (LIMIT,) = pair; let n = LIMIT; assert!(true);",
        "use crate::other::LIMIT; let n = LIMIT; assert!(true);",
        "let n = LIMIT; const LIMIT: usize = 1; assert!(true);",
        "let n = LIMIT; #[cfg(unix)] const LIMIT: usize = 1; assert!(true);",
    ] {
        let repo = fixture(&format!(
            "const LIMIT: usize = 4;\n#[cfg(test)] mod tests {{ use super::*; fn check() {{ {body} }} }}\n"
        ));
        let mut args = request(
            &repo,
            json!([entry(
                &repo,
                "const LIMIT: usize = 4;",
                new("cases/layout/moved.rs")
            )]),
        );
        args["acknowledge_test_consumers"] = json!(true);
        let value = run(&repo, args);
        code(&value, "MODULE_CONTEXT");
        assert!(
            value["plan"]["decisions"]
                .as_array()
                .unwrap()
                .iter()
                .any(|d| d["blocks_applicability"] == true && !d["lexical_uncertainty"].is_null()),
            "{body}: {value}"
        );
    }
}

#[test]
fn test_consumer_hoist_acknowledgment_keeps_value_item_in_type_position_blocking() {
    let repo = fixture(include_str!(
        "../fixtures/test_consumers/namespace_mismatch.rs"
    ));
    let mut args = request(
        &repo,
        json!([entry(
            &repo,
            "const LIMIT: usize = 4;",
            new("cases/layout/moved.rs")
        )]),
    );
    args["acknowledge_test_consumers"] = json!(true);
    let value = run(&repo, args);
    code(&value, "MODULE_CONTEXT");
    let reference = value["plan"]["decisions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["anchors"][0]["expected_text"] == "LIMIT")
        .unwrap();
    assert_eq!(reference["reason"], "conditional_or_inherited_context");
    assert_eq!(reference["blocks_applicability"], true);
    assert_eq!(reference["action"]["route"], "unsupported_in_engine");
    assert_eq!(
        reference["lexical_uncertainty"]["witness_relation"],
        "hoist_possibility"
    );
}

#[test]
fn test_consumer_hoist_acknowledgment_accepts_type_items_in_type_position() {
    for selected in ["struct LIMIT;", "enum LIMIT { Value }"] {
        let repo = fixture(&format!(
            "{selected}\n#[cfg(test)] mod tests {{ use super::*; fn check() {{ let _: LIMIT = value; assert_eq!(1, 1); }} }}\n"
        ));
        let mut args = request(
            &repo,
            json!([entry(&repo, selected, new("cases/layout/moved.rs"))]),
        );
        args["acknowledge_test_consumers"] = json!(true);
        let value = run(&repo, args);
        assert_eq!(value["plan"]["applicable"], true, "{selected}: {value}");
        assert_eq!(value["plan"]["integrity"]["semantic"], "not_performed");
        let risks = acknowledged(&value);
        let reference = risks
            .iter()
            .find(|d| d["anchors"][0]["expected_text"] == "LIMIT")
            .unwrap();
        assert_eq!(reference["blocks_applicability"], false);
        assert_eq!(
            reference["lexical_uncertainty"]["witness_relation"],
            "hoist_possibility"
        );
        apply(&repo, &value);
    }
}

#[test]
fn test_consumer_hoist_acknowledgment_checks_type_item_value_constructors() {
    for (selected, compatible) in [
        ("struct LIMIT;", true),
        ("struct LIMIT(usize);", true),
        ("struct LIMIT {}", false),
        ("enum LIMIT { Value }", false),
        ("union LIMIT { value: usize }", false),
        ("trait LIMIT {}", false),
        ("type LIMIT = usize;", false),
    ] {
        let repo = fixture(&format!(
            "{selected}\n#[cfg(test)] mod tests {{ use super::*; fn check() {{ let n = LIMIT; assert_eq!(1, 1); }} }}\n"
        ));
        let mut args = request(
            &repo,
            json!([entry(&repo, selected, new("cases/layout/moved.rs"))]),
        );
        args["acknowledge_test_consumers"] = json!(true);
        let value = run(&repo, args);
        assert_eq!(
            value["plan"]["applicable"], compatible,
            "{selected}: {value}"
        );
        let reference = value["plan"]["decisions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|d| d["anchors"][0]["expected_text"] == "LIMIT")
            .unwrap();
        assert_eq!(reference["blocks_applicability"], !compatible);
        assert_eq!(
            reference["lexical_uncertainty"]["witness_relation"],
            "hoist_possibility"
        );
        if compatible {
            assert_eq!(reference["reason"], "test_consumer_acknowledged");
        } else {
            withheld(&value);
            assert_eq!(reference["action"]["route"], "unsupported_in_engine");
        }
    }
}

#[test]
fn test_consumer_acknowledgment_does_not_clear_stale_anchors_or_broken_structure() {
    for (source, expected) in [
        (
            "const LIMIT: usize = 4;\n#[cfg(test)] mod tests { fn check() { let n = super::LIMIT; assert!(true); } }\n",
            "STALE_SELECTION",
        ),
        (
            "const LIMIT: usize = 4;\n#[cfg(test)] mod tests { fn check() { @ super::LIMIT; } }\n",
            "PREEXISTING_SYNTAX_ERROR",
        ),
    ] {
        let repo = fixture(source);
        let mut args = request(
            &repo,
            json!([entry(
                &repo,
                "const LIMIT: usize = 4;",
                new("cases/layout/moved.rs")
            )]),
        );
        args["acknowledge_test_consumers"] = json!(true);
        if expected == "STALE_SELECTION" {
            args["moves"][0]["item"]["expected_text"] = json!("const LIMIT: usize = 5;");
        }
        let value = run(&repo, args);
        code(&value, expected);
        if expected == "STALE_SELECTION" {
            assert!(acknowledged(&value).is_empty());
        }
    }
}

#[test]
fn test_consumer_flag_off_serialized_responses_are_byte_identical() {
    for source in [
        "const LIMIT: usize = 4;\n",
        "const LIMIT: usize = 4;\n#[cfg(test)] mod tests { fn check() { assert!(super::LIMIT > 0); } }\n",
        "const LIMIT: usize = 4;\nfn outside() { assert!(LIMIT > 0); }\n",
        "const LIMIT: usize = 4;\n#[cfg(test)] mod tests { use super::*; fn check() { let n = LIMIT; assert!(true); } }\n",
    ] {
        let repo = fixture(source);
        let args = request(
            &repo,
            json!([entry(
                &repo,
                "const LIMIT: usize = 4;",
                new("cases/layout/moved.rs")
            )]),
        );
        let absent = serde_json::to_vec(&run(&repo, args.clone())).unwrap();
        let mut explicit = args;
        explicit["acknowledge_test_consumers"] = json!(false);
        assert_eq!(absent, serde_json::to_vec(&run(&repo, explicit)).unwrap());
    }
}
