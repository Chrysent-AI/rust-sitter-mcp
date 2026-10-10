use super::*;

const IMPORT: &str = "use serde::Serialize;";
const DERIVE: &str = "#[derive(Serialize)]";
const TYPE: &str = "struct Record { value: u8 }";
fn setup(extra: &str, body: &str) -> Fixture {
    let repo = fixture(&format!("{IMPORT}\n{DERIVE}\n{extra}{body}\n"));
    repo.write(
        "cases/layout/destination.rs",
        &format!("{IMPORT}\nfn keep() {{}}\n"),
    );
    repo
}
fn asserted(repo: &Fixture, body: &str) -> Value {
    let mut args = request(
        repo,
        json!([entry(
            repo,
            body,
            json!({"kind":"existing","path":"cases/layout/destination.rs"})
        )]),
    );
    args["assume_standard_prelude"] = json!(true);
    args["limits"]["diagnostic_count"] = json!(100_000);
    args["derived_attribute_relocations"] = json!([{
        "invocation":anchor(repo,"cases/layout/source.rs",DERIVE),
        "helpers":[], "bindings":[{
            "name":"Serialize", "provider_path":"serde::Serialize",
            "original_import":anchor(repo,"cases/layout/source.rs",IMPORT),
            "destination_import":anchor(repo,"cases/layout/destination.rs",IMPORT)
        }]
    }]);
    args
}
#[test]
fn derive_relocation_is_exact_conditional_and_default_off() {
    let repo = setup("", TYPE);
    let args = asserted(&repo, TYPE);
    let mut off = args.clone();
    off.as_object_mut()
        .unwrap()
        .remove("derived_attribute_relocations");
    let refused = run(&repo, off.clone());
    withheld(&refused);
    assert!(
        refused["plan"]["decisions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| {
                d["refusal_basis"]
                    .as_array()
                    .is_some_and(|b| b.iter().any(|b| b["class"] == "derive_veto"))
            }),
        "{refused}"
    );
    for empty in [Value::Null, json!([])] {
        off["derived_attribute_relocations"] = empty;
        assert_eq!(refused, run(&repo, off.clone()));
    }
    let result = run(&repo, args);
    assert_eq!(result["plan"]["applicable"], true, "{result}");
    assert_eq!(result["coverage"]["assumed_derive_relocation"], 1);
    assert!(result["coverage"].get("ra_resolved").is_none());
    assert_eq!(result["plan"]["integrity"]["semantic"], "not_performed");
    let proof = &result["plan"]["binding_proofs"][0];
    assert_eq!(proof["class"], "assumed_derive_relocation");
    assert_eq!(proof["invocation"]["expected_text"], DERIVE);
    assert!(
        proof["basis"]
            .as_str()
            .unwrap()
            .contains("generated imports")
    );
    let copy = apply(&repo, &result);
    let moved = fs::read_to_string(copy.0.join("cases/layout/destination.rs")).unwrap();
    assert!(moved.contains(&format!("{DERIVE}\n{TYPE}")), "{moved}");
    assert!(
        !fs::read_to_string(copy.0.join("cases/layout/source.rs"))
            .unwrap()
            .contains(DERIVE)
    );
    // Compilation is a caller-side check in a disposable externally applied copy,
    // not engine evidence. The server itself never starts Cargo or proc macros.
    copy.write("Cargo.toml", "[package]\nname = \"derive-relocation-probe\"\nversion = \"0.0.0\"\nedition = \"2024\"\n[lib]\npath = \"cases/layout/lib.rs\"\n[dependencies]\nserde = { version = \"1\", features = [\"derive\"] }\n");
    let output = std::process::Command::new("cargo")
        .args(["check", "--all-targets", "--offline"])
        .current_dir(&copy.0)
        .output()
        .unwrap();
    fs::write(
        "/tmp/derive-relocation-disposable-check.log",
        [&output.stdout[..], &output.stderr[..]].concat(),
    )
    .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
#[test]
fn derive_relocation_helpers_travel_byte_exact_without_nominal_flag() {
    let helper = "#[serde(rename = \"wire_record\")]";
    let field_helper = "#[serde(rename = \"wire_value\")]";
    let body = format!("struct Record {{ {field_helper} value: u8 }}");
    let repo = setup(&format!("{helper}\n"), &body);
    let mut args = asserted(&repo, &body);
    args["derived_attribute_relocations"][0]["helpers"] = json!([
        {"anchor":anchor(&repo,"cases/layout/source.rs",helper),"derive_name":"Serialize"},
        {"anchor":anchor(&repo,"cases/layout/source.rs",field_helper),"derive_name":"Serialize"}
    ]);
    let result = run(&repo, args.clone());
    assert_eq!(result["plan"]["applicable"], true, "{result}");
    let copy = apply(&repo, &result);
    assert!(
        fs::read_to_string(copy.0.join("cases/layout/destination.rs"))
            .unwrap()
            .contains(&format!("{DERIVE}\n{helper}\n{body}"))
    );
    // Caller-side compilation in the externally applied disposable helper copy.
    copy.write("Cargo.toml", "[package]\nname = \"derive-helper-probe\"\nversion = \"0.0.0\"\nedition = \"2024\"\n[lib]\npath = \"cases/layout/lib.rs\"\n[dependencies]\nserde = { version = \"1\", features = [\"derive\"] }\n");
    let output = std::process::Command::new("cargo")
        .args(["check", "--all-targets", "--offline"])
        .current_dir(&copy.0)
        .output()
        .unwrap();
    fs::write(
        "/tmp/derive-relocation-helper-disposable-check.log",
        [&output.stdout[..], &output.stderr[..]].concat(),
    )
    .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    args["derived_attribute_relocations"][0]["helpers"] = json!([]);
    withheld(&run(&repo, args));
    for control in [
        "#[r#cfg(any())]",
        "#[r#repr(C)]",
        "#[feature(some_feature)]",
    ] {
        let repo = setup(&format!("{control}\n"), TYPE);
        let mut args = asserted(&repo, TYPE);
        args["derived_attribute_relocations"][0]["helpers"] = json!([
            {"anchor":anchor(&repo,"cases/layout/source.rs",control),"derive_name":"Serialize"}
        ]);
        assert_eq!(run(&repo, args)["error"]["code"], "INVALID_PARAMS");
    }
}
#[test]
fn derive_relocation_refuses_unasserted_providers_and_independent_vetoes() {
    for extra in [
        "#[derive(Unknown)]\n",
        "#[cfg(missing)]\n",
        "#[cfg(any())]\n",
        "#[cfg_attr(missing, derive(Serialize))]\n",
        "#[replace]\n",
    ] {
        let repo = setup(extra, TYPE);
        withheld(&run(&repo, asserted(&repo, TYPE)));
    }
    for path in [
        "cases/layout/source.rs",
        "cases/layout/destination.rs",
        "cases/layout/lib.rs",
    ] {
        for provider in [
            "use external::*;",
            "introduce!();",
            "macro_rules! introduce { () => { use external::*; } }",
            "#[derive(Importing)] struct Other;",
            "use external::replace as serde;",
            "struct Serialize;",
            "struct r#Serialize;",
            "use external::r#Serialize;",
            "mod serde {}",
        ] {
            let helper = "#[serde(rename = \"record\")]";
            let repo = setup(&format!("{helper}\n"), TYPE);
            let original = fs::read_to_string(repo.0.join(path)).unwrap();
            repo.write(path, &format!("{original}\n{provider}\n"));
            let mut args = asserted(&repo, TYPE);
            args["derived_attribute_relocations"][0]["helpers"] = json!([
                {"anchor":anchor(&repo,"cases/layout/source.rs",helper),"derive_name":"Serialize"}
            ]);
            let result = run(&repo, args);
            assert_eq!(result["plan"]["applicable"], false, "{path}: {provider}");
            withheld(&result);
        }
    }
    let body = "struct Record { value: Missing }";
    let repo = setup("", body);
    withheld(&run(&repo, asserted(&repo, body)));
}
#[test]
fn derive_relocation_refuses_separate_unasserted_custom_derive_in_each_scope() {
    for path in [
        "cases/layout/source.rs",
        "cases/layout/destination.rs",
        "cases/layout/lib.rs",
    ] {
        let repo = setup("", TYPE);
        let original = fs::read_to_string(repo.0.join(path)).unwrap();
        repo.write(
            path,
            &format!("{original}\n#[derive(Other)] struct Neighbor;\n"),
        );
        let result = run(&repo, asserted(&repo, TYPE));
        withheld(&result);
        assert!(
            result["coverage"]
                .get("assumed_derive_relocation")
                .is_none(),
            "{path}: {result}"
        );
        eprintln!(
            "unasserted Other in {path}: non-applicable; edits, created_files and patch withheld; no derive-relocation admission"
        );
    }
}

#[test]
fn derive_relocation_refuses_second_unasserted_custom_attribute_on_moved_type() {
    let repo = setup("#[derive(Other)]\n", TYPE);
    let result = run(&repo, asserted(&repo, TYPE));
    withheld(&result);
    assert!(
        result["coverage"]
            .get("assumed_derive_relocation")
            .is_none(),
        "{result}"
    );
    eprintln!(
        "unasserted Other on moved Record beside asserted Serialize: non-applicable; edits, created_files and patch withheld; no derive-relocation admission"
    );
}

#[test]
fn derive_relocation_refuses_builtin_spelling_with_visible_macro_shadow() {
    for path in [
        "cases/layout/source.rs",
        "cases/layout/destination.rs",
        "cases/layout/lib.rs",
    ] {
        let repo = setup("", TYPE);
        let original = fs::read_to_string(repo.0.join(path)).unwrap();
        // A compiler builtin on a retained neighbor alone does not block relocation.
        let neighbor = "#[derive(Debug)] struct Neighbor;";
        repo.write(path, &format!("{original}\n{neighbor}\n"));
        let control = run(&repo, asserted(&repo, TYPE));
        assert_eq!(control["plan"]["applicable"], true, "{path}: {control}");
        repo.write(
            path,
            &format!("{original}\nmacro_rules! Debug {{ () => {{}} }}\n{neighbor}\n"),
        );
        let result = run(&repo, asserted(&repo, TYPE));
        withheld(&result);
        assert!(
            result["coverage"]
                .get("assumed_derive_relocation")
                .is_none(),
            "{path}: {result}"
        );
        eprintln!(
            "Debug in {path}: builtin-only control applicable; visible macro_rules shadow non-applicable; edits, created_files and patch withheld; no derive-relocation admission"
        );
    }
}

#[test]
fn derive_relocation_binding_story_is_required_for_aliases_and_qualified_paths_refuse() {
    for spelling in ["Alias", "serde::Serialize", "Unknown"] {
        let repo = setup("", TYPE);
        let source = fs::read_to_string(repo.0.join("cases/layout/source.rs"))
            .unwrap()
            .replace(DERIVE, &format!("#[derive({spelling})]"));
        repo.write("cases/layout/source.rs", &source);
        let mut args = request(
            &repo,
            json!([entry(
                &repo,
                TYPE,
                json!({"kind":"existing","path":"cases/layout/destination.rs"})
            )]),
        );
        args["derived_attribute_relocations"] = json!([{
            "invocation":anchor(&repo,"cases/layout/source.rs",&format!("#[derive({spelling})]")),
            "helpers":[],"bindings":[]
        }]);
        withheld(&run(&repo, args));
    }
    let repo = setup("", TYPE);
    let mut args = asserted(&repo, TYPE);
    args["derived_attribute_relocations"][0]["bindings"][0]["provider_path"] =
        json!("other::Serialize");
    withheld(&run(&repo, args));
}
#[test]
fn derive_relocation_invalid_anchors_and_dispositions_are_not_silent() {
    let repo = setup("", TYPE);
    let mut args = asserted(&repo, TYPE);
    args["derived_attribute_relocations"][0]["invocation"]["expected_text"] = json!("stale");
    let result = run(&repo, args);
    assert_eq!(result["error"]["code"], "INVALID_PARAMS");
    assert_eq!(result["error"]["field"], "derived_attribute_relocations[0]");
    let mut args = asserted(&repo, TYPE);
    let assertion = args["derived_attribute_relocations"][0].clone();
    args["derived_attribute_relocations"]
        .as_array_mut()
        .unwrap()
        .push(assertion);
    assert_eq!(
        run(&repo, args)["error"]["field"],
        "derived_attribute_relocations[1]"
    );
    let mut args = asserted(&repo, TYPE);
    args["trivia_overrides"] = json!([{"trivia":anchor(&repo,"cases/layout/source.rs",DERIVE),"disposition":"keep_in_place","target_item":null}]);
    withheld(&run(&repo, args));
}

#[test]
fn derive_relocation_mixed_builtin_and_alias_routes_keep_separate_evidence() {
    let repo = setup("", TYPE);
    let mixed = "#[derive(Debug, Serialize)]";
    let original = fs::read_to_string(repo.0.join("cases/layout/source.rs"))
        .unwrap()
        .replace(DERIVE, mixed);
    repo.write("cases/layout/source.rs", &original);
    let mut args = request(
        &repo,
        json!([entry(
            &repo,
            TYPE,
            json!({"kind":"existing","path":"cases/layout/destination.rs"})
        )]),
    );
    args["assume_standard_prelude"] = json!(true);
    args["derived_attribute_relocations"] = json!([{
        "invocation":anchor(&repo,"cases/layout/source.rs",mixed), "helpers":[],
        "bindings":[{"name":"Serialize","provider_path":"serde::Serialize",
            "original_import":anchor(&repo,"cases/layout/source.rs",IMPORT),
            "destination_import":anchor(&repo,"cases/layout/destination.rs",IMPORT)}]
    }]);
    let result = run(&repo, args);
    assert_eq!(result["plan"]["applicable"], true, "{result}");
    assert_eq!(result["coverage"]["standard_builtin_derive"], 1);
    assert_eq!(result["coverage"]["assumed_derive_relocation"], 1);
    assert!(result["coverage"].get("ra_resolved").is_none());

    let repo = setup("", TYPE);
    let alias_import = "use serde::Serialize as Encode;";
    for path in ["cases/layout/source.rs", "cases/layout/destination.rs"] {
        let text = fs::read_to_string(repo.0.join(path))
            .unwrap()
            .replace(IMPORT, alias_import)
            .replace(DERIVE, "#[derive(Encode)]");
        repo.write(path, &text);
    }
    let mut args = request(
        &repo,
        json!([entry(
            &repo,
            TYPE,
            json!({"kind":"existing","path":"cases/layout/destination.rs"})
        )]),
    );
    args["derived_attribute_relocations"] = json!([{
        "invocation":anchor(&repo,"cases/layout/source.rs","#[derive(Encode)]"), "helpers":[],
        "bindings":[{"name":"Encode","provider_path":"serde::Serialize",
            "original_import":anchor(&repo,"cases/layout/source.rs",alias_import),
            "destination_import":anchor(&repo,"cases/layout/destination.rs",alias_import)}]
    }]);
    assert_eq!(run(&repo, args)["plan"]["applicable"], true);
}

#[test]
fn derive_relocation_never_proves_generated_members_under_resolution() {
    let repo = setup("", TYPE);
    let implementation = "impl Record { fn inspect(&self) { self.generated(); } }";
    let text = fs::read_to_string(repo.0.join("cases/layout/source.rs")).unwrap();
    repo.write(
        "cases/layout/source.rs",
        &format!("{text}\n{implementation}\n"),
    );
    let mut args = asserted(&repo, TYPE);
    args["moves"].as_array_mut().unwrap().push(entry(
        &repo,
        implementation,
        json!({"kind":"existing","path":"cases/layout/destination.rs"}),
    ));
    args["resolve_semantic"] = json!(true);
    args["semantic_configuration"] = json!({"crates":[{
        "name":"application", "root_file":"cases/layout/lib.rs", "edition":"2024",
        "features":[],"cfg":[],"dependencies":[]
    }]});
    let result = run(&repo, args);
    assert_eq!(result["plan"]["applicable"], false, "{result}");
    assert_eq!(result["coverage"]["assumed_derive_relocation"], 1);
    assert!(
        result["plan"]["decisions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| { d["reason"] == "member_or_constructor_unproved" }),
        "{result}"
    );
    assert!(
        result["plan"]["binding_proofs"]
            .as_array()
            .unwrap()
            .iter()
            .all(|p| p["class"] != "ra_resolved")
    );
}
