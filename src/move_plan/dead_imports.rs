//! Advisory-only review of imports in the assembled source overlay.
use super::*;
use tree_sitter::Node;

/// Count written tokens, not substrings or resolved uses. Even a shadow/binder
/// counts conservatively; strings, comments and their payloads never do.
fn names(
    node: Node<'_>,
    source: &str,
    controls: (Instant, &AtomicBool),
) -> Result<BTreeMap<String, usize>, DomainError> {
    let mut counts = BTreeMap::new();
    let mut stack = vec![node];
    while let Some(node) = stack.pop() {
        items::check(controls.0, controls.1)?;
        if matches!(
            node.kind(),
            "line_comment"
                | "block_comment"
                | "string_literal"
                | "raw_string_literal"
                | "char_literal"
        ) {
            continue;
        }
        if matches!(
            node.kind(),
            "identifier" | "type_identifier" | "field_identifier" | "metavariable"
        ) {
            let name = source[node.byte_range()]
                .trim_start_matches('$')
                .trim_start_matches("r#");
            *counts.entry(name.to_owned()).or_default() += 1;
        }
        for i in (0..node.named_child_count()).rev() {
            stack.push(node.named_child(i as u32).expect("child"));
        }
    }
    Ok(counts)
}

/// An unchanged byte inside a surviving declaration identifies its original
/// enclosing use, including declarations with an engine-repaired path. Entirely
/// synthesized imports have no original declaration and are not cleanup targets.
fn original_use(
    node: Node<'_>,
    path: &str,
    data: &ParsedFile,
    origins: &[MoveOrigin],
    controls: (Instant, &AtomicBool),
) -> Result<Option<ByteRange>, DomainError> {
    for origin in origins {
        items::check(controls.0, controls.1)?;
        if origin.source_path != path || origin.output_path != path {
            continue;
        }
        let at = origin.output_range.start_byte.max(node.start_byte());
        if at >= origin.output_range.end_byte.min(node.end_byte()) {
            continue;
        }
        let original = origin.source_range.start_byte + at - origin.output_range.start_byte;
        let mut candidate = data
            .tree
            .root_node()
            .named_descendant_for_byte_range(original, original + 1);
        while let Some(current) = candidate {
            items::check(controls.0, controls.1)?;
            if current.kind() == "use_declaration" {
                return Ok(Some(range(current.start_byte(), current.end_byte())));
            }
            candidate = current.parent();
        }
    }
    Ok(None)
}

pub(super) fn review(
    files: &BTreeMap<String, FileSnapshot>,
    parsed: &BTreeMap<String, ParsedFile>,
    outputs: &BTreeMap<String, (String, Vec<trivia::Trivia>)>,
    controls: (Instant, &AtomicBool),
    result: &mut MoveEnvelope,
) -> Result<(), DomainError> {
    let source_paths: BTreeSet<_> = result
        .plan
        .moves
        .iter()
        .map(|m| m.item.path.clone())
        .collect();
    for path in source_paths {
        items::check(controls.0, controls.1)?;
        let Some((source, _)) = outputs.get(&path) else {
            continue;
        };
        let tree = trivia::parse(source, controls.0, controls.1)?.ok_or_else(|| {
            DomainError::new("planning_deadline", "post-move import parse stopped")
        })?;
        // A recovery tree cannot supply reliable token counts. The assembly's
        // syntax blocker already withholds this overlay, so make no absence claim.
        if tree.root_node().has_error() {
            continue;
        }
        let counts = names(tree.root_node(), source, controls)?;
        let mut stack = vec![tree.root_node()];
        while let Some(node) = stack.pop() {
            items::check(controls.0, controls.1)?;
            if node.kind() != "use_declaration" {
                for i in (0..node.named_child_count()).rev() {
                    stack.push(node.named_child(i as u32).expect("child"));
                }
                continue;
            }
            if node.child_by_field_name("visibility").is_some()
                || (0..node.named_child_count()).any(|i| {
                    node.named_child(i as u32)
                        .is_some_and(|n| n.kind() == "visibility_modifier")
                })
            {
                continue;
            }
            let leaves = items::use_leaves(node, source, controls)?;
            let own = names(node, source, controls)?;
            let dead: BTreeSet<_> = leaves
                .iter()
                .filter_map(|leaf| {
                    let name = leaf.binding.trim_start_matches("r#");
                    (counts.get(name).copied().unwrap_or(0) == own.get(name).copied().unwrap_or(0))
                        .then(|| leaf.binding.clone())
                })
                .collect();
            let glob = items::use_facts(node, source, controls)?.0;
            if dead.is_empty() && !glob {
                continue;
            }
            let Some(original) =
                original_use(node, &path, &parsed[&path], &result.plan.origins, controls)?
            else {
                continue;
            };
            let mut anchors = vec![anchor(&path, &files[&path].source, &original)];
            let item_ids = result
                .plan
                .moves
                .iter()
                .filter(|m| m.item.path == path)
                .map(|m| {
                    anchors.push(anchor(&path, &files[&path].source, &m.item.span.range));
                    m.id.clone()
                })
                .collect();
            let consequence = format!(
                "post-move bindings with zero remaining written-name references outside this use declaration: [{}]; glob bindings unenumerated: {glob}. Strings/comments are excluded. Import retained: trait method lookup, macro expansion, cfg and binding identity are not proved by token absence; unused-import warnings may remain until compiler-backed cleanup",
                dead.into_iter().collect::<Vec<_>>().join(", ")
            );
            let action = DecisionAction::cause(DecisionReason::PostMoveImportReview);
            let decision = Decision {
                reason: DecisionReason::PostMoveImportReview,
                next_action: action.next_action(),
                action,
                id: format!("d/{}", result.plan.decisions.len()),
                category: "post_move_import".into(),
                anchors,
                item_ids,
                evidence: Vec::new(),
                unresolved_consequence: consequence,
                resolution: "advisory".into(),
                supported_choices: Vec::new(),
                selected_choice: Some("retain".into()),
                blocks_applicability: false,
                chain_diagnostic_ids: Vec::new(),
                lexical_uncertainty: None,
                refusal_basis: Vec::new(),
                removal_gap: None,
            };
            if result.plan.decisions.len() >= 100_000 {
                return Err(DomainError::new(
                    "analysis_descriptor_bytes",
                    "decision cap reached",
                ));
            }
            result.account(descriptor_bytes(&decision)?)?;
            result.plan.decisions.push(decision);
        }
    }
    Ok(())
}
