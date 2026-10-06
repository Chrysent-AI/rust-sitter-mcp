use super::*;
use std::time::Duration;

fn assess(source: &str, name: &str, proven_import: bool) -> LexicalAssessment {
    let cancelled = AtomicBool::new(false);
    let controls = (Instant::now() + Duration::from_secs(5), &cancelled);
    let tree = crate::trivia::parse(source, controls.0, controls.1)
        .unwrap()
        .unwrap();
    assert!(
        !tree.root_node().has_error(),
        "{}",
        tree.root_node().to_sexp()
    );
    let node = source
        .rmatch_indices(name)
        .find_map(|(at, _)| {
            let node = tree
                .root_node()
                .named_descendant_for_byte_range(at, at + name.len())?;
            crate::items::reference_role(node).then_some(node)
        })
        .unwrap_or_else(|| panic!("queried reference, not a pattern or declaration: {source}"));
    lexical_assessment("probe.rs", node, source, name, controls, proven_import).unwrap()
}
#[test]
fn disjoint_binding_positions_and_scope_boundaries() {
    for source in [
        "fn f() { let (x, mut y) = pair; selected(); }",
        "fn f() { let [x, y] = pair; selected(); }",
        "fn f() { let C(x, y) = pair; selected(); }",
        "fn f() { let C { selected: other, x, .. } = pair; selected(); }",
        "fn f() { for other in selected() {} }",
        "fn f() { for other in values { selected(); } }",
        "fn f() { if let Some(other) = values { selected(); } }",
        "fn f() { while let Some(other) = values { selected(); } }",
        "fn f() { match values { Some(other) if selected() => {}, _ => {} } }",
        "fn f() { match values { -1 => selected(), _ => {} } }",
        "fn f() { match values { true => selected(), _ => {} } }",
        "fn f() { match values { r#\"literal\"# => selected(), _ => {} } }",
        "fn f() { let other @ _ = value; selected(); }",
        "fn f() { match values { Some(event_type @ (\"message_start\" | \"message_end\" | \"turn_end\")) => selected(), _ => {} } }",
        "fn f() { match values { other @ (1 | 2 | 3) => selected(), _ => {} } }",
        "fn f() { match values { | 1 | 2 => selected(), _ => {} } }",
        "fn f() { match values { other @ (r#\"start\"# | \"stop\") => selected(), _ => {} } }",
        "fn f() { match values { other @ (1 /* alternate */ | 2) if selected() => {}, _ => {} } }",
        "fn f() { match selected() { selected @ (1 | 2) => {}, _ => {} } }",
        "fn f() { match values { selected @ (1 | 2) => {}, _ => selected() } }",
        "fn f() { let (&other, _) = values; selected(); }",
        "fn f() { match selected() { Some(other) => {}, _ => {} } }",
        "fn f() { match values { Some(selected) => {}, _ => selected() } }",
        "fn f() { if let Some(selected) = value {} else { selected(); } }",
        "fn f() { let selected = selected(); }",
        "fn f() { if let Some(selected) = selected() {} }",
        "fn f() { while let Some(selected) = selected() {} }",
        "fn f() { for selected in selected() {} }",
        "fn f() { let _ = |(x, y)| selected(); }",
        "fn f() { let _ = async move |x| selected(); }",
    ] {
        let assessment = assess(source, "selected", false);
        assert_eq!(
            assessment.binding,
            LexicalBinding::Absent,
            "{source}: {:?}",
            assessment.uncertainty
        );
    }
    // Later/nested let patterns do not affect a preceding sibling occurrence.
    let source = "fn f() { selected(); let (selected,) = value; { let (selected,) = value; } }";
    let cancelled = AtomicBool::new(false);
    let controls = (Instant::now() + Duration::from_secs(5), &cancelled);
    let tree = crate::trivia::parse(source, controls.0, controls.1)
        .unwrap()
        .unwrap();
    let at = source.find("selected()").unwrap();
    let node = tree
        .root_node()
        .named_descendant_for_byte_range(at, at + 8)
        .unwrap();
    assert_eq!(
        lexical_assessment("probe.rs", node, source, "selected", controls, false)
            .unwrap()
            .binding,
        LexicalBinding::Absent
    );
}
#[test]
fn forced_and_existing_simple_binders_are_independent() {
    for source in [
        "fn f(selected: fn()) { selected(); }",
        "fn f() { let selected = value; selected(); }",
        "fn f() { let (mut selected,) = value; selected(); }",
        "fn f() { let (ref selected,) = value; selected(); }",
        "fn f() { let C { selected } = value; selected(); }",
        "fn f() { let _ = |selected: fn()| selected(); }",
        "fn f() { let _ = move |selected| selected(); }",
        "fn f() { for mut selected in value { selected(); } }",
        "fn f() { if let Some(ref selected) = value { selected(); } }",
        "fn f() { while let Some(mut selected) = value { selected(); } }",
        "fn f() { match value { Some(ref selected) => selected(), _ => {} } }",
        "fn f<selected>() { let x: selected; }",
        "fn f<const selected: usize>() { let x = selected; }",
        "fn f() { selected(); fn selected() {} }",
        "fn f() { selected(); const selected: usize = 1; }",
        "fn f() { let (mut r#selected,) = value; r#selected(); }",
        "fn f() { let selected @ _ = value; selected(); }",
        "fn f() { match value { Some(selected @ (\"start\" | \"stop\")) => selected, _ => {} } }",
        "fn f() { match value { selected @ (1 | 2) if selected > 0 => selected, _ => {} } }",
        "fn f() { match value { r#selected @ (1 | 2) => r#selected, _ => {} } }",
        "fn f() { match value { selected /* capture */ @ (1 | 2) => selected, _ => {} } }",
        "fn f() { match value { other @ Some(selected @ (1 | 2)) => selected, _ => {} } }",
        "fn f() { match value { other @ Some(ref selected) => selected(), _ => {} } }",
    ] {
        assert_eq!(
            assess(source, "selected", false).binding,
            LexicalBinding::Independent,
            "{source}"
        );
    }
}
#[test]
fn value_bindings_do_not_prove_type_references() {
    for source in [
        "fn f(selected: u8) { let _: selected = 2; }",
        "fn f() { let selected = 1; let _: selected = 2; }",
        "fn f() { let (mut selected,) = (1,); let _: selected = 2; }",
        "fn f() { let (ref selected,) = (1,); let _: selected = 2; }",
        "fn f() { let C { selected } = value; let _: selected = 2; }",
        "fn f() { let _ = |selected: u8| { let _: selected = 2; }; }",
        "fn f() { for mut selected in value { let _: selected = 2; } }",
        "fn f() { if let Some(ref selected) = value { let _: selected = 2; } }",
        "fn f() { while let Some(mut selected) = value { let _: selected = 2; } }",
        "fn f() { match value { Some(ref selected) => { let _: selected = 2; }, _ => {} } }",
        "fn f() { match value { selected @ (1 | 2) => { let _: selected = 2; }, _ => {} } }",
        "fn f<const selected: usize>() { let _: selected = 2; }",
        "fn f() { const selected: u8 = 1; let _: selected = 2; }",
        "fn f() { static selected: u8 = 1; let _: selected = 2; }",
        "fn f() { fn selected() {} let _: selected = 2; }",
    ] {
        let result = assess(source, "selected", false);
        assert_eq!(result.binding, LexicalBinding::Uncertain, "{source}");
        let witness = result.uncertainty.unwrap();
        assert_eq!(witness.reason, LexicalReason::ValueBindingInTypePosition);
        assert_eq!(witness.spelling, "selected");
        assert_eq!(witness.scope.path, "probe.rs");
        let pattern = witness.pattern.unwrap();
        assert!(source[pattern.range.start_byte..pattern.range.end_byte].contains("selected"));
    }
}
#[test]
fn genuine_unknowns_veto_outer_proofs_with_precise_witnesses() {
    for (source, reason) in [
        (
            "fn f(selected: fn()) { const selected: u8 = 1; let (selected,) = value; selected(); }",
            LexicalReason::IdentifierPatternBindingOrConstant,
        ),
        (
            "fn f(selected: fn()) { for selected in value { selected(); } }",
            LexicalReason::IdentifierPatternBindingOrConstant,
        ),
        (
            "fn f() { const selected: u8 = 1; if let Some(selected) = value { selected(); } }",
            LexicalReason::IdentifierPatternBindingOrConstant,
        ),
        (
            "fn f() { const selected: u8 = 1; while let Some(selected) = value { selected(); } }",
            LexicalReason::IdentifierPatternBindingOrConstant,
        ),
        (
            "fn f() { const selected: u8 = 1; match value { Some(selected) => selected(), _ => {} } }",
            LexicalReason::IdentifierPatternBindingOrConstant,
        ),
        (
            "fn f() { let (mut selected, p!()) = value; selected(); }",
            LexicalReason::UnsupportedPattern,
        ),
        (
            "fn f() { let x @ p!() = value; selected(); }",
            LexicalReason::UnsupportedPattern,
        ),
        (
            "fn f() { const selected: u8 = 1; match value { other @ Some(selected) => selected(), _ => {} } }",
            LexicalReason::IdentifierPatternBindingOrConstant,
        ),
        (
            "fn f() { const selected: u8 = 1; match value { selected @ Some(selected) => selected(), _ => {} } }",
            LexicalReason::IdentifierPatternBindingOrConstant,
        ),
        (
            "fn f() { match value { selected @ p!() => selected(), _ => {} } }",
            LexicalReason::UnsupportedPattern,
        ),
        (
            "fn f(selected: fn()) { match value { other @ p!() => selected(), _ => {} } }",
            LexicalReason::UnsupportedPattern,
        ),
        (
            "fn f() { match value { selected @ 1 | 2 => selected(), _ => {} } }",
            LexicalReason::UnsupportedPattern,
        ),
        (
            "fn f() { match value { other @ (\"start\" | selected) => selected(), _ => {} } }",
            LexicalReason::UnsupportedPattern,
        ),
        (
            "fn f() { match value { other @ (A(mut selected) | B(mut selected)) => selected(), _ => {} } }",
            LexicalReason::UnsupportedPattern,
        ),
        (
            "fn f() { match value { selected @ (1 | 2) if let Some(x) = value && x > 0 => selected(), _ => {} } }",
            LexicalReason::UnsupportedPattern,
        ),
        (
            "fn f() { match value { #[cfg(any())] selected @ (1 | 2) => selected(), _ => {} } }",
            LexicalReason::ConditionalLocalContext,
        ),
        (
            "fn f() { match value { const { 1 } => selected(), _ => {} } }",
            LexicalReason::UnsupportedPattern,
        ),
        (
            "fn f() { if let A(x) | B(x) = value { selected(); } }",
            LexicalReason::UnsupportedPattern,
        ),
        (
            "fn f() { if let Some(x) = value && x > 0 { selected(); } }",
            LexicalReason::UnsupportedPattern,
        ),
        (
            "fn f() { m!(); selected(); }",
            LexicalReason::UnsupportedPattern,
        ),
        (
            "fn f() { m!(); selected(); m!(); }",
            LexicalReason::UnsupportedPattern,
        ),
        (
            "fn f() { m!(); let selected = value; selected(); }",
            LexicalReason::UnsupportedPattern,
        ),
        (
            "fn f() { let selected = value; m!(); selected(); }",
            LexicalReason::UnsupportedPattern,
        ),
        (
            "fn f() { #[cfg(any())] let other = value; selected(); }",
            LexicalReason::ConditionalLocalContext,
        ),
        (
            "fn f() { match value { #[cfg(any())] Some(other) => selected(), _ => {} } }",
            LexicalReason::ConditionalLocalContext,
        ),
        (
            "fn f() { use crate::other as selected; selected(); }",
            LexicalReason::RelevantLocalImport,
        ),
        (
            "fn f() { use crate::other::*; selected(); }",
            LexicalReason::RelevantLocalImport,
        ),
    ] {
        let result = assess(source, "selected", false);
        assert_eq!(result.binding, LexicalBinding::Uncertain, "{source}");
        let witness = result.uncertainty.unwrap();
        assert_eq!(witness.reason, reason, "{source}");
        assert_eq!(witness.scope.path, "probe.rs");
        let pattern = witness.pattern.unwrap();
        assert!(pattern.range.end_byte <= source.len());
        assert!(pattern.range.start_byte >= witness.scope.range.start_byte);
        assert!(pattern.range.end_byte <= witness.scope.range.end_byte);
    }
    assert_eq!(
        assess(
            "fn f() { use crate::other as selected; selected(); }",
            "selected",
            true
        )
        .binding,
        LexicalBinding::Absent
    );
    assert_eq!(
        assess(
            "fn f() { use crate::other::*; selected(); }",
            "selected",
            true
        )
        .binding,
        LexicalBinding::Uncertain
    );
}
#[test]
fn statement_macro_witness_is_disclosed_as_same_block_not_containing_pattern() {
    for source in [
        "fn f() { m!(); let n = selected; }",
        "fn f() { m!(); { let n = selected; } }",
    ] {
        let assessment = assess(source, "selected", false);
        assert_eq!(assessment.binding, LexicalBinding::Uncertain);
        let witness = serde_json::to_value(assessment.uncertainty.unwrap()).unwrap();
        assert_eq!(
            witness["witness_relation"],
            "statement_macro_before_reference"
        );
        assert_eq!(witness["scope"]["kind"], "block");
        let at = source.find("selected").unwrap();
        let range = &witness["pattern"]["range"];
        let start = range["start_byte"].as_u64().unwrap() as usize;
        let end = range["end_byte"].as_u64().unwrap() as usize;
        assert!(end <= at);
        assert_eq!(&source[start..end], "m!();");
    }
    let assessment = assess(
        "fn f() { const selected: u8 = 1; let (selected,) = pair; selected(); }",
        "selected",
        false,
    );
    let witness = serde_json::to_value(assessment.uncertainty.unwrap()).unwrap();
    assert!(witness.get("witness_relation").is_none());
}
#[test]
fn sibling_and_body_macros_do_not_poison_outside_references() {
    for (source, expected) in [
        (
            "fn f() { { m!(); } let n = selected; }",
            LexicalBinding::Absent,
        ),
        ("fn f(value: selected) { m!(); }", LexicalBinding::Absent),
    ] {
        let assessment = assess(source, "selected", false);
        assert_eq!(assessment.binding, expected, "{source}");
        assert!(assessment.uncertainty.is_none(), "{source}");
    }
}
#[test]
fn later_block_macros_veto_including_enclosing_blocks_and_nested_items() {
    for (source, macro_text, kind) in [
        (
            "fn f() { let n: selected; m!(); }",
            "m!();",
            "expression_statement",
        ),
        (
            "fn f() { let n: selected; m!{} }",
            "m!{}",
            "macro_invocation",
        ),
        (
            "fn f() { { let n: selected; } m!(); }",
            "m!();",
            "expression_statement",
        ),
        (
            "fn f() { fn inner(_: selected) {} m!(); }",
            "m!();",
            "expression_statement",
        ),
        (
            "fn f() { { fn inner(_: selected) {} } m!(); }",
            "m!();",
            "expression_statement",
        ),
        (
            "fn f() { let selected = value; selected(); m!(); }",
            "m!();",
            "expression_statement",
        ),
    ] {
        let assessment = assess(source, "selected", false);
        assert_eq!(assessment.binding, LexicalBinding::Uncertain, "{source}");
        let witness = assessment.uncertainty.unwrap();
        assert_eq!(
            witness.witness_relation,
            Some(WitnessRelation::BlockMacroMayIntroduceItems)
        );
        let pattern = witness.pattern.unwrap();
        assert_eq!(pattern.kind, kind, "{source}");
        assert_eq!(
            &source[pattern.range.start_byte..pattern.range.end_byte],
            macro_text
        );
        assert!(pattern.range.start_byte > source.find("selected").unwrap());
        assert_eq!(witness.scope.kind, "block");
    }
}
#[test]
fn unexamined_block_attributes_veto_before_local_proofs_and_item_boundaries() {
    for source in [
        "fn f() { let _: selected; #[inject] fn unrelated() {} }",
        "fn f() { { let _: selected; } #[inject] fn unrelated() {} }",
        "fn f() { fn inner(_: selected) {} #[inject] fn unrelated() {} }",
        "fn f() { { fn inner(_: selected) {} } #[inject] fn unrelated() {} }",
        "fn f<selected>() { let _: selected; #[inject] fn unrelated() {} }",
        "fn f() { #[inject] #[inline] fn unrelated() {} let _: selected; }",
        "fn f() { let _: selected; #[inject]\n/// inert docs\nfn unrelated() {} }",
        "fn f() { let _: selected; #[inject] impl Other {} }",
    ] {
        let assessment = assess(source, "selected", false);
        assert_eq!(assessment.binding, LexicalBinding::Uncertain, "{source}");
        let witness = assessment.uncertainty.unwrap();
        assert_eq!(witness.reason, LexicalReason::ConditionalLocalContext);
        assert_eq!(witness.scope.kind, "block");
        assert!(witness.witness_relation.is_none());
        let attribute = witness.pattern.unwrap();
        assert_eq!(attribute.kind, "attribute_item");
        assert_eq!(
            &source[attribute.range.start_byte..attribute.range.end_byte],
            "#[inject]"
        );

        // Ignoring standalone macro risk must not ignore attribute uncertainty.
        let flag = AtomicBool::new(false);
        let controls = (Instant::now() + Duration::from_secs(5), &flag);
        let tree = crate::trivia::parse(source, controls.0, controls.1)
            .unwrap()
            .unwrap();
        let at = source.rfind("selected").unwrap();
        let node = tree
            .root_node()
            .named_descendant_for_byte_range(at, at + "selected".len())
            .unwrap();
        assert_eq!(
            test_consumer_binding("probe.rs", node, source, "selected", controls).unwrap(),
            LexicalBinding::Uncertain,
            "{source}"
        );
    }
}
#[test]
fn block_attribute_inertness_uses_the_strict_classifier_and_scope() {
    for source in [
        "fn f() { let _: selected; /// inert docs\nfn unrelated() {} }",
        "fn f() { let _: selected; /** inert docs */ fn unrelated() {} }",
        "fn f() { let _: selected; #[allow(dead_code)] fn unrelated() {} }",
        "fn f() { let _: selected; #[inline] fn unrelated() {} }",
        "fn f() { let _: selected; #[inline(always)] fn unrelated() {} }",
        "fn f() { let _: selected; #[inline(never)] fn unrelated() {} }",
        "fn f() { let _: selected; #[repr(C)] struct Other; }",
        "fn f() { let _: selected; { #[inject] fn unrelated() {} } }",
        "fn f(_: selected) { #[inject] fn unrelated() {} }",
    ] {
        assert_eq!(
            assess(source, "selected", false).binding,
            LexicalBinding::Absent,
            "{source}"
        );
    }
    // These spellings are not inert under the existing strict predicate.
    // In particular, a derive spelling alone is not proof of built-in identity.
    for attribute in [
        "#[derive(Debug)]",
        "#[derive(custom::Debug)]",
        "#[cfg_attr(test, inject)]",
        "#[doc = \"docs\"]",
        "#[expect(dead_code)]",
        "#[custom::inline]",
    ] {
        let source = format!("fn f() {{ let _: selected; {attribute} struct Other; }}");
        let assessment = assess(&source, "selected", false);
        assert_eq!(assessment.binding, LexicalBinding::Uncertain, "{source}");
        let witness = assessment.uncertainty.unwrap();
        let pattern = witness.pattern.unwrap();
        assert_eq!(
            &source[pattern.range.start_byte..pattern.range.end_byte],
            attribute
        );
    }
}
#[test]
fn definite_bindings_retain_declaration_coordinates() {
    for (source, declaration) in [
        ("fn f<selected: Copy>(input: selected) {}", "selected: Copy"),
        (
            "fn f() { struct selected; let _: selected; }",
            "struct selected;",
        ),
        ("fn f() { let selected = value; selected(); }", "selected"),
    ] {
        let assessment = assess(source, "selected", false);
        assert_eq!(assessment.binding, LexicalBinding::Independent);
        let binding = assessment.definite_binding.unwrap();
        assert_eq!(
            &source[binding.range.start_byte..binding.range.end_byte],
            declaration
        );
        assert!(binding.range.end_byte <= source.rfind("selected").unwrap());
    }
}
#[test]
fn syntax_recovery_is_not_a_disjointness_proof() {
    for source in [
        "fn f() { @ selected(); }",
        "fn f() { match value { selected @ => selected(), _ => {} } }",
        "fn f() { match value { selected @ (1 | ) => selected(), _ => {} } }",
    ] {
        let cancelled = AtomicBool::new(false);
        let controls = (Instant::now() + Duration::from_secs(5), &cancelled);
        let tree = crate::trivia::parse(source, controls.0, controls.1)
            .unwrap()
            .unwrap();
        assert!(tree.root_node().has_error(), "{source}");
        let at = source.find("selected()").unwrap();
        let node = tree
            .root_node()
            .named_descendant_for_byte_range(at, at + 8)
            .unwrap();
        let assessment =
            lexical_assessment("probe.rs", node, source, "selected", controls, false).unwrap();
        assert_eq!(assessment.binding, LexicalBinding::Uncertain, "{source}");
        assert_eq!(
            assessment.uncertainty.unwrap().reason,
            LexicalReason::SyntaxRecovery,
            "{source}"
        );
    }
}
#[test]
fn nested_item_capture_context_does_not_inherit_outer_local_proof() {
    let result = assess(
        "fn f(selected: fn()) { fn inner() { selected(); } }",
        "selected",
        false,
    );
    assert_eq!(result.binding, LexicalBinding::Uncertain);
    let witness = result.uncertainty.unwrap();
    assert_eq!(witness.reason, LexicalReason::ConditionalLocalContext);
    assert_eq!(witness.scope.kind, "function_item");
    assert!(witness.pattern.is_none());
}
#[test]
fn lexical_refusal_basis_keeps_the_actual_positional_witness() {
    for (source, class, expected) in [
        (
            "fn f() { m!(); selected(); }",
            "chain_macro_statement",
            "m!();",
        ),
        (
            "fn f() { const selected: u8 = 1; let (selected,) = value; selected(); }",
            "lexical_uncertainty",
            "selected",
        ),
        (
            "fn f() { selected(); m!(); }",
            "chain_macro_statement",
            "m!();",
        ),
        (
            "fn f() { selected(); #[inject] fn unrelated() {} }",
            "lexical_uncertainty",
            "#[inject]",
        ),
    ] {
        let flag = AtomicBool::new(false);
        let controls = (Instant::now() + Duration::from_secs(5), &flag);
        let tree = crate::trivia::parse(source, controls.0, controls.1)
            .unwrap()
            .unwrap();
        let mut need = crate::items::need(
            crate::items::DecisionReason::LexicalContextUnproved,
            "binding_collision",
            "probe.rs",
            tree.root_node(),
            "unproved",
        );
        need.lexical(assess(source, "selected", false));
        need.disclose_refusal();
        let basis = &need.refusal_basis[0];
        assert_eq!(basis.class, class);
        assert_eq!(basis.name.as_deref(), Some("selected"));
        let range = basis.anchor.range.as_ref().unwrap();
        assert_eq!(&source[range.start_byte..range.end_byte], expected);
        need.disclose_refusal();
        assert_eq!(need.refusal_basis.len(), 1);
    }
}
#[test]
fn pattern_arguments_bind_without_spelling_heuristics() {
    for pattern in [
        "(selected,)",
        "[selected]",
        "[Effect::Record(selected)]",
        "Effect::Record(selected)",
        "Record { field: selected }",
        "(&selected,)",
        "(r#selected,)",
    ] {
        for source in [
            format!("fn f() {{ let {pattern} = value else {{ return; }}; selected; }}"),
            format!("fn f() {{ match value {{ {pattern} if selected => selected, _ => false }} }}"),
        ] {
            assert_eq!(
                assess(&source, "selected", false).binding,
                LexicalBinding::Independent,
                "{source}"
            );
        }
    }
    assert_eq!(
        assess(
            "fn f() { let [UPPERCASE] = value; UPPERCASE; }",
            "UPPERCASE",
            false
        )
        .binding,
        LexicalBinding::Independent
    );
}
#[test]
fn written_pattern_competitors_are_hoisted_and_module_scoped() {
    for evidence in [
        "const selected: u8 = 1;",
        "enum Visible { selected, Payload(u8) }",
        "enum Visible { selected = 1 }",
        "use external::selected;",
        "use external::Other as selected;",
        "use external::*;",
        "struct selected;",
    ] {
        for source in [
            format!("{evidence} fn f() {{ let [selected] = value; selected; }}"),
            format!("fn f() {{ let [selected] = value; {evidence} selected; }}"),
            format!("fn f() {{ {evidence} {{ let Some(selected) = value; selected; }} }}"),
        ] {
            let assessment = assess(&source, "selected", true);
            assert_eq!(assessment.binding, LexicalBinding::Uncertain, "{source}");
            assert_eq!(
                assessment.uncertainty.unwrap().reason,
                LexicalReason::IdentifierPatternBindingOrConstant
            );
        }
    }
    for source in [
        "use external::*; mod child { fn f() { let [selected] = value; selected; } }",
        "enum Outer { selected } mod child { fn f() { let [selected] = value; selected; } }",
        "fn unrelated() { const selected: u8 = 1; } fn f() { let [selected] = value; selected; }",
        "enum Visible { selected(u8) } fn f() { let [selected] = value; selected; }",
        "fn f() { { use external::*; } let [selected] = value; selected; }",
    ] {
        assert_eq!(
            assess(source, "selected", false).binding,
            LexicalBinding::Independent,
            "{source}"
        );
    }
}
#[test]
fn scoped_paths_are_disjoint_and_let_else_keeps_failure_outside_binding_scope() {
    for source in [
        "fn f() { match value { Outcome::Pass => selected(), _ => {} } }",
        "fn f() { match value { Outcome::Pass if selected() => {}, _ => {} } }",
        "fn f() { match value { Outcome::Pass | Outcome::Fail => selected(), _ => {} } }",
        "fn f() { match value { true | false => selected(), _ => {} } }",
        "fn f() { match value { Outcome::selected => selected(), _ => {} } }",
        "fn f() { let [Effect::Record(selected)] = selected() else { return; }; }",
        "fn f() { let [Effect::Record(selected)] = value else { selected(); return; }; }",
        "fn f() { match value { Outcome::Pass => { selected(); }, Outcome::Record(selected) => {} } }",
    ] {
        assert_eq!(
            assess(source, "selected", false).binding,
            LexicalBinding::Absent,
            "{source}"
        );
    }
    // The failure block still sees a prior successful binding of the same name.
    assert_eq!(
        assess(
            "fn f(selected: fn()) { let [selected] = value else { selected(); return; }; }",
            "selected",
            false
        )
        .binding,
        LexicalBinding::Independent
    );
    // The constructor path is not in the match-arm binder's scope.
    let source = "fn f() { match value { selected::Record(selected) => {} } }";
    let flag = AtomicBool::new(false);
    let controls = (Instant::now() + Duration::from_secs(5), &flag);
    let tree = crate::trivia::parse(source, controls.0, controls.1)
        .unwrap()
        .unwrap();
    let at = source.find("selected::").unwrap();
    let node = tree
        .root_node()
        .named_descendant_for_byte_range(at, at + 8)
        .unwrap();
    assert_eq!(
        lexical_assessment("probe.rs", node, source, "selected", controls, false)
            .unwrap()
            .binding,
        LexicalBinding::Absent
    );
}
#[test]
fn interrupted_work_never_supplies_positive_proof() {
    let source = "fn f() { for x in values { selected(); } }";
    let flag = AtomicBool::new(false);
    let tree = crate::trivia::parse(source, Instant::now() + Duration::from_secs(5), &flag)
        .unwrap()
        .unwrap();
    let at = source.find("selected()").unwrap();
    let node = tree
        .root_node()
        .named_descendant_for_byte_range(at, at + 8)
        .unwrap();
    flag.store(true, std::sync::atomic::Ordering::Relaxed);
    assert_eq!(
        lexical_assessment(
            "probe.rs",
            node,
            source,
            "selected",
            (Instant::now() + Duration::from_secs(5), &flag),
            false
        )
        .err()
        .unwrap()
        .code,
        "CANCELLED"
    );
    flag.store(false, std::sync::atomic::Ordering::Relaxed);
    assert_eq!(
        lexical_assessment(
            "probe.rs",
            node,
            source,
            "selected",
            (Instant::now(), &flag),
            false
        )
        .err()
        .unwrap()
        .code,
        "planning_deadline"
    );
}
