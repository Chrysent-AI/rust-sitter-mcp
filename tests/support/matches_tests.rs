use super::*;
const ENUM: &str = "pub(crate) enum ChildDirectoryPolicy { ConventionalFlatFile, Other }";
const INVOCATION: &str = "matches!(policy, ChildDirectoryPolicy::ConventionalFlatFile)";
fn setup(invocation: &str, prefix: &str) -> (Fixture, Value) {
    let repo = fixture(&format!(
        "{prefix}{ENUM}\npub(crate) fn probe(policy: ChildDirectoryPolicy) -> bool {{ {invocation}.then_some(true).unwrap_or(false) }}\n"
    ));
    let mut args = request(
        &repo,
        json!([entry(&repo, ENUM, new("cases/layout/moved.rs"))]),
    );
    args["standard_matches_assertions"] =
        json!([anchor(&repo, "cases/layout/source.rs", invocation)]);
    (repo, args)
}
#[test]
fn asserted_matches_consumer_applies_and_compiles() {
    let (repo, args) = setup(INVOCATION, "");
    let result = run(&repo, args);
    assert_eq!(result["plan"]["applicable"], true, "{result}");
    assert_eq!(result["coverage"]["assumed_standard_matches"], 1);
    assert!(result["coverage"].get("ra_resolved").is_none());
    assert_eq!(
        result["plan"]["binding_proofs"][0]["class"],
        "assumed_standard_matches"
    );
    assert_eq!(
        result["plan"]["binding_proofs"][0]["invocation"]["expected_text"],
        INVOCATION
    );
    let copy = apply(&repo, &result);
    copy.write("Cargo.toml", "[package]\nname=\"enum_probe\"\nversion=\"0.1.0\"\nedition=\"2024\"\n[lib]\npath=\"cases/layout/lib.rs\"\n");
    let output = std::process::Command::new("cargo")
        .args(["check", "--all-targets", "--manifest-path"])
        .arg(copy.0.join("Cargo.toml"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
#[test]
fn asserted_matches_audits_unrelated_expression_before_discharge() {
    for expression in ["unresolved", "crate::missing::policy"] {
        let invocation =
            format!("matches!({expression}, ChildDirectoryPolicy::ConventionalFlatFile)");
        let (repo, args) = setup(&invocation, "");
        let result = run(&repo, args);
        code(&result, "MACRO_DEPENDENCY");
        assert!(
            result["coverage"].get("assumed_standard_matches").is_none(),
            "{result}"
        );
        assert!(
            result["plan"]["binding_proofs"]
                .as_array()
                .is_none_or(|proofs| proofs
                    .iter()
                    .all(|p| p["class"] != "assumed_standard_matches")),
            "{result}"
        );
        let decision = result["plan"]["decisions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|d| d["reason"] == "macro_context_unexamined")
            .expect("invocation macro need retained");
        assert!(
            decision["anchors"]
                .as_array()
                .unwrap()
                .iter()
                .any(|a| a["expected_text"] == invocation),
            "{result}"
        );
        assert!(
            decision["refusal_basis"]
                .as_array()
                .unwrap()
                .iter()
                .any(|b| b["class"] == "chain_macro_statement"
                    && b["anchor"]["path"] == decision["anchors"][0]["path"]
                    && b["anchor"]["range"] == decision["anchors"][0]["range"]),
            "{result}"
        );
    }
    // The unrelated parameter is independently bound, not assumption coverage.
    let (repo, args) = setup(INVOCATION, "");
    let result = run(&repo, args);
    assert_eq!(result["plan"]["applicable"], true, "{result}");
    assert_eq!(result["coverage"]["assumed_standard_matches"], 1);
    let proofs = result["plan"]["binding_proofs"].as_array().unwrap();
    assert_eq!(proofs.len(), 1, "{result}");
    assert_eq!(
        proofs[0]["anchor"]["expected_text"],
        "ChildDirectoryPolicy::ConventionalFlatFile"
    );
    apply(&repo, &result);
}
#[test]
fn default_and_shadowed_matches_stay_blocked() {
    let (repo, mut args) = setup(INVOCATION, "");
    args.as_object_mut()
        .unwrap()
        .remove("standard_matches_assertions");
    code(&run(&repo, args), "MACRO_DEPENDENCY");
    for prefix in [
        "macro_rules! matches { ($($t:tt)*) => { true }; }\n",
        "use crate::destination::keep as matches;\n",
        "use crate::destination::*;\n",
        "#[cfg(any())]\nmacro_rules! matches { ($($t:tt)*) => { true }; }\n",
    ] {
        let (repo, args) = setup(INVOCATION, prefix);
        withheld(&run(&repo, args));
    }
}
#[test]
fn unsupported_matches_shapes_stay_blocked() {
    for invocation in [
        "matches!(policy, ChildDirectoryPolicy::ConventionalFlatFile if true)",
        "matches!(policy, ChildDirectoryPolicy::ConventionalFlatFile(value))",
        "matches!(policy, ChildDirectoryPolicy::Missing)",
        "matches!(policy, concat!(ChildDirectoryPolicy, Other))",
        "matches!(policy, ChildDirectoryPolicy::ConventionalFlatFile, extra)",
        "matches!(policy, ChildDirectoryPolicy::ConventionalFlatFile | binder)",
    ] {
        let (repo, args) = setup(invocation, "");
        withheld(&run(&repo, args));
    }
}
#[test]
fn assertions_are_exact_indexed_inputs() {
    let (repo, args) = setup(INVOCATION, "");
    let mut duplicate = args.clone();
    let assertion = duplicate["standard_matches_assertions"][0].clone();
    duplicate["standard_matches_assertions"]
        .as_array_mut()
        .unwrap()
        .push(assertion);
    let result = run(&repo, duplicate);
    assert_eq!(result["error"]["code"], "INVALID_PARAMS", "{result}");
    assert_eq!(result["error"]["field"], "standard_matches_assertions[1]");
    let mut stale = args;
    stale["standard_matches_assertions"][0]["expected_text"] = json!("stale");
    assert_eq!(run(&repo, stale)["error"]["code"], "INVALID_PARAMS");
}
#[test]
fn moved_owner_checks_all_admitted_roles_and_destination_shadows() {
    let function =
        format!("pub(crate) fn probe(policy: ChildDirectoryPolicy) -> bool {{ {INVOCATION} }}");
    for shadow in [false, true] {
        let repo = fixture(&format!("{ENUM}\n{function}\n"));
        if shadow {
            repo.write(
                "cases/layout/destination.rs",
                "macro_rules! matches { ($($t:tt)*) => { true }; }\n",
            );
        }
        let mut args = request(
            &repo,
            json!([entry(
                &repo,
                &function,
                json!({"kind":"existing", "path":"cases/layout/destination.rs"})
            )]),
        );
        args["standard_matches_assertions"] =
            json!([anchor(&repo, "cases/layout/source.rs", INVOCATION)]);
        let result = run(&repo, args);
        if shadow {
            withheld(&result);
        } else {
            assert_eq!(result["plan"]["applicable"], true, "{result}");
            assert_eq!(result["coverage"]["assumed_standard_matches"], 1);
            apply(&repo, &result);
        }
    }
}
#[test]
fn imported_and_qualified_matches_consumers_keep_original_anchors() {
    for qualified in [false, true] {
        let repo = fixture(ENUM);
        repo.write(
            "cases/layout/lib.rs",
            "mod source;\nmod destination;\nmod consumer;\n",
        );
        let invocation = if qualified {
            "matches!(policy, crate::source::ChildDirectoryPolicy::ConventionalFlatFile)"
        } else {
            INVOCATION
        };
        let consumer = format!(
            "use crate::source::ChildDirectoryPolicy;\nfn probe(policy: ChildDirectoryPolicy) -> bool {{ {invocation} }}\n"
        );
        repo.write("cases/layout/consumer.rs", &consumer);
        let mut args = request(
            &repo,
            json!([entry(&repo, ENUM, new("cases/layout/moved.rs"))]),
        );
        args["standard_matches_assertions"] =
            json!([anchor(&repo, "cases/layout/consumer.rs", invocation)]);
        let result = run(&repo, args);
        assert_eq!(result["plan"]["applicable"], true, "{result}");
        assert_eq!(result["coverage"]["assumed_standard_matches"], 1);
        let copy = apply(&repo, &result);
        let after = fs::read_to_string(copy.0.join("cases/layout/consumer.rs")).unwrap();
        assert!(after.contains("::ConventionalFlatFile)"));
        assert!(
            after.contains("use crate::moved::ChildDirectoryPolicy;"),
            "{after}"
        );
        if qualified {
            assert!(after.contains("crate::moved::ChildDirectoryPolicy::ConventionalFlatFile"));
        }
        assert_eq!(
            result["plan"]["binding_proofs"][0]["invocation"]["expected_text"],
            invocation
        );
    }
}
#[test]
fn one_assertion_cannot_discharge_another_invocation() {
    let source = format!(
        "{ENUM}\nfn probe(policy: ChildDirectoryPolicy) -> bool {{ {INVOCATION} || {INVOCATION} }}\n"
    );
    let repo = fixture(&source);
    let mut args = request(
        &repo,
        json!([entry(&repo, ENUM, new("cases/layout/moved.rs"))]),
    );
    args["standard_matches_assertions"] =
        json!([anchor(&repo, "cases/layout/source.rs", INVOCATION)]);
    withheld(&run(&repo, args));
}
#[test]
fn statements_exempt_only_the_asserted_invocation() {
    for other in ["", "opaque!();", "after"] {
        let source = format!(
            "{ENUM}\nfn probe(policy: ChildDirectoryPolicy) {{ {}{INVOCATION};{} }}\n",
            if other == "opaque!();" { other } else { "" },
            if other == "after" { "opaque!();" } else { "" }
        );
        let repo = fixture(&source);
        let mut args = request(
            &repo,
            json!([entry(&repo, ENUM, new("cases/layout/moved.rs"))]),
        );
        args["standard_matches_assertions"] =
            json!([anchor(&repo, "cases/layout/source.rs", INVOCATION)]);
        let result = run(&repo, args);
        if other.is_empty() {
            assert_eq!(result["plan"]["applicable"], true, "{result}");
        } else {
            withheld(&result);
        }
    }
}
#[test]
fn assertion_on_string_is_invalid_and_token_text_is_not_proof() {
    let invocation = "matches!(\"ChildDirectoryPolicy\", _)";
    let (repo, args) = setup(invocation, "");
    let result = run(&repo, args.clone());
    assert!(
        result["coverage"].get("assumed_standard_matches").is_none(),
        "{result}"
    );
    let mut wrong = args;
    wrong["standard_matches_assertions"] = json!([anchor(
        &repo,
        "cases/layout/source.rs",
        "\"ChildDirectoryPolicy\""
    )]);
    assert_eq!(run(&repo, wrong)["error"]["code"], "INVALID_PARAMS");
    let (repo, args) = setup("matches!(policy, /* ChildDirectoryPolicy */ _)", "");
    assert!(
        run(&repo, args)["coverage"]
            .get("assumed_standard_matches")
            .is_none()
    );
}
#[test]
fn malformed_and_nonunit_variants_stay_blocked() {
    let (repo, args) = setup(
        "matches!(policy,, ChildDirectoryPolicy::ConventionalFlatFile)",
        "",
    );
    withheld(&run(&repo, args));
    for declaration in [
        "pub(crate) enum ChildDirectoryPolicy { ConventionalFlatFile(u8), Other }",
        "pub(crate) enum ChildDirectoryPolicy { ConventionalFlatFile { value: u8 }, Other }",
        "pub(crate) enum ChildDirectoryPolicy { #[cfg(any())] ConventionalFlatFile, Other }",
    ] {
        let repo = fixture(&format!(
            "{declaration}\nfn probe(policy: ChildDirectoryPolicy) -> bool {{ {INVOCATION} }}\n"
        ));
        let mut args = request(
            &repo,
            json!([entry(&repo, declaration, new("cases/layout/moved.rs"))]),
        );
        args["standard_matches_assertions"] =
            json!([anchor(&repo, "cases/layout/source.rs", INVOCATION)]);
        withheld(&run(&repo, args));
    }
}
#[test]
fn local_prefix_competition_and_unclassed_second_occurrences_refuse() {
    let source = format!(
        "{ENUM}\nfn probe(policy: ChildDirectoryPolicy) -> bool {{ let ChildDirectoryPolicy = policy; {INVOCATION} }}\n"
    );
    let repo = fixture(&source);
    let mut args = request(
        &repo,
        json!([entry(&repo, ENUM, new("cases/layout/moved.rs"))]),
    );
    args["standard_matches_assertions"] =
        json!([anchor(&repo, "cases/layout/source.rs", INVOCATION)]);
    withheld(&run(&repo, args));
    let invocation =
        "matches!(policy, ChildDirectoryPolicy::ConventionalFlatFile | Different::Tuple)";
    let (repo, args) = setup(invocation, "enum Different { Tuple(u8) }\n");
    let result = run(&repo, args);
    withheld(&result);
    assert!(result["coverage"].get("assumed_standard_matches").is_none());
}
#[test]
fn unit_alternatives_and_trailing_comma_keep_suffix_bytes() {
    let invocation = "matches!(policy, ChildDirectoryPolicy::ConventionalFlatFile | ChildDirectoryPolicy::Other,)";
    let (repo, args) = setup(invocation, "// é\r\n");
    let result = run(&repo, args);
    assert_eq!(
        result["coverage"]["assumed_standard_matches"], 2,
        "{result}"
    );
    let copy = apply(&repo, &result);
    assert!(
        fs::read_to_string(copy.0.join("cases/layout/source.rs"))
            .unwrap()
            .contains(invocation)
    );
}
