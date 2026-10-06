use super::*;

fn enabled(mut args: Value) -> Value {
    args["assume_standard_prelude"] = json!(true);
    args["limits"]["diagnostic_count"] = json!(100_000);
    args
}
fn existing() -> Value {
    json!({"kind":"existing","path":"cases/layout/destination.rs"})
}
fn proof_count(result: &Value) -> usize {
    result["plan"]["binding_proofs"]
        .as_array()
        .map_or(0, |proofs| {
            proofs
                .iter()
                .filter(|p| {
                    p["class"] == "standard_prelude" && p["anchor"]["expected_text"] == "Option"
                })
                .count()
        })
}

#[test]
fn probe_a_signatures_discharge_with_conditional_root_and_unrelated_derives() {
    let a = "fn claim_review_status<T>(input: Option<&T>) -> Option<&T> { input }";
    let b = "fn single_transition<T>(input: Option<&T>) -> Option<&T> { input }";
    let repo = fixture(&format!(
        "{a}\n{b}\nmod unrelated {{ #[derive(custom::Debug)] struct Other; }}\n"
    ));
    repo.write("cases/layout/lib.rs", "#[cfg(test)] mod tests;\n#[derive(custom::Debug)] struct RootRecord;\nuse unknown::*;\nstruct Option;\nmod source;\nmod destination;\n");
    repo.write(
        "cases/layout/tests.rs",
        "#[derive(custom::Debug)] struct Unrelated;\n",
    );
    for destination in [existing(), new("cases/layout/status.rs")] {
        let result = run(
            &repo,
            enabled(request(
                &repo,
                json!([
                    entry(&repo, a, destination.clone()),
                    entry(&repo, b, destination)
                ]),
            )),
        );
        assert_eq!(proof_count(&result), 4, "{result}");
        assert_eq!(result["plan"]["applicable"], true, "{result}");
        assert_eq!(result["plan"]["integrity"]["semantic"], "not_performed");
        for proof in result["plan"]["binding_proofs"].as_array().unwrap() {
            assert!(
                proof["basis"]
                    .as_str()
                    .unwrap()
                    .contains("written token-tree not expanded")
            );
        }
        apply(&repo, &result);
    }
}

#[test]
fn body_macro_veto_is_occurrence_local_not_signature_or_batch_wide() {
    for (selected, proofs) in [
        ("fn selected(input: Option<u8>) { introduce!(); }", 1),
        ("fn selected() { introduce!(); let value: Option<u8>; }", 0),
        ("fn selected() { let value: Option<u8>; introduce!(); }", 0),
        ("fn selected() { let value: Option<u8>; introduce!{} }", 0),
        (
            "fn selected() { { introduce!(); } let value: Option<u8>; }",
            1,
        ),
        (
            "fn selected() { introduce!(); { let value: Option<u8>; } }",
            0,
        ),
        (
            "fn selected() { { let value: Option<u8>; } introduce!(); }",
            0,
        ),
        (
            "fn selected() { fn inner(_: Option<u8>) {} introduce!(); }",
            0,
        ),
        (
            "fn selected() { { fn inner(_: Option<u8>) {} } introduce!(); }",
            0,
        ),
    ] {
        let repo = fixture(selected);
        let result = run(
            &repo,
            enabled(request(&repo, json!([entry(&repo, selected, existing())]))),
        );
        withheld(&result); // The macro invocation itself remains unsupported.
        assert_eq!(proof_count(&result), proofs, "{selected}: {result}");
        if proofs == 0 {
            let decision = result["plan"]["decisions"]
                .as_array()
                .unwrap()
                .iter()
                .find(|d| d["anchors"][0]["expected_text"] == "Option")
                .unwrap();
            let relation =
                if selected.find("introduce!").unwrap() < selected.find("Option").unwrap() {
                    "statement_macro_before_reference"
                } else {
                    "block_macro_may_introduce_items"
                };
            assert_eq!(
                decision["lexical_uncertainty"]["witness_relation"],
                relation
            );
            let basis = decision["refusal_basis"]
                .as_array()
                .unwrap()
                .iter()
                .find(|b| b["class"] == "chain_macro_statement")
                .unwrap();
            let range = &basis["anchor"]["range"];
            let start = range["start_byte"].as_u64().unwrap() as usize;
            let end = range["end_byte"].as_u64().unwrap() as usize;
            assert!(["introduce!();", "introduce!{}"].contains(&&selected[start..end]));
        }
    }
    let record = "struct Record { option: Option<u8> }";
    let carrier = "fn carrier() { introduce!(); }";
    let repo = fixture(&format!("{record}\n{carrier}\n"));
    let result = run(
        &repo,
        enabled(request(
            &repo,
            json!([
                entry(&repo, record, existing()),
                entry(&repo, carrier, existing())
            ]),
        )),
    );
    withheld(&result);
    assert_eq!(proof_count(&result), 1, "{result}");
}

#[test]
fn no_implicit_prelude_in_an_unrelated_inline_module_retains_chain_refusal() {
    let record = "struct Record { option: Option<u8> }";
    for file in ["lib.rs", "source.rs", "destination.rs"] {
        let repo = fixture(record);
        let path = format!("cases/layout/{file}");
        let old = fs::read_to_string(repo.0.join(&path)).unwrap();
        repo.write(
            &path,
            &format!("{old}\nmod isolated {{ #![no_implicit_prelude] }}"),
        );
        let result = run(
            &repo,
            enabled(request(&repo, json!([entry(&repo, record, existing())]))),
        );
        // The prelude audit no longer promotes this control (unit-tested), but
        // the independent module-identity audit still refuses scope attributes.
        assert_eq!(proof_count(&result), 0, "{file}: {result}");
        withheld(&result);
        assert!(
            result["plan"]["decisions"]
                .as_array()
                .unwrap()
                .iter()
                .any(|d| d["reason"] == "module_chain_failure"),
            "{result}"
        );
    }
}

#[test]
fn finite_attributed_names_are_local_but_unbounded_context_stays_chain_wide() {
    let selected = "struct Record { option: Option<u8> }";
    for prefix in [
        "#[cfg(test)] mod tests;",
        "#[cfg_attr(test, derive(Clone))] struct Other;",
        "#[custom] struct Other;",
        "mod child { macro_rules! Debug { () => {} } }",
    ] {
        let repo = fixture(&format!("{prefix}\n{selected}"));
        let result = run(
            &repo,
            enabled(request(&repo, json!([entry(&repo, selected, existing())]))),
        );
        assert_eq!(proof_count(&result), 1, "{prefix}: {result}");
        assert_eq!(result["plan"]["applicable"], true, "{result}");
    }
    for prefix in [
        "#[cfg_attr(test, derive(Clone))] struct Option;",
        "mod unrelated { introduce!(); }",
        "#[introduce] mod unrelated {}",
        "#[cfg_attr(test, introduce)] mod unrelated {}",
        "#![no_std]",
        "#![no_core]",
        "#![prelude_import]",
        "#[cfg(] mod unrelated;",
    ] {
        let repo = fixture(selected);
        let old = fs::read_to_string(repo.0.join("cases/layout/lib.rs")).unwrap();
        let path = if prefix.contains("struct Option") {
            "cases/layout/source.rs"
        } else {
            "cases/layout/lib.rs"
        };
        repo.write(
            path,
            &format!(
                "{prefix}\n{}",
                if path.ends_with("source.rs") {
                    selected
                } else {
                    &old
                }
            ),
        );
        let result = run(
            &repo,
            enabled(request(&repo, json!([entry(&repo, selected, existing())]))),
        );
        withheld(&result);
        assert_eq!(proof_count(&result), 0, "{prefix}: {result}");
    }
}

#[test]
fn destination_shadow_departures_are_removed_before_arrivals_are_audited() {
    let record = "struct Record { option: Option<u8> }";
    let repo = fixture(record);
    repo.write("cases/layout/destination.rs", "struct Option;\n");
    repo.write("cases/layout/other.rs", "");
    repo.write(
        "cases/layout/lib.rs",
        "mod source;\nmod destination;\nmod other;\n",
    );
    let result = run(
        &repo,
        enabled(request(
            &repo,
            json!([
                entry(&repo, record, existing()),
                {"item":anchor(&repo,"cases/layout/destination.rs","struct Option;"),"destination":{"kind":"existing","path":"cases/layout/other.rs"}}
            ]),
        )),
    );
    assert_eq!(proof_count(&result), 1, "{result}");
    assert_eq!(result["plan"]["applicable"], true, "{result}");
    apply(&repo, &result);
}
