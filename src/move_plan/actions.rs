//! Shared request-local routing; categories and display strings never establish repairability.
use super::*;

impl DecisionAction {
    pub(crate) fn field(tool: &str, field: &str, purpose: DecisionPurpose) -> Self {
        Self::RequestField {
            tool: tool.into(),
            field: field.into(),
            target: None,
            trivia: None,
            target_item: None,
            choices: Vec::new(),
            purpose,
        }
    }
    pub(crate) fn rewrite(target: RewriteTarget) -> Self {
        Self::RequestField {
            tool: "move_item".into(),
            field: "rewrite_overrides[]".into(),
            target: Some(Box::new(target)),
            trivia: None,
            target_item: None,
            choices: vec!["accept_default".into(), "replace".into()],
            purpose: DecisionPurpose::ResolveDecision,
        }
    }
    pub(crate) fn cause(reason: DecisionReason) -> Self {
        match reason {
            DecisionReason::DestinationBindingConflict => Self::SelectionChangeRequired {
                fields: vec!["moves[index].destination".into(), "moves".into()],
                instruction: "change the affected destination or selected batch to avoid the anchored binding conflict".into(),
            },
            DecisionReason::SourceBindingAmbiguous | DecisionReason::UnsupportedUnitKind => Self::SelectionChangeRequired {
                fields: vec!["moves".into()],
                instruction: "change the affected selected items; acknowledgment does not establish supported binding identity or unit context".into(),
            },
            DecisionReason::CrossGroupReferenceReview => Self::field("move_item", "moves", DecisionPurpose::SubmitForAnalysis),
            DecisionReason::OrdinaryTriviaChoice => Self::RequestField {
                tool: "move_item".into(), field: "trivia_overrides[]".into(),
                target: None, trivia: None, target_item: None,
                choices: vec!["keep_in_place".into(), "carry_with_item".into()], purpose: DecisionPurpose::ReviewDefault,
            },
            _ => Self::UnsupportedInEngine {
                construct: serde_json::to_value(reason).expect("reason serializes").as_str().expect("reason string").into(),
                instruction: "no supported request field or acknowledgment proves this anchored construct; choose a different supported move if needed".into(),
            },
        }
    }
    pub(crate) fn chain(diagnostic: &items::ChainDiagnostic, tool: &str) -> Self {
        use items::ChainReason;
        match diagnostic.reason {
            ChainReason::SourceNotInRootChain => Self::field(tool, "crate_root", DecisionPurpose::SubmitForAnalysis),
            ChainReason::ChainFileUnadmitted => Self::field(tool, "paths", DecisionPurpose::SubmitForAnalysis),
            ChainReason::ChainFileMissing => Self::SelectionChangeRequired {
                fields: if tool == "move_item" { vec!["moves".into()] } else { vec!["source_path".into()] },
                instruction: "the evidenced ordinary chain file is absent; select a different provable source/destination chain".into(),
            },
            _ => Self::UnsupportedInEngine {
                construct: serde_json::to_value(diagnostic.reason).expect("reason serializes").as_str().expect("reason string").into(),
                instruction: "the linked chain diagnostic has no admitted ordinary repair; cfg, path mappings and layout uncertainty cannot be acknowledged away".into(),
            },
        }
    }
    pub(crate) fn summary(&self) -> Self {
        match self {
            Self::RequestField {
                tool,
                field,
                choices,
                purpose,
                ..
            } => Self::RequestField {
                tool: tool.clone(),
                field: field.clone(),
                target: None,
                trivia: None,
                target_item: None,
                choices: choices.clone(),
                purpose: *purpose,
            },
            _ => self.clone(),
        }
    }
    pub(crate) fn route(&self) -> DecisionRoute {
        match self {
            Self::RequestField { .. } => DecisionRoute::RequestField,
            Self::SelectionChangeRequired { .. } => DecisionRoute::SelectionChangeRequired,
            Self::UnsupportedInEngine { .. } => DecisionRoute::UnsupportedInEngine,
        }
    }
    pub(crate) fn next_action(&self) -> String {
        match self {
            Self::RequestField {
                tool,
                field,
                choices,
                purpose,
                ..
            } => match purpose {
                DecisionPurpose::ResolveDecision => format!(
                    "replay the full target in {tool}.{field} using {}; other blockers may remain",
                    choices.join(" or ")
                ),
                DecisionPurpose::ReviewDefault => format!(
                    "review {tool}.{field} using {}; obtain full original trivia/item anchors before replay",
                    choices.join(" or ")
                ),
                DecisionPurpose::SubmitForAnalysis => format!(
                    "submit corrected {tool}.{field} with full original item anchors for analysis; this is not a repairability guarantee"
                ),
            },
            Self::SelectionChangeRequired { instruction, .. }
            | Self::UnsupportedInEngine { instruction, .. } => instruction.clone(),
        }
    }
}

pub(crate) fn decision_groups<'a>(
    decisions: impl Iterator<
        Item = (
            &'a str,
            DecisionReason,
            &'a DecisionAction,
            bool,
            &'a str,
            Option<&'a str>,
        ),
    >,
    bytes: &mut usize,
    controls: (Instant, &AtomicBool),
) -> Result<Vec<DecisionGroup>, DomainError> {
    let mut groups: BTreeMap<_, DecisionGroup> = BTreeMap::new();
    for (category, reason, action, blocks, id, consequence) in decisions {
        items::check(controls.0, controls.1)?;
        let record = DecisionGroup {
            category: category.into(),
            reason,
            route: action.route(),
            blocks_applicability: blocks,
            decision_ids: vec![id.into()],
            count: 1,
        };
        // Collapse only identical causes, routing summaries and consequences when
        // supplied; per-occurrence anchors stay in the full decisions.
        let detail = consequence.map(|text| {
            (
                serde_json::to_string(&action.summary()).expect("action JSON"),
                text.to_owned(),
            )
        });
        // Reserve keys and linked records before growing; shared keys are
        // conservatively counted again for each decision.
        *bytes = bytes.saturating_add(descriptor_bytes(&(&record, &detail))?);
        if *bytes > 128 * 1024 * 1024 {
            return Err(DomainError::new(
                "analysis_descriptor_bytes",
                "decision group descriptor guard reached",
            ));
        }
        let key = (
            record.category.clone(),
            reason,
            record.route,
            blocks,
            detail,
        );
        if let Some(group) = groups.get_mut(&key) {
            group.decision_ids.push(id.into());
            group.count += 1;
        } else {
            groups.insert(key, record);
        }
    }
    Ok(groups.into_values().collect())
}

pub(crate) fn id_runs(ids: &[String]) -> Vec<DecisionIdRun> {
    let mut runs: Vec<DecisionIdRun> = Vec::new();
    let mut previous = None;
    for id in ids {
        let ordinal = id
            .strip_prefix("d/")
            .expect("decision prefix")
            .parse::<usize>()
            .expect("decision index");
        if previous.is_some_and(|prior| ordinal == prior + 1) {
            runs.last_mut().expect("preceding run").count += 1;
        } else {
            runs.push(DecisionIdRun {
                first_id: id.clone(),
                count: 1,
            });
        }
        previous = Some(ordinal);
    }
    runs
}
