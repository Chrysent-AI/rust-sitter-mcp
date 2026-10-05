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
            "fn f(selected: fn()) { let (selected,) = value; selected(); }",
            LexicalReason::IdentifierPatternBindingOrConstant,
        ),
        (
            "fn f(selected: fn()) { for selected in value { selected(); } }",
            LexicalReason::IdentifierPatternBindingOrConstant,
        ),
        (
            "fn f() { if let Some(selected) = value { selected(); } }",
            LexicalReason::IdentifierPatternBindingOrConstant,
        ),
        (
            "fn f() { while let Some(selected) = value { selected(); } }",
            LexicalReason::IdentifierPatternBindingOrConstant,
        ),
        (
            "fn f() { match value { Some(selected) => selected(), _ => {} } }",
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
            "fn f() { match value { other @ Some(selected) => selected(), _ => {} } }",
            LexicalReason::IdentifierPatternBindingOrConstant,
        ),
        (
            "fn f() { match value { selected @ Some(selected) => selected(), _ => {} } }",
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
            "fn f() { selected(); m!(); }",
            LexicalReason::UnsupportedPattern,
        ),
        (
            "fn f() { m!(); let selected = value; selected(); }",
            LexicalReason::UnsupportedPattern,
        ),
        (
            "fn f() { let selected = value; selected(); m!(); }",
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
