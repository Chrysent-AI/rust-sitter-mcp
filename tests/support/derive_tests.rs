use super::*;

fn enabled(mut args: Value) -> Value {
    args["assume_standard_prelude"] = json!(true);
    args["limits"]["diagnostic_count"] = json!(100_000);
    args
}
fn existing() -> Value {
    json!({"kind":"existing","path":"cases/layout/destination.rs"})
}
fn proofs(result: &Value, name: &str) -> usize {
    result["plan"]["binding_proofs"]
        .as_array()
        .map_or(0, |proofs| {
            proofs
                .iter()
                .filter(|p| p["anchor"]["expected_text"] == name)
                .count()
        })
}
fn conditional(result: &Value) -> bool {
    result["plan"]["decisions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|d| d["reason"] == "conditional_or_inherited_context")
}

#[test]
fn builtin_derive_opt_in_is_lossless_labeled_counted_and_strict_off() {
    let selected = "struct Record { value: u8 }";
    let original = format!("#[derive(Debug, Clone)]\r\n{selected}\r\n");
    let repo = fixture(&original);
    for destination in [existing(), new("cases/layout/derived.rs")] {
        let args = request(&repo, json!([entry(&repo, selected, destination.clone())]));
        let default = run(&repo, args.clone());
        let mut off = args.clone();
        off["assume_standard_prelude"] = json!(false);
        assert_eq!(
            serde_json::to_vec(&default).unwrap(),
            serde_json::to_vec(&run(&repo, off)).unwrap()
        );
        withheld(&default);
        assert!(conditional(&default));
        assert!(default["plan"].get("binding_proofs").is_none());
        assert!(default["coverage"].get("standard_builtin_derive").is_none());
        let result = run(&repo, enabled(args));
        assert_eq!(result["coverage"]["standard_builtin_derive"], 2);
        assert!(result["coverage"].get("standard_prelude").is_none());
        assert_eq!(result["plan"]["integrity"]["semantic"], "not_performed");
        for proof in result["plan"]["binding_proofs"].as_array().unwrap() {
            assert_eq!(proof["class"], "standard_builtin_derive");
            assert_eq!(proof["destination_path"], destination["path"]);
            assert_eq!(proof["item_ids"].as_array().unwrap().len(), 1);
            assert!(
                proof["basis"]
                    .as_str()
                    .unwrap()
                    .contains("caller enabled assume_standard_prelude")
            );
            let anchor = &proof["anchor"];
            let start = anchor["range"]["start_byte"].as_u64().unwrap() as usize;
            let end = anchor["range"]["end_byte"].as_u64().unwrap() as usize;
            assert_eq!(
                &original[start..end],
                anchor["expected_text"].as_str().unwrap()
            );
        }
        assert_eq!(proofs(&result, "Debug"), 1);
        assert_eq!(proofs(&result, "Clone"), 1);
        let copy = apply(&repo, &result);
        assert!(
            fs::read_to_string(copy.0.join(destination["path"].as_str().unwrap()))
                .unwrap()
                .contains(original.strip_suffix("\r\n").unwrap())
        );
    }
}

#[test]
fn builtin_derive_macro_identity_refuses_both_chains_and_batch_arrivals() {
    let selected = "struct Record { value: u8 }";
    for path in ["source.rs", "destination.rs", "lib.rs"] {
        for shadow in [
            "macro_rules! Debug { () => {} }",
            "use external::Debug;",
            "use external::{Other as Debug};",
            "use external::*;",
            "struct Debug;",
            "fn Debug() {}",
            "#![no_implicit_prelude]",
        ] {
            let repo = fixture(&format!("#[derive(Debug)]\n{selected}\n"));
            let file = format!("cases/layout/{path}");
            let old = fs::read_to_string(repo.0.join(&file)).unwrap();
            repo.write(&file, &format!("{shadow}\n{old}"));
            let result = run(
                &repo,
                enabled(request(&repo, json!([entry(&repo, selected, existing())]))),
            );
            let veto =
                path != "lib.rs" || shadow.starts_with("macro_rules!") || shadow.starts_with("#!");
            assert_eq!(
                proofs(&result, "Debug"),
                usize::from(!veto),
                "{path}: {shadow}: {result}"
            );
            if veto {
                withheld(&result);
            } else {
                apply(&repo, &result);
            }
        }
    }
    let repo = fixture(&format!("#[derive(Debug)]\n{selected}\n"));
    repo.write("cases/layout/other.rs", "struct Debug;\n");
    repo.write(
        "cases/layout/lib.rs",
        "mod source;\nmod destination;\nmod other;\n",
    );
    for destination in [
        existing(),
        json!({"kind":"existing","path":"cases/layout/lib.rs"}),
    ] {
        let result = run(
            &repo,
            enabled(request(
                &repo,
                json!([
                    entry(&repo, selected, existing()),
                    {"item":anchor(&repo,"cases/layout/other.rs","struct Debug;"),"destination":destination},
                ]),
            )),
        );
        let veto = destination == existing();
        assert_eq!(proofs(&result, "Debug"), usize::from(!veto), "{result}");
        if veto {
            withheld(&result);
        } else {
            apply(&repo, &result);
        }
    }
}

#[test]
fn builtin_derive_unknown_path_mixed_and_other_attributes_keep_vetoes() {
    let selected = "struct Record { value: u8 }";
    for (attribute, count) in [
        ("#[derive(serde::Serialize)]", 0),
        ("#[derive(foo::Debug)]", 0),
        ("#[derive(::Debug)]", 0),
        ("#[derive(Debug, Args)]", 1),
        ("#[derive(Debug, serde::Serialize)]", 1),
        ("#[derive(Debug)]\n#[derive(Args)]", 1),
        ("#[derive(Args)]", 0),
        ("#[serde(default)]", 0),
        ("#[expect(dead_code)]", 0),
        ("#[derive(Debug)]\n#[expect(dead_code)]", 0),
    ] {
        let repo = fixture(&format!("{attribute}\n{selected}\n"));
        let result = run(
            &repo,
            enabled(request(&repo, json!([entry(&repo, selected, existing())]))),
        );
        withheld(&result);
        assert!(conditional(&result), "{attribute}: {result}");
        assert_eq!(proofs(&result, "Debug"), count, "{attribute}: {result}");
        if count != 0 {
            assert_eq!(result["coverage"]["standard_builtin_derive"], count);
        }
    }
}

#[test]
fn builtin_derive_fixed_table_comments_and_type_bridge_share_audit() {
    let selected = "struct Record { value: Option<u8> }";
    let attributes = "#[derive(Debug /* comma, :: */, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default,)]";
    let repo = fixture(&format!("{attributes}\n{selected}\n"));
    let result = run(
        &repo,
        enabled(request(&repo, json!([entry(&repo, selected, existing())]))),
    );
    assert_eq!(result["coverage"]["standard_builtin_derive"], 9);
    assert_eq!(result["coverage"]["standard_prelude"], 1);
    assert_eq!(
        result["plan"]["binding_proofs"].as_array().unwrap().len(),
        10
    );
    apply(&repo, &result);
    repo.write("cases/layout/destination.rs", "struct Debug;\n");
    let result = run(
        &repo,
        enabled(request(&repo, json!([entry(&repo, selected, existing())]))),
    );
    withheld(&result);
    assert_eq!(proofs(&result, "Debug"), 0);
    assert_eq!(proofs(&result, "Option"), 0);
    assert_eq!(result["coverage"]["standard_builtin_derive"], 8);
}

#[test]
fn builtin_derive_scoped_children_do_not_shadow_parent_but_enclosing_scopes_do() {
    let selected = "struct Record { value: u8 }";
    for path in ["source.rs", "destination.rs", "lib.rs"] {
        let repo = fixture(&format!("#[derive(Debug)]\n{selected}\n"));
        let file = format!("cases/layout/{path}");
        let old = fs::read_to_string(repo.0.join(&file)).unwrap();
        repo.write(
            &file,
            &format!("{old}\n#[cfg(test)] mod tests {{ use external::*; struct Debug; }}"),
        );
        let result = run(
            &repo,
            enabled(request(&repo, json!([entry(&repo, selected, existing())]))),
        );
        assert_eq!(proofs(&result, "Debug"), 1, "{result}");
        apply(&repo, &result);
    }
    for shadow in [
        "",
        "use external::Debug;",
        "use external::*;",
        "struct Debug;",
    ] {
        let selected = format!(
            "fn selected() {{ mod enclosing {{ {shadow} #[derive(Debug)] struct Record; }} }}"
        );
        let repo = fixture(&selected);
        let result = run(
            &repo,
            enabled(request(&repo, json!([entry(&repo, &selected, existing())]))),
        );
        assert_eq!(
            proofs(&result, "Debug"),
            usize::from(shadow.is_empty()),
            "{result}"
        );
        if !shadow.is_empty() {
            withheld(&result);
        }
    }
}

#[test]
fn strict_attribute_classifier_parity_in_advice_move_and_required_binding() {
    for (attribute, veto) in [
        ("#[allow(dead_code)]", false),
        ("#[inline]", false),
        ("#[inline(always)]", false),
        ("#[inline(never)]", false),
        ("#[repr(C)]", false),
        ("#[derive(Debug)]", true),
        ("#[derive(Args)]", true),
        ("#[expect(dead_code)]", true),
        ("#[serde(default)]", true),
        ("#[cfg(any())]", true),
    ] {
        let record = "struct Record { value: u8 }";
        let selected = "fn selected(value: Record) {}";
        let repo = fixture(&format!("{attribute}\n{record}\n{selected}\n"));
        let moved = run(
            &repo,
            request(&repo, json!([entry(&repo, record, existing())])),
        );
        assert_eq!(conditional(&moved), veto, "move: {attribute}: {moved}");
        let binding = run(
            &repo,
            request(&repo, json!([entry(&repo, selected, existing())])),
        );
        assert_eq!(
            conditional(&binding),
            veto,
            "binding: {attribute}: {binding}"
        );
        let args = json!({"repo_path":repo.0,"crate_root":"cases/layout/lib.rs","source_path":"cases/layout/source.rs","paths":["cases/layout"],"limits":{"diagnostic_count":100_000}});
        let before = observe(&repo.0);
        let advice = Engine::new(repo.0.clone()).unwrap().suggest_split(
            serde_json::from_value(args).unwrap(),
            &AtomicBool::new(false),
        );
        assert_eq!(observe(&repo.0), before);
        let advice = serde_json::to_value(advice).unwrap();
        let conditional = advice["decisions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["reason"] == "conditional_or_inherited_context");
        assert_eq!(conditional, veto, "advice: {attribute}: {advice}");
    }
}

#[test]
fn builtin_derive_context_proofs_deduplicate_and_counts_survive_fitting() {
    let a = "struct First { value: Option<u8> }";
    let b = "struct Second { value: Option<u8> }";
    let repo = fixture(&format!("#[derive(Debug)] struct Retained;\n{a}\n{b}\n"));
    let result = run(
        &repo,
        enabled(request(
            &repo,
            json!([entry(&repo, a, existing()), entry(&repo, b, existing())]),
        )),
    );
    assert_eq!(result["coverage"]["standard_builtin_derive"], 1);
    assert_eq!(result["coverage"]["standard_prelude"], 2);
    let proof = result["plan"]["binding_proofs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["class"] == "standard_builtin_derive")
        .unwrap();
    assert_eq!(proof["item_ids"].as_array().unwrap().len(), 2);
    apply(&repo, &result);
    let attributes = (0..100)
        .map(|_| "#[derive(Debug, Clone)]\n")
        .collect::<String>();
    let selected = "struct Record;";
    let repo = fixture(&format!("{attributes}{selected}"));
    let mut args = enabled(request(&repo, json!([entry(&repo, selected, existing())])));
    args["limits"]["response_bytes"] = json!(65_536);
    let result = run(&repo, args);
    withheld(&result);
    assert_eq!(result["coverage"]["standard_builtin_derive"], 200);
    assert_eq!(result["counts"]["omissions"]["binding_proofs"], 200);
}

#[test]
fn shared_attribute_predicate_preserves_inner_and_unfinished_binding_guards() {
    for helper in [
        "fn helper() { #![allow(dead_code)] }",
        "#[derive(Debug)] struct Helper;",
    ] {
        let selected = if helper.starts_with("fn") {
            "fn selected() { helper(); }"
        } else {
            "fn selected(value: Helper) {}"
        };
        let repo = fixture(&format!("{helper}\n{selected}\n"));
        let args = request(&repo, json!([entry(&repo, selected, existing())]));
        let off = run(&repo, args.clone());
        withheld(&off);
        assert!(conditional(&off));
        let on = run(&repo, enabled(args));
        withheld(&on);
        assert!(conditional(&on));
        assert_eq!(proofs(&on, "Debug"), 0);
    }
}

#[test]
fn builtin_derive_arriving_attributes_audit_their_original_source_identity() {
    let selected = "struct Record { value: Option<u8> }";
    let other = "struct Other;";
    for prefix in [
        "#[derive(Debug)]",
        "use external::Debug;\n#[derive(Debug)]",
        "#[derive(Args)]",
    ] {
        let repo = fixture(selected);
        repo.write(
            "cases/layout/lib.rs",
            "mod source;\nmod destination;\nmod other;\n",
        );
        repo.write("cases/layout/other.rs", &format!("{prefix}\n{other}\n"));
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
        let clean = prefix == "#[derive(Debug)]";
        assert_eq!(proofs(&result, "Debug"), usize::from(clean), "{result}");
        assert_eq!(
            proofs(&result, "Option"),
            usize::from(!prefix.starts_with("use ")),
            "{result}"
        );
        if clean {
            apply(&repo, &result);
        } else {
            withheld(&result);
        }
    }
}
