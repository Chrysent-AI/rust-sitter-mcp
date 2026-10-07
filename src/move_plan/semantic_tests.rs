use super::*;
use std::sync::atomic::Ordering;

#[test]
fn context_grammar_reports_false_unknown_and_parser_recovery_without_shortcuts() {
    let flag = AtomicBool::new(false);
    let controls = (Instant::now() + Duration::from_secs(30), &flag);
    let config: Configuration = serde_json::from_value(serde_json::json!({"crates":[{
        "name":"probe", "root_file":"lib.rs", "edition":"2024", "features":["enabled"],
        "cfg":[{"key":"declared","value":null}], "dependencies":[]
    }]}))
    .unwrap();
    for (attribute, admitted, reason) in [
        ("#[cfg(not(declared))]", true, "declared_configuration"),
        ("#[cfg(any())]", true, "declared_configuration"),
        ("#[cfg(all())]", true, "declared_configuration"),
        (
            "#[cfg(all(not(declared), missing))]",
            false,
            "undeclared_cfg_atom",
        ),
        (
            "#[cfg(not(declared, declared))]",
            false,
            "unsupported_cfg_predicate",
        ),
        ("#[cfg_attr(declared,)]", false, "unparseable_cfg_attr"),
        ("#[cfg()]", false, "unparseable_attribute"),
    ] {
        let text = format!("{attribute} struct Value;");
        let inputs =
            Inputs::new(BTreeMap::from([("lib.rs".into(), text)]), &config, controls).unwrap();
        let sema = Semantics::new(&inputs.db);
        let file = inputs.ids["lib.rs"];
        let root = sema.parse_guess_edition(file);
        let module = sema.file_to_module_defs(file).next().unwrap();
        let attr = root
            .syntax()
            .descendants()
            .find_map(ast::Attr::cast)
            .unwrap();
        assert_eq!(
            safe_attr(&inputs, &sema, module, &attr, FactClass::GeneratedItems),
            admitted,
            "{attribute}"
        );
        assert!(
            inputs.context.borrow().values().any(|e| e.reason == reason),
            "{attribute}: {:?}",
            inputs.context.borrow()
        );
        if attribute == "#[cfg(not(declared))]" || attribute == "#[cfg(any())]" {
            assert!(
                inputs
                    .context
                    .borrow()
                    .values()
                    .any(|e| e.value == Some(false))
            );
        }
    }
}

#[test]
fn same_spelled_variant_in_final_overlay_must_keep_parent_declaration_identity() {
    let flag = AtomicBool::new(false);
    let controls = (Instant::now() + Duration::from_secs(30), &flag);
    let config: Configuration = serde_json::from_value(serde_json::json!({"crates":[{
        "name":"probe", "root_file":"lib.rs", "edition":"2024", "features":[], "cfg":[], "dependencies":[]
    }]})).unwrap();
    let text = "mod a { pub enum Value { Unit } } mod b { pub enum Value { Unit } } use a::Value; fn selected() { let _ = Value::Unit; }";
    let old = Inputs::new(
        BTreeMap::from([("lib.rs".into(), text.into())]),
        &config,
        controls,
    )
    .unwrap();
    let new = Inputs::new(
        BTreeMap::from([(
            "lib.rs".into(),
            text.replace("use a::Value", "use b::Value"),
        )]),
        &config,
        controls,
    )
    .unwrap();
    let start = text.find("Value::Unit").unwrap();
    let anchor = SourceAnchor {
        path: "lib.rs".into(),
        range: range(start, start + "Value::Unit".len()),
        expected_text: "Value::Unit".into(),
    };
    let before = fact(&old, &anchor).unwrap();
    let after = fact(&new, &anchor).unwrap();
    assert_eq!(before.classification, "variant_path");
    assert_eq!(after.classification, "variant_path");
    assert_ne!(before.declaration, after.declaration);
    assert_ne!(before.original, after.original);
    let mut needs = vec![Need {
        path: anchor.path.clone(),
        range: anchor.range.clone(),
        item_ids: Vec::new(),
        reason: DecisionReason::ExternalOrMissingBinding,
        category: "test",
        message: "unproved".into(),
        attribute_range: None,
        choice_target: None,
        lexical_uncertainty: None,
        refusal_basis: Vec::new(),
    }];
    let coverage = ResolutionCoverage {
        decisions: 0,
        configuration: config,
        snapshot_id: String::new(),
        semantic_input_digest: String::new(),
        final_overlay_digest: String::new(),
        analyzer: ANALYZER.into(),
        statement: String::new(),
        omissions: Vec::new(),
        context_evaluations: Vec::new(),
    };
    assert!(
        evaluate(&old, &new, &mut needs, &[], &coverage, controls)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        needs[0].refusal_basis[0].class,
        "semantic_identity_unproved"
    );
}

#[test]
fn nominal_variant_facts_require_stable_written_bindings() {
    let flag = AtomicBool::new(false);
    let controls = (Instant::now() + Duration::from_secs(30), &flag);
    let config: Configuration = serde_json::from_value(serde_json::json!({"crates":[{
        "name":"probe", "root_file":"lib.rs", "edition":"2024", "features":[], "cfg":[], "dependencies":[]
    }]})).unwrap();
    for (context, occurrence, proved) in [
        (
            "enum A { V } use A::*; #[derive(Inject)] struct Other;",
            "V",
            false,
        ),
        (
            "enum A { V } use A::V; #[derive(Inject)] struct Other;",
            "V",
            true,
        ),
        (
            "enum A { V } #[cfg(any())] use A::V; use A::*; #[derive(Inject)] struct Other;",
            "V",
            false,
        ),
        (
            "enum A { V } #[cfg_attr(all(), cfg(any()))] use A::V; use A::*; #[derive(Inject)] struct Other;",
            "V",
            false,
        ),
        (
            "enum A { V } #[cfg(all())] use A::V; #[derive(Inject)] struct Other;",
            "V",
            true,
        ),
        (
            "enum A { V } #[cfg_attr(any(), cfg(any()))] use A::V; #[derive(Inject)] struct Other;",
            "V",
            true,
        ),
        ("enum A { V } #[derive(Inject)] struct Other;", "A::V", true),
        (
            "mod a { pub enum A { V } } use a::*; #[derive(Inject)] struct Other;",
            "A::V",
            false,
        ),
        (
            "mod a { pub enum A { V } } use a::{A}; #[derive(Inject)] struct Other;",
            "A::V",
            true,
        ),
        (
            "mod a { pub enum A { V } } mod route { pub use crate::a::*; #[derive(Inject)] struct Other; } use route::A;",
            "A::V",
            false,
        ),
        (
            "mod a { pub enum A { V } } mod route { pub use crate::a::A; #[derive(Inject)] struct Other; } use route::A;",
            "A::V",
            true,
        ),
    ] {
        let text = format!("{context} fn selected() {{ let _ = {occurrence}; }}");
        let at = text.rfind(occurrence).unwrap();
        let anchor = SourceAnchor {
            path: "lib.rs".into(),
            range: range(at, at + occurrence.len()),
            expected_text: occurrence.into(),
        };
        let inputs =
            Inputs::new(BTreeMap::from([("lib.rs".into(), text)]), &config, controls).unwrap();
        let resolved = fact(&inputs, &anchor);
        assert_eq!(resolved.is_some(), proved, "{context}: {occurrence}");
        if !proved {
            assert!(inputs.context.borrow().values().any(|e| e.kind == "binding"
                && e.status == "skipped"
                && e.reason == "stable_written_identity_unproved"
                && e.anchor == anchor));
        }
    }
}

#[test]
fn configured_extern_prelude_roots_never_gain_custom_derive_admission() {
    let flag = AtomicBool::new(false);
    let controls = (Instant::now() + Duration::from_secs(30), &flag);
    let config: Configuration = serde_json::from_value(serde_json::json!({"crates":[
        {"name":"probe", "root_file":"lib.rs", "edition":"2024", "features":[], "cfg":[], "dependencies":[{"name":"dep","crate_name":"dependency"}]},
        {"name":"dependency", "root_file":"dep.rs", "edition":"2024", "features":[], "cfg":[], "dependencies":[]}
    ]})).unwrap();
    for (context, proved) in [
        ("", true),
        ("#[derive(Inject)] struct Other;", false),
        (
            "mod routes { pub use dep as dep; #[derive(Inject)] struct Other; } use routes::*;",
            false,
        ),
    ] {
        let text = format!("{context} fn selected() {{ let _ = dep::A::V; }}");
        let at = text.find("dep::A::V").unwrap();
        let anchor = SourceAnchor {
            path: "lib.rs".into(),
            range: range(at, at + 9),
            expected_text: "dep::A::V".into(),
        };
        let inputs = Inputs::new(
            BTreeMap::from([
                ("lib.rs".into(), text),
                ("dep.rs".into(), "pub enum A { V }".into()),
            ]),
            &config,
            controls,
        )
        .unwrap();
        let resolved = fact(&inputs, &anchor);
        assert_eq!(resolved.is_some(), proved, "{context}");
        if let Some(fact) = resolved {
            assert!(fact.fact_class == FactClass::GeneratedItems);
            assert!(
                inputs
                    .context
                    .borrow()
                    .values()
                    .any(|e| e.reason == "configured_dependency_root_with_conservative_context")
            );
        }
    }
}

#[test]
fn final_overlay_glob_identity_does_not_pass_the_nominal_derive_gate() {
    let flag = AtomicBool::new(false);
    let controls = (Instant::now() + Duration::from_secs(30), &flag);
    let config: Configuration = serde_json::from_value(serde_json::json!({"crates":[{
        "name":"probe", "root_file":"lib.rs", "edition":"2024", "features":[], "cfg":[], "dependencies":[]
    }]})).unwrap();
    let text = "fn selected() { let _ = A::V; }";
    let make = |import: &str| {
        Inputs::new(
            BTreeMap::from([
                (
                    "lib.rs".into(),
                    "mod source; mod a { pub enum A { V } }".into(),
                ),
                (
                    "source.rs".into(),
                    format!("{text} {import} #[derive(Inject)] struct Other;"),
                ),
            ]),
            &config,
            controls,
        )
        .unwrap()
    };
    let old = make("use crate::a::A;");
    let new = make("use crate::a::*;");
    let at = text.find("A::V").unwrap();
    let mut needs = vec![Need {
        path: "source.rs".into(),
        range: range(at, at + 4),
        item_ids: Vec::new(),
        reason: DecisionReason::ExternalOrMissingBinding,
        category: "test",
        message: "unproved".into(),
        attribute_range: None,
        choice_target: None,
        lexical_uncertainty: None,
        refusal_basis: Vec::new(),
    }];
    let coverage = ResolutionCoverage {
        decisions: 0,
        configuration: config,
        snapshot_id: String::new(),
        semantic_input_digest: String::new(),
        final_overlay_digest: String::new(),
        analyzer: ANALYZER.into(),
        statement: String::new(),
        omissions: Vec::new(),
        context_evaluations: Vec::new(),
    };
    assert!(
        evaluate(&old, &new, &mut needs, &[], &coverage, controls)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        needs[0].refusal_basis[0].class,
        "semantic_final_fact_unproved"
    );
    assert!(
        new.context
            .borrow()
            .values()
            .any(|e| e.reason == "stable_written_identity_unproved")
    );
}

#[test]
fn controller_cancels_both_live_databases_and_joins() {
    let old = RootDatabase::default();
    let new = RootDatabase::default();
    let cancelled = AtomicBool::new(false);
    let (ready, started) = mpsc::channel();
    std::thread::scope(|threads| {
        let flag = &cancelled;
        let stop = threads.spawn(move || {
            started.recv_timeout(Duration::from_secs(2)).unwrap();
            flag.store(true, Ordering::Relaxed);
        });
        let outcome = controlled(
            &old,
            &new,
            (Instant::now() + Duration::from_secs(10), &cancelled),
            || {
                ready.send(()).unwrap();
                // Real Salsa cancellation unwinding, not an adapter preflight check.
                loop {
                    old.unwind_if_revision_cancelled();
                    new.unwind_if_revision_cancelled();
                    std::thread::yield_now();
                }
            },
        );
        stop.join().unwrap();
        assert_eq!(outcome.unwrap_err().code, "CANCELLED");
        assert!(old.cancellation_token().is_cancelled());
        assert!(new.cancellation_token().is_cancelled());
    });
}

#[test]
fn controller_deadline_is_reported_as_incomplete_planning_not_success() {
    let old = RootDatabase::default();
    let new = RootDatabase::default();
    let cancelled = AtomicBool::new(false);
    let outcome = controlled(
        &old,
        &new,
        (Instant::now() + Duration::from_millis(20), &cancelled),
        || {
            loop {
                new.unwind_if_revision_cancelled();
                std::thread::yield_now();
            }
        },
    );
    assert_eq!(outcome.unwrap_err().code, "planning_deadline");
}

#[test]
fn non_cancellation_panic_still_stops_and_joins_the_controller() {
    let old = RootDatabase::default();
    let new = RootDatabase::default();
    let cancelled = AtomicBool::new(false);
    let outcome = std::panic::catch_unwind(|| {
        controlled(
            &old,
            &new,
            (Instant::now() + Duration::from_secs(10), &cancelled),
            || {
                panic!("test programmer error");
            },
        )
    });
    assert!(outcome.is_err());
    assert!(!old.cancellation_token().is_cancelled());
    assert!(!new.cancellation_token().is_cancelled());
}

#[test]
fn failed_semantic_stages_disclose_only_the_stage_actually_reached() {
    let flag = AtomicBool::new(false);
    let controls = (Instant::now() + Duration::from_secs(30), &flag);
    let source = "fn called() {} fn selected() { called(); }";
    let config: Configuration = serde_json::from_value(serde_json::json!({"crates":[{
        "name":"probe", "root_file":"lib.rs", "edition":"2024", "features":[], "cfg":[], "dependencies":[]
    }]})).unwrap();
    let texts = |text: &str| BTreeMap::from([("lib.rs".into(), text.into())]);
    let old = Inputs::new(texts(source), &config, controls).unwrap();
    let coverage = ResolutionCoverage {
        decisions: 0,
        configuration: config.clone(),
        snapshot_id: String::new(),
        semantic_input_digest: String::new(),
        final_overlay_digest: String::new(),
        analyzer: ANALYZER.into(),
        statement: String::new(),
        omissions: Vec::new(),
        context_evaluations: Vec::new(),
    };
    let start = source.rfind("called").unwrap();
    let need = Need {
        attribute_range: None,
        reason: DecisionReason::ExternalOrMissingBinding,
        choice_target: None,
        lexical_uncertainty: None,
        refusal_basis: Vec::new(),
        category: "test",
        path: "lib.rs".into(),
        range: range(start, start + 6),
        message: "unproved".into(),
        item_ids: Vec::new(),
    };
    for (final_text, class) in [
        (
            "fn called() {} fn selected() { other_(); }",
            "semantic_mapping_unproved",
        ),
        (
            "fn absent() {} fn selected() { called(); }",
            "semantic_final_fact_unproved",
        ),
        (
            "fn absent() {} fn selected() { called(); } fn called() {}",
            "semantic_identity_unproved",
        ),
    ] {
        let new = Inputs::new(texts(final_text), &config, controls).unwrap();
        let mut needs = vec![need.clone()];
        assert!(
            evaluate(&old, &new, &mut needs, &[], &coverage, controls)
                .unwrap()
                .is_empty()
        );
        assert_eq!(needs.len(), 1);
        assert_eq!(needs[0].refusal_basis[0].class, class);
        assert_eq!(
            needs[0].refusal_basis[0].anchor.range,
            Some(need.range.clone())
        );
    }
    let absent = Inputs::new(
        texts("fn absent() {} fn selected() { called(); }"),
        &config,
        controls,
    )
    .unwrap();
    let mut needs = vec![need.clone()];
    assert!(
        evaluate(&absent, &old, &mut needs, &[], &coverage, controls)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        needs[0].refusal_basis[0].class,
        "semantic_source_fact_unproved"
    );
    let mut needs = vec![need.clone()];
    configuration_refusal(&mut needs);
    assert_eq!(
        needs[0].refusal_basis[0].class,
        "semantic_configuration_unproved"
    );
    let mut need = need;
    need.refuse_at_occurrence("semantic_overlay_unavailable");
    assert_eq!(need.refusal_basis[0].class, "semantic_overlay_unavailable");
    assert_eq!(need.refusal_basis[0].anchor.range, Some(need.range.clone()));
}
