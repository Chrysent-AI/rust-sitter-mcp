use super::*;
use std::time::Duration;
#[test]
fn copied_roles_map_exact_bytes_without_classing_text() {
    let cancelled = AtomicBool::new(false);
    let controls = (Instant::now() + Duration::from_secs(5), &cancelled);
    let source = "fn probe(){ matches!(policy, /* é */ Choice::First | Choice::Second,); }";
    let tree = crate::trivia::parse(source, controls.0, controls.1)
        .unwrap()
        .unwrap();
    let mut stack = vec![tree.root_node()];
    let node = loop {
        let n = stack.pop().unwrap();
        if n.kind() == "macro_invocation" {
            break n;
        }
        for i in 0..n.named_child_count() {
            stack.push(n.named_child(i as u32).unwrap());
        }
    };
    let sites = shape(node, source, controls, true).unwrap();
    assert_eq!(sites.len(), 3);
    assert_eq!(
        &source[sites[0].range.start_byte..sites[0].range.end_byte],
        "policy"
    );
    assert_eq!(
        &source[sites[1].owner.start_byte..sites[1].owner.end_byte],
        "Choice"
    );
    assert_eq!(
        &source[sites[1].range.start_byte..sites[1].range.end_byte],
        "Choice::First"
    );
    assert_eq!(sites[2].variant.as_deref(), Some("Second"));
}
#[test]
fn assertions_and_proofs_have_separate_schema_classes() {
    let request = serde_json::to_value(rmcp::schemars::schema_for!(MoveRequest)).unwrap();
    assert!(
        request["properties"]
            .get("standard_matches_assertions")
            .is_some()
    );
    assert!(
        !request["required"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v == "standard_matches_assertions")
    );
    let proof = serde_json::to_value(rmcp::schemars::schema_for!(MatchesProof)).unwrap();
    assert!(proof.to_string().contains("assumed_standard_matches"));
}

fn roles(source: &str, allow_fields: bool) -> Option<Vec<Site>> {
    let cancelled = AtomicBool::new(false);
    let controls = (Instant::now() + Duration::from_secs(5), &cancelled);
    let tree = crate::trivia::parse(source, controls.0, controls.1)
        .unwrap()
        .unwrap();
    let mut stack = vec![tree.root_node()];
    while let Some(node) = stack.pop() {
        if node.kind() == "macro_invocation" {
            return shape(node, source, controls, allow_fields);
        }
        for i in 0..node.named_child_count() {
            stack.push(node.named_child(i as u32).unwrap());
        }
    }
    None
}

#[test]
fn tuple_roles_map_receivers_and_enum_owners_not_field_names() {
    let source = "matches!((need.category, need.reason), (\"binding_collision\", DecisionReason::LexicalContextUnproved) | (\"macro_dependency\", DecisionReason::MacroContextUnexamined))";
    let sites = roles(source, true).unwrap();
    let owners: Vec<_> = sites
        .iter()
        .map(|s| &source[s.owner.start_byte..s.owner.end_byte])
        .collect();
    assert_eq!(owners, ["need", "need", "DecisionReason", "DecisionReason"]);
    assert_eq!(sites[2].variant.as_deref(), Some("LexicalContextUnproved"));
    assert_eq!(sites[3].variant.as_deref(), Some("MacroContextUnexamined"));
    assert!(
        roles(source, false).is_none(),
        "relocating field access still needs member proof"
    );
}

#[test]
fn tuple_grammar_keeps_exact_leaf_roles() {
    for source in [
        "matches!((value, 1, true, 'x'), (Choice::First, 1, true, 'x'))",
        "matches!(((value,),), ((Choice::First,),))",
        "matches!((need . /* é */ r#category, need.0), (\"binding_collision\", _),)",
        "matches!((crate::source::category, value), (_, Choice::First))",
    ] {
        assert!(roles(source, true).is_some(), "{source}");
    }
    let source = "matches!((crate::source::category, value), (_, Choice::First))";
    let sites = roles(source, true).unwrap();
    assert_eq!(
        &source[sites[0].range.start_byte..sites[0].range.end_byte],
        "crate::source::category"
    );
}

#[test]
fn tuple_grammar_refuses_unsupported_or_unclassed_children() {
    for source in [
        "matches![(need.category, need.reason), (\"binding_collision\", Reason::First)]",
        "matches!{(need.category, need.reason), (\"binding_collision\", Reason::First)}",
        "other!((need.category, need.reason), (\"binding_collision\", Reason::First))",
        "std::matches!((need.category, need.reason), (\"binding_collision\", Reason::First))",
        "matches!((nested!(need.category), need.reason), (_, Reason::First))",
        "matches!((need.category, need.reason), (_, nested!(Reason::First)))",
        "matches!((need.category(), need.reason), (_, Reason::First))",
        "matches!(([need.category], need.reason), (_, Reason::First))",
        "matches!((need.category, need.reason), ([binder], Reason::First))",
        "matches!((need.category, need.reason), (binder, Reason::First))",
        "matches!((need.category, need.reason), (_, Reason::Tuple(value)))",
        "matches!((need.category, need.reason), (_, Reason::Record { value }))",
        "matches!((need.category, need.reason), (_, Reason::First) if true)",
        "matches!((need.category, need.reason), (_, Reason::<u8>::First))",
        "matches!((need.category,, need.reason), (_, Reason::First))",
        "matches!((need.category, need.reason), (_, Reason::First), extra)",
    ] {
        assert!(roles(source, true).is_none(), "{source}");
    }
}
