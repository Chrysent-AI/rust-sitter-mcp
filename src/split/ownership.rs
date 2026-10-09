//! Written ownership advice. Cores are seeds; traversal never changes their selection.
use super::*;
use std::collections::VecDeque;

#[derive(Debug, Clone, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct OwnershipCandidate {
    pub id: String,
    pub core_item_ids: Vec<String>,
    pub consequence_summary: ConsequenceSummary,
    pub structural_signal_ids: Vec<String>,
    pub companions: Vec<InspectionCompanion>,
    pub alternatives: Vec<OwnershipAlternative>,
    pub ranking: OwnershipRanking,
    pub observation_scope: String,
    pub selection_policy: String,
}
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct InspectionCompanion {
    pub id: String,
    pub item_id: String,
    pub role: String,
    pub review_obligation: String,
    pub classification: String,
    pub signal_ids: Vec<String>,
    pub boundary_observation_ids: Vec<String>,
    pub observed_consumer_item_ids: Vec<String>,
    pub stop_reasons: Vec<String>,
    pub exclusions: Vec<String>,
    pub observation_scope: String,
}
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct OwnershipAlternative {
    pub kind: String,
    pub item_ids: Vec<String>,
    pub excluded_item_ids: Vec<String>,
    pub overlap_ids: Vec<String>,
    pub applicability: String,
}
#[derive(Debug, Clone, Default, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct OwnershipRanking {
    pub version: u8,
    pub structural_support: usize,
    pub distinct_internal_relationships: usize,
    pub reference_occurrences: usize,
    pub boundary_relationships: usize,
    pub uncertain_relationships: usize,
    pub weak_agreements: usize,
    pub source_order: usize,
}

pub(super) fn structural_signals(
    source: &FileSnapshot,
    data: &ParsedFile,
    lines: &Lines<'_>,
    controls: Controls<'_>,
    result: &mut SuggestSplitEnvelope,
) -> Result<(), DomainError> {
    let mut nominal: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    let mut bindings: BTreeMap<String, usize> = BTreeMap::new();
    for item in &result.inventory {
        controls.check()?;
        if item.enclosing_impl_id.is_some() {
            continue;
        }
        if let Some(name) = &item.name {
            *bindings
                .entry(name.trim_start_matches("r#").into())
                .or_default() += 1;
        }
        if item.kind == "use_declaration" {
            let node = data
                .tree
                .root_node()
                .named_descendant_for_byte_range(
                    item.span.range.start_byte,
                    item.span.range.end_byte,
                )
                .expect("import");
            for leaf in items::use_leaves(
                node,
                &source.source,
                (controls.deadline, controls.cancelled),
            )? {
                *bindings
                    .entry(leaf.binding.trim_start_matches("r#").into())
                    .or_default() += 1;
            }
        }
    }
    let mut members: BTreeMap<(String, String), Vec<usize>> = BTreeMap::new();
    for (i, item) in result.inventory.iter().enumerate() {
        controls.check()?;
        if matches!(
            item.kind.as_str(),
            "struct_item" | "enum_item" | "union_item"
        ) && let Some(name) = &item.name
        {
            nominal
                .entry(name.trim_start_matches("r#"))
                .or_default()
                .push(i);
        }
        if let (Some(owner), Some(name)) = (&item.enclosing_impl_id, &item.name) {
            members
                .entry((owner.clone(), name.trim_start_matches("r#").into()))
                .or_default()
                .push(i);
        }
    }
    let mut added = Vec::new();
    for (i, item) in result.inventory.iter().enumerate() {
        controls.check()?;
        let node = data
            .tree
            .root_node()
            .named_descendant_for_byte_range(item.span.range.start_byte, item.span.range.end_byte)
            .expect("inventory node");
        if item.kind == "impl_item"
            && let Some(identity) = items::associated::identity(source, node, &data.trivia)
            && let Some(owners) = nominal.get(identity.written_type.trim_start_matches("r#"))
            && owners.len() == 1
        {
            let owner = owners[0];
            let ty = node.child_by_field_name("type").expect("written type");
            let mut value = signals::signal(
                "impl_owner_bundle",
                vec![result.inventory[owner].id.clone(), item.id.clone()],
                vec![lines.slice(ty.start_byte(), ty.end_byte(), result.limits.text_bytes)],
            );
            value.from_item_id = Some(result.inventory[owner].id.clone());
            value.to_item_id = Some(item.id.clone());
            value.count = 1;
            value.basis = "written_type".into();
            value.facts.insert(
                "ambiguous_binding".into(),
                usize::from(
                    bindings.get(identity.written_type.trim_start_matches("r#")) != Some(&1),
                ),
            );
            value.facts.insert(
                "excluded_impl".into(),
                usize::from(!identity.exclusions.is_empty()),
            );
            value.limitations.push(format!("unique local written nominal association only; no trait or generic semantics; impl exclusions: {}", identity.exclusions.join(", ")));
            added.push(value);
        }
        let Some(owner) = &item.enclosing_impl_id else {
            continue;
        };
        // Only the written self receiver in its own method is supported. Arbitrary
        // receivers, shadowed self and nested functions do not establish member identity.
        if item.kind != "function_item" || !item.reasons.is_empty() {
            continue;
        }
        let mut stack = vec![node];
        while let Some(current) = stack.pop() {
            controls.check()?;
            if current != node
                && matches!(
                    current.kind(),
                    "function_item"
                        | "closure_expression"
                        | "mod_item"
                        | "macro_invocation"
                        | "token_tree"
                )
            {
                continue;
            }
            if current.kind() == "call_expression"
                && let Some(function) = current.child_by_field_name("function")
                && function.kind() == "field_expression"
                && let (Some(receiver), Some(field)) = (
                    function.child_by_field_name("value"),
                    function.child_by_field_name("field"),
                )
                && &source.source[receiver.byte_range()] == "self"
                && let Some(targets) = members.get(&(
                    owner.clone(),
                    source.source[field.byte_range()]
                        .trim_start_matches("r#")
                        .into(),
                ))
            {
                for target in targets {
                    controls.check()?;
                    result.counts.reference_candidates += 1;
                    if result.counts.reference_candidates
                        > result.effective_work_limits.reference_candidates
                    {
                        return Err(DomainError::new(
                            "reference_work_limit",
                            "written member candidate guard reached",
                        ));
                    }
                    let mut value = signals::signal(
                        "reference_candidate",
                        vec![item.id.clone(), result.inventory[*target].id.clone()],
                        vec![lines.slice(
                            field.start_byte(),
                            field.end_byte(),
                            result.limits.text_bytes,
                        )],
                    );
                    value.from_item_id = Some(item.id.clone());
                    value.to_item_id = Some(result.inventory[*target].id.clone());
                    value.count = 1;
                    value.basis = "written_self_member".into();
                    value
                        .facts
                        .insert("ambiguous_binding".into(), usize::from(targets.len() != 1));
                    value.limitations.push("written self call within one impl only; no type-directed resolution or move proof".into());
                    added.push(value);
                }
            }
            for n in (0..current.named_child_count()).rev() {
                stack.push(current.named_child(n as u32).expect("child"));
            }
        }
        let _ = i;
    }
    for value in added {
        signals::push(result, value)?;
    }
    Ok(())
}

struct Union(Vec<usize>);
impl Union {
    fn root(&mut self, mut i: usize) -> usize {
        while self.0[i] != i {
            self.0[i] = self.0[self.0[i]];
            i = self.0[i];
        }
        i
    }
    fn join(&mut self, a: usize, b: usize) {
        let a = self.root(a);
        let b = self.root(b);
        self.0[a.max(b)] = a.min(b);
    }
}
struct Index {
    outgoing: Vec<Vec<usize>>,
    incoming: Vec<Vec<usize>>,
    bundles: Vec<Vec<usize>>,
    members: Vec<Vec<usize>>,
    boundary: Vec<Vec<usize>>,
    weak: Vec<Vec<usize>>,
    consumers: Vec<BTreeSet<usize>>,
    ids: BTreeMap<String, usize>,
}
impl Index {
    fn new(result: &mut SuggestSplitEnvelope, controls: Controls<'_>) -> Result<Self, DomainError> {
        let n = result.inventory.len();
        result.account(n.saturating_mul(160) + result.signals.len().saturating_mul(48))?;
        let mut index = Self {
            outgoing: vec![Vec::new(); n],
            incoming: vec![Vec::new(); n],
            bundles: vec![Vec::new(); n],
            members: vec![Vec::new(); n],
            boundary: vec![Vec::new(); n],
            weak: vec![Vec::new(); n],
            consumers: vec![BTreeSet::new(); n],
            ids: result
                .inventory
                .iter()
                .enumerate()
                .map(|(i, item)| (item.id.clone(), i))
                .collect(),
        };
        for (s, signal) in result.signals.iter().enumerate() {
            controls.check()?;
            if let (Some(a), Some(b)) = (&signal.from_item_id, &signal.to_item_id) {
                let a = index.ids[a];
                let b = index.ids[b];
                if signal.kind == "impl_owner_bundle" {
                    index.bundles[a].push(s);
                } else if matches!(
                    signal.kind.as_str(),
                    "reference_candidate" | "cfg_test_consumer"
                ) {
                    index.outgoing[a].push(s);
                    index.incoming[b].push(s);
                    if a != b {
                        index.consumers[b].insert(a);
                    }
                }
            }
            if matches!(
                signal.kind.as_str(),
                "name_prefix" | "banner_section" | "shared_attribute" | "doc_heading" | "adjacency"
            ) {
                for id in &signal.item_ids {
                    index.weak[index.ids[id]].push(s);
                }
            }
        }
        for (i, item) in result.inventory.iter().enumerate() {
            controls.check()?;
            if let Some(owner) = &item.enclosing_impl_id {
                index.members[index.ids[owner]].push(i);
            }
        }
        for (b, observation) in result.boundary_observations.records.iter().enumerate() {
            controls.check()?;
            for id in &observation.item_ids {
                index.boundary[index.ids[id]].push(b);
            }
        }
        Ok(index)
    }
    fn target(&self, signal: &Signal) -> usize {
        self.ids[signal.to_item_id.as_ref().expect("target")]
    }
    fn source(&self, signal: &Signal) -> usize {
        self.ids[signal.from_item_id.as_ref().expect("source")]
    }
}
fn supported(signal: &Signal) -> bool {
    signal.kind == "reference_candidate" && signal.facts.get("ambiguous_binding") != Some(&1)
}

pub(super) fn build(
    result: &mut SuggestSplitEnvelope,
    controls: Controls<'_>,
) -> Result<(), DomainError> {
    let index = Index::new(result, controls)?;
    let n = result.inventory.len();
    let mut union = Union((0..n).collect());
    let mut graph = vec![Vec::new(); n];
    let mut reverse = graph.clone();
    for signal in &result.signals {
        controls.check()?;
        if supported(signal) {
            let a = index.source(signal);
            let b = index.target(signal);
            if a != b && draftable(&result.inventory[a]) && draftable(&result.inventory[b]) {
                graph[a].push(b);
                reverse[b].push(a);
            }
        }
    }
    // Sparse iterative SCCs: a shared caller or dependency cannot join free functions.
    let mut visited = vec![false; n];
    let mut finish = Vec::new();
    for start in 0..n {
        controls.check()?;
        if visited[start] {
            continue;
        }
        visited[start] = true;
        let mut stack = vec![(start, 0)];
        while let Some((node, child)) = stack.last_mut() {
            controls.check()?;
            if *child < graph[*node].len() {
                let next = graph[*node][*child];
                *child += 1;
                if !visited[next] {
                    visited[next] = true;
                    stack.push((next, 0));
                }
            } else {
                finish.push(*node);
                stack.pop();
            }
        }
    }
    visited.fill(false);
    for start in finish.into_iter().rev() {
        if visited[start] {
            continue;
        }
        let mut stack = vec![start];
        visited[start] = true;
        while let Some(node) = stack.pop() {
            controls.check()?;
            union.join(start, node);
            for next in &reverse[node] {
                if !visited[*next] {
                    visited[*next] = true;
                    stack.push(*next);
                }
            }
        }
    }
    let mut member_targets = vec![BTreeSet::new(); n];
    let mut signature_payloads = vec![BTreeSet::new(); n];
    for signal in &result.signals {
        controls.check()?;
        if supported(signal) {
            let a = index.source(signal);
            let b = index.target(signal);
            if signal.facts.get("signature_type") == Some(&1)
                && index.bundles[b].is_empty()
                && result.inventory[b].visibility_key == "private"
                && matches!(
                    result.inventory[b].kind.as_str(),
                    "struct_item" | "enum_item"
                )
            {
                signature_payloads[a].insert(b);
            }
            if a != b
                && result.inventory[a].enclosing_impl_id.is_some()
                && result.inventory[a].enclosing_impl_id == result.inventory[b].enclosing_impl_id
            {
                member_targets[a].insert(b);
            }
        }
    }
    // Acyclic edges can seed members only within the same written owner, and
    // only into a singly consumed member. Shared helpers and callers are not hubs.
    for signal in &result.signals {
        controls.check()?;
        if !supported(signal) {
            continue;
        }
        let a = index.source(signal);
        let b = index.target(signal);
        let owner = &result.inventory[a].enclosing_impl_id;
        if a != b
            && owner.is_some()
            && *owner == result.inventory[b].enclosing_impl_id
            && draftable(&result.inventory[a])
            && draftable(&result.inventory[b])
            && member_targets[a].len() == 1
            && index.consumers[b].len() == 1
            && index.consumers[b].contains(&a)
            && index.boundary[b].is_empty()
        {
            union.join(a, b);
        }
    }
    // Signature payloads seed only one impl's members, with no other written
    // consumers, public type boundary or uncertainty. Self is never a payload.
    for target in 0..n {
        controls.check()?;
        let item = &result.inventory[target];
        if item.visibility_key != "private"
            || !matches!(item.kind.as_str(), "struct_item" | "enum_item")
            || !index.boundary[target].is_empty()
            || !index.bundles[target].is_empty()
        {
            continue;
        }
        let mut consumers = BTreeSet::new();
        let mut owner = None;
        let mut valid = true;
        for s in &index.incoming[target] {
            controls.check()?;
            let signal = &result.signals[*s];
            let from = index.source(signal);
            let current_owner = &result.inventory[from].enclosing_impl_id;
            if signature_payloads[from].len() > 1 || member_targets[from].len() > 1 {
                continue;
            }
            if !supported(signal)
                || signal.facts.get("signature_type") != Some(&1)
                || current_owner.is_none()
                || !draftable(&result.inventory[from])
            {
                valid = false;
                break;
            }
            if owner
                .as_ref()
                .is_some_and(|o| Some(o) != current_owner.as_ref())
            {
                valid = false;
                break;
            }
            owner = current_owner.clone();
            consumers.insert(from);
        }
        if valid && consumers.len() >= 2 {
            let first = *consumers.first().expect("consumers");
            for member in consumers {
                union.join(first, member);
            }
        }
    }
    let mut groups: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for i in 0..n {
        controls.check()?;
        if draftable(&result.inventory[i]) {
            groups.entry(union.root(i)).or_default().push(i);
        }
    }
    let mut cores: Vec<_> = groups
        .into_values()
        .filter(|g| g.len() >= 2)
        .map(|g| (g, 2usize))
        .collect();
    for i in 0..n {
        controls.check()?;
        if draftable(&result.inventory[i])
            && index.bundles[i]
                .iter()
                .any(|s| result.signals[*s].facts.get("ambiguous_binding") != Some(&1))
        {
            let mut support = 1;
            for s in &index.bundles[i] {
                let implementation = index.target(&result.signals[*s]);
                let members = &index.members[implementation];
                if !members.is_empty()
                    && members.len() >= 2
                    && members.iter().all(|m| {
                        union.root(*m) == union.root(members[0]) && draftable(&result.inventory[*m])
                    })
                {
                    support = 3;
                }
            }
            cores.push((vec![i], support));
        }
    }
    let mut candidates = Vec::new();
    for (core, support) in cores {
        controls.check()?;
        let candidate = candidate(&core, support, &index, result, controls)?;
        result.account(descriptor_bytes(&candidate)?)?;
        candidates.push(candidate);
    }
    candidates.sort_by_key(|c| {
        (
            std::cmp::Reverse(c.ranking.structural_support),
            std::cmp::Reverse(c.ranking.distinct_internal_relationships),
            c.ranking.boundary_relationships,
            c.ranking.uncertain_relationships,
            std::cmp::Reverse(c.ranking.weak_agreements),
            c.ranking.source_order,
        )
    });
    for (i, c) in candidates.iter_mut().enumerate() {
        c.id = format!("candidate/{i}");
    }
    let mut companion_order: Vec<_> = candidates
        .iter()
        .enumerate()
        .flat_map(|(c, candidate)| (0..candidate.companions.len()).map(move |m| (c, m)))
        .collect();
    result.account(
        companion_order
            .len()
            .saturating_mul(std::mem::size_of::<(usize, usize)>()),
    )?;
    companion_order.sort_by_key(|(c, m)| {
        let item = &result.inventory[index.ids[&candidates[*c].companions[*m].item_id]];
        (&item.path, &item.span.range, &item.kind, *c)
    });
    for (i, (c, m)) in companion_order.into_iter().enumerate() {
        controls.check()?;
        let id = format!("companion/{i}");
        result.account(id.len())?;
        candidates[c].companions[m].id = id;
    }
    result.counts.ownership_candidates = candidates.len();
    result.ownership_candidates = candidates;
    result.partition_outcome = if result.inventory.len() < 2 {
        "insufficient_input"
    } else if result.ownership_candidates.is_empty() {
        "no_credible_written_partition"
    } else {
        "credible_written_candidates"
    }
    .into();
    Ok(())
}

fn candidate(
    core: &[usize],
    support: usize,
    index: &Index,
    result: &SuggestSplitEnvelope,
    controls: Controls<'_>,
) -> Result<OwnershipCandidate, DomainError> {
    let core_set: BTreeSet<_> = core.iter().copied().collect();
    let mut ranking = OwnershipRanking {
        version: 1,
        structural_support: support,
        source_order: core[0],
        ..OwnershipRanking::default()
    };
    let mut structural = BTreeSet::new();
    let mut internal = BTreeSet::new();
    let mut boundary = BTreeSet::new();
    let mut uncertain = BTreeSet::new();
    let mut companions: BTreeMap<usize, InspectionCompanion> = BTreeMap::new();
    let mut alternatives = Vec::new();
    let mut visited = core_set.clone();
    let mut reached = core_set.clone();
    let mut queue: VecDeque<_> = core.iter().copied().collect();
    while let Some(from) = queue.pop_front() {
        controls.check()?;
        let mut links = index.outgoing[from].clone();
        links.extend(&index.bundles[from]);
        for s in links {
            controls.check()?;
            let signal = &result.signals[s];
            let to = index.target(signal);
            reached.insert(to);
            if core_set.contains(&to) {
                continue;
            }
            let item = &result.inventory[to];
            let mut stops = BTreeSet::new();
            if signal.facts.get("ambiguous_binding") == Some(&1) {
                stops.insert("uncertain_identity".into());
            }
            if item.visibility_key != "private" {
                stops.insert("public_or_exposed_boundary".into());
            }
            if matches!(
                item.kind.as_str(),
                "use_declaration" | "extern_crate_declaration"
            ) {
                stops.insert("import_boundary".into());
            }
            if (!draftable(item) && !partitioned_impl(item))
                || signal.facts.get("excluded_impl") == Some(&1)
            {
                stops.insert("unsupported_context".into());
            }
            let shared = index.incoming[to].iter().any(|s| {
                let from = index.source(&result.signals[*s]);
                from != to && !core_set.contains(&from) && !visited.contains(&from)
            }) || index.boundary[to]
                .iter()
                .any(|b| result.boundary_observations.records[*b].direction == "incoming");
            if shared {
                stops.insert("observed_shared_node".into());
            }
            if result.boundary_observations.coverage.attribution_uncertain {
                stops.insert("unexamined_context".into());
            }
            let record = companions.entry(to).or_insert_with(|| InspectionCompanion {
                id: String::new(),
                item_id: item.id.clone(),
                role: match item.kind.as_str() {
                    "impl_item" => "implementation",
                    "struct_item" | "enum_item" | "type_item" => "payload_or_type",
                    "const_item" | "static_item" => "constant",
                    "function_item" => "helper_or_validator",
                    _ => "written_dependency",
                }
                .into(),
                review_obligation: "association_unproved".into(),
                classification: "undetermined".into(),
                signal_ids: Vec::new(),
                boundary_observation_ids: index.boundary[to]
                    .iter()
                    .map(|b| result.boundary_observations.records[*b].id.clone())
                    .collect(),
                observed_consumer_item_ids: index.incoming[to]
                    .iter()
                    .map(|s| result.signals[*s].from_item_id.clone().expect("consumer"))
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect(),
                stop_reasons: Vec::new(),
                exclusions: item.reasons.clone(),
                observation_scope:
                    "boundary_observations.coverage; completed admitted written observations only"
                        .into(),
            });
            if !record.signal_ids.contains(&signal.id) {
                record.signal_ids.push(signal.id.clone());
            }
            record.stop_reasons.extend(stops.iter().cloned());
            record.stop_reasons.sort();
            record.stop_reasons.dedup();
            // Association evidence determines the obligation independently of
            // consumer attribution, traversal stops and move eligibility.
            if signal.kind == "impl_owner_bundle"
                && signal.facts.get("ambiguous_binding") != Some(&1)
            {
                record.review_obligation = "selection_completeness".into();
            } else if supported(signal) && record.review_obligation != "selection_completeness" {
                record.review_obligation = "boundary_dependency".into();
            }
            // Implementation membership is disclosed as a choice, not inserted into core.
            if signal.kind == "impl_owner_bundle" {
                if !alternatives
                    .iter()
                    .any(|a: &OwnershipAlternative| a.item_ids == vec![item.id.clone()])
                {
                    alternatives.push(OwnershipAlternative {
                        kind: "whole_impl".into(),
                        item_ids: vec![item.id.clone()],
                        excluded_item_ids: if signal.facts.get("excluded_impl") == Some(&1) {
                            vec![item.id.clone()]
                        } else {
                            Vec::new()
                        },
                        overlap_ids: item.overlap_ids.clone(),
                        applicability: "not_assessed; mutually exclusive with member selection"
                            .into(),
                    });
                    alternatives.push(OwnershipAlternative { kind: "members".into(), item_ids: index.members[to].iter().filter(|m| draftable(&result.inventory[**m])).map(|m| result.inventory[*m].id.clone()).collect(), excluded_item_ids: index.members[to].iter().filter(|m| !draftable(&result.inventory[**m])).map(|m| result.inventory[*m].id.clone()).collect(), overlap_ids: item.overlap_ids.clone(), applicability: "not_assessed; explicit subset required, never select whole impl together".into() });
                }
                if stops.is_empty() {
                    for member in &index.members[to] {
                        reached.insert(*member);
                        if draftable(&result.inventory[*member]) && visited.insert(*member) {
                            queue.push_back(*member);
                        }
                    }
                }
            }
            if stops.is_empty() && visited.insert(to) {
                queue.push_back(to);
            }
        }
    }
    // Classify against the completed reached observation set, not occurrence popularity.
    for (to, record) in &mut companions {
        controls.check()?;
        let incoming_uncertain = index.incoming[*to]
            .iter()
            .any(|s| result.signals[*s].facts.get("ambiguous_binding") == Some(&1));
        let boundary_uncertain = index.boundary[*to]
            .iter()
            .any(|b| result.boundary_observations.records[*b].certainty != "written_route");
        let shared = index.incoming[*to]
            .iter()
            .any(|s| !reached.contains(&index.source(&result.signals[*s])))
            || index.boundary[*to]
                .iter()
                .any(|b| result.boundary_observations.records[*b].direction == "incoming");
        record.classification = if incoming_uncertain
            || boundary_uncertain
            || record.stop_reasons.iter().any(|r| {
                matches!(
                    r.as_str(),
                    "uncertain_identity" | "unexamined_context" | "unsupported_context"
                )
            }) {
            "undetermined"
        } else if shared {
            "observed_shared"
        } else {
            "observed_exclusive"
        }
        .into();
    }
    for from in core {
        controls.check()?;
        for s in index.outgoing[*from]
            .iter()
            .chain(&index.bundles[*from])
            .chain(&index.incoming[*from])
        {
            let signal = &result.signals[*s];
            let a = index.source(signal);
            let b = index.target(signal);
            if signal.kind == "impl_owner_bundle"
                || (core_set.contains(&a) && core_set.contains(&b) && supported(signal))
            {
                let new_signal = structural.insert(signal.id.clone());
                if a != b {
                    internal.insert((a, b));
                    if new_signal {
                        ranking.reference_occurrences += signal.count;
                    }
                }
            } else if a != b {
                boundary.insert((a, b));
            }
            if signal.facts.get("ambiguous_binding") == Some(&1) {
                uncertain.insert(signal.id.clone());
            }
        }
        for b in &index.boundary[*from] {
            boundary.insert((n_boundary(result, *b), *from));
            if result.boundary_observations.records[*b].certainty != "written_route" {
                uncertain.insert(result.boundary_observations.records[*b].id.clone());
            }
        }
        // Signature payload references supporting a member subset remain linked.
        for s in &index.outgoing[*from] {
            let signal = &result.signals[*s];
            if signal.facts.get("signature_type") == Some(&1) && core.len() >= 2 {
                structural.insert(signal.id.clone());
            }
        }
    }
    let weak: BTreeSet<_> = core
        .iter()
        .flat_map(|m| index.weak[*m].iter().copied())
        .collect();
    for s in weak {
        controls.check()?;
        let signal = &result.signals[s];
        if matches!(
            signal.kind.as_str(),
            "name_prefix" | "banner_section" | "shared_attribute" | "doc_heading" | "adjacency"
        ) {
            let count = signal
                .item_ids
                .iter()
                .filter(|id| core_set.contains(&index.ids[*id]))
                .count();
            ranking.weak_agreements += count.saturating_sub(1);
        }
    }
    ranking.distinct_internal_relationships = internal.len();
    ranking.boundary_relationships = boundary.len();
    ranking.uncertain_relationships = uncertain.len();
    // Member cores expose their retained owner's whole-impl alternative too.
    for from in core {
        if let Some(owner) = &result.inventory[*from].enclosing_impl_id
            && !alternatives
                .iter()
                .any(|a| a.kind == "whole_impl" && a.item_ids.contains(owner))
        {
            let implementation = &result.inventory[index.ids[owner]];
            alternatives.push(OwnershipAlternative { kind: "whole_impl".into(), item_ids: vec![owner.clone()], excluded_item_ids: Vec::new(), overlap_ids: implementation.overlap_ids.clone(), applicability: "not_assessed; includes unrelated members; alternative, not core closure".into() });
            alternatives.push(OwnershipAlternative {
                kind: "members".into(),
                item_ids: core
                    .iter()
                    .map(|m| result.inventory[*m].id.clone())
                    .collect(),
                excluded_item_ids: index.members[index.ids[owner]]
                    .iter()
                    .filter(|m| !core_set.contains(m))
                    .map(|m| result.inventory[*m].id.clone())
                    .collect(),
                overlap_ids: implementation.overlap_ids.clone(),
                applicability: "not_assessed; explicit member anchors required".into(),
            });
        }
    }
    Ok(OwnershipCandidate { id: String::new(), consequence_summary: ConsequenceSummary::default(), core_item_ids: core.iter().map(|m| result.inventory[*m].id.clone()).collect(), structural_signal_ids: structural.into_iter().collect(), companions: companions.into_values().collect(), alternatives, ranking, observation_scope: "boundary_observations.coverage; written references, not resolved ownership".into(), selection_policy: "core only; companions and alternatives are inspection advice, never automatically selected".into() })
}
fn n_boundary(result: &SuggestSplitEnvelope, b: usize) -> usize {
    result.inventory.len() + b
}
