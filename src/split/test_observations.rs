//! Advisory written test relationships; never execution acknowledgement evidence.
use super::*;
use tree_sitter::Node;

type Attribution = (Vec<String>, Vec<AdviceAnchor>, Vec<String>);

#[derive(Debug, Clone, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct TestObservations {
    pub coverage: TestCoverage,
    pub routes: Vec<TestRoute>,
    pub records: Vec<TestObservation>,
    pub limitations: Vec<TestLimitation>,
}
#[derive(Debug, Clone, Default, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct TestCoverage {
    pub completed: bool,
    pub admitted_paths: Vec<String>,
    pub traversed_paths: Vec<String>,
    pub unlinked_test_roots: Vec<String>,
    pub not_assessed: Vec<String>,
    pub macro_token_coverage: String,
    pub note: String,
}
impl Default for TestObservations {
    fn default() -> Self {
        Self {
            coverage: TestCoverage {
                not_assessed: vec!["unadmitted_files".into(), "Cargo_targets_and_library_aliases".into(), "cfg_evaluation".into(), "macro_expansion".into(), "type_inference".into(), "assertion_equivalence".into(), "test_placement".into(), "move_applicability".into()],
                macro_token_coverage: "written identifier paths and simple identifier.field token shapes inside macro inputs; comments and literals excluded; arbitrary receivers, expansion and binding identity unproved".into(),
                note: "completed means the admitted written discovery scan completed, not complete test coverage; zero observations never means tests unaffected; possible companions require caller inspection and are never selected".into(),
                ..TestCoverage::default()
            },
            routes: Vec::new(), records: Vec::new(), limitations: Vec::new(),
        }
    }
}
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct TestRoute {
    pub id: String,
    pub declaration: AdviceAnchor,
    pub attributes: Vec<AdviceAnchor>,
    pub conditional: bool,
    pub candidate_paths: Vec<String>,
    pub candidate_states: Vec<String>,
    pub traversed_path: Option<String>,
    pub status: String,
    pub basis: String,
}
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct TestLimitation {
    pub reason: String,
    pub anchor: Option<AdviceAnchor>,
    pub paths: Vec<String>,
    pub note: String,
}
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct TestObservation {
    pub id: String,
    pub channel: String,
    pub access_kind: String,
    pub anchor: AdviceAnchor,
    pub enclosing_function: Option<AdviceAnchor>,
    pub enclosing_modules: Vec<AdviceAnchor>,
    pub module_segments: Option<Vec<String>>,
    pub route_ids: Vec<String>,
    pub route_evidence: Vec<AdviceAnchor>,
    pub target_item_ids: Vec<String>,
    pub attribution: String,
    pub uncertainty: Vec<String>,
    pub labels: Vec<String>,
    pub candidate_couplings: Vec<TestCandidateCoupling>,
    pub possible_companion: bool,
}
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct TestCandidateCoupling {
    pub ownership_candidate_id: String,
    pub labels: Vec<String>,
    pub possible_companion: bool,
}
#[derive(Clone)]
struct Context {
    path: String,
    start: usize,
    end: usize,
    segments: Option<Vec<String>>,
    testing: bool,
    route_ids: Vec<String>,
    evidence: Vec<AdviceAnchor>,
    modules: Vec<AdviceAnchor>,
    uncertain: bool,
}
struct Projection<'a> {
    request: &'a SuggestSplitRequest,
    scope: &'a Scope,
    files: &'a BTreeMap<String, FileSnapshot>,
    parsed: &'a BTreeMap<String, ParsedFile>,
    globs: &'a items::GlobRoutes<'a>,
    controls: Controls<'a>,
    names: BTreeMap<String, Vec<String>>,
    source_segments: Option<Vec<String>>,
    inventory_indices: BTreeMap<String, usize>,
    fields: BTreeMap<(String, String), (bool, bool)>,
    candidate_links: BTreeMap<String, Vec<(String, bool)>>,
}
fn attributes<'a>(node: Node<'a>) -> Vec<Node<'a>> {
    let mut result = Vec::new();
    let mut previous = node.prev_named_sibling();
    while let Some(node) = previous {
        match node.kind() {
            "attribute_item" => result.push(node),
            "line_comment" | "block_comment" => {}
            _ => break,
        }
        previous = node.prev_named_sibling();
    }
    result.reverse();
    result
}
fn exact(source: &str, node: Node<'_>, form: &str) -> bool {
    source[node.byte_range()]
        .chars()
        .filter(|c| !c.is_whitespace())
        .eq(form.chars())
}
fn test_cfg_marker(source: &str, node: Node<'_>) -> bool {
    exact(source, node, "#[cfg(test)]")
}
fn unsupported_test_cfg(source: &str, node: Node<'_>) -> bool {
    if test_cfg_marker(source, node) {
        return false;
    }
    let compact: String = source[node.byte_range()].split_whitespace().collect();
    if !(compact.starts_with("#[cfg(") || compact.starts_with("#[cfg_attr(")) {
        return false;
    }
    let mut stack = vec![node];
    while let Some(n) = stack.pop() {
        if n.kind() == "identifier" && &source[n.byte_range()] == "test" {
            return true;
        }
        if excluded(n) {
            continue;
        }
        for i in 0..n.named_child_count() {
            stack.push(n.named_child(i as u32).expect("attribute child"));
        }
    }
    false
}
fn token_name(node: Node<'_>) -> bool {
    matches!(
        node.kind(),
        "identifier" | "type_identifier" | "crate" | "self" | "super"
    )
}
fn excluded(node: Node<'_>) -> bool {
    matches!(
        node.kind(),
        "line_comment"
            | "block_comment"
            | "string_literal"
            | "raw_string_literal"
            | "char_literal"
            | "integer_literal"
            | "float_literal"
            | "boolean_literal"
    )
}
fn spelling(source: &str, node: Node<'_>) -> String {
    source[node.byte_range()]
        .split_whitespace()
        .collect::<String>()
        .trim_start_matches("r#")
        .into()
}
impl Projection<'_> {
    fn anchor(&self, path: &str, node: Node<'_>, result: &SuggestSplitEnvelope) -> AdviceAnchor {
        AdviceAnchor {
            path: path.into(),
            span: Lines::new(&self.files[path].source).slice(
                node.start_byte(),
                node.end_byte(),
                result.limits.text_bytes,
            ),
        }
    }
    fn charge(
        &self,
        result: &mut SuggestSplitEnvelope,
        value: &impl Serialize,
    ) -> Result<(), DomainError> {
        self.controls.check()?;
        result.counts.reference_candidates += 1;
        if result.counts.reference_candidates > result.effective_work_limits.reference_candidates {
            return Err(DomainError::new(
                "reference_work_limit",
                "test observation guard reached",
            ));
        }
        result.account(descriptor_bytes(value)?)
    }
    fn limitation(
        &self,
        result: &mut SuggestSplitEnvelope,
        reason: &str,
        anchor: Option<AdviceAnchor>,
        paths: Vec<String>,
        note: &str,
    ) -> Result<(), DomainError> {
        let value = TestLimitation {
            reason: reason.into(),
            anchor,
            paths,
            note: note.into(),
        };
        self.charge(result, &value)?;
        result.test_observations.limitations.push(value);
        Ok(())
    }
    fn route(
        &self,
        ctx: &Context,
        node: Node<'_>,
        result: &mut SuggestSplitEnvelope,
    ) -> Result<Option<Context>, DomainError> {
        self.controls.check()?;
        let source = &self.files[&ctx.path].source;
        let attrs = attributes(node);
        let conditional = attrs.iter().any(|a| test_cfg_marker(source, *a));
        // Reuse the legacy recognition for directly inventoried inline modules,
        // without altering its same-file execution predicate or its exclusions.
        let legacy = self.parsed[&ctx.path]
            .items
            .iter()
            .find(|i| i.span.range.start_byte == node.start_byte());
        let recognized = legacy
            .map(|i| {
                items::test_scope(
                    i,
                    node,
                    source,
                    (self.controls.deadline, self.controls.cancelled),
                    self.globs,
                )
            })
            .transpose()?
            .flatten()
            .is_some();
        let testing = ctx.testing || conditional || recognized;
        let unsupported = attrs.iter().any(|a| {
            !test_cfg_marker(source, *a)
                && !items::context_independent_attribute(&source[a.byte_range()])
        });
        let declaration = self.anchor(&ctx.path, node, result);
        let attr_anchors: Vec<_> = attrs
            .iter()
            .map(|a| self.anchor(&ctx.path, *a, result))
            .collect();
        let name = node
            .child_by_field_name("name")
            .map(|n| spelling(source, n))
            .unwrap_or_default();
        let mut segments = ctx.segments.clone();
        if let Some(parts) = &mut segments {
            parts.push(name.clone());
        }
        let mut next = ctx.clone();
        next.testing = testing;
        next.segments = segments;
        next.uncertain |= unsupported || node.has_error();
        next.modules.push(declaration.clone());
        next.evidence.extend(attr_anchors.clone());
        next.evidence.push(declaration.clone());
        if let Some(body) = node.child_by_field_name("body") {
            next.start = body.start_byte();
            next.end = body.end_byte();
            if testing {
                let value = TestRoute {
                    id: format!("test-route/{}", result.test_observations.routes.len()),
                    declaration,
                    attributes: attr_anchors,
                    conditional,
                    candidate_paths: Vec::new(),
                    candidate_states: Vec::new(),
                    traversed_path: Some(ctx.path.clone()),
                    status: if next.uncertain {
                        "inline_uncertain"
                    } else {
                        "inline_written"
                    }
                    .into(),
                    basis: "written_inline_test_scope; no_cfg_evaluation_or_execution_admission"
                        .into(),
                };
                next.route_ids.push(value.id.clone());
                self.charge(result, &value)?;
                result.test_observations.routes.push(value);
            }
            return Ok(Some(next));
        }
        let flat = if ctx.start == 0 {
            items::child_path(&ctx.path, &self.request.crate_root, &name)
        } else {
            let mut base = items::child_path(&ctx.path, &self.request.crate_root, "placeholder");
            base.truncate(base.len() - "placeholder.rs".len());
            for module in ctx.modules.iter().filter(|m| m.path == ctx.path) {
                let n = self.parsed[&ctx.path]
                    .tree
                    .root_node()
                    .named_descendant_for_byte_range(
                        module.span.range.start_byte,
                        module.span.range.end_byte,
                    )
                    .and_then(|n| n.child_by_field_name("name"));
                if let Some(n) = n {
                    base.push_str(&spelling(source, n));
                    base.push('/');
                }
            }
            format!("{base}{name}.rs")
        };
        let candidates = vec![
            flat.clone(),
            format!("{}/mod.rs", flat.trim_end_matches(".rs")),
        ];
        let mut states = Vec::new();
        for path in &candidates {
            self.controls.check()?;
            states.push(
                if self.files.contains_key(path) {
                    "admitted"
                } else if items::present(self.scope, path)? {
                    "unadmitted"
                } else {
                    "missing"
                }
                .to_owned(),
            );
        }
        let present: Vec<_> = states
            .iter()
            .enumerate()
            .filter(|(_, s)| s.as_str() != "missing")
            .map(|(i, _)| i)
            .collect();
        let status = if unsupported || node.has_error() {
            "unsupported_attributes_or_remapping"
        } else if present.len() > 1 {
            "competing_layouts"
        } else if present.is_empty() {
            "missing_test_file"
        } else if states[present[0]] == "unadmitted" {
            "unadmitted_test_file"
        } else {
            "conditional_written_route"
        };
        let traversed =
            (status == "conditional_written_route").then(|| candidates[present[0]].clone());
        if testing {
            let value = TestRoute {
                id: format!("test-route/{}", result.test_observations.routes.len()),
                declaration: declaration.clone(),
                attributes: attr_anchors,
                conditional,
                candidate_paths: candidates.clone(),
                candidate_states: states,
                traversed_path: traversed.clone(),
                status: status.into(),
                basis:
                    "conventional_written_module_discovery_only; not_execution_admissible_cfg_edge"
                        .into(),
            };
            next.route_ids.push(value.id.clone());
            self.charge(result, &value)?;
            result.test_observations.routes.push(value);
            if traversed.is_none() {
                self.limitation(result, status, Some(declaration), candidates, "declared test route not scanned; missing observations do not show tests unaffected")?;
            }
        }
        if let Some(path) = traversed {
            next.start = 0;
            next.end = self.files[&path].source.len();
            next.path = path;
            return Ok(Some(next));
        }
        Ok(None)
    }
    fn scope_node<'a>(&self, ctx: &Context, data: &'a ParsedFile) -> Node<'a> {
        data.tree
            .root_node()
            .named_descendant_for_byte_range(ctx.start, ctx.end)
            .expect("admitted scope")
    }
    fn resolve_path(&self, ctx: &Context, parts: &[String]) -> Option<Vec<String>> {
        ctx.segments.as_ref()?;
        let first = parts.first()?.as_str();
        let mut skip = 1;
        let mut route = match first {
            "crate" => Vec::new(),
            "self" => ctx.segments.clone()?,
            "super" => {
                let mut route = ctx.segments.clone()?;
                for part in parts {
                    if part != "super" {
                        break;
                    }
                    route.pop()?;
                }
                skip = parts.iter().take_while(|p| p.as_str() == "super").count();
                route
            }
            _ => {
                skip = 0;
                ctx.segments.clone()?
            }
        };
        route.extend_from_slice(&parts[skip..]);
        Some(route)
    }
    fn targets(&self, route: &[String]) -> Vec<String> {
        let Some(source) = &self.source_segments else {
            return Vec::new();
        };
        if route.len() <= source.len() || route[..source.len()] != *source {
            return Vec::new();
        }
        self.names
            .get(&route[source.len()])
            .cloned()
            .unwrap_or_default()
    }
    fn path_targets(
        &self,
        ctx: &Context,
        node: Node<'_>,
        text: &str,
        result: &SuggestSplitEnvelope,
    ) -> Result<Attribution, DomainError> {
        let parts: Vec<String> = text
            .split("::")
            .map(|p| p.trim().trim_start_matches("r#").into())
            .collect();
        let Some(first) = parts.first() else {
            return Ok((Vec::new(), Vec::new(), vec!["unsupported_path".into()]));
        };
        if matches!(first.as_str(), "crate" | "self" | "super") {
            let targets = self
                .resolve_path(ctx, &parts)
                .map(|r| self.targets(&r))
                .unwrap_or_default();
            return Ok((
                targets,
                Vec::new(),
                if ctx.segments.is_none() {
                    vec!["module_route_unresolved".into()]
                } else {
                    Vec::new()
                },
            ));
        }
        let source = &self.files[&ctx.path].source;
        let mut base = node;
        while let Some(n) = base.child_by_field_name("path") {
            base = n;
        }
        let assessment = items::lexical_assessment(
            &ctx.path,
            base,
            source,
            first,
            (self.controls.deadline, self.controls.cancelled),
            false,
        )?;
        let mut evidence = Vec::new();
        let mut targets = BTreeSet::new();
        let mut uncertainty = Vec::new();
        let mut competing_declaration = false;
        // Named imports are interpreted only in the reference's actual lexical
        // module/block scope. Parent modules do not donate their imports.
        let mut parent = node.parent();
        while let Some(scope) = parent {
            self.controls.check()?;
            if matches!(scope.kind(), "source_file" | "block" | "declaration_list") {
                for i in 0..scope.named_child_count() {
                    self.controls.check()?;
                    let import = scope.named_child(i as u32).expect("scope child");
                    if import.kind() != "use_declaration" {
                        competing_declaration |= import
                            .child_by_field_name("name")
                            .is_some_and(|n| spelling(source, n) == *first);
                        continue;
                    }
                    for leaf in items::use_leaves(
                        import,
                        source,
                        (self.controls.deadline, self.controls.cancelled),
                    )? {
                        if leaf.binding.trim_start_matches("r#") != first {
                            continue;
                        }
                        let mut route: Vec<String> = leaf
                            .path
                            .split("::")
                            .map(|p| p.trim().trim_start_matches("r#").into())
                            .collect();
                        route.extend_from_slice(&parts[1..]);
                        if let Some(route) = self.resolve_path(ctx, &route) {
                            targets.extend(self.targets(&route));
                        }
                        evidence.push(self.anchor(&ctx.path, import, result));
                        if !attributes(import).is_empty() {
                            uncertainty.push("attributed_import".into());
                        }
                    }
                }
                if !evidence.is_empty() || scope.kind() != "block" {
                    break;
                }
            }
            parent = scope.parent();
        }
        if competing_declaration && !evidence.is_empty() {
            uncertainty.push("competing_written_binding".into());
        }
        if evidence.len() > 1 {
            uncertainty.push("ambiguous_import_routes".into());
        }
        if assessment.binding == items::LexicalBinding::Uncertain {
            uncertainty.push("lexical_context_unproved".into());
        }
        if assessment.binding == items::LexicalBinding::Independent
            && (evidence.is_empty()
                || assessment.definite_binding.as_ref().is_some_and(|b| {
                    !evidence.iter().any(|a| {
                        a.span.range.start_byte <= b.range.start_byte
                            && a.span.range.end_byte >= b.range.end_byte
                    })
                }))
        {
            return Ok((
                Vec::new(),
                Vec::new(),
                vec!["independent_written_binding".into()],
            ));
        }
        if evidence.is_empty() && ctx.segments == self.source_segments && !ctx.uncertain {
            targets.extend(self.names.get(first).into_iter().flatten().cloned());
        }
        if evidence.is_empty() && targets.is_empty() {
            let imports =
                self.globs
                    .consumer_imports(&ctx.path, base, &self.request.source_path, first)?;
            for r in imports {
                evidence.push(AdviceAnchor {
                    path: ctx.path.clone(),
                    span: Lines::new(source).slice(
                        r.start_byte,
                        r.end_byte,
                        result.limits.text_bytes,
                    ),
                });
            }
            if !evidence.is_empty() {
                targets.extend(self.names.get(first).into_iter().flatten().cloned());
                uncertainty.push("glob_binding_candidate".into());
            }
        }
        Ok((targets.into_iter().collect(), evidence, uncertainty))
    }
    fn receiver_targets(
        &self,
        ctx: &Context,
        receiver: Node<'_>,
        result: &SuggestSplitEnvelope,
    ) -> Result<Attribution, DomainError> {
        let source = &self.files[&ctx.path].source;
        if receiver.kind() == "struct_expression"
            && let Some(name) = receiver.child_by_field_name("name")
        {
            let (ids, mut evidence, uncertainty) =
                self.path_targets(ctx, name, &spelling(source, name), result)?;
            evidence.push(self.anchor(&ctx.path, name, result));
            return Ok((ids, evidence, uncertainty));
        }
        if receiver.kind() != "identifier" {
            return Ok((
                Vec::new(),
                Vec::new(),
                vec!["receiver_chain_or_inference_unproved".into()],
            ));
        }
        let name = spelling(source, receiver);
        let assessment = items::lexical_assessment(
            &ctx.path,
            receiver,
            source,
            &name,
            (self.controls.deadline, self.controls.cancelled),
            false,
        )?;
        let macro_uncertain = assessment
            .uncertainty
            .as_ref()
            .is_some_and(|u| u.witness_relation.is_some());
        let data = &self.parsed[&ctx.path];
        let test_marker_only = assessment.uncertainty.as_ref().is_some_and(|u| {
            u.reason == items::LexicalReason::ConditionalLocalContext
                && data
                    .tree
                    .root_node()
                    .named_descendant_for_byte_range(
                        u.scope.range.start_byte,
                        u.scope.range.end_byte,
                    )
                    .is_some_and(|n| {
                        n.kind() == "function_item"
                            && attributes(n).iter().all(|a| {
                                exact(source, *a, "#[test]")
                                    || items::context_independent_attribute(&source[a.byte_range()])
                            })
                    })
        });
        let binding_node = assessment.definite_binding.as_ref().and_then(|b| {
            data.tree
                .root_node()
                .named_descendant_for_byte_range(b.range.start_byte, b.range.end_byte)
        });
        // A block macro prevents certainty, but advice can retain a simple
        // written parameter/let candidate with that explicit caveat.
        let binding_node = binding_node.or_else(|| {
            (macro_uncertain || test_marker_only)
                .then(|| self.simple_receiver_binding(receiver, source, &name))
                .flatten()
        });
        let Some(mut node) = binding_node else {
            return Ok((
                Vec::new(),
                Vec::new(),
                vec!["receiver_binding_unproved".into()],
            ));
        };
        while !matches!(node.kind(), "parameter" | "let_declaration") {
            let Some(parent) = node.parent() else {
                return Ok((
                    Vec::new(),
                    Vec::new(),
                    vec!["receiver_nominal_type_unproved".into()],
                ));
            };
            node = parent;
        }
        let nominal = node.child_by_field_name("type").or_else(|| {
            node.child_by_field_name("value")
                .filter(|n| n.kind() == "struct_expression")
                .and_then(|n| n.child_by_field_name("name"))
        });
        let Some(mut nominal) = nominal else {
            return Ok((
                Vec::new(),
                Vec::new(),
                vec!["receiver_nominal_type_unproved".into()],
            ));
        };
        while nominal.kind() == "reference_type" {
            let Some(n) = nominal.child_by_field_name("type") else {
                break;
            };
            nominal = n;
        }
        if !matches!(
            nominal.kind(),
            "type_identifier" | "scoped_type_identifier" | "identifier" | "scoped_identifier"
        ) {
            return Ok((
                Vec::new(),
                Vec::new(),
                vec!["receiver_nominal_type_unproved".into()],
            ));
        }
        let (ids, mut evidence, mut uncertainty) =
            self.path_targets(ctx, nominal, &spelling(source, nominal), result)?;
        if macro_uncertain {
            uncertainty.push("lexical_macro_context_unproved".into());
        }
        evidence.push(self.anchor(&ctx.path, node, result));
        evidence.push(self.anchor(&ctx.path, nominal, result));
        Ok((ids, evidence, uncertainty))
    }
    fn simple_receiver_binding<'a>(
        &self,
        receiver: Node<'a>,
        source: &str,
        name: &str,
    ) -> Option<Node<'a>> {
        let mut child = receiver;
        while let Some(parent) = child.parent() {
            if self.controls.check().is_err() {
                return None;
            }
            if parent.kind() == "block" {
                for i in (0..parent.named_child_count()).rev() {
                    let node = parent.named_child(i as u32)?;
                    if node.kind() == "let_declaration"
                        && node.end_byte() <= child.start_byte()
                        && node.child_by_field_name("pattern").is_some_and(|p| {
                            p.kind() == "identifier" && spelling(source, p) == name
                        })
                    {
                        return Some(node);
                    }
                }
            }
            if parent.kind() == "function_item" {
                let params = parent.child_by_field_name("parameters")?;
                for i in 0..params.named_child_count() {
                    let node = params.named_child(i as u32)?;
                    if node
                        .child_by_field_name("pattern")
                        .is_some_and(|p| p.kind() == "identifier" && spelling(source, p) == name)
                    {
                        return Some(node);
                    }
                }
                return None;
            }
            if matches!(
                parent.kind(),
                "mod_item" | "source_file" | "closure_expression"
            ) {
                return None;
            }
            child = parent;
        }
        None
    }
    fn field_targets(
        &self,
        owner_ids: Vec<String>,
        field: &str,
        uncertainty: &mut Vec<String>,
    ) -> Result<(Vec<String>, bool), DomainError> {
        let mut ids = Vec::new();
        let mut private = false;
        for id in owner_ids {
            self.controls.check()?;
            if let Some((field_private, attributed)) = self.fields.get(&(id.clone(), field.into()))
            {
                ids.push(id);
                private |= field_private;
                if *attributed {
                    uncertainty.push("field_attributes_unproved".into());
                }
            }
        }
        Ok((ids, private))
    }
    #[allow(clippy::too_many_arguments)] // One advisory occurrence and its original evidence, not planner state.
    fn emit(
        &self,
        ctx: &Context,
        node: Node<'_>,
        channel: &str,
        kind: &str,
        targets: Vec<String>,
        evidence: Vec<AdviceAnchor>,
        mut uncertainty: Vec<String>,
        private: bool,
        result: &mut SuggestSplitEnvelope,
    ) -> Result<(), DomainError> {
        if ctx.uncertain {
            uncertainty.push("scope_attributes_or_syntax_unproved".into());
        }
        if ctx.segments.is_none() {
            uncertainty.push("unlinked_test_root".into());
        }
        if targets.len() > 1 {
            uncertainty.push("ambiguous_target_identity".into());
        }
        if targets.is_empty() {
            uncertainty.push("target_unresolved".into());
        }
        if channel == "macro_token_candidate" {
            uncertainty.push("macro_tokens_not_expanded_or_bound".into());
        }
        let mut parent = node.parent();
        let mut function = None;
        while let Some(n) = parent {
            self.controls.check()?;
            if n.kind() == "function_item" && function.is_none() {
                function = Some(self.anchor(&ctx.path, n, result));
            }
            if attributes(n).iter().any(|a| {
                !exact(&self.files[&ctx.path].source, *a, "#[test]")
                    && !exact(&self.files[&ctx.path].source, *a, "#[cfg(test)]")
                    && !items::context_independent_attribute(
                        &self.files[&ctx.path].source[a.byte_range()],
                    )
            }) {
                uncertainty.push("enclosing_attributes_unproved".into());
            }
            parent = n.parent();
        }
        uncertainty.sort();
        uncertainty.dedup();
        let mut labels = BTreeSet::new();
        for id in &targets {
            if let Some(item) = self
                .inventory_indices
                .get(id)
                .map(|i| &result.inventory[*i])
            {
                labels.insert(
                    if item.visibility_key == "pub" {
                        "facade"
                    } else {
                        "moved_implementation"
                    }
                    .to_owned(),
                );
            }
        }
        if private {
            labels.insert("private_state".into());
        }
        if !uncertainty.is_empty() {
            labels.insert("unresolved".into());
        }
        let mut linked: BTreeMap<String, bool> = BTreeMap::new();
        for id in &targets {
            for (candidate, core) in self.candidate_links.get(id).into_iter().flatten() {
                self.controls.check()?;
                *linked.entry(candidate.clone()).or_default() |= core;
            }
        }
        let mut couplings = Vec::new();
        for (id, core) in linked {
            let mut per = labels.clone();
            if core {
                per.insert("moved_implementation".into());
            }
            if per.contains("facade")
                && (per.contains("private_state") || per.contains("moved_implementation"))
            {
                per.insert("mixed".into());
            }
            couplings.push(TestCandidateCoupling {
                ownership_candidate_id: id,
                possible_companion: private || per.contains("moved_implementation"),
                labels: per.into_iter().collect(),
            });
        }
        if labels.contains("facade")
            && (labels.contains("private_state") || labels.contains("moved_implementation"))
        {
            labels.insert("mixed".into());
        }
        let mut route_evidence = ctx.evidence.clone();
        route_evidence.extend(evidence);
        let possible_companion = private || labels.contains("moved_implementation");
        let value = TestObservation {
            id: format!(
                "test-observation/{}",
                result.test_observations.records.len()
            ),
            channel: channel.into(),
            access_kind: kind.into(),
            anchor: self.anchor(&ctx.path, node, result),
            enclosing_function: function,
            enclosing_modules: ctx.modules.clone(),
            module_segments: ctx.segments.clone(),
            route_ids: ctx.route_ids.clone(),
            route_evidence,
            target_item_ids: targets,
            attribution: if uncertainty.is_empty() {
                "written_nominal_or_path_route"
            } else {
                "candidate_or_unresolved"
            }
            .into(),
            uncertainty,
            labels: labels.into_iter().collect(),
            candidate_couplings: couplings,
            possible_companion,
        };
        self.charge(result, &value)?;
        result.test_observations.records.push(value);
        Ok(())
    }
    fn macro_tokens(
        &self,
        ctx: &Context,
        node: Node<'_>,
        result: &mut SuggestSplitEnvelope,
    ) -> Result<(), DomainError> {
        let source = &self.files[&ctx.path].source;
        let mut stack = vec![node];
        while let Some(tree) = stack.pop() {
            self.controls.check()?;
            if excluded(tree) {
                continue;
            }
            if tree.kind() != "token_tree" {
                for i in (0..tree.named_child_count()).rev() {
                    stack.push(tree.named_child(i as u32).expect("macro child"));
                }
                continue;
            }
            // Separators are siblings in token trees; literals/groups break a
            // shape. Never splice across them or parse expansion as Rust code.
            let mut tokens = Vec::new();
            for i in 0..tree.child_count() {
                self.controls.check()?;
                let child = tree.child(i).expect("token");
                if child.kind() == "token_tree" {
                    stack.push(child);
                }
                if matches!(child.kind(), "line_comment" | "block_comment") {
                    continue;
                }
                result.account(64)?;
                tokens.push(child);
            }
            let mut index = 0;
            while index < tokens.len() {
                self.controls.check()?;
                let first = tokens[index];
                if !token_name(first) {
                    index += 1;
                    continue;
                }
                let mut end = index;
                while end + 2 < tokens.len()
                    && source[tokens[end + 1].byte_range()] == *"::"
                    && token_name(tokens[end + 2])
                {
                    end += 2;
                }
                if end > index {
                    let text = &source[first.start_byte()..tokens[end].end_byte()];
                    let (ids, evidence, uncertainty) = self.path_targets(
                        ctx,
                        first,
                        &text.split_whitespace().collect::<String>(),
                        result,
                    )?;
                    self.emit(
                        ctx,
                        first,
                        "macro_token_candidate",
                        "path_tokens",
                        ids,
                        evidence,
                        uncertainty,
                        false,
                        result,
                    )?;
                    self.macro_anchor(first, tokens[end], ctx, result)?;
                } else if index + 2 < tokens.len()
                    && source[tokens[index + 1].byte_range()] == *"."
                    && tokens[index + 2].kind() == "identifier"
                {
                    let chained = index >= 2 && source[tokens[index - 1].byte_range()] == *".";
                    let (ids, evidence, mut uncertainty) = if chained {
                        (
                            Vec::new(),
                            Vec::new(),
                            vec!["receiver_chain_or_inference_unproved".into()],
                        )
                    } else {
                        self.receiver_targets(ctx, first, result)?
                    };
                    let (ids, private) = self.field_targets(
                        ids,
                        &spelling(source, tokens[index + 2]),
                        &mut uncertainty,
                    )?;
                    uncertainty.push("token_shape_only".into());
                    self.emit(
                        ctx,
                        tokens[index + 2],
                        "macro_token_candidate",
                        "field_tokens",
                        ids,
                        evidence,
                        uncertainty,
                        private,
                        result,
                    )?;
                    self.macro_anchor(first, tokens[index + 2], ctx, result)?;
                }
                index = end + 1;
            }
        }
        Ok(())
    }
    fn macro_anchor(
        &self,
        first: Node<'_>,
        last: Node<'_>,
        ctx: &Context,
        result: &mut SuggestSplitEnvelope,
    ) -> Result<(), DomainError> {
        let span = Lines::new(&self.files[&ctx.path].source).slice(
            first.start_byte(),
            last.end_byte(),
            result.limits.text_bytes,
        );
        result.account(descriptor_bytes(&span)?)?;
        result
            .test_observations
            .records
            .last_mut()
            .expect("emitted token candidate")
            .anchor
            .span = span;
        Ok(())
    }
    fn scan(&self, ctx: &Context, result: &mut SuggestSplitEnvelope) -> Result<(), DomainError> {
        let data = &self.parsed[&ctx.path];
        let source = &self.files[&ctx.path].source;
        let root = self.scope_node(ctx, data);
        let mut stack = vec![(root, ctx.testing)];
        while let Some((node, inherited_test)) = stack.pop() {
            self.controls.check()?;
            if excluded(node)
                || matches!(
                    node.kind(),
                    "attribute_item"
                        | "inner_attribute_item"
                        | "use_declaration"
                        | "macro_definition"
                )
            {
                continue;
            }
            if node.kind() == "mod_item" {
                continue;
            }
            let testing = inherited_test
                || (node.kind() == "function_item"
                    && attributes(node)
                        .iter()
                        .any(|a| exact(source, *a, "#[test]")));
            if node.kind() == "macro_invocation" {
                if testing {
                    self.macro_tokens(ctx, node, result)?;
                }
                continue;
            }
            if testing
                && node.kind() == "field_expression"
                && let (Some(receiver), Some(field)) = (
                    node.child_by_field_name("value"),
                    node.child_by_field_name("field"),
                )
            {
                let (owners, evidence, mut uncertainty) =
                    self.receiver_targets(ctx, receiver, result)?;
                let method = node.parent().is_some_and(|p| {
                    p.kind() == "call_expression" && p.child_by_field_name("function") == Some(node)
                });
                let (ids, private) = if method {
                    uncertainty.push("method_identity_unproved".into());
                    (owners, false)
                } else {
                    self.field_targets(owners, &spelling(source, field), &mut uncertainty)?
                };
                self.emit(
                    ctx,
                    node,
                    "cst",
                    if method {
                        "method_access"
                    } else {
                        "field_access"
                    },
                    ids,
                    evidence,
                    uncertainty,
                    private,
                    result,
                )?;
            }
            if testing
                && matches!(
                    node.kind(),
                    "identifier"
                        | "type_identifier"
                        | "scoped_identifier"
                        | "scoped_type_identifier"
                )
                && items::reference_role(node)
            {
                let text = spelling(source, node);
                let (ids, evidence, uncertainty) = self.path_targets(ctx, node, &text, result)?;
                if !ids.is_empty() || text.contains("::") {
                    self.emit(
                        ctx,
                        node,
                        "cst",
                        "path_access",
                        ids,
                        evidence,
                        uncertainty,
                        false,
                        result,
                    )?;
                }
                if text.contains("::") {
                    continue;
                }
            }
            for i in (0..node.named_child_count()).rev() {
                stack.push((node.named_child(i as u32).expect("child"), testing));
            }
        }
        Ok(())
    }
}

#[allow(clippy::too_many_arguments)] // Reuses the existing parsed corpus and request-local work guards.
pub(super) fn collect(
    request: &SuggestSplitRequest,
    scope: &Scope,
    files: &BTreeMap<String, FileSnapshot>,
    parsed: &BTreeMap<String, ParsedFile>,
    contexts: &BTreeMap<String, items::ModuleEvidence>,
    globs: &items::GlobRoutes<'_>,
    controls: Controls<'_>,
    result: &mut SuggestSplitEnvelope,
) -> Result<(), DomainError> {
    let mut names: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for item in &result.inventory {
        controls.check()?;
        if item.enclosing_impl_id.is_none()
            && let Some(name) = &item.name
        {
            names
                .entry(name.trim_start_matches("r#").into())
                .or_default()
                .push(item.id.clone());
        }
    }
    let inventory_indices: BTreeMap<String, usize> = result
        .inventory
        .iter()
        .enumerate()
        .map(|(i, item)| (item.id.clone(), i))
        .collect();
    let mut fields = BTreeMap::new();
    let source = &files[&request.source_path].source;
    let data = &parsed[&request.source_path];
    for index in 0..result.inventory.len() {
        controls.check()?;
        let item = result.inventory[index].clone();
        if !matches!(item.kind.as_str(), "struct_item" | "union_item") {
            continue;
        }
        let node = data
            .tree
            .root_node()
            .named_descendant_for_byte_range(item.span.range.start_byte, item.span.range.end_byte)
            .expect("type");
        if let Some(body) = node.child_by_field_name("body") {
            for i in 0..body.named_child_count() {
                controls.check()?;
                let field = body.named_child(i as u32).expect("field");
                if let Some(name) = field.child_by_field_name("name") {
                    result.account(128 + item.id.len() + name.end_byte() - name.start_byte())?;
                    fields.insert(
                        (item.id.clone(), spelling(source, name)),
                        (
                            !source[field.byte_range()].trim_start().starts_with("pub"),
                            attributes(field).iter().any(|a| {
                                !items::context_independent_attribute(&source[a.byte_range()])
                            }),
                        ),
                    );
                }
            }
        }
    }
    let mut candidate_links: BTreeMap<String, Vec<(String, bool)>> = BTreeMap::new();
    for candidate in &result.ownership_candidates {
        controls.check()?;
        for id in &candidate.core_item_ids {
            candidate_links
                .entry(id.clone())
                .or_default()
                .push((candidate.id.clone(), true));
        }
        for companion in &candidate.companions {
            candidate_links
                .entry(companion.item_id.clone())
                .or_default()
                .push((candidate.id.clone(), false));
        }
    }
    result.account(descriptor_bytes(&(&inventory_indices, &candidate_links))?)?;
    let mut projection = Projection {
        request,
        scope,
        files,
        parsed,
        globs,
        controls,
        names,
        inventory_indices,
        fields,
        candidate_links,
        source_segments: contexts
            .get(&request.source_path)
            .filter(|m| m.unresolved.is_empty())
            .map(|m| m.module_segments.clone()),
    };
    result.account(descriptor_bytes(&projection.names)?)?;
    result.test_observations.coverage.admitted_paths = files.keys().cloned().collect();
    let root = Context {
        path: request.crate_root.clone(),
        start: 0,
        end: files[&request.crate_root].source.len(),
        segments: Some(Vec::new()),
        testing: false,
        route_ids: Vec::new(),
        evidence: Vec::new(),
        modules: Vec::new(),
        uncertain: false,
    };
    // First discover contexts independently of observations, so a conditional
    // source route can supply written relative identity without cfg admission.
    let mut queue = vec![root];
    let mut discovered = Vec::new();
    let mut seen = BTreeSet::new();
    let mut cursor = 0;
    let mut conflict_paths = BTreeSet::new();
    while cursor < queue.len() {
        controls.check()?;
        let ctx = queue[cursor].clone();
        cursor += 1;
        if !seen.insert((ctx.path.clone(), ctx.start, ctx.end)) {
            conflict_paths.insert(ctx.path.clone());
            if ctx.path == request.source_path {
                projection.source_segments = None;
            }
            projection.limitation(
                result,
                "competing_module_routes",
                ctx.evidence.last().cloned(),
                vec![ctx.path.clone()],
                "repeated written module context is not a unique identity",
            )?;
            continue;
        }
        if ctx.path == request.source_path && ctx.start == 0 {
            projection.source_segments = ctx.segments.clone();
        }
        let data = &parsed[&ctx.path];
        let scope_node = projection.scope_node(&ctx, data);
        for i in 0..scope_node.named_child_count() {
            controls.check()?;
            let child = scope_node.named_child(i as u32).expect("scope child");
            if child.kind() == "mod_item"
                && let Some(next) = projection.route(&ctx, child, result)?
            {
                result.account(descriptor_bytes(&(
                    &next.path,
                    &next.segments,
                    &next.evidence,
                    &next.modules,
                    &next.route_ids,
                ))?)?;
                queue.push(next);
            }
        }
        discovered.push(ctx);
    }
    let linked_paths: BTreeSet<_> = discovered.iter().map(|c| c.path.clone()).collect();
    for (path, data) in parsed {
        controls.check()?;
        let source = &files[path].source;
        let mut stack = vec![data.tree.root_node()];
        let mut marker = None;
        while let Some(node) = stack.pop() {
            controls.check()?;
            if node.kind() == "attribute_item" {
                if exact(source, node, "#[test]") || test_cfg_marker(source, node) {
                    marker.get_or_insert(node);
                } else if unsupported_test_cfg(source, node) {
                    projection.limitation(
                        result,
                        "unsupported_test_cfg",
                        Some(projection.anchor(path, node, result)),
                        vec![path.clone()],
                        "cfg expression mentions test but is not a supported positive test marker; not evaluated and does not establish a test root or coupling",
                    )?;
                }
                continue;
            }
            if excluded(node) || matches!(node.kind(), "macro_invocation" | "macro_definition") {
                continue;
            }
            for i in 0..node.named_child_count() {
                stack.push(node.named_child(i as u32).expect("child"));
            }
        }
        if let Some(marker) = marker.filter(|_| !linked_paths.contains(path)) {
            result
                .test_observations
                .coverage
                .unlinked_test_roots
                .push(path.clone());
            projection.limitation(result, "unlinked_test_root", Some(projection.anchor(path, marker, result)), vec![path.clone()], "written test marker in admitted file without a supported route; no Cargo target or library-alias identity inferred")?;
            queue.push(Context {
                path: path.clone(),
                start: 0,
                end: source.len(),
                segments: None,
                testing: false,
                route_ids: Vec::new(),
                evidence: Vec::new(),
                modules: Vec::new(),
                uncertain: true,
            });
        }
    }
    while cursor < queue.len() {
        controls.check()?;
        let ctx = queue[cursor].clone();
        cursor += 1;
        if !seen.insert((ctx.path.clone(), ctx.start, ctx.end)) {
            continue;
        }
        let scope_node = projection.scope_node(&ctx, &parsed[&ctx.path]);
        for i in 0..scope_node.named_child_count() {
            controls.check()?;
            let child = scope_node.named_child(i as u32).expect("scope child");
            if child.kind() == "mod_item"
                && let Some(next) = projection.route(&ctx, child, result)?
            {
                queue.push(next);
            }
        }
        discovered.push(ctx);
    }
    // Context discovery owns module traversal; observation scans never assign
    // Cargo identity to an unlinked root or duplicate nested module occurrences.
    for mut ctx in discovered {
        controls.check()?;
        ctx.uncertain |=
            conflict_paths.contains(&ctx.path) || parsed[&ctx.path].tree.root_node().has_error();
        let scope_node = projection.scope_node(&ctx, &parsed[&ctx.path]);
        for i in 0..scope_node.named_child_count() {
            controls.check()?;
            let child = scope_node.named_child(i as u32).expect("scope child");
            if child.kind() == "inner_attribute_item"
                && !items::context_independent_attribute(
                    &files[&ctx.path].source[child.byte_range()],
                )
            {
                ctx.uncertain = true;
                projection.limitation(
                    result,
                    "unsupported_scope_attributes",
                    Some(projection.anchor(&ctx.path, child, result)),
                    vec![ctx.path.clone()],
                    "scope attributes are not evaluated; attributed observations remain candidates",
                )?;
            }
        }
        projection.scan(&ctx, result)?;
        result
            .test_observations
            .coverage
            .traversed_paths
            .push(ctx.path);
    }
    result.test_observations.coverage.traversed_paths.sort();
    result.test_observations.coverage.traversed_paths.dedup();
    result.test_observations.coverage.completed = true;
    result.counts.test_observations = result.test_observations.records.len();
    result.account(descriptor_bytes(&result.test_observations)?)?;
    Ok(())
}

pub(super) fn recheck(
    scope: &Scope,
    result: &SuggestSplitEnvelope,
    controls: Controls<'_>,
) -> Result<(), DomainError> {
    for route in &result.test_observations.routes {
        for (path, state) in route.candidate_paths.iter().zip(&route.candidate_states) {
            controls.check()?;
            if items::present(scope, path)? != (state != "missing") {
                return Err(DomainError::new(
                    "SOURCE_CHANGED",
                    "test discovery candidate state changed; obtain fresh advice",
                ));
            }
        }
    }
    Ok(())
}
impl TestObservations {
    pub(super) fn omit_text(&mut self) -> usize {
        let mut count = 0;
        for route in &mut self.routes {
            count += super::omit_text(&mut route.declaration.span);
            for anchor in &mut route.attributes {
                count += super::omit_text(&mut anchor.span);
            }
        }
        for record in &mut self.records {
            count += super::omit_text(&mut record.anchor.span);
            for anchor in record
                .enclosing_function
                .iter_mut()
                .chain(&mut record.enclosing_modules)
                .chain(&mut record.route_evidence)
            {
                count += super::omit_text(&mut anchor.span);
            }
        }
        for limitation in &mut self.limitations {
            if let Some(anchor) = &mut limitation.anchor {
                count += super::omit_text(&mut anchor.span);
            }
        }
        count
    }
}
