use super::*;

fn import_fixture(imports: &str, selected: &str, retained: &str) -> (Fixture, Value) {
    let repo = fixture(&format!("{imports}\n{selected}\n{retained}\n"));
    repo.write(
        "cases/layout/lib.rs",
        "mod source;\nmod destination;\nmod types;\n",
    );
    repo.write("cases/layout/types.rs", "pub(crate) struct Token;\npub(crate) struct Shared;\npub(crate) struct Other;\npub(crate) struct r#type;\npub(crate) struct É;\npub(crate) trait Method {}\n");
    let args = request(
        &repo,
        json!([entry(&repo, selected, new("cases/layout/moved.rs"))]),
    );
    (repo, args)
}

fn advisories(result: &Value) -> Vec<&Value> {
    result["plan"]["decisions"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|d| d["reason"] == "post_move_import_review")
        .collect()
}

#[test]
fn dead_import_orphan_is_anchored_nonblocking_and_losslessly_retained() {
    let import = "use crate::types::Token;";
    let (repo, args) = import_fixture(import, "fn selected(_: Token) {}", "fn retained() {}");
    let result = run(&repo, args.clone());
    let decisions = advisories(&result);
    assert_eq!(decisions.len(), 1, "{result}");
    let d = decisions[0];
    assert_eq!(
        d["anchors"][0],
        anchor(&repo, "cases/layout/source.rs", import)
    );
    assert_eq!(d["blocks_applicability"], false);
    assert_eq!(d["resolution"], "advisory");
    assert_eq!(d["selected_choice"], "retain");
    assert_eq!(d["supported_choices"], json!([]));
    assert_eq!(d["action"]["route"], "unsupported_in_engine");
    assert!(
        d["unresolved_consequence"]
            .as_str()
            .unwrap()
            .contains("[Token]")
    );
    let copy = apply(&repo, &result);
    assert!(
        fs::read_to_string(copy.0.join("cases/layout/source.rs"))
            .unwrap()
            .starts_with(import)
    );
    assert_eq!(result["plan"], run(&repo, args)["plan"]);
    assert!(result["plan"]["decision_groups"].as_array().unwrap().iter().any(|g|
        g["reason"] == "post_move_import_review" && g["blocks_applicability"] == false));
    assert!(
        result["plan"]["moves"][0]["decision_ids"]
            .as_array()
            .unwrap()
            .contains(&d["id"])
    );
}

#[test]
fn dead_import_shared_binding_and_other_written_tokens_are_not_candidates() {
    for retained in [
        "fn retained(_: Token) {}",
        "fn retained() { let Token = 1; }",
        "fn retained() { mention!(Token); }",
        "mod inner { use super::Token; }",
    ] {
        let (repo, args) = import_fixture(
            "use crate::types::Token;",
            "fn selected(_: Token) {}",
            retained,
        );
        let result = run(&repo, args);
        apply(&repo, &result);
        assert!(advisories(&result).is_empty(), "{retained}: {result}");
    }
}

#[test]
fn dead_import_partial_group_alias_raw_and_unicode_names_stay_byte_exact() {
    for (import, selected, retained, dead) in [
        (
            "use crate::types::{Token, Shared};",
            "fn selected(_: Token) {}",
            "fn retained(_: Shared) {}",
            "Token",
        ),
        (
            "use crate::types::{Token, Shared, Other};",
            "fn selected(_: Token, _: Other) {}",
            "fn retained(_: Shared) {}",
            "Token, Other",
        ),
        (
            "use crate::types::Token as Renamed;",
            "fn selected(_: Renamed) {}",
            "fn retained() {}",
            "Renamed",
        ),
        (
            "use crate::types::r#type;",
            "fn selected() {}",
            "fn retained() {}",
            "r#type",
        ),
        (
            "use crate::types::É;",
            "fn selected() {}",
            "fn retained() {}",
            "É",
        ),
    ] {
        let (repo, args) = import_fixture(import, selected, retained);
        let result = run(&repo, args);
        let copy = apply(&repo, &result);
        assert_eq!(advisories(&result).len(), 1, "{result}");
        let consequence = advisories(&result)[0]["unresolved_consequence"]
            .as_str()
            .unwrap();
        for name in dead.split(", ") {
            assert!(consequence.contains(name), "{consequence}");
        }
        assert!(!consequence.contains("[Shared]"), "{consequence}");
        assert!(
            fs::read_to_string(copy.0.join("cases/layout/source.rs"))
                .unwrap()
                .starts_with(import)
        );
    }
}

#[test]
fn dead_import_strings_comments_and_substrings_do_not_count_but_import_stays() {
    for retained in [
        "fn retained() { let _ = \"Token\"; }",
        "fn retained() { let _ = r###\"Token\"###; }",
        "fn retained() { let _ = b\"Token\"; }",
        "fn retained() { let _ = c\"Token\"; }",
        "fn retained() { /* Token /* Token */ */ }",
        "fn retained() { // Token\n}",
        "fn retained() { let TokenSuffix = 1; }",
        "fn retained() { mention!(\"Token\"); }",
    ] {
        let (repo, args) = import_fixture(
            "use crate::types::Token;",
            "fn selected(_: Token) {}",
            retained,
        );
        let result = run(&repo, args);
        let copy = apply(&repo, &result);
        assert_eq!(advisories(&result).len(), 1, "{retained}: {result}");
        assert!(
            advisories(&result)[0]["unresolved_consequence"]
                .as_str()
                .unwrap()
                .contains("[Token]")
        );
        assert!(
            fs::read_to_string(copy.0.join("cases/layout/source.rs"))
                .unwrap()
                .starts_with("use crate::types::Token;")
        );
    }
}

#[test]
fn dead_import_public_reexports_unchanged_and_private_globs_unenumerated() {
    for import in [
        "pub use crate::types::Token;",
        "pub(crate) use crate::types::Token;",
    ] {
        let (repo, args) = import_fixture(import, "fn selected(_: Token) {}", "fn retained() {}");
        let result = run(&repo, args);
        apply(&repo, &result);
        assert!(advisories(&result).is_empty(), "{result}");
    }
    let import = "use crate::types::*;";
    let (repo, args) = import_fixture(import, "fn selected() {}", "fn retained() {}");
    let result = run(&repo, args);
    let copy = apply(&repo, &result);
    assert_eq!(advisories(&result).len(), 1, "{result}");
    assert!(
        advisories(&result)[0]["unresolved_consequence"]
            .as_str()
            .unwrap()
            .contains("glob bindings unenumerated: true")
    );
    assert!(
        fs::read_to_string(copy.0.join("cases/layout/source.rs"))
            .unwrap()
            .starts_with(import)
    );
}

#[test]
fn dead_import_scan_uses_simultaneous_final_overlay_and_only_source_files() {
    let import = "use crate::types::Token;";
    let (repo, mut args) = import_fixture(import, "fn selected(_: Token) {}", "fn retained() {}");
    repo.write(
        "cases/layout/destination.rs",
        "fn incoming(_: crate::types::Token) {}\nuse crate::types::Other;\n",
    );
    args["moves"].as_array_mut().unwrap().push(json!({"item": anchor(&repo, "cases/layout/destination.rs", "fn incoming(_: crate::types::Token) {}"), "destination": {"kind":"existing", "path":"cases/layout/source.rs"}}));
    let result = run(&repo, args);
    apply(&repo, &result);
    // Incoming code's written Token keeps the source import conservatively alive.
    assert!(
        advisories(&result)
            .iter()
            .all(|d| d["anchors"][0]["path"] != "cases/layout/source.rs"),
        "{result}"
    );
    assert_eq!(advisories(&result).len(), 1, "{result}");
    assert!(
        advisories(&result)[0]["unresolved_consequence"]
            .as_str()
            .unwrap()
            .contains("[Other]")
    );
}

#[test]
fn dead_import_raw_unicode_and_nested_scope_tokens_preserve_bindings() {
    for (import, retained) in [
        ("use crate::types::r#type;", "fn retained(_: r#type) {}"),
        ("use crate::types::É;", "fn retained(_: É) {}"),
    ] {
        let (repo, args) = import_fixture(import, "fn selected() {}", retained);
        let result = run(&repo, args);
        apply(&repo, &result);
        assert!(advisories(&result).is_empty(), "{result}");
    }
    let (repo, args) = import_fixture(
        "",
        "fn selected() {}",
        "fn retained() { use crate::types::Token; }",
    );
    let result = run(&repo, args);
    apply(&repo, &result);
    assert_eq!(advisories(&result).len(), 1, "{result}");
    assert_eq!(
        advisories(&result)[0]["anchors"][0],
        anchor(&repo, "cases/layout/source.rs", "use crate::types::Token;")
    );
}

#[test]
fn dead_import_destination_only_files_are_not_cleanup_targets() {
    let (repo, args) = import_fixture("", "fn selected() {}", "fn retained() {}");
    repo.write(
        "cases/layout/destination.rs",
        "use crate::types::Other;\nfn keep() {}\n",
    );
    let result = run(&repo, args);
    apply(&repo, &result);
    assert!(advisories(&result).is_empty(), "{result}");
}

#[test]
fn dead_import_trait_method_lookup_is_advisory_not_unsafe_deletion() {
    let import = "use crate::types::Method;";
    let (repo, args) = import_fixture(
        import,
        "fn selected() {}",
        "fn retained(value: crate::types::Token) { value.method(); }",
    );
    let result = run(&repo, args);
    let copy = apply(&repo, &result);
    assert_eq!(advisories(&result).len(), 1, "{result}");
    assert!(
        advisories(&result)[0]["unresolved_consequence"]
            .as_str()
            .unwrap()
            .contains("trait method lookup")
    );
    assert!(
        fs::read_to_string(copy.0.join("cases/layout/source.rs"))
            .unwrap()
            .starts_with(import)
    );
}
