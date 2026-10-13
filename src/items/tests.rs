use super::*;
use std::time::Duration;

#[test]
fn macro_token_candidates_include_dot_names_and_paths() {
    for (source, expected) in [
        (
            "matches!((need.category, need.reason), (\"binding_collision\", DecisionReason::LexicalContextUnproved))",
            true,
        ),
        ("some_macro!(x.category)", true),
        ("some_macro!(x.category())", true),
        ("some_macro!(x . category)", true),
        ("some_macro!(x . /* field */ category)", true),
        ("some_macro!(x . /* first */ /* second */ category())", true),
        ("some_macro!(x . // field\n category)", true),
        (
            "some_macro!((x . /* outer /* nested */ */ r#category))",
            true,
        ),
        ("some_macro!(x.0)", false),
        ("some_macro!(category::SUB)", true),
        ("some_macro!(module::category)", true),
        ("some_macro!(module:: /* path */ category)", true),
        ("some_macro!(::category)", true),
        ("some_macro!(category)", true),
        ("some_macro!(r#category)", true),
        ("some_macro!(x.category(), category)", true),
        ("some_macro!(x..category)", true),
        ("some_macro!(x..=category)", true),
        ("some_macro!(x.(category))", true),
        ("category!(x.field)", true),
        (
            "some_macro!(\"category\", r#\"category\"#, /* category */ x)",
            false,
        ),
        ("macro_rules! example { () => { x.category() } }", true),
        ("macro_rules! example { () => { category() } }", true),
    ] {
        let cancelled = AtomicBool::new(false);
        let deadline = Instant::now() + Duration::from_secs(5);
        let tree = crate::trivia::parse(source, deadline, &cancelled)
            .unwrap()
            .unwrap();
        assert!(
            !tree.root_node().has_error(),
            "{source}: {}",
            tree.root_node().to_sexp()
        );
        let node = tree.root_node().named_child(0).unwrap();
        assert!(matches!(
            node.kind(),
            "macro_invocation" | "macro_definition"
        ));
        assert_eq!(
            token_candidate(node, source, "category", deadline, &cancelled).unwrap(),
            expected,
            "{source}: {}",
            node.to_sexp()
        );
    }
}

#[test]
fn macro_token_candidates_preserve_cancellation_and_deadline() {
    let source = "invoke!(unused.category)";
    let cancelled = AtomicBool::new(false);
    let tree = crate::trivia::parse(source, Instant::now() + Duration::from_secs(5), &cancelled)
        .unwrap()
        .unwrap();
    for (cancelled, deadline, code) in [
        (
            AtomicBool::new(true),
            Instant::now() + Duration::from_secs(5),
            "CANCELLED",
        ),
        (AtomicBool::new(false), Instant::now(), "planning_deadline"),
    ] {
        assert_eq!(
            token_candidate(tree.root_node(), source, "category", deadline, &cancelled)
                .unwrap_err()
                .code,
            code
        );
    }
}
