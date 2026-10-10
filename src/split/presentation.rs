//! Presentation-only selection obligations and extraction payoff; no move proof.
use super::*;

#[derive(Debug, Clone, Default, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct SelectionCompleteness {
    pub state: String,
    pub missing_item_ids: Vec<String>,
    pub uncertain_item_ids: Vec<String>,
    pub applicability: String,
}
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct InspectionGroup {
    pub consequence_summary: ConsequenceSummary,
    pub confidence: Confidence,
    pub kind: String,
    pub item_ids: Vec<String>,
    pub selection_completeness: SelectionCompleteness,
    pub sizes: Sizes,
    pub rationale: String,
    pub warnings: Vec<String>,
}
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct InspectionPartition {
    pub rationale: String,
    pub id: String,
    pub source_snapshot_id: String,
    pub groups: Vec<InspectionGroup>,
    pub applicability: String,
}
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct SizedItem {
    pub item_id: String,
    pub bytes: usize,
    pub lines: usize,
    pub supported_member_partition: bool,
}
#[derive(Debug, Clone, Default, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct ExtractionPayoff {
    pub completed: bool,
    pub basis: String,
    pub source_bytes: usize,
    pub largest_items: Vec<SizedItem>,
    pub dominant_item_ids: Vec<String>,
    pub expected_useful_extraction: Sizes,
    pub extraction_item_ids: Vec<String>,
    pub limitations: Vec<String>,
}

// Obligations are only local written nominal associations, not arbitrary reference
// dependencies. Treat ambiguity separately rather than asserting missing ownership.
pub(super) fn completeness(
    ids: &[String],
    result: &SuggestSplitEnvelope,
    controls: Controls<'_>,
) -> Result<SelectionCompleteness, DomainError> {
    let selected: BTreeSet<_> = ids.iter().collect();
    let mut missing = BTreeSet::new();
    let mut uncertain = BTreeSet::new();
    for signal in &result.signals {
        controls.check()?;
        if signal.kind != "impl_owner_bundle" {
            continue;
        }
        let owner = signal.from_item_id.as_ref().expect("nominal owner");
        let implementation = signal.to_item_id.as_ref().expect("impl");
        if !selected.contains(owner) {
            continue;
        }
        if signal.facts.get("ambiguous_binding") == Some(&1) {
            uncertain.insert(implementation.clone());
            continue;
        }
        let members: Vec<_> = result
            .inventory
            .iter()
            .filter(|i| i.enclosing_impl_id.as_ref() == Some(implementation))
            .collect();
        // Either explicit whole impl or its complete supported member set can
        // represent the obligation, but never infer a partial subset is complete.
        let represented = selected.contains(implementation)
            || (signal.facts.get("excluded_impl") != Some(&1)
                && signal.facts.get("generic_impl") != Some(&1)
                && !members.is_empty()
                && members
                    .iter()
                    .all(|m| draftable(m) && selected.contains(&m.id)));
        if !represented {
            missing.insert(implementation.clone());
            for member in members {
                if !selected.contains(&member.id) {
                    missing.insert(member.id.clone());
                }
            }
        }
    }
    // A selected nominal with no inspectable association is not a semantic
    // completeness claim; macros, generated and outside-file obligations remain unknown.
    Ok(SelectionCompleteness {
        state: if !missing.is_empty() {
            "incomplete"
        } else if !uncertain.is_empty() {
            "undetermined"
        } else {
            "no_observed_missing_obligations"
        }
        .into(),
        missing_item_ids: missing.into_iter().collect(),
        uncertain_item_ids: uncertain.into_iter().collect(),
        applicability: "not_assessed".into(),
    })
}

pub(super) fn bundle_alternative(
    core: &[String],
    result: &SuggestSplitEnvelope,
    controls: Controls<'_>,
) -> Result<Option<OwnershipAlternative>, DomainError> {
    let core_set: BTreeSet<_> = core.iter().collect();
    let mut included: BTreeSet<String> = core.iter().cloned().collect();
    let mut excluded = BTreeSet::new();
    let mut overlaps = BTreeSet::new();
    let inventory: BTreeMap<_, _> = result.inventory.iter().map(|i| (&i.id, i)).collect();
    let mut observed = false;
    for signal in &result.signals {
        controls.check()?;
        if signal.kind != "impl_owner_bundle"
            || !core_set.contains(signal.from_item_id.as_ref().expect("signal source"))
        {
            continue;
        }
        observed = true;
        let id = signal.to_item_id.as_ref().expect("signal target");
        let implementation = inventory[id];
        let members: Vec<_> = result
            .inventory
            .iter()
            .filter(|i| i.enclosing_impl_id.as_ref() == Some(id))
            .collect();
        overlaps.extend(implementation.overlap_ids.iter().cloned());
        if signal.facts.get("ambiguous_binding") == Some(&1)
            || signal.facts.get("excluded_impl") == Some(&1)
            || signal.facts.get("generic_impl") == Some(&1)
        {
            excluded.insert(id.clone());
            excluded.extend(members.iter().map(|m| m.id.clone()));
        } else if members.is_empty() && draftable(implementation) {
            included.insert(id.clone());
        } else if !members.is_empty() {
            // The whole descriptor is excluded because this alternative explicitly
            // chooses members. This overlap exclusion alone does not break completeness.
            excluded.insert(id.clone());
            for member in members {
                if draftable(member) {
                    included.insert(member.id.clone());
                } else {
                    excluded.insert(member.id.clone());
                }
            }
        } else {
            excluded.insert(id.clone());
        }
    }
    if !observed {
        return Ok(None);
    }
    // Source-order IDs make the explicit alternative stable and easy to inspect.
    let ids: Vec<_> = result
        .inventory
        .iter()
        .filter(|i| included.contains(&i.id))
        .map(|i| i.id.clone())
        .collect();
    let complete = completeness(&ids, result, controls)?;
    Ok(Some(OwnershipAlternative {
        kind: if complete.state == "no_observed_missing_obligations" {
            "complete_supported_bundle"
        } else { "supported_bundle_subset" }.into(),
        item_ids: ids,
        excluded_item_ids: result.inventory.iter().filter(|i| excluded.contains(&i.id))
            .map(|i| i.id.clone()).collect(),
        overlap_ids: overlaps.into_iter().collect(),
        applicability: "not_assessed; reviewed alternative requiring explicit selection; written local obligations only, no semantic completeness; excluded whole impls must not be selected with members".into(),
    }))
}

pub(super) fn payoff(
    result: &mut SuggestSplitEnvelope,
    controls: Controls<'_>,
) -> Result<(), DomainError> {
    let Some(source) = &result.source else {
        return Ok(());
    };
    let mut top: Vec<_> = result
        .inventory
        .iter()
        .filter(|i| i.enclosing_impl_id.is_none())
        .collect();
    top.sort_by_key(|i| (std::cmp::Reverse(i.bytes), i.span.range.start_byte));
    let mut partitionable = BTreeSet::new();
    for item in &result.inventory {
        controls.check()?;
        if draftable(item)
            && let Some(owner) = &item.enclosing_impl_id
        {
            partitionable.insert(owner.clone());
        }
    }
    let dominant: Vec<_> = result
        .inventory
        .iter()
        .filter(|i| {
            let indivisible = match &i.enclosing_impl_id {
                Some(owner) => partitionable.contains(owner),
                None => !partitionable.contains(&i.id),
            };
            indivisible && i.bytes >= source.bytes.div_ceil(2)
        })
        .map(|i| i.id.clone())
        .collect();
    // Match the primary's explicit core-only choice without selecting companions.
    let mut moved = BTreeSet::new();
    let mut groups = 0;
    for candidate in &result.ownership_candidates {
        controls.check()?;
        if candidate.core_item_ids.iter().any(|id| moved.contains(id)) {
            continue;
        }
        moved.extend(candidate.core_item_ids.iter().cloned());
        groups += 1;
        if groups == 2 {
            break;
        }
    }
    if moved.len() == result.inventory.len()
        && let Some(first) = result.inventory.first()
    {
        moved.remove(&first.id);
    }
    let mut sizes = Sizes::default();
    let mut ranges: Vec<_> = result
        .inventory
        .iter()
        .filter(|i| moved.contains(&i.id))
        .map(|i| (i.span.range.start_byte, i.span.range.end_byte, i.lines))
        .collect();
    ranges.sort_unstable();
    let mut end = 0;
    for (start, stop, lines) in ranges {
        controls.check()?;
        sizes.bytes += stop.saturating_sub(start.max(end));
        // Primary cores never select whole impls with their members. Avoid
        // counting contained lines should a future supported seed overlap.
        if start >= end {
            sizes.lines += lines;
        }
        end = end.max(stop);
    }
    sizes.items = moved.len();
    let mut limitations = vec!["original written core ranges only; not generated file size, move applicability, compilation or semantic cohesion".into()];
    if !dominant.is_empty() {
        limitations.push("dominant_indivisible_item: at least half of source bytes belong to one whole unit; small extractions cannot remove it".into());
    }
    let value = ExtractionPayoff {
        completed: true,
        basis: "primary_core_original_ranges; dominance_threshold_half_source_bytes; largest_three_top_level_units".into(),
        source_bytes: source.bytes,
        largest_items: top.into_iter().take(3).map(|i| SizedItem {
            item_id: i.id.clone(), bytes: i.bytes, lines: i.lines,
            supported_member_partition: partitionable.contains(&i.id),
        }).collect(),
        dominant_item_ids: dominant,
        expected_useful_extraction: sizes,
        extraction_item_ids: result.inventory.iter().filter(|i| moved.contains(&i.id))
            .map(|i| i.id.clone()).collect(),
        limitations,
    };
    result.account(descriptor_bytes(&value)?)?;
    result.extraction_payoff = value;
    Ok(())
}
