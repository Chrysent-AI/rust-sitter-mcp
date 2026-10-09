//! Deterministic cluster selection, balanced alternatives and admitted sibling layout search.
use super::*;
use std::collections::{BTreeMap, BTreeSet};
#[cfg(test)]
mod tests;

struct Layout<'a> {
    scope: &'a Scope,
    request: &'a SuggestSplitRequest,
    files: &'a BTreeMap<String, FileSnapshot>,
    parsed: &'a BTreeMap<String, ParsedFile>,
    contexts: &'a BTreeMap<String, items::ModuleEvidence>,
    analysis: &'a items::ModuleAnalysis,
    controls: Controls<'a>,
}
struct GroupFacts {
    sizes: Sizes,
    facts: BTreeMap<String, usize>,
    signal_ids: Vec<String>,
    prefix: Option<String>,
}
fn balanced(
    result: &SuggestSplitEnvelope,
    controls: Controls<'_>,
) -> Result<Vec<Vec<usize>>, DomainError> {
    let mut groups = vec![Vec::new(), Vec::new()];
    let mut eligible = Vec::new();
    for (i, item) in result.inventory.iter().enumerate() {
        controls.check()?;
        if draftable(item) {
            eligible.push(i);
        } else {
            groups[0].push(i);
        }
    }
    // B always retains the earliest eligible unit, even if context units already stay.
    groups[0].push(eligible.remove(0));
    if eligible.len() == 1 {
        groups[1] = eligible;
    } else {
        let total: usize = eligible.iter().map(|i| result.inventory[*i].bytes).sum();
        let mut cumulative = 0usize;
        let mut best = (usize::MAX, 1usize);
        for boundary in 1..eligible.len() {
            controls.check()?;
            cumulative += result.inventory[eligible[boundary - 1]].bytes;
            let delta = cumulative.abs_diff(total - cumulative);
            if delta < best.0 {
                best = (delta, boundary);
            }
        }
        groups[1] = eligible[..best.1].to_vec();
        groups.push(eligible[best.1..].to_vec());
    }
    groups[0].sort_unstable();
    Ok(groups)
}
fn primary(
    result: &SuggestSplitEnvelope,
    controls: Controls<'_>,
) -> Result<Vec<Vec<usize>>, DomainError> {
    let index: BTreeMap<_, _> = result
        .inventory
        .iter()
        .enumerate()
        .map(|(i, item)| (&item.id, i))
        .collect();
    let mut groups = vec![Vec::new()];
    let mut moved: BTreeSet<usize> = BTreeSet::new();
    for candidate in &result.ownership_candidates {
        controls.check()?;
        let members: Vec<_> = candidate.core_item_ids.iter().map(|id| index[id]).collect();
        if members.iter().any(|m| moved.contains(m)) {
            continue;
        }
        moved.extend(members.iter().copied());
        groups.push(members);
        if groups.len() == 3 {
            break;
        }
    }
    for i in 0..result.inventory.len() {
        controls.check()?;
        if !moved.contains(&i) {
            groups[0].push(i);
        }
    }
    if groups[0].is_empty() {
        let earliest = groups
            .iter()
            .skip(1)
            .flatten()
            .copied()
            .min()
            .expect("eligible units");
        groups[0].push(earliest);
        for group in groups.iter_mut().skip(1) {
            controls.check()?;
            group.retain(|i| *i != earliest);
        }
        groups.retain(|g| !g.is_empty());
    }
    Ok(groups)
}
fn group_facts(
    members: &[usize],
    result: &SuggestSplitEnvelope,
    controls: Controls<'_>,
) -> Result<GroupFacts, DomainError> {
    let ids: BTreeSet<_> = members
        .iter()
        .map(|i| result.inventory[*i].id.as_str())
        .collect();
    let mut sizes = Sizes {
        items: members.len(),
        ..Sizes::default()
    };
    for i in members {
        controls.check()?;
        sizes.bytes += result.inventory[*i].bytes;
        sizes.lines += result.inventory[*i].lines;
    }
    let mut facts = BTreeMap::from([
        ("internal_reference_occurrences".into(), 0),
        ("same_prefix_members".into(), 0),
        ("section_attribute_doc_agreements".into(), 0),
        ("independent_signal_families".into(), 0),
    ]);
    let mut families = BTreeSet::new();
    let mut links = Vec::new();
    let mut prefix = None;
    for signal in &result.signals {
        controls.check()?;
        let participating = signal
            .item_ids
            .iter()
            .filter(|id| ids.contains(id.as_str()))
            .count();
        if participating == 0 {
            continue;
        }
        links.push(signal.id.clone());
        match signal.kind.as_str() {
            "reference_candidate"
                if participating == signal.item_ids.len()
                    && signal.from_item_id != signal.to_item_id
                    && signal.facts.get("ambiguous_binding") != Some(&1) =>
            {
                *facts
                    .get_mut("internal_reference_occurrences")
                    .expect("fact") += signal.count;
                families.insert("references");
            }
            "name_prefix"
                if participating >= 2
                    && signal.facts.get("common_prefix") != Some(&1)
                    && signal.facts.get("duplicate_names").copied().unwrap_or(0) == 0 =>
            {
                let n = facts.get_mut("same_prefix_members").expect("fact");
                *n = (*n).max(participating);
                families.insert("prefix");
                let label = signal.label.as_ref().expect("prefix label");
                // Prefer a prefix shared by the entire group, then longest, then ASCII order.
                if participating == members.len()
                    && prefix.as_ref().is_none_or(|old: &String| {
                        (std::cmp::Reverse(label.len()), label)
                            < (std::cmp::Reverse(old.len()), old)
                    })
                {
                    prefix = Some(label.clone());
                }
            }
            "banner_section" | "shared_attribute" | "doc_heading" if participating >= 2 => {
                *facts
                    .get_mut("section_attribute_doc_agreements")
                    .expect("fact") += participating - 1;
                families.insert(match signal.kind.as_str() {
                    "banner_section" => "section",
                    "shared_attribute" => "attribute",
                    _ => "doc",
                });
            }
            _ => {}
        }
    }
    facts.insert("independent_signal_families".into(), families.len());
    Ok(GroupFacts {
        sizes,
        facts,
        signal_ids: links,
        prefix,
    })
}
enum ParentOutcome<'a> {
    Supported(&'a str),
    Unsupported(Vec<items::ChainDiagnostic>),
}
fn supported_parent<'a>(layout: &Layout<'a>) -> Result<ParentOutcome<'a>, DomainError> {
    let Layout {
        request,
        parsed,
        contexts,
        controls,
        ..
    } = *layout;
    if contexts
        .get(&request.source_path)
        .is_none_or(|e| !e.unresolved.is_empty())
    {
        return Ok(ParentOutcome::Unsupported(layout.analysis.project(
            &request.crate_root,
            &request.source_path,
            items::ChainRole::Source,
            (controls.deadline, controls.cancelled),
        )?));
    }
    let directory = Path::new(&request.source_path)
        .parent()
        .expect("source directory");
    let sample = directory
        .join("split_part.rs")
        .to_str()
        .expect("UTF-8")
        .to_owned();
    let mut candidates = Vec::new();
    for (path, evidence) in contexts {
        controls.check()?;
        if evidence.unresolved.is_empty()
            && items::child_path(path, &request.crate_root, "split_part") == sample
            && trivia::move_clean(&parsed[path].tree, controls.deadline, controls.cancelled)?
        {
            candidates.push(path.as_str());
        }
    }
    // Multiple candidate parents are not resolved by choosing one alphabetically.
    if candidates.len() == 1 {
        Ok(ParentOutcome::Supported(
            parsed
                .get_key_value(candidates[0])
                .expect("parsed parent")
                .0
                .as_str(),
        ))
    } else {
        let mut diagnostic = items::ChainDiagnostic::boundary(
            &request.crate_root,
            &request.source_path,
            items::ChainRole::DeclarationParent,
            if candidates.is_empty() {
                items::ChainReason::NoOrdinarySiblingParent
            } else {
                items::ChainReason::AmbiguousParent
            },
        );
        diagnostic.evidenced_prefix_paths = contexts[&request.source_path].filesystem_paths.clone();
        diagnostic.at_file_path = request.source_path.clone();
        diagnostic.parent_candidates = candidates.into_iter().map(str::to_owned).collect();
        Ok(ParentOutcome::Unsupported(vec![diagnostic]))
    }
}
fn filename(request: &SuggestSplitRequest, prefix: Option<&str>) -> String {
    if let Some(prefix) = prefix
        && items::module_name(&format!("{prefix}.rs")).is_ok()
    {
        return prefix.into();
    }
    match items::module_name(&request.source_path) {
        Ok(stem) => format!("{stem}_part"),
        Err(_) => "split_part".into(),
    }
}
fn choose_destination(
    layout: &Layout<'_>,
    parent: &str,
    base: &str,
    reserved: &mut BTreeSet<String>,
) -> Result<Option<ProposedDestination>, DomainError> {
    let Layout {
        scope,
        request,
        files,
        parsed,
        contexts,
        controls,
        ..
    } = *layout;
    let data = &parsed[parent];
    let source = &files[parent].source;
    let mut names: BTreeMap<String, Vec<&Item>> = BTreeMap::new();
    let mut imports = BTreeSet::new();
    for item in &data.items {
        controls.check()?;
        if let Some(name) = &item.name {
            names
                .entry(name.trim_start_matches("r#").into())
                .or_default()
                .push(item);
        }
        if item.kind == "use_declaration" {
            let node = data
                .tree
                .root_node()
                .named_descendant_for_byte_range(
                    item.span.range.start_byte,
                    item.span.range.end_byte,
                )
                .expect("item");
            for leaf in items::use_leaves(node, source, (controls.deadline, controls.cancelled))? {
                controls.check()?;
                imports.insert(leaf.binding.trim_start_matches("r#").to_owned());
            }
        }
    }
    let directory = Path::new(&request.source_path)
        .parent()
        .expect("source directory");
    for suffix in 1..=99 {
        controls.check()?;
        let name = if suffix == 1 {
            base.into()
        } else {
            format!("{base}_{suffix}")
        };
        let path = directory
            .join(format!("{name}.rs"))
            .to_str()
            .expect("UTF-8 path")
            .to_owned();
        if items::module_name(&path).is_err()
            || path == request.source_path
            || reserved.contains(&path.to_lowercase())
            || items::child_path(parent, &request.crate_root, &name) != path
            || imports.contains(&name)
        {
            continue;
        }
        let mut declaration = None;
        if let Some(collisions) = names.get(&name) {
            let item = collisions[0];
            let node = data
                .tree
                .root_node()
                .named_descendant_for_byte_range(
                    item.span.range.start_byte,
                    item.span.range.end_byte,
                )
                .expect("item");
            if collisions.len() != 1
                || item.kind != "mod_item"
                || !item.attributes.is_empty()
                || node.child_by_field_name("body").is_some()
                || item.visibility_key == "restricted"
            {
                continue;
            }
            declaration = Some(AdviceAnchor {
                path: parent.into(),
                span: Lines::new(source).slice(
                    item.span.range.start_byte,
                    item.span.range.end_byte,
                    request.limits.text_bytes,
                ),
            });
        }
        if items::present(scope, &format!("{}/mod.rs", path.trim_end_matches(".rs")))? {
            continue;
        }
        match scope.admit_new_path(&path, controls.deadline, controls.cancelled) {
            Ok(()) => {}
            Err(e)
                if matches!(
                    e.code.as_str(),
                    "INVALID_DESTINATION" | "DESTINATION_ALREADY_EXISTS"
                ) =>
            {
                continue;
            }
            Err(e) => return Err(e),
        }
        reserved.insert(path.to_lowercase());
        return Ok(Some(ProposedDestination {
            kind: "new_sibling".into(),
            path,
            parent_path: parent.into(),
            parent_module: module_description(&contexts[parent], files, request.limits.text_bytes),
            existing_declaration: declaration,
        }));
    }
    Ok(None)
}
fn make_draft(
    membership: Vec<Vec<usize>>,
    fallback: bool,
    layout: &Layout<'_>,
    parent: &str,
    result: &mut SuggestSplitEnvelope,
) -> Result<Option<Draft>, DomainError> {
    let controls = layout.controls;
    let request = layout.request;
    let mut seen = BTreeSet::new();
    let mut assignments = vec![0; result.inventory.len()];
    let mut groups = Vec::new();
    let mut reserved = BTreeSet::new();
    for (index, members) in membership.iter().enumerate() {
        controls.check()?;
        if members.is_empty() {
            return Err(DomainError::new("INTERNAL", "empty advisory group"));
        }
        for member in members {
            controls.check()?;
            if !seen.insert(*member) {
                return Err(DomainError::new("INTERNAL", "duplicate advisory member"));
            }
            assignments[*member] = index;
        }
        let GroupFacts {
            sizes,
            facts,
            signal_ids,
            prefix,
        } = group_facts(members, result, controls)?;
        let destination = if index == 0 {
            None
        } else {
            let base = filename(request, prefix.as_deref());
            let Some(value) = choose_destination(layout, parent, &base, &mut reserved)? else {
                return Ok(None);
            };
            Some(value)
        };
        let mut warnings = Vec::new();
        if sizes.bytes > 64 * 1024 || sizes.lines > 256 {
            warnings
                .push("group exceeds soft 64-KiB/256-line advice; not an execution limit".into());
        }
        for member in members {
            controls.check()?;
            let item = &result.inventory[*member];
            if item.bytes > 64 * 1024 || item.lines > 256 {
                warnings.push(format!(
                    "{} is indivisible and exceeds soft size advice",
                    item.id
                ));
            }
        }
        let high = !fallback && facts["independent_signal_families"] >= 2;
        let rationale = if index == 0 {
            format!(
                "retain unsupported/context-sensitive and unallocated units ({} items); scope trivia remains in this file",
                sizes.items
            )
        } else if fallback {
            format!(
                "original-order byte-balanced alternative, not a natural semantic boundary; {} bytes, {} lines; {} internal candidate occurrences, {} same-prefix members, {} section/attribute/doc agreements",
                sizes.bytes,
                sizes.lines,
                facts["internal_reference_occurrences"],
                facts["same_prefix_members"],
                facts["section_attribute_doc_agreements"]
            )
        } else {
            format!(
                "ranked cohesion cluster: {} internal candidate occurrences, {} same-prefix members, {} section/attribute/doc agreements; {} independent families; whole units only",
                facts["internal_reference_occurrences"],
                facts["same_prefix_members"],
                facts["section_attribute_doc_agreements"],
                facts["independent_signal_families"]
            )
        };
        groups.push(Group {
            kind: if index == 0 { "retain" } else { "new_sibling" }.into(), destination,
            item_ids: members.iter().map(|i| result.inventory[*i].id.clone()).collect(), rationale,
            confidence: Confidence { basis: "syntactic_heuristic".into(), level: if high { "high" } else { "low" }.into(), limitations: vec!["integer organization facts, not probability or move safety; symbols/types/cfg/macros/public API not verified".into()] },
            sizes, overlap_ids: Vec::new(), size_interpretation: None, signal_ids, facts, warnings,
            expected_to_block: ExpectedBlocks {
                lower_bound: true,
                counts: ExpectedBlockCounts::default(),
                decision_ids: Vec::new(),
                note: "observed lower bound, not a move plan or safe-move verdict; move_item adds consumer/destination/batch/module-chain/trivia checks and may deduplicate or repair written dependencies".into(),
            },
            consequence_summary: ConsequenceSummary::default(),
            test_coupled: false,
            assessment_scope: AssessmentScope {
                assessed: "local_only".into(),
                not_assessed: vec!["other_consumers".into(), "destination".into(), "batch".into(), "module_chain".into(), "trivia_ownership".into()],
                note: "only written source-local risks and observed same-file cfg(test) consumers; zero counts mean no observed local risks, not a safe move".into(),
            },
        });
    }
    if seen.len() != result.inventory.len() || groups.len() < 2 {
        return Err(DomainError::new(
            "INTERNAL",
            "incomplete/nontrivial advisory membership",
        ));
    }
    let index: BTreeMap<_, _> = result
        .inventory
        .iter()
        .enumerate()
        .map(|(i, item)| (item.id.clone(), i))
        .collect();
    let mut cross_group_signal_ids = Vec::new();
    let crossing: Vec<_> = result
        .signals
        .iter()
        .filter(|s| {
            s.kind == "reference_candidate"
                && assignments[index[s.from_item_id.as_ref().expect("source")]]
                    != assignments[index[s.to_item_id.as_ref().expect("target")]]
        })
        .cloned()
        .collect();
    for signal in crossing {
        controls.check()?;
        cross_group_signal_ids.push(signal.id.clone());
        for id in &signal.item_ids {
            groups[assignments[index[id]]].warnings.push(format!("{} has {} crossing written-reference occurrences; review imports/paths/visibility via explicit batch", signal.id, signal.count));
        }
        let consequence = "cross-group written reference candidates require import/path/visibility review; grouping alone does not establish any repair or binding proof";
        if !result
            .decisions
            .iter()
            .any(|d| d.unresolved_consequence == consequence && d.item_ids == signal.item_ids)
        {
            let action = DecisionAction::cause(DecisionReason::CrossGroupReferenceReview);
            let decision = AdviceDecision {
                reason: DecisionReason::CrossGroupReferenceReview,
                next_action: action.next_action(),
                action,
                id: String::new(),
                category: "scope_dependency".into(),
                anchors: signal
                    .evidence
                    .iter()
                    .map(|span| AdviceAnchor {
                        path: request.source_path.clone(),
                        span: span.clone(),
                    })
                    .collect(),
                item_ids: signal.item_ids,
                evidence: signal.evidence,
                unresolved_consequence: consequence.into(),
                resolution: "choice_available".into(),
                supported_choices: vec![
                    "review_move_item_defaults".into(),
                    "submit_supported_anchored_rewrite_alternative".into(),
                ],
                selected_choice: None,
                blocks_applicability: true,
                chain_diagnostic_ids: Vec::new(),
                lexical_uncertainty: None,
            };
            result.account(descriptor_bytes(&decision)?)?;
            result.decisions.push(decision);
        }
    }
    Ok(Some(Draft { id: String::new(), advisory: true, source_snapshot_id: result.snapshot_id.clone().expect("complete snapshot"), groups,
        rationale: if fallback { "low-confidence original-order balanced alternative; no semantic boundary claimed" } else { "rank written cores by structural support, distinct unit relationships, fewer boundary costs/uncertainties, weak agreements, then source order; companions are not selected" }.into(),
        cross_group_signal_ids, unresolved_decision_ids: Vec::new(),
    }))
}
pub(super) fn build(
    scope: &Scope,
    request: &SuggestSplitRequest,
    files: &BTreeMap<String, FileSnapshot>,
    parsed: &BTreeMap<String, ParsedFile>,
    analysis: &items::ModuleAnalysis,
    controls: Controls<'_>,
    result: &mut SuggestSplitEnvelope,
) -> Result<(), DomainError> {
    let layout = Layout {
        scope,
        request,
        files,
        parsed,
        contexts: &analysis.contexts,
        analysis,
        controls,
    };
    let parent = match supported_parent(&layout)? {
        ParentOutcome::Supported(parent) => parent,
        ParentOutcome::Unsupported(diagnostics) => {
            if result.partition_outcome == "credible_written_candidates" {
                result.partition_outcome = "unsupported_layout".into();
            }
            result
                .draft_eligibility
                .reasons
                .push("unsupported_or_uncertain_ordinary_layout".into());
            for mut diagnostic in diagnostics {
                controls.check()?;
                diagnostic.id = format!("chain/{}", result.chain_diagnostics.len());
                let anchor = diagnostic
                    .declaration
                    .as_ref()
                    .map(|d| AdviceAnchor {
                        path: d.path.clone(),
                        span: Lines::new(&files[&d.path].source).slice(
                            d.range.start_byte,
                            d.range.end_byte,
                            request.limits.text_bytes,
                        ),
                    })
                    .unwrap_or_else(|| AdviceAnchor {
                        path: request.source_path.clone(),
                        span: result.inventory[0].span.clone(),
                    });
                let action = DecisionAction::chain(&diagnostic, "suggest_split");
                let decision = AdviceDecision {
                    reason: DecisionReason::ModuleChainFailure,
                    next_action: action.next_action(),
                    action,
                    id: String::new(),
                    category: "module_context".into(),
                    anchors: vec![anchor],
                    item_ids: Vec::new(),
                    evidence: Vec::new(),
                    unresolved_consequence: diagnostic.message(),
                    resolution: "request_change_required".into(),
                    supported_choices: Vec::new(),
                    selected_choice: None,
                    blocks_applicability: true,
                    chain_diagnostic_ids: vec![diagnostic.id.clone()],
                    lexical_uncertainty: None,
                };
                result.account(descriptor_bytes(&(&diagnostic, &decision))?)?;
                result.chain_diagnostics.push(diagnostic);
                result.decisions.push(decision);
            }
            return Ok(());
        }
    };
    if result.ownership_candidates.is_empty() {
        result
            .draft_eligibility
            .reasons
            .push(result.partition_outcome.clone());
        return Ok(());
    }
    let a = primary(result, controls)?;
    let Some(first) = make_draft(a.clone(), false, &layout, parent, result)? else {
        result
            .draft_eligibility
            .reasons
            .push("no_admitted_nonconflicting_name_in_base_or_suffix_2_through_99".into());
        return Ok(());
    };
    result.drafts.push(first);
    if request.include_balanced.unwrap_or(false) {
        let b = balanced(result, controls)?;
        if a != b
            && let Some(second) = make_draft(b, true, &layout, parent, result)?
        {
            result.drafts.push(second);
        }
    }
    result.counts.drafts = result.drafts.len();
    result.draft_eligibility.state = "drafted".into();
    Ok(())
}
pub(super) fn finalize(
    result: &mut SuggestSplitEnvelope,
    controls: Controls<'_>,
) -> Result<(), DomainError> {
    controls.check()?;
    let links = items::finalize_chain(&mut result.chain_diagnostics);
    for decision in &mut result.decisions {
        for id in &mut decision.chain_diagnostic_ids {
            *id = links[id].clone();
        }
    }
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
    result.decision_groups = decision_groups(
        result.decisions.iter().map(|d| {
            (
                d.category.as_str(),
                d.reason,
                &d.action,
                d.blocks_applicability,
                d.id.as_str(),
                Some(d.unresolved_consequence.as_str()),
            )
        }),
        &mut result.counts.analysis_descriptor_bytes,
        (controls.deadline, controls.cancelled),
    )?
    .into_iter()
    .map(|summary| {
        let decision = &result.decisions[summary.decision_ids[0]
            .strip_prefix("d/")
            .expect("decision prefix")
            .parse::<usize>()
            .expect("decision index")];
        AdviceDecisionGroup {
            unresolved_consequence: decision.unresolved_consequence.clone(),
            actions: vec![decision.action.summary()],
            summary: DecisionGroup {
                decision_ids: id_runs(&summary.decision_ids),
                category: summary.category,
                reason: summary.reason,
                route: summary.route,
                blocks_applicability: summary.blocks_applicability,
                count: summary.count,
            },
        }
    })
    .collect();
    for (index, draft) in result.drafts.iter_mut().enumerate() {
        controls.check()?;
        draft.id = format!("draft/{index}");
        let assignments: BTreeMap<_, _> = draft
            .groups
            .iter()
            .enumerate()
            .flat_map(|(i, group)| group.item_ids.iter().map(move |id| (id.as_str(), i)))
            .collect();
        for overlap in &mut result.overlaps {
            controls.check()?;
            let impl_group_index = assignments[overlap.item_ids[0].as_str()];
            let member_group_index = assignments[overlap.item_ids[1].as_str()];
            overlap.draft_groups.push(OverlapDraftGroups {
                draft_id: draft.id.clone(),
                impl_group_index,
                member_group_index,
            });
        }
        for overlap in &result.overlaps {
            controls.check()?;
            let link = overlap.draft_groups.last().expect("draft overlap link");
            for group_index in BTreeSet::from([link.impl_group_index, link.member_group_index]) {
                let group = &mut draft.groups[group_index];
                group.overlap_ids.push(overlap.id.clone());
                group.size_interpretation = Some(SizeInterpretation::non_additive());
            }
        }
        let moved: BTreeSet<_> = draft
            .groups
            .iter()
            .skip(1)
            .flat_map(|g| &g.item_ids)
            .collect();
        for decision in &result.decisions {
            controls.check()?;
            if decision.item_ids.is_empty() || decision.item_ids.iter().any(|id| moved.contains(id))
            {
                draft.unresolved_decision_ids.push(decision.id.clone());
            }
        }
        for group in &mut draft.groups {
            controls.check()?;
            let members: BTreeSet<_> = group.item_ids.iter().collect();
            let mut test_spans = BTreeSet::new();
            for signal in &result.signals {
                controls.check()?;
                if signal.kind == "cfg_test_consumer"
                    && signal.item_ids.iter().any(|id| members.contains(id))
                {
                    test_spans.extend(signal.evidence.iter().map(|e| &e.range));
                }
            }
            group.test_coupled = !test_spans.is_empty();
            for decision in &result.decisions {
                controls.check()?;
                // Cross-group concerns depend on a batch, not a local lower bound.
                if !decision.blocks_applicability
                    || decision.reason == DecisionReason::CrossGroupReferenceReview
                    || !(decision.item_ids.is_empty()
                        || decision.item_ids.iter().any(|id| members.contains(id)))
                {
                    continue;
                }
                let counts = &mut group.expected_to_block.counts;
                if decision
                    .anchors
                    .iter()
                    .any(|a| test_spans.contains(&a.span.range))
                {
                    counts.cfg_test_consumer += 1;
                } else {
                    match decision.reason {
                        DecisionReason::MemberOrConstructorUnproved => counts.member_call += 1,
                        DecisionReason::MacroContextUnexamined => counts.macro_context += 1,
                        DecisionReason::ExternalOrMissingBinding => counts.external_binding += 1,
                        DecisionReason::ConditionalOrInheritedContext => {
                            counts.conditional_or_derive += 1
                        }
                        _ => counts.other_local += 1,
                    }
                }
                group
                    .expected_to_block
                    .decision_ids
                    .push(decision.id.clone());
            }
        }
    }
    result.account(descriptor_bytes(&result.drafts)?)?;
    result.account(descriptor_bytes(&result.overlaps)?)?;
    consequences::attach(result, controls)
}
