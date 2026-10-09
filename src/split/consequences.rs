//! Uncapped routing projection; observations never become planner repairability proofs.
use super::*;

const CLASSES: [&str; 6] = [
    "advice_only_review_risk",
    "selection_incomplete",
    "written_context_unprovable",
    "public_path_change",
    "consumer_outside_supported_repair",
    "missing_evidence",
];
#[derive(Debug, Clone, Default, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct ConsequenceMembership {
    pub count: usize,
    pub decision_refs: Vec<ConsequenceDecisionRef>,
}
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct ConsequenceDecisionRef {
    pub collection: String,
    pub id_runs: Vec<DecisionIdRun>,
}
#[derive(Debug, Clone, Default, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct UnmappedConsequences {
    #[serde(flatten)]
    pub membership: ConsequenceMembership,
    pub reasons: Vec<String>,
}
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct ConsequenceSummary {
    pub basis: &'static str,
    pub count_unit: &'static str,
    pub non_additive: bool,
    pub classes: BTreeMap<String, ConsequenceMembership>,
    pub unmapped: UnmappedConsequences,
    pub destination_and_batch_applicability: &'static str,
}
impl Default for ConsequenceSummary {
    fn default() -> Self {
        Self {
            basis: "uncapped_written_evidence",
            count_unit: "distinct_decision_records_per_class",
            non_additive: true,
            classes: CLASSES
                .into_iter()
                .map(|c| (c.into(), ConsequenceMembership::default()))
                .collect(),
            unmapped: UnmappedConsequences::default(),
            destination_and_batch_applicability: "not_assessed",
        }
    }
}
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct ConsequenceEvidenceRef {
    pub collection: String,
    /// Limitations and coverage lack record IDs; their stable array index/field is explicit.
    pub id: String,
}
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct ConsequenceAdviceDecision {
    pub id: String,
    pub reasons: Vec<String>,
    /// Empty means source-wide uncertainty, not evidence of zero affected units.
    pub item_ids: Vec<String>,
    pub evidence_refs: Vec<ConsequenceEvidenceRef>,
    pub applicability: &'static str,
}

/// Explicit reason mapping, not a severity/category or outside-consumer heuristic.
fn classes(reason: &str) -> &'static [&'static str] {
    match reason {
        "selection_completeness" => &["advice_only_review_risk", "selection_incomplete"],
        "boundary_dependency"
        | "boundary_observation_review"
        | "test_coupling_review"
        | "cross_group_reference_review"
        | "test_consumer_acknowledged"
        | "ordinary_trivia_choice"
        | "removal_gap_choice"
        | "post_move_import_review"
        | "public_or_exposed_boundary"
        | "import_boundary"
        | "observed_shared_node" => &["advice_only_review_risk"],
        "association_unproved"
        | "uncertain_identity"
        | "unsupported_context"
        | "boundary_identity_unproved"
        | "attributed_import"
        | "competing_written_binding"
        | "ambiguous_import_routes"
        | "glob_binding_candidate"
        | "lexical_macro_context_unproved"
        | "field_attributes_unproved"
        | "scope_attributes_or_syntax_unproved"
        | "ambiguous_target_identity"
        | "target_unresolved"
        | "macro_tokens_not_expanded_or_bound"
        | "enclosing_attributes_unproved"
        | "method_identity_unproved"
        | "token_shape_only" => &["advice_only_review_risk", "written_context_unprovable"],
        "unexamined_context"
        | "boundary_context_unexamined"
        | "unlinked_test_root"
        | "unsupported_attributes_or_remapping"
        | "competing_layouts"
        | "competing_module_routes"
        | "unsupported_test_cfg"
        | "unsupported_scope_attributes" => &[
            "advice_only_review_risk",
            "written_context_unprovable",
            "missing_evidence",
        ],
        "missing_test_file" | "unadmitted_test_file" => {
            &["advice_only_review_risk", "missing_evidence"]
        }
        "module_chain_failure" | "external_or_missing_binding" => {
            &["written_context_unprovable", "missing_evidence"]
        }
        "lexical_context_unproved"
        | "source_binding_ambiguous"
        | "destination_binding_conflict"
        | "final_alias_conflict"
        | "member_or_constructor_unproved"
        | "visibility_scope_unproved"
        | "conditional_or_inherited_context"
        | "macro_context_unexamined"
        | "glob_binding_unproved"
        | "unsupported_construct"
        | "trivia_preservation_unproved" => &["written_context_unprovable"],
        "public_path_change" => &["public_path_change"],
        "glob_consumer_unrepaired" => &[
            "written_context_unprovable",
            "consumer_outside_supported_repair",
        ],
        "unsupported_unit_kind" | "required_rewrite_retained" => &["selection_incomplete"],
        _ => &[],
    }
}
fn observations(
    result: &SuggestSplitEnvelope,
    controls: Controls<'_>,
) -> Result<Vec<ConsequenceAdviceDecision>, DomainError> {
    let mut records = Vec::new();
    let mut push = |collection: &str,
                    id: String,
                    item_ids: Vec<String>,
                    reasons: Vec<String>|
     -> Result<(), DomainError> {
        controls.check()?;
        records.push(ConsequenceAdviceDecision {
            id: format!("ad/{}", records.len()),
            reasons,
            item_ids,
            evidence_refs: vec![ConsequenceEvidenceRef {
                collection: collection.into(),
                id,
            }],
            applicability: "not_assessed",
        });
        Ok(())
    };
    for candidate in &result.ownership_candidates {
        for companion in &candidate.companions {
            let mut reasons = vec![companion.review_obligation.clone()];
            reasons.extend(companion.stop_reasons.clone());
            reasons.sort();
            reasons.dedup();
            push(
                "companions",
                companion.id.clone(),
                candidate.core_item_ids.clone(),
                reasons,
            )?;
        }
    }
    for observation in &result.boundary_observations.records {
        let mut reasons = vec!["boundary_observation_review".into()];
        if observation.certainty != "written_route" {
            reasons.push("boundary_identity_unproved".into());
        }
        push(
            "boundary_observations",
            observation.id.clone(),
            observation.item_ids.clone(),
            reasons,
        )?;
    }
    if result
        .boundary_observations
        .coverage
        .unsupported_context_count
        != 0
    {
        push(
            "boundary_observations",
            "coverage".into(),
            Vec::new(),
            vec!["boundary_context_unexamined".into()],
        )?;
    }
    for observation in &result.test_observations.records {
        let mut reasons = vec!["test_coupling_review".into()];
        reasons.extend(observation.uncertainty.clone());
        push(
            "test_observations",
            observation.id.clone(),
            observation.target_item_ids.clone(),
            reasons,
        )?;
    }
    for (index, limitation) in result.test_observations.limitations.iter().enumerate() {
        push(
            "test_limitations",
            index.to_string(),
            Vec::new(),
            vec![limitation.reason.clone()],
        )?;
    }
    Ok(records)
}
struct Record<'a> {
    collection: &'static str,
    id: &'a str,
    classes: BTreeSet<&'static str>,
    unmapped: BTreeSet<String>,
}
fn membership(records: &[Record<'_>], indices: &BTreeSet<usize>) -> ConsequenceMembership {
    let mut collections: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for index in indices {
        let record = &records[*index];
        collections
            .entry(record.collection)
            .or_default()
            .push(record.id);
    }
    ConsequenceMembership {
        count: indices.len(),
        decision_refs: collections
            .into_iter()
            .map(|(collection, ids)| {
                let mut runs: Vec<DecisionIdRun> = Vec::new();
                let mut previous = None;
                for id in ids {
                    let ordinal = id
                        .rsplit_once('/')
                        .expect("decision prefix")
                        .1
                        .parse::<usize>()
                        .expect("finalized decision ordinal");
                    if previous.is_some_and(|p| ordinal == p + 1) {
                        runs.last_mut().expect("preceding run").count += 1;
                    } else {
                        runs.push(DecisionIdRun {
                            first_id: id.into(),
                            count: 1,
                        });
                    }
                    previous = Some(ordinal);
                }
                ConsequenceDecisionRef {
                    collection: collection.into(),
                    id_runs: runs,
                }
            })
            .collect(),
    }
}
pub(super) fn attach(
    result: &mut SuggestSplitEnvelope,
    controls: Controls<'_>,
) -> Result<(), DomainError> {
    let advice = observations(result, controls)?;
    result.account(descriptor_bytes(&advice)?)?;
    result.counts.advice_decisions = advice.len();
    result.advice_decisions = advice;
    let mut records = Vec::new();
    let mut index: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    let mut global = BTreeSet::new();
    let local = result.decisions.iter().map(|d| {
        (
            "decisions",
            d.id.as_str(),
            &d.item_ids,
            vec![
                serde_json::to_value(d.reason)
                    .expect("reason")
                    .as_str()
                    .expect("reason string")
                    .to_owned(),
            ],
        )
    });
    let advice = result.advice_decisions.iter().map(|d| {
        (
            "advice_decisions",
            d.id.as_str(),
            &d.item_ids,
            d.reasons.clone(),
        )
    });
    for (collection, id, items, reasons) in local.chain(advice) {
        controls.check()?;
        let mut record = Record {
            collection,
            id,
            classes: BTreeSet::new(),
            unmapped: BTreeSet::new(),
        };
        for reason in reasons {
            let mapped = classes(&reason);
            if mapped.is_empty() {
                record.unmapped.insert(reason);
            }
            record.classes.extend(mapped);
        }
        if items.is_empty() {
            global.insert(records.len());
        }
        for item in items {
            index.entry(item).or_default().push(records.len());
        }
        records.push(record);
    }
    let mut bytes = 0usize;
    let accounted = result.counts.analysis_descriptor_bytes;
    let limit = result.effective_work_limits.analysis_descriptor_bytes;
    let mut summarize = |items: &[String]| -> Result<ConsequenceSummary, DomainError> {
        let mut relevant = global.clone();
        for item in items {
            controls.check()?;
            if let Some(indices) = index.get(item.as_str()) {
                relevant.extend(indices);
            }
        }
        let mut summary = ConsequenceSummary::default();
        let mut buckets: BTreeMap<&str, BTreeSet<usize>> = BTreeMap::new();
        let mut unmapped = BTreeSet::new();
        for i in relevant {
            controls.check()?;
            for class in &records[i].classes {
                buckets.entry(class).or_default().insert(i);
            }
            if !records[i].unmapped.is_empty() {
                unmapped.insert(i);
                summary
                    .unmapped
                    .reasons
                    .extend(records[i].unmapped.iter().cloned());
            }
        }
        for (class, indices) in buckets {
            summary
                .classes
                .insert(class.into(), membership(&records, &indices));
        }
        summary.unmapped.membership = membership(&records, &unmapped);
        summary.unmapped.reasons.sort();
        summary.unmapped.reasons.dedup();
        bytes = bytes.saturating_add(descriptor_bytes(&summary)?);
        if accounted.saturating_add(bytes) > limit {
            return Err(DomainError::new(
                "analysis_descriptor_bytes",
                "consequence summary guard reached",
            ));
        }
        Ok(summary)
    };
    for candidate in &mut result.ownership_candidates {
        candidate.consequence_summary = summarize(&candidate.core_item_ids)?;
    }
    for group in result.drafts.iter_mut().flat_map(|d| &mut d.groups) {
        group.consequence_summary = summarize(&group.item_ids)?;
    }
    result.account(bytes)
}

#[cfg(test)]
mod tests;
