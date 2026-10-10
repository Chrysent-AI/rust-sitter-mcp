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
    let sites = shape(node, source, controls).unwrap();
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
