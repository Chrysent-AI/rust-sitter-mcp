//! Separately scoped written outside-consumer observations, never planner audits.
use super::*;
use tree_sitter::Node;

#[derive(Debug, Clone, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct BoundaryObservations {
    pub coverage: BoundaryCoverage,
    pub records: Vec<BoundaryObservation>,
}
#[derive(Debug, Clone, Default, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct BoundaryCoverage {
    pub completed: bool,
    pub admitted_paths: Vec<String>,
    pub observed_paths: Vec<String>,
    pub attribution_uncertain: bool,
    pub unsupported_context_count: usize,
    pub unsupported_context_examples: Vec<AdviceAnchor>,
    pub not_assessed: Vec<String>,
    pub note: String,
}
impl Default for BoundaryObservations {
    fn default() -> Self {
        Self { coverage: BoundaryCoverage { not_assessed: vec!["unadmitted_files".into(), "generated_consumers".into(), "macro_expansion".into(), "trait_and_receiver_resolution".into(), "Cargo_targets".into(), "destination".into(), "batch_applicability".into(), "repairability".into()], note: "admitted written paths, explicit file-level imports and glob candidates only; completed observations are not proof of no other consumers or architectural exclusivity; legacy local forecasts are unchanged".into(), ..BoundaryCoverage::default() }, records: Vec::new() }
    }
}
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct BoundaryObservation {
    pub id: String,
    pub direction: String,
    pub item_ids: Vec<String>,
    pub ownership_candidate_ids: Vec<String>,
    pub anchor: AdviceAnchor,
    pub enclosing_unit: Option<AdviceAnchor>,
    pub counterpart: Option<AdviceAnchor>,
    pub route_evidence: Vec<AdviceAnchor>,
    pub certainty: String,
    pub basis: String,
}
struct Import {
    binding: String,
    route: String,
    anchor: AdviceAnchor,
    certain: bool,
}
fn anchor(path: &str, source: &str, node: Node<'_>, text_bytes: usize) -> AdviceAnchor {
    AdviceAnchor {
        path: path.into(),
        span: Lines::new(source).slice(node.start_byte(), node.end_byte(), text_bytes),
    }
}
fn route_segments(route: &str, module: Option<&items::ModuleEvidence>) -> Option<Vec<String>> {
    let module = module.filter(|m| m.unresolved.is_empty())?;
    let mut parts = route
        .split("::")
        .map(|s| s.trim_start_matches("r#").to_owned())
        .peekable();
    let mut out = match parts.peek()?.as_str() {
        "crate" => {
            parts.next();
            Vec::new()
        }
        "self" => {
            parts.next();
            module.module_segments.clone()
        }
        "super" => {
            let mut out = module.module_segments.clone();
            while parts.peek().is_some_and(|s| s == "super") {
                parts.next();
                out.pop()?;
            }
            out
        }
        _ => module.module_segments.clone(),
    };
    out.extend(parts);
    Some(out)
}
fn local_targets<'a>(
    route: &[String],
    source_module: Option<&items::ModuleEvidence>,
    names: &'a BTreeMap<String, Vec<String>>,
) -> Option<&'a Vec<String>> {
    let module = source_module.filter(|m| m.unresolved.is_empty())?;
    if route.len() != module.module_segments.len() + 1
        || route[..route.len() - 1] != module.module_segments
    {
        return None;
    }
    names.get(route.last()?)
}
fn record(
    result: &mut SuggestSplitEnvelope,
    value: BoundaryObservation,
) -> Result<(), DomainError> {
    result.counts.reference_candidates += 1;
    if result.counts.reference_candidates > result.effective_work_limits.reference_candidates {
        return Err(DomainError::new(
            "reference_work_limit",
            "boundary observation guard reached",
        ));
    }
    result.account(descriptor_bytes(&value)?)?;
    result.boundary_observations.records.push(value);
    Ok(())
}
fn unsupported(result: &mut SuggestSplitEnvelope, value: AdviceAnchor) -> Result<(), DomainError> {
    let coverage = &mut result.boundary_observations.coverage;
    coverage.attribution_uncertain = true;
    coverage.unsupported_context_count += 1;
    if coverage.unsupported_context_examples.len() < 16 {
        result.account(descriptor_bytes(&value)?)?;
        result
            .boundary_observations
            .coverage
            .unsupported_context_examples
            .push(value);
    }
    Ok(())
}

pub(super) fn collect(
    request: &SuggestSplitRequest,
    files: &BTreeMap<String, FileSnapshot>,
    parsed: &BTreeMap<String, ParsedFile>,
    contexts: &BTreeMap<String, items::ModuleEvidence>,
    globs: &items::GlobRoutes<'_>,
    controls: Controls<'_>,
    result: &mut SuggestSplitEnvelope,
) -> Result<(), DomainError> {
    let mut names: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for item in &result.inventory {
        if item.enclosing_impl_id.is_none()
            && let Some(name) = &item.name
        {
            names
                .entry(name.trim_start_matches("r#").into())
                .or_default()
                .push(item.id.clone());
        }
    }
    let mut declarations: BTreeMap<Vec<String>, Vec<AdviceAnchor>> = BTreeMap::new();
    for (path, module) in contexts {
        if !module.unresolved.is_empty() || path == &request.source_path {
            continue;
        }
        for item in &parsed[path].items {
            controls.check()?;
            if let Some(name) = &item.name {
                let mut route = module.module_segments.clone();
                route.push(name.trim_start_matches("r#").into());
                let value = AdviceAnchor {
                    path: path.clone(),
                    span: Lines::new(&files[path].source).slice(
                        item.span.range.start_byte,
                        item.span.range.end_byte,
                        result.limits.text_bytes,
                    ),
                };
                result.account(descriptor_bytes(&(&route, &value))?)?;
                declarations.entry(route).or_default().push(value);
            }
        }
    }
    let source_units: BTreeMap<_, _> = result
        .inventory
        .iter()
        .map(|i| {
            (
                i.span.range.start_byte,
                (i.span.range.end_byte, i.id.clone()),
            )
        })
        .collect();
    let source_module = contexts.get(&request.source_path);
    result.boundary_observations.coverage.admitted_paths = files.keys().cloned().collect();
    result.account(descriptor_bytes(&result.boundary_observations.coverage)?)?;
    for (path, data) in parsed {
        controls.check()?;
        let source = &files[path].source;
        let module = contexts.get(path);
        let enclosing_units: BTreeMap<_, _> = data
            .items
            .iter()
            .map(|i| (i.span.range.start_byte, i))
            .collect();
        let mut imports = Vec::new();
        for item in &data.items {
            controls.check()?;
            if item.kind != "use_declaration" {
                continue;
            }
            let node = data
                .tree
                .root_node()
                .named_descendant_for_byte_range(
                    item.span.range.start_byte,
                    item.span.range.end_byte,
                )
                .expect("import");
            for leaf in items::use_leaves(node, source, (controls.deadline, controls.cancelled))? {
                let import = Import {
                    binding: leaf.binding.trim_start_matches("r#").into(),
                    route: leaf.path,
                    anchor: anchor(path, source, node, result.limits.text_bytes),
                    certain: item.attributes.iter().all(|a| {
                        items::context_independent_attribute(
                            &source[a.range.start_byte..a.range.end_byte],
                        )
                    }),
                };
                result.account(descriptor_bytes(&(
                    &import.binding,
                    &import.route,
                    &import.anchor,
                ))?)?;
                imports.push(import);
            }
        }
        let mut by_binding: BTreeMap<&str, Vec<&Import>> = BTreeMap::new();
        for import in &imports {
            by_binding.entry(&import.binding).or_default().push(import);
        }
        // Outgoing import routes are observed once, linked to the source import
        // inventory unit. Consumers of that unit are already in local signals.
        if path == &request.source_path {
            for import in &imports {
                controls.check()?;
                let Some(route) = route_segments(&import.route, module) else {
                    continue;
                };
                if let Some(targets) = declarations.get(&route) {
                    for declaration in targets {
                        controls.check()?;
                        let local = &source_units[&import.anchor.span.range.start_byte].1;
                        record(result, BoundaryObservation { id: String::new(), direction: "outgoing".into(), item_ids: vec![local.clone()], ownership_candidate_ids: Vec::new(), anchor: import.anchor.clone(), enclosing_unit: None, counterpart: Some(declaration.clone()), route_evidence: vec![import.anchor.clone()], certainty: if import.certain && targets.len() == 1 { "written_route" } else { "candidate" }.into(), basis: "explicit_import_to_admitted_written_declaration; not symbol resolution".into() })?;
                    }
                }
            }
        }
        let mut stack = vec![data.tree.root_node()];
        while let Some(node) = stack.pop() {
            controls.check()?;
            if matches!(
                node.kind(),
                "macro_invocation" | "macro_definition" | "mod_item" | "use_declaration"
            ) {
                if matches!(node.kind(), "macro_invocation" | "macro_definition")
                    || (node.kind() == "mod_item" && node.child_by_field_name("body").is_some())
                    || (node.kind() == "use_declaration"
                        && node.parent().is_some_and(|p| p.kind() != "source_file"))
                {
                    unsupported(result, anchor(path, source, node, result.limits.text_bytes))?;
                }
                continue;
            }
            if matches!(
                node.kind(),
                "line_comment"
                    | "block_comment"
                    | "attribute_item"
                    | "inner_attribute_item"
                    | "token_tree"
            ) {
                continue;
            }
            let simple = matches!(node.kind(), "scoped_identifier" | "scoped_type_identifier");
            let bare = matches!(node.kind(), "identifier" | "type_identifier");
            if (bare || simple) && items::reference_role(node) {
                let text = &source[node.byte_range()];
                let first = text
                    .split("::")
                    .next()
                    .expect("reference")
                    .trim_start_matches("r#");
                let mut routes = Vec::new();
                if let Some(imports) = by_binding.get(first) {
                    for import in imports {
                        let route = format!(
                            "{}{}",
                            import.route,
                            &text[text.split("::").next().expect("first").len()..]
                        );
                        routes.push((
                            route,
                            vec![import.anchor.clone()],
                            import.certain && imports.len() == 1,
                        ));
                    }
                } else if simple {
                    routes.push((text.into(), Vec::new(), true));
                }
                let mut binding_node = node;
                while let Some(base) = binding_node.child_by_field_name("path") {
                    binding_node = base;
                }
                let assessment = items::lexical_assessment(
                    path,
                    binding_node,
                    source,
                    first,
                    (controls.deadline, controls.cancelled),
                    false,
                )?;
                if assessment.binding == items::LexicalBinding::Uncertain {
                    unsupported(result, anchor(path, source, node, result.limits.text_bytes))?;
                }
                if path == &request.source_path
                    && assessment.binding != items::LexicalBinding::Independent
                {
                    let local = source_units
                        .range(..=node.start_byte())
                        .next_back()
                        .filter(|(_, (end, _))| *end >= node.end_byte())
                        .map(|(_, (_, id))| id.clone());
                    if let Some(local) = local {
                        for (route, evidence, certain) in &routes {
                            if let Some(route) = route_segments(route, module)
                                && let Some(targets) = declarations.get(&route)
                            {
                                for target in targets {
                                    record(result, BoundaryObservation { id: String::new(), direction: "outgoing".into(), item_ids: vec![local.clone()], ownership_candidate_ids: Vec::new(), anchor: anchor(path, source, node, result.limits.text_bytes), enclosing_unit: None, counterpart: Some(target.clone()), route_evidence: evidence.clone(), certainty: if *certain && targets.len() == 1 && assessment.binding == items::LexicalBinding::Absent { "written_route" } else { "candidate" }.into(), basis: "written_outgoing_route; not repairability_or_symbol_resolution".into() })?;
                                }
                            }
                        }
                    }
                }
                if path != &request.source_path
                    && assessment.binding != items::LexicalBinding::Independent
                {
                    for (route, evidence, certain) in routes {
                        controls.check()?;
                        if let Some(route) = route_segments(&route, module)
                            && let Some(targets) = local_targets(&route, source_module, &names)
                        {
                            let enclosing = enclosing_units
                                .range(..=node.start_byte())
                                .next_back()
                                .map(|(_, item)| *item)
                                .filter(|i| i.span.range.end_byte >= node.end_byte());
                            record(result, BoundaryObservation { id: String::new(), direction: "incoming".into(), item_ids: targets.clone(), ownership_candidate_ids: Vec::new(), anchor: anchor(path, source, node, result.limits.text_bytes), enclosing_unit: enclosing.map(|i| AdviceAnchor { path: path.clone(), span: Lines::new(source).slice(i.span.range.start_byte, i.span.range.end_byte, result.limits.text_bytes) }), counterpart: None, route_evidence: evidence, certainty: if certain && targets.len() == 1 && assessment.binding == items::LexicalBinding::Absent { "written_route" } else { "candidate" }.into(), basis: "ordinary_written_path_or_explicit_import; not resolved identity".into() })?;
                        }
                    }
                    if bare && let Some(targets) = names.get(first) {
                        let evidence =
                            globs.consumer_imports(path, node, &request.source_path, first)?;
                        if !evidence.is_empty() {
                            record(result, BoundaryObservation { id: String::new(), direction: "incoming".into(), item_ids: targets.clone(), ownership_candidate_ids: Vec::new(), anchor: anchor(path, source, node, result.limits.text_bytes), enclosing_unit: None, counterpart: None, route_evidence: evidence.into_iter().map(|r| AdviceAnchor { path: path.clone(), span: Lines::new(source).slice(r.start_byte, r.end_byte, result.limits.text_bytes) }).collect(), certainty: "candidate".into(), basis: "visible_glob_route_candidate; forwarding_or_identity_unproved".into() })?;
                        }
                    }
                }
                if simple {
                    continue;
                }
            }
            for i in (0..node.named_child_count()).rev() {
                stack.push(node.named_child(i as u32).expect("child"));
            }
        }
        result
            .boundary_observations
            .coverage
            .observed_paths
            .push(path.clone());
    }
    let records = &mut result.boundary_observations.records;
    records.sort_by(|a, b| {
        (
            &a.direction,
            &a.anchor.path,
            &a.anchor.span.range,
            &a.item_ids,
            &a.basis,
        )
            .cmp(&(
                &b.direction,
                &b.anchor.path,
                &b.anchor.span.range,
                &b.item_ids,
                &b.basis,
            ))
    });
    for (i, record) in records.iter_mut().enumerate() {
        record.id = format!("boundary/{i}");
    }
    result.counts.boundary_observations = records.len();
    result.boundary_observations.coverage.completed = true;
    Ok(())
}

pub(super) fn link(
    result: &mut SuggestSplitEnvelope,
    controls: Controls<'_>,
) -> Result<(), DomainError> {
    let mut candidates: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for c in &result.ownership_candidates {
        controls.check()?;
        for id in c
            .core_item_ids
            .iter()
            .chain(c.companions.iter().map(|c| &c.item_id))
        {
            candidates.entry(id).or_default().insert(&c.id);
        }
    }
    for record in &mut result.boundary_observations.records {
        controls.check()?;
        record.ownership_candidate_ids = record
            .item_ids
            .iter()
            .filter_map(|id| candidates.get(id.as_str()))
            .flatten()
            .copied()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .map(str::to_owned)
            .collect();
    }
    result.account(descriptor_bytes(&result.boundary_observations)?)?;
    Ok(())
}
