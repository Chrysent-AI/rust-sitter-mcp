//! Sparse written-reference candidates and source-linked organization facts.
use super::*;
use std::collections::{BTreeMap, BTreeSet};
use tree_sitter::Node;

pub(super) fn scope_trivia(
    data: &ParsedFile,
    lines: &Lines<'_>,
    controls: Controls<'_>,
    result: &mut SuggestSplitEnvelope,
) -> Result<(), DomainError> {
    for trivia in &data.trivia {
        controls.check()?;
        // Internal trivia travels with its inventory unit, not a second scope descriptor.
        if data.items.iter().any(|i| {
            i.span.range.start_byte <= trivia.range.start
                && i.span.range.end_byte >= trivia.range.end
                || trivia.owned_by(i.span.range.start_byte..i.span.range.end_byte)
        }) {
            continue;
        }
        let previous = result
            .inventory
            .iter()
            .rev()
            .find(|i| i.span.range.end_byte <= trivia.range.start);
        let next = result
            .inventory
            .iter()
            .find(|i| i.span.range.start_byte >= trivia.range.end);
        let record = ScopeTrivia {
            id: format!("t/{}/{}", trivia.range.start, trivia.range.end),
            span: lines.slice(
                trivia.range.start,
                trivia.range.end,
                result.limits.text_bytes,
            ),
            classification: trivia.classification.clone(),
            reason: trivia.reason.clone(),
            protected: trivia.protected(),
            default_disposition: "keep_in_place".into(),
            adjacent_item_ids: previous
                .into_iter()
                .chain(next)
                .map(|i| i.id.clone())
                .collect(),
        };
        result.account(descriptor_bytes(&record)?)?;
        result.scope_trivia.push(record);
    }
    Ok(())
}
fn signal(kind: &str, members: Vec<String>, evidence: Vec<SourceSlice>) -> Signal {
    Signal {
        id: String::new(),
        kind: kind.into(),
        basis: "syntactic_heuristic".into(),
        item_ids: members,
        evidence,
        from_item_id: None,
        to_item_id: None,
        count: 0,
        label: None,
        facts: BTreeMap::new(),
        limitations: Vec::new(),
    }
}
fn push(result: &mut SuggestSplitEnvelope, value: Signal) -> Result<(), DomainError> {
    if result.signals.len() >= 100_000 {
        return Err(DomainError::new(
            "reference_work_limit",
            "advice signal record guard reached",
        ));
    }
    result.account(descriptor_bytes(&value)?)?;
    result.signals.push(value);
    result.counts.signals = result.signals.len();
    Ok(())
}
/// Word boundaries include snake_case, lower→upper and acronym→capitalized word.
fn words(name: &str, controls: Controls<'_>) -> Result<Vec<String>, DomainError> {
    let name = name.trim_start_matches("r#");
    if !name.is_ascii() {
        return Ok(Vec::new());
    }
    let bytes = name.as_bytes();
    let mut out = Vec::new();
    let mut start = 0;
    for i in 0..bytes.len() {
        controls.check()?;
        let b = bytes[i];
        let separator = !b.is_ascii_alphanumeric();
        let boundary = i > start
            && b.is_ascii_uppercase()
            && (bytes[i - 1].is_ascii_lowercase()
                || (bytes[i - 1].is_ascii_uppercase()
                    && bytes.get(i + 1).is_some_and(u8::is_ascii_lowercase)));
        if separator || boundary {
            if start < i {
                out.push(name[start..i].to_ascii_lowercase());
            }
            start = if separator { i + 1 } else { i };
        }
    }
    if start < name.len() {
        out.push(name[start..].to_ascii_lowercase());
    }
    Ok(out)
}
fn named_groups(
    result: &mut SuggestSplitEnvelope,
    controls: Controls<'_>,
) -> Result<(), DomainError> {
    let mut prefixes: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    let mut duplicates: BTreeMap<String, usize> = BTreeMap::new();
    for item in &result.inventory {
        controls.check()?;
        if let Some(name) = &item.name {
            *duplicates.entry(name.clone()).or_default() += 1;
        }
    }
    for index in 0..result.inventory.len() {
        controls.check()?;
        let item = &result.inventory[index];
        let Some(name) = &item.name else {
            continue;
        };
        let words = words(name, controls)?;
        let mut prefix = String::new();
        for word in words {
            controls.check()?;
            if !prefix.is_empty() {
                prefix.push('_');
            }
            prefix.push_str(&word);
            if prefix.len() >= 3 {
                result.account(prefix.len() + 32)?;
                prefixes.entry(prefix.clone()).or_default().push(index);
            }
        }
    }
    for (prefix, members) in prefixes {
        controls.check()?;
        if members.len() < 2 {
            continue;
        }
        let mut value = signal(
            "name_prefix",
            members
                .iter()
                .map(|i| result.inventory[*i].id.clone())
                .collect(),
            members
                .iter()
                .map(|i| result.inventory[*i].span.clone())
                .collect(),
        );
        value.count = members.len();
        let common = matches!(prefix.as_str(), "get" | "set" | "new");
        value
            .facts
            .insert("same_prefix_members".into(), members.len());
        value
            .facts
            .insert("common_prefix".into(), usize::from(common));
        value.facts.insert(
            "duplicate_names".into(),
            members
                .iter()
                .filter(|i| {
                    result.inventory[**i]
                        .name
                        .as_ref()
                        .is_some_and(|n| duplicates[n] > 1)
                })
                .count(),
        );
        value.label = Some(prefix);
        value
            .limitations
            .push("word-prefix similarity is organization evidence, not shared semantics".into());
        if common {
            value
                .limitations
                .push("get/set/new alone is not strong cohesion evidence".into());
        }
        push(result, value)?;
    }
    Ok(())
}
fn sections(result: &mut SuggestSplitEnvelope, controls: Controls<'_>) -> Result<(), DomainError> {
    let banners: Vec<_> = result
        .scope_trivia
        .iter()
        .filter(|t| t.reason == "banner")
        .cloned()
        .collect();
    for (index, banner) in banners.iter().enumerate() {
        controls.check()?;
        let end = banners
            .get(index + 1)
            .map_or(usize::MAX, |b| b.span.range.start_byte);
        let members: Vec<_> = result
            .inventory
            .iter()
            .filter(|i| {
                i.span.range.start_byte >= banner.span.range.end_byte
                    && i.span.range.start_byte < end
            })
            .map(|i| i.id.clone())
            .collect();
        let mut value = signal("banner_section", members, vec![banner.span.clone()]);
        value.count = value.item_ids.len();
        value.label = Some(banner.id.clone());
        value.facts.insert("section_members".into(), value.count);
        value.limitations.push(
            "adjacency after a written banner; banner remains in source and is not group-owned"
                .into(),
        );
        push(result, value)?;
    }
    Ok(())
}
fn shared_headings(
    source: &str,
    result: &mut SuggestSplitEnvelope,
    controls: Controls<'_>,
) -> Result<(), DomainError> {
    let mut by_text: BTreeMap<(String, String), Vec<(usize, SourceSlice)>> = BTreeMap::new();
    for index in 0..result.inventory.len() {
        controls.check()?;
        let item = result.inventory[index].clone();
        let spans: Vec<_> = item
            .attributes
            .iter()
            .chain(&item.trivia)
            .cloned()
            .collect();
        let mut seen = BTreeSet::new();
        for span in spans {
            controls.check()?;
            // Internal scope forms are not shared *outer* organization evidence.
            if span.range.start_byte >= item.span.range.start_byte {
                continue;
            }
            let text = &source[span.range.start_byte..span.range.end_byte];
            let (kind, key) = if text.starts_with("#[") {
                ("shared_attribute", text.to_owned())
            } else if text.starts_with("///") || text.starts_with("/**") {
                let heading = text.lines().find_map(|line| {
                    let line = line
                        .trim()
                        .trim_start_matches('/')
                        .trim_start_matches('*')
                        .trim();
                    line.strip_prefix('#').map(|rest| rest.trim().to_owned())
                });
                let Some(heading) = heading else {
                    continue;
                };
                ("doc_heading", heading)
            } else {
                continue;
            };
            if !seen.insert((kind, key.clone())) {
                continue;
            }
            result.account(key.len() + 256)?;
            by_text
                .entry((kind.into(), key))
                .or_default()
                .push((index, span));
        }
    }
    for ((kind, _text), members) in by_text {
        controls.check()?;
        // Adjacency clusters: shared attributes far apart do not join unrelated sections.
        let mut run = Vec::new();
        let mut previous = None;
        for (index, span) in members {
            controls.check()?;
            if previous.is_some_and(|p| index != p + 1) {
                heading_run(&kind, &run, result)?;
                run.clear();
            }
            previous = Some(index);
            run.push((index, span));
        }
        heading_run(&kind, &run, result)?;
    }
    Ok(())
}
fn heading_run(
    kind: &str,
    run: &[(usize, SourceSlice)],
    result: &mut SuggestSplitEnvelope,
) -> Result<(), DomainError> {
    if run.len() < 2 {
        return Ok(());
    }
    let mut value = signal(
        kind,
        run.iter()
            .map(|(i, _)| result.inventory[*i].id.clone())
            .collect(),
        run.iter().map(|(_, span)| span.clone()).collect(),
    );
    value.count = run.len();
    value
        .facts
        .insert("adjacent_agreements".into(), run.len() - 1);
    value.limitations.push(
        "shared written outer text/heading and adjacency; not trivia ownership or cfg evaluation"
            .into(),
    );
    push(result, value)
}
fn range(node: Node<'_>) -> ByteRange {
    ByteRange {
        start_byte: node.start_byte(),
        end_byte: node.end_byte(),
    }
}
fn item_node<'a>(data: &'a ParsedFile, item: &Item) -> Node<'a> {
    data.tree
        .root_node()
        .named_descendant_for_byte_range(item.span.range.start_byte, item.span.range.end_byte)
        .expect("inventoried item")
}
fn index_name(index: &mut BTreeMap<String, Vec<usize>>, name: &str, item: usize) {
    let values = index
        .entry(name.trim_start_matches("r#").into())
        .or_default();
    if values.last() != Some(&item) {
        values.push(item);
    }
}
fn references(
    source: &FileSnapshot,
    data: &ParsedFile,
    module: Option<&items::ModuleEvidence>,
    routes: &items::GlobRoutes<'_>,
    lines: &Lines<'_>,
    controls: Controls<'_>,
    result: &mut SuggestSplitEnvelope,
) -> Result<(), DomainError> {
    let mut index = BTreeMap::new();
    for (i, item) in result.inventory.iter().enumerate() {
        controls.check()?;
        if let Some(name) = &item.name {
            index_name(&mut index, name, i);
        }
        if item.kind == "use_declaration" {
            for leaf in items::use_leaves(
                item_node(data, item),
                &source.source,
                (controls.deadline, controls.cancelled),
            )? {
                controls.check()?;
                index_name(&mut index, &leaf.binding, i);
            }
        }
    }
    // Each occurrence is stored once per candidate target. No dense pairwise substring scans.
    let mut edges: BTreeMap<(usize, usize), Vec<ByteRange>> = BTreeMap::new();
    let mut ambiguous_edges = BTreeSet::new();
    let mut test_edges = BTreeSet::new();
    for owner in 0..result.inventory.len() {
        controls.check()?;
        let item = &result.inventory[owner];
        let root = item_node(data, item);
        let tests = items::test_scope(
            item,
            root,
            &source.source,
            (controls.deadline, controls.cancelled),
            routes,
        )?;
        if matches!(
            item.kind.as_str(),
            "use_declaration" | "extern_crate_declaration" | "macro_definition"
        ) || (item.kind == "mod_item" && tests.is_none())
        {
            continue;
        }
        let mut stack = vec![tests.as_ref().map_or(root, |_| {
            root.child_by_field_name("body").expect("test body")
        })];
        while let Some(node) = stack.pop() {
            controls.check()?;
            if matches!(
                node.kind(),
                "line_comment"
                    | "block_comment"
                    | "attribute_item"
                    | "inner_attribute_item"
                    | "token_tree"
                    | "macro_definition"
                    | "macro_invocation"
                    | "mod_item"
                    | "use_declaration"
            ) {
                continue;
            }
            let simple = matches!(node.kind(), "scoped_identifier" | "scoped_type_identifier");
            let bare = matches!(node.kind(), "identifier" | "type_identifier");
            if (simple || bare) && items::reference_role(node) {
                let text = &source.source[node.byte_range()];
                // Explicit self or canonical current-module paths denote candidates in this file.
                // Other qualified paths may denote a local/import base, never their suffix alone.
                let segments: Vec<_> = text.split("::").collect();
                let (name, binding_node) = if let Some(tests) = &tests {
                    if simple && segments.first() == Some(&"super") && segments.len() >= 2 {
                        (segments[1], None)
                    } else {
                        let spelling = segments[0].trim_start_matches("r#");
                        if tests.shadowed.contains(spelling) {
                            continue;
                        }
                        let Some(target) = tests
                            .aliases
                            .get(spelling)
                            .map(String::as_str)
                            .or_else(|| tests.super_glob.then_some(spelling))
                        else {
                            continue;
                        };
                        let mut first = node;
                        while let Some(child) = first.child_by_field_name("path") {
                            first = child;
                        }
                        (target, Some(first))
                    }
                } else if simple && segments.first() == Some(&"self") && segments.len() == 2 {
                    (segments[1], None)
                } else if simple
                    && segments.first() == Some(&"crate")
                    && module.is_some_and(|m| {
                        m.unresolved.is_empty()
                            && segments.len() == m.module_segments.len() + 2
                            && segments[1..segments.len() - 1]
                                .iter()
                                .zip(&m.module_segments)
                                .all(|(a, b)| {
                                    a.trim_start_matches("r#") == b.trim_start_matches("r#")
                                })
                    })
                {
                    (segments[segments.len() - 1], None)
                } else if simple {
                    let first = segments[0];
                    // First path segment is the bound name; UFCS/absolute/relative unknowns are not suffix matches.
                    if matches!(first, "crate" | "super" | "self")
                        || first.starts_with('<')
                        || first.is_empty()
                    {
                        continue;
                    }
                    let mut first_node = node;
                    while let Some(child) = first_node.child_by_field_name("path") {
                        first_node = child;
                    }
                    (first, Some(first_node))
                } else {
                    (text, Some(node))
                };
                let targets = index.get(name.trim_start_matches("r#"));
                // No new dependency pass: unknown bare bindings use this same lexical walk.
                // Qualified unknown paths, impl Self and unknown lexical uncertainties
                // remain outside this lower bound; preserve existing candidate decisions.
                if targets.is_some() || (!simple && tests.is_none() && name != "Self") {
                    let assessment = binding_node
                        .map(|n| {
                            items::lexical_assessment(
                                &source.path,
                                n,
                                &source.source,
                                &source.source[n.byte_range()],
                                (controls.deadline, controls.cancelled),
                                false,
                            )
                        })
                        .transpose()?;
                    let lexical = assessment
                        .as_ref()
                        .map_or(items::LexicalBinding::Absent, |a| a.binding);
                    controls.check()?;
                    if lexical == items::LexicalBinding::Uncertain && targets.is_some() {
                        let id = result.inventory[owner].id.clone();
                        add_decision(
                            result,
                            ("binding_collision", DecisionReason::LexicalContextUnproved),
                            &source.path,
                            lines.slice(
                                node.start_byte(),
                                node.end_byte(),
                                result.limits.text_bytes,
                            ),
                            vec![id],
                            (
                                "containing patterns/local imports leave this spelling's binding uncertain; no cohesion edge was inferred",
                                "inspect lexical binding or narrow/change the proposed moves; acknowledgment is not binding proof",
                            ),
                            false,
                        )?;
                        let witness = assessment.expect("uncertain assessment").uncertainty;
                        result.account(descriptor_bytes(&witness)?)?;
                        let decision = result
                            .decisions
                            .iter_mut()
                            .find(|d| {
                                d.reason == DecisionReason::LexicalContextUnproved
                                    && d.anchors.first().is_some_and(|a| {
                                        a.path == source.path && a.span.range == range(node)
                                    })
                            })
                            .expect("lexical decision");
                        decision.lexical_uncertainty = witness;
                    } else if lexical == items::LexicalBinding::Absent && targets.is_none() {
                        add_decision(
                            result,
                            (
                                "unsupported_dependency_form",
                                DecisionReason::ExternalOrMissingBinding,
                            ),
                            &source.path,
                            lines.slice(
                                node.start_byte(),
                                node.end_byte(),
                                result.limits.text_bytes,
                            ),
                            vec![result.inventory[owner].id.clone()],
                            (
                                "written bare binding has no inventoried declaration/import or proven local binding; relocation requires binding evidence",
                                "inspect the binding and submit supported explicit import/path repairs or change the selection",
                            ),
                            false,
                        )?;
                    } else if lexical == items::LexicalBinding::Absent {
                        for target in targets.expect("candidate targets") {
                            controls.check()?;
                            result.counts.reference_candidates += 1;
                            if result.counts.reference_candidates
                                > result.effective_work_limits.reference_candidates
                            {
                                return Err(DomainError::new(
                                    "reference_work_limit",
                                    "written candidate guard reached; narrow source",
                                ));
                            }
                            let displayed_bytes = if node.end_byte() - node.start_byte()
                                <= result.limits.text_bytes
                            {
                                node.end_byte() - node.start_byte()
                            } else {
                                0
                            };
                            // Reserve occurrence display before allocating repeated target evidence.
                            result.account(
                                128 + result.inventory[owner].id.len()
                                    + result.inventory[*target].id.len()
                                    + displayed_bytes,
                            )?;
                            edges.entry((owner, *target)).or_default().push(range(node));
                            if tests.is_some() {
                                test_edges.insert((owner, *target));
                            }
                            if targets.expect("candidate targets").len() > 1 {
                                ambiguous_edges.insert((owner, *target));
                            }
                        }
                    }
                }
                // The full path has been considered once, not one edge per path component.
                if simple {
                    continue;
                }
            }
            for i in (0..node.named_child_count()).rev() {
                controls.check()?;
                stack.push(node.named_child(i as u32).expect("child"));
            }
        }
    }
    for ((from, to), occurrences) in edges {
        controls.check()?;
        let mut evidence = Vec::with_capacity(occurrences.len());
        for occurrence in &occurrences {
            controls.check()?;
            evidence.push(lines.slice(
                occurrence.start_byte,
                occurrence.end_byte,
                result.limits.text_bytes,
            ));
        }
        let test_consumer = test_edges.contains(&(from, to));
        let mut value = signal(
            if test_consumer {
                "cfg_test_consumer"
            } else {
                "reference_candidate"
            },
            if test_consumer {
                vec![result.inventory[to].id.clone()]
            } else if from == to {
                vec![result.inventory[from].id.clone()]
            } else {
                vec![
                    result.inventory[from].id.clone(),
                    result.inventory[to].id.clone(),
                ]
            },
            evidence,
        );
        value.from_item_id = Some(result.inventory[from].id.clone());
        value.to_item_id = Some(result.inventory[to].id.clone());
        value.count = occurrences.len();
        value.facts.insert("occurrences".into(), occurrences.len());
        let ambiguous = ambiguous_edges.contains(&(from, to));
        value
            .facts
            .insert("ambiguous_binding".into(), usize::from(ambiguous));
        value.limitations.push("written bare/simple-path candidates, not symbol resolution or a call graph; namespaces conflated; no macro expansion or type-directed resolution".into());
        if test_consumer {
            value.label = Some(
                "test-coupled: consumers in this file's test module will block relocation".into(),
            );
            value.limitations.push("observed super:: paths or references through direct super imports in a written #[cfg(test)] inline module only; nested modules and macro tokens not assessed; execution semantics unchanged".into());
            for span in &value.evidence {
                controls.check()?;
                add_decision(
                    result,
                    (
                        "module_context",
                        DecisionReason::ConditionalOrInheritedContext,
                    ),
                    &source.path,
                    span.clone(),
                    value.item_ids.clone(),
                    (
                        "test-coupled: consumers in this file's test module will block relocation; conditional consumer context is not proved by grouping",
                        "retain the binding or change the explicit move selection; cfg(test) is not evaluated and has no execution override",
                    ),
                    false,
                )?;
            }
        }
        if ambiguous {
            value.limitations.push(
                "several written bindings share this spelling; candidate targets are not resolved"
                    .into(),
            );
        }
        push(result, value)?;
    }
    Ok(())
}
pub(super) fn collect(
    source: &FileSnapshot,
    data: &ParsedFile,
    module: Option<&items::ModuleEvidence>,
    routes: &items::GlobRoutes<'_>,
    lines: &Lines<'_>,
    controls: Controls<'_>,
    result: &mut SuggestSplitEnvelope,
) -> Result<(), DomainError> {
    named_groups(result, controls)?;
    sections(result, controls)?;
    shared_headings(&source.source, result, controls)?;
    references(source, data, module, routes, lines, controls, result)?;
    for index in 0..result.inventory.len() {
        controls.check()?;
        let item = &result.inventory[index];
        let mut value = signal("item_size", vec![item.id.clone()], vec![item.span.clone()]);
        value.count = 1;
        value.facts.insert("bytes".into(), item.bytes);
        value.facts.insert("lines".into(), item.lines);
        value.facts.insert(
            "soft_size_exceeded".into(),
            usize::from(item.bytes > 64 * 1024 || item.lines > 256),
        );
        push(result, value)?;
    }
    controls.check()?;
    // Total order: kind, earliest contributing range, full member order, directed endpoints.
    result.signals.sort_by(|a, b| {
        (
            &a.kind,
            a.evidence.first().map(|s| &s.range),
            &a.item_ids,
            &a.from_item_id,
            &a.to_item_id,
            &a.label,
        )
            .cmp(&(
                &b.kind,
                b.evidence.first().map(|s| &s.range),
                &b.item_ids,
                &b.from_item_id,
                &b.to_item_id,
                &b.label,
            ))
    });
    let by_id: BTreeMap<_, _> = result
        .inventory
        .iter()
        .enumerate()
        .map(|(i, item)| (item.id.clone(), i))
        .collect();
    for (ordinal, value) in result.signals.iter_mut().enumerate() {
        controls.check()?;
        value.id = format!("s/{ordinal}");
        for id in &value.item_ids {
            controls.check()?;
            result.inventory[*by_id.get(id).expect("signal member")]
                .signal_ids
                .push(value.id.clone());
        }
    }
    Ok(())
}
pub(super) fn add_decision(
    result: &mut SuggestSplitEnvelope,
    cause: (&str, DecisionReason),
    path: &str,
    span: SourceSlice,
    item_ids: Vec<String>,
    explanation: (&str, &str),
    banner: bool,
) -> Result<(), DomainError> {
    let (category, reason) = cause;
    let (consequence, _) = explanation;
    if let Some(prior) = result.decisions.iter().position(|d| {
        d.reason == reason
            && d.category == category
            && d.anchors
                .first()
                .is_some_and(|a| a.path == path && a.span.range == span.range)
            && d.unresolved_consequence == consequence
    }) {
        for id in item_ids {
            if !result.decisions[prior].item_ids.contains(&id) {
                result.account(descriptor_bytes(&id)?)?;
                result.decisions[prior].item_ids.push(id);
            }
        }
        return Ok(());
    }
    if result.decisions.len() >= 100_000 {
        return Err(DomainError::new(
            "analysis_descriptor_bytes",
            "advice decision guard reached",
        ));
    }
    let action = DecisionAction::cause(reason);
    let value = AdviceDecision {
        reason,
        next_action: action.next_action(),
        action,
        id: String::new(),
        category: category.into(),
        anchors: vec![AdviceAnchor {
            path: path.into(),
            span: span.clone(),
        }],
        item_ids,
        evidence: vec![span],
        unresolved_consequence: consequence.into(),
        resolution: if banner {
            "choice_available"
        } else {
            "request_change_required"
        }
        .into(),
        supported_choices: if banner {
            vec![
                "keep_in_place".into(),
                "carry_with_item_using_full_anchors".into(),
            ]
        } else {
            Vec::new()
        },
        selected_choice: banner.then(|| "keep_in_place".into()),
        blocks_applicability: !banner,
        chain_diagnostic_ids: Vec::new(),
        lexical_uncertainty: None,
    };
    result.account(descriptor_bytes(&value)?)?;
    result.decisions.push(value);
    result.counts.decisions = result.decisions.len();
    Ok(())
}
pub(super) fn risks(
    source: &FileSnapshot,
    data: &ParsedFile,
    lines: &Lines<'_>,
    controls: Controls<'_>,
    result: &mut SuggestSplitEnvelope,
) -> Result<(), DomainError> {
    for record in result.scope_trivia.clone() {
        controls.check()?;
        if record.classification == "ambiguous" {
            add_decision(
                result,
                ("trivia_ownership", DecisionReason::OrdinaryTriviaChoice),
                &source.path,
                record.span,
                record.adjacent_item_ids,
                (
                    "ordinary ambiguous trivia stays in source by default; section evidence assigns no ownership",
                    "retain by default or submit a supported ordinary-comment carry choice with full anchors to move_item",
                ),
                true,
            )?;
        } else if record.protected
            && record.reason == "scope_prologue"
            && source.source[record.span.range.start_byte..record.span.range.end_byte]
                .starts_with("#![")
            && !items::context_independent_attribute(
                &source.source[record.span.range.start_byte..record.span.range.end_byte],
            )
        {
            add_decision(
                result,
                (
                    "scope_dependency",
                    DecisionReason::ConditionalOrInheritedContext,
                ),
                &source.path,
                record.span,
                Vec::new(),
                (
                    "scope attributes remain in source and their inherited effects are not evaluated or copied",
                    "preserve the scope through a supported explicit move or change the request; cfg is not evaluated",
                ),
                false,
            )?;
        }
    }
    for item in result.inventory.clone() {
        controls.check()?;
        let node = item_node(data, &item);
        let mut risks: BTreeMap<(&str, DecisionReason), Vec<ByteRange>> = BTreeMap::new();
        // Reserve transient typed risk records before accumulating their ranges.
        result.account((item.attributes.len() + 3).saturating_mul(128))?;
        if let Some(category) = items::category(&item.kind) {
            risks.insert(
                (category, DecisionReason::UnsupportedUnitKind),
                vec![item.span.range.clone()],
            );
        }
        if item.visibility_key == "restricted" {
            risks.insert(
                (
                    "visibility_context",
                    DecisionReason::VisibilityScopeUnproved,
                ),
                vec![item.span.range.clone()],
            );
        }
        if item.visibility_key == "pub" {
            risks.insert(
                ("reexport_dependency", DecisionReason::PublicPathChange),
                vec![item.span.range.clone()],
            );
        }
        for attribute in &item.attributes {
            controls.check()?;
            if attribute.range.start_byte >= item.span.range.start_byte {
                continue;
            }
            let text = &source.source[attribute.range.start_byte..attribute.range.end_byte];
            if !items::context_independent_attribute(text) {
                risks
                    .entry((
                        "scope_dependency",
                        DecisionReason::ConditionalOrInheritedContext,
                    ))
                    .or_default()
                    .push(attribute.range.clone());
            }
        }
        let mut stack = vec![node];
        while let Some(node) = stack.pop() {
            controls.check()?;
            result.account(128)?;
            match node.kind() {
                "line_comment"
                | "block_comment"
                | "attribute_item"
                | "inner_attribute_item"
                | "token_tree" => continue,
                "macro_invocation" | "macro_definition" => {
                    risks
                        .entry(("macro_dependency", DecisionReason::MacroContextUnexamined))
                        .or_default()
                        .push(range(node));
                    continue;
                }
                "mod_item" => {
                    risks
                        .entry(("module_context", DecisionReason::UnsupportedConstruct))
                        .or_default()
                        .push(range(node));
                    continue;
                }
                "use_wildcard" => {
                    risks
                        .entry(("glob_dependency", DecisionReason::GlobBindingUnproved))
                        .or_default()
                        .push(range(node));
                }
                "field_expression" => {
                    risks
                        .entry((
                            "visibility_context",
                            DecisionReason::MemberOrConstructorUnproved,
                        ))
                        .or_default()
                        .push(range(node));
                }
                "qualified_type" | "scoped_type_identifier" => {
                    risks
                        .entry(("visibility_context", DecisionReason::UnsupportedConstruct))
                        .or_default()
                        .push(range(node));
                }
                _ => {}
            }
            for i in (0..node.named_child_count()).rev() {
                controls.check()?;
                stack.push(node.named_child(i as u32).expect("child"));
            }
        }
        for ((category, reason), ranges) in risks {
            for range in ranges {
                controls.check()?;
                let consequence = match category {
                    "module_context" => {
                        "module relocation/context is not established by partition membership"
                    }
                    "scope_dependency" => {
                        "written scope/import/attribute context may affect unselected code; partitioning does not preserve or evaluate it"
                    }
                    "macro_dependency" => {
                        "macro expansion and path-sensitive token context are unexamined; no token-tree cohesion/rewrites were inferred"
                    }
                    "glob_dependency" => {
                        "wildcard binding provenance is unknown; no wildcard imports will be synthesized"
                    }
                    "reexport_dependency" => {
                        "public/reexport paths may change; no API compatibility guarantee or automatic reexport is supplied"
                    }
                    "visibility_context" => {
                        "restricted/member/associated/type-directed access is not resolved by advisory grouping"
                    }
                    _ => "the written construct is outside mechanically supported move evidence",
                };
                add_decision(
                    result,
                    (category, reason),
                    &source.path,
                    lines.slice(range.start_byte, range.end_byte, result.limits.text_bytes),
                    vec![item.id.clone()],
                    (
                        consequence,
                        "inspect the anchored context and submit explicit supported moves/choices or change/narrow the request; acknowledgment alone is not execution proof",
                    ),
                    false,
                )?;
            }
        }
    }
    controls.check()?;
    result.decisions.sort_by(|a, b| {
        (
            &a.category,
            &a.anchors[0].path,
            &a.anchors[0].span.range,
            &a.item_ids,
            &a.unresolved_consequence,
        )
            .cmp(&(
                &b.category,
                &b.anchors[0].path,
                &b.anchors[0].span.range,
                &b.item_ids,
                &b.unresolved_consequence,
            ))
    });
    for (index, decision) in result.decisions.iter_mut().enumerate() {
        controls.check()?;
        decision.id = format!("d/{index}");
    }
    Ok(())
}
