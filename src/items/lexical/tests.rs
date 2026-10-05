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
    let at = source.rfind(name).unwrap();
    let node = tree
        .root_node()
        .named_descendant_for_byte_range(at, at + name.len())
        .unwrap();
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
        assert_eq!(
            assess(source, "selected", false).binding,
            LexicalBinding::Absent,
            "{source}"
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
        "fn f() { selected(); fn selected() {} }",
        "fn f() { selected(); const selected: usize = 1; }",
        "fn f() { let (mut r#selected,) = value; r#selected(); }",
    ] {
        assert_eq!(
            assess(source, "selected", false).binding,
            LexicalBinding::Independent,
            "{source}"
        );
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
            "fn f() { let x @ _ = value; selected(); }",
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
