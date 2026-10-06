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
fn test_consumer_flag_off_serialized_responses_are_byte_identical() {
    for source in [
        "const LIMIT: usize = 4;\n",
        "const LIMIT: usize = 4;\n#[cfg(test)] mod tests { fn check() { assert!(super::LIMIT > 0); } }\n",
        "const LIMIT: usize = 4;\nfn outside() { assert!(LIMIT > 0); }\n",
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
