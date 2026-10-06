//! Anchored assembly boundaries and opt-in shaping of removal-owned gaps.
use super::*;

/// Include attached trailing comments (including their line ending), never insert inside them.
fn after_item(data: &ParsedFile, item: &Item, source: &str) -> usize {
    let at = data
        .trivia
        .iter()
        .filter(|t| {
            t.owned_by(item.span.range.start_byte..item.span.range.end_byte)
                && t.range.start >= item.span.range.end_byte
        })
        .map(|t| t.range.end)
        .max()
        .unwrap_or(item.span.range.end_byte);
    // Reuse the existing complete line ending, including CR already inside a comment node.
    source[at..]
        .find('\n')
        .filter(|offset| {
            source[at..at + offset]
                .bytes()
                .all(|b| matches!(b, b' ' | b'\t' | b'\r'))
        })
        .map(|offset| at + offset + 1)
        .unwrap_or(at)
}
fn before_item(data: &ParsedFile, item: &Item) -> usize {
    data.trivia
        .iter()
        .filter(|t| {
            t.owned_by(item.span.range.start_byte..item.span.range.end_byte)
                && t.range.end <= item.span.range.start_byte
        })
        .map(|t| t.range.start)
        .min()
        .unwrap_or(item.span.range.start_byte)
}
/// Keep every existing import group intact; attach new imports after the last whole use.
pub(crate) fn import_boundary<'a>(data: &'a ParsedFile, source: &str) -> (usize, Option<&'a Item>) {
    if let Some(item) = data
        .items
        .iter()
        .rev()
        .find(|i| i.kind == "use_declaration")
    {
        (after_item(data, item, source), Some(item))
    } else if let Some(item) = data.items.first() {
        (before_item(data, item), Some(item))
    } else {
        (source.len(), None)
    }
}
fn module_boundary<'a>(data: &'a ParsedFile, source: &str) -> (usize, Option<&'a Item>) {
    if let Some(item) = data.items.iter().rev().find(|i| {
        i.kind == "mod_item"
            && data
                .tree
                .root_node()
                .named_descendant_for_byte_range(i.span.range.start_byte, i.span.range.end_byte)
                .is_some_and(|n| n.child_by_field_name("body").is_none())
    }) {
        (after_item(data, item, source), Some(item))
    } else if let Some(item) = data.items.iter().find(|i| {
        i.attributes.iter().any(|a| {
            source[a.range.start_byte..a.range.end_byte]
                .split_whitespace()
                .collect::<String>()
                == "#[cfg(test)]"
        })
    }) {
        (before_item(data, item), Some(item))
    } else {
        (source.len(), None)
    }
}
pub(super) fn declaration_boundary(
    path: &str,
    files: &BTreeMap<String, FileSnapshot>,
    parsed: &BTreeMap<String, ParsedFile>,
) -> (usize, Vec<SourceAnchor>) {
    let source = &files[path].source;
    let (at, item) = module_boundary(&parsed[path], source);
    let mut anchors: Vec<_> = item
        .into_iter()
        .map(|i| anchor(path, source, &i.span.range))
        .collect();
    // The exact boundary, including owned attachments, is replay evidence, not a display ID.
    anchors.push(anchor(path, source, &range(at, at)));
    (at, anchors)
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct RemovalGapChoice {
    /// Exact whitespace remaining after the selected removals, before optional shaping.
    pub before_text: String,
    /// The only supported replacement; preserve the first terminator and one blank line.
    pub after_text: String,
    pub default_disposition: String,
    pub selected_disposition: String,
}
fn whitespace(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\r' | b'\n')
}

/// Only pure removal components qualify. Any repair/insertion sharing the boundary excludes it.
#[allow(clippy::too_many_arguments)] // Snapshot, splice ownership and replay accounting stay explicit.
pub(super) fn removal_gaps(
    request: &MoveRequest,
    files: &BTreeMap<String, FileSnapshot>,
    parsed: &BTreeMap<String, ParsedFile>,
    insertions: &[Insertion],
    edits: &mut Vec<Edit>,
    used: &mut BTreeSet<usize>,
    controls: (Instant, &AtomicBool),
    result: &mut MoveEnvelope,
) -> Result<(), DomainError> {
    let mut removals: Vec<_> = edits
        .iter()
        .filter(|e| {
            e.replacement_text.is_empty() && e.rewrite_ids.is_empty() && !e.item_ids.is_empty()
        })
        .cloned()
        .collect();
    removals.sort_by_key(|e| (e.path.clone(), e.range.clone()));
    let mut cursor = 0;
    while cursor < removals.len() {
        items::check(controls.0, controls.1)?;
        let first = cursor;
        let path = &removals[first].path;
        let source = &files[path].source;
        cursor += 1;
        while cursor < removals.len()
            && removals[cursor].path == *path
            && removals[cursor - 1].range.end_byte <= removals[cursor].range.start_byte
            && source[removals[cursor - 1].range.end_byte..removals[cursor].range.start_byte]
                .bytes()
                .all(whitespace)
        {
            cursor += 1;
        }
        let component = &removals[first..cursor];
        let mut start = component[0].range.start_byte;
        let mut end = component.last().expect("component").range.end_byte;
        while start > 0 && whitespace(source.as_bytes()[start - 1]) {
            start -= 1;
        }
        while end < source.len() && whitespace(source.as_bytes()[end]) {
            end += 1;
        }
        // CR may belong to a retained line-comment CST span. Never shape any trivia bytes.
        for t in &parsed[path].trivia {
            items::check(controls.0, controls.1)?;
            if t.range.end <= component[0].range.start_byte {
                start = start.max(t.range.end);
            }
            if t.range.start >= component.last().expect("component").range.end_byte {
                end = end.min(t.range.start);
            }
        }
        if insertions
            .iter()
            .any(|i| i.path == *path && start <= i.at && i.at <= end)
            || edits.iter().any(|e| {
                e.path == *path
                    && e.range.start_byte <= end
                    && e.range.end_byte >= start
                    && !component.iter().any(|r| r.range == e.range)
            })
        {
            continue;
        }
        let mut before = String::new();
        let mut at = start;
        for e in component {
            before.push_str(&source[at..e.range.start_byte]);
            at = e.range.end_byte;
        }
        before.push_str(&source[at..end]);
        // Between retained tokens, the first newline terminates the previous line.
        // At BOF every complete whitespace line is already a blank line.
        let keep = if start == 0 { 1 } else { 2 };
        let newlines: Vec<_> = before.match_indices('\n').map(|(at, _)| at + 1).collect();
        if newlines.len() <= keep {
            continue;
        }
        let after = format!(
            "{}{}",
            &before[..newlines[keep - 1]],
            &before[*newlines.last().expect("newlines")..]
        );
        if after.len() > 64 * 1024 {
            continue;
        }
        let target = RewriteTarget::Source {
            anchor: anchor(path, source, &range(start, end)),
        };
        let mut selected = "keep_in_place";
        let mut selected_action = "accept_default";
        let mut chosen = false;
        for (index, choice) in request
            .rewrite_overrides
            .as_deref()
            .unwrap_or_default()
            .iter()
            .enumerate()
        {
            if choice.target != target {
                continue;
            }
            if chosen {
                return Err(error(
                    "INVALID_REWRITE_OVERRIDE",
                    "duplicate/conflicting removal gap target",
                    "rewrite_overrides",
                ));
            }
            chosen = true;
            match choice.action {
                RewriteAction::AcceptDefault | RewriteAction::Retain => {
                    if choice.replacement_text.is_some() {
                        return Err(error(
                            "INVALID_REWRITE_OVERRIDE",
                            "only replace accepts replacement_text",
                            "rewrite_overrides",
                        ));
                    }
                    used.insert(index);
                    if matches!(choice.action, RewriteAction::Retain) {
                        selected_action = "retain";
                    }
                }
                RewriteAction::Replace => {
                    if choice.replacement_text.as_deref() != Some(&after) {
                        return Err(error(
                            "INVALID_REWRITE_OVERRIDE",
                            "removal gap replacement must equal the published collapsed whitespace",
                            "rewrite_overrides",
                        ));
                    }
                    selected = "collapse";
                    selected_action = "replace";
                }
            }
        }
        let mut ids: Vec<_> = component.iter().flat_map(|e| e.item_ids.clone()).collect();
        ids.sort();
        ids.dedup();
        let mut action = DecisionAction::rewrite(target.clone());
        if let DecisionAction::RequestField {
            choices, purpose, ..
        } = &mut action
        {
            *choices = vec!["accept_default".into(), "retain".into(), "replace".into()];
            *purpose = DecisionPurpose::ReviewDefault;
        }
        let decision_id = format!("d/{}", result.plan.decisions.len());
        let decision = Decision {
            reason: DecisionReason::RemovalGapChoice, next_action: "keep the gap unchanged, or replay the target with action replace and removal_gap.after_text".into(),
            action, id: decision_id.clone(), category: "removal_gap".into(),
            anchors: match &target { RewriteTarget::Source { anchor } => vec![anchor.clone()], _ => unreachable!() },
            item_ids: ids.clone(), evidence: Vec::new(), unresolved_consequence: "removal-boundary blank lines stay byte-identical unless explicitly collapsed".into(),
            resolution: "choice_available".into(), supported_choices: vec!["accept_default".into(), "retain".into(), "replace".into()],
            selected_choice: Some(selected_action.into()),
            blocks_applicability: false, chain_diagnostic_ids: Vec::new(), lexical_uncertainty: None,
            removal_gap: Some(RemovalGapChoice { before_text: before.clone(), after_text: after.clone(), default_disposition: "keep_in_place".into(), selected_disposition: selected.into() }),
        };
        if result.plan.decisions.len() >= 100_000 {
            return Err(DomainError::new(
                "analysis_descriptor_bytes",
                "decision cap reached",
            ));
        }
        result.account(descriptor_bytes(&decision)?)?;
        result.plan.decisions.push(decision);
        if selected == "collapse" {
            let (id, text) = rewrite(
                request,
                used,
                result,
                target,
                "removal_gap",
                &before,
                &ids,
                Some(&after),
            )?;
            let audit = result.plan.rewrites.last_mut().expect("gap audit");
            audit.before_text = source[start..end].into();
            audit.anchors = result
                .plan
                .decisions
                .last()
                .expect("gap decision")
                .anchors
                .clone();
            audit.decision_ids.push(decision_id);
            audit.default_action = "retain".into();
            audit.evidence = vec!["exact selected removals and adjacent whitespace; explicit caller-selected collapse".into()];
            audit.rationale = "explicitly collapse only whitespace left by these engine-owned removals; no retained syntax is changed".into();
            let mut edit = Edit::new(path, source, start, end, text, "");
            edit.match_ids.clear();
            edit.item_ids = ids;
            edit.rewrite_ids.push(id);
            edit.trivia_ids = component
                .iter()
                .flat_map(|e| e.trivia_ids.clone())
                .collect();
            edits.retain(|e| e.path != *path || !component.iter().any(|r| r.range == e.range));
            edits.push(edit);
        }
    }
    Ok(())
}
