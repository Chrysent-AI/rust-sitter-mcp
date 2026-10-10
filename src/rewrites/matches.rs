//! Exact-source role adapter for caller-asserted standard matches! invocations.
use super::*;
use rmcp::schemars::JsonSchema;
use serde::Serialize;

#[derive(Debug, Clone, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct MatchesProof {
    pub class: MatchesProofClass,
    pub anchor: SourceAnchor,
    pub invocation: SourceAnchor,
    pub item_ids: Vec<String>,
    pub destination_path: String,
    pub declaration: SourceAnchor,
    pub variant: Option<SourceAnchor>,
    pub written_route: String,
    pub basis: String,
}
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(rename_all = "snake_case")]
pub enum MatchesProofClass {
    AssumedStandardMatches,
}
#[derive(Clone)]
pub(super) struct Invocation {
    anchor: SourceAnchor,
}
#[derive(Clone)]
struct Site {
    range: ByteRange,
    owner: ByteRange,
    variant: Option<String>,
}
struct Segment {
    scratch: usize,
    original: usize,
    length: usize,
}
impl Segment {
    fn map(&self, node: Node<'_>, scratch: &str, source: &str) -> Option<ByteRange> {
        if node.start_byte() < self.scratch || node.end_byte() > self.scratch + self.length {
            return None;
        }
        let range = span(
            self.original + node.start_byte() - self.scratch,
            self.original + node.end_byte() - self.scratch,
        );
        (scratch.get(node.byte_range()) == source.get(range.start_byte..range.end_byte))
            .then_some(range)
    }
}
fn invalid(index: usize, message: &str) -> DomainError {
    let mut error = DomainError::new("INVALID_PARAMS", message);
    error.field = Some(format!("standard_matches_assertions[{index}]"));
    error
}
pub(super) fn validate(
    request: &MoveRequest,
    files: &BTreeMap<String, FileSnapshot>,
    parsed: &BTreeMap<String, ParsedFile>,
    controls: (Instant, &AtomicBool),
) -> Result<Vec<Invocation>, DomainError> {
    let mut ledger: Vec<Invocation> = Vec::new();
    let assertions = request
        .standard_matches_assertions
        .as_deref()
        .unwrap_or_default();
    if assertions.len() > 100_000 {
        return Err(invalid(0, "too many invocation assertions"));
    }
    let mut bytes = 0usize;
    for (index, a) in assertions.iter().enumerate() {
        items::check(controls.0, controls.1)?;
        let Some(file) = files.get(&a.path) else {
            return Err(invalid(
                index,
                "assertion path is outside the admitted captured files",
            ));
        };
        let r = &a.range;
        if r.start_byte >= r.end_byte
            || file.source.get(r.start_byte..r.end_byte) != Some(a.expected_text.as_str())
        {
            return Err(invalid(
                index,
                "assertion is stale or not a valid exact UTF-8 range",
            ));
        }
        if ledger.iter().any(|v| {
            v.anchor.path == a.path
                && v.anchor.range.start_byte < r.end_byte
                && r.start_byte < v.anchor.range.end_byte
        }) {
            return Err(invalid(
                index,
                "duplicate or overlapping invocation assertion",
            ));
        }
        let Some(data) = parsed.get(&a.path) else {
            return Err(invalid(index, "assertion file was not parsed"));
        };
        let mut node = data
            .tree
            .root_node()
            .named_descendant_for_byte_range(r.start_byte, r.end_byte);
        let mut exact = false;
        while let Some(n) = node {
            if n.start_byte() != r.start_byte || n.end_byte() != r.end_byte {
                break;
            }
            if n.kind() == "macro_invocation" {
                exact = true;
                break;
            }
            node = n.parent();
        }
        if !exact {
            return Err(invalid(
                index,
                "assertion must name exactly one whole macro invocation",
            ));
        }
        bytes += a.path.len() + a.expected_text.len() + 256;
        if bytes > 128 * 1024 * 1024 {
            return Err(DomainError::new(
                "analysis_descriptor_bytes",
                "invocation assertion guard reached",
            ));
        }
        ledger.push(Invocation { anchor: a.clone() });
    }
    Ok(ledger)
}
fn shape(node: Node<'_>, source: &str, controls: (Instant, &AtomicBool)) -> Option<Vec<Site>> {
    if node.has_error()
        || node.is_missing()
        || node.end_byte() - node.start_byte() > 64 * 1024
        || node
            .child_by_field_name("macro")
            .is_none_or(|n| &source[n.byte_range()] != "matches")
    {
        return None;
    }
    let tokens = (0..node.named_child_count())
        .filter_map(|i| node.named_child(i as u32))
        .find(|n| n.kind() == "token_tree")?;
    let mut significant = Vec::new();
    for i in 0..tokens.child_count() {
        items::check(controls.0, controls.1).ok()?;
        let child = tokens.child(i)?;
        if !matches!(child.kind(), "line_comment" | "block_comment") {
            significant.push(child);
        }
    }
    if significant.first()?.kind() != "(" || significant.last()?.kind() != ")" {
        return None;
    }
    let commas: Vec<_> = significant
        .iter()
        .enumerate()
        .filter(|(_, n)| n.kind() == ",")
        .map(|(i, _)| i)
        .collect();
    if commas.is_empty()
        || commas.len() > 2
        || (commas.len() == 2 && commas[1] != significant.len() - 2)
    {
        return None;
    }
    let comma = commas[0];
    let end = if commas.len() == 2 {
        commas[1]
    } else {
        significant.len() - 1
    };
    if comma <= 1 || end <= comma + 1 {
        return None;
    }
    let expr = span(
        significant[1].start_byte(),
        significant[comma - 1].end_byte(),
    );
    let pattern = span(
        significant[comma + 1].start_byte(),
        significant[end - 1].end_byte(),
    );
    let mut scratch = String::from("fn __probe(){ match ");
    let expr_segment = Segment {
        scratch: scratch.len(),
        original: expr.start_byte,
        length: expr.end_byte - expr.start_byte,
    };
    scratch.push_str(&source[expr.start_byte..expr.end_byte]);
    scratch.push_str(" { ");
    let pattern_segment = Segment {
        scratch: scratch.len(),
        original: pattern.start_byte,
        length: pattern.end_byte - pattern.start_byte,
    };
    scratch.push_str(&source[pattern.start_byte..pattern.end_byte]);
    scratch.push_str(" => true, _ => false } }");
    let tree = crate::trivia::parse(&scratch, controls.0, controls.1).ok()??;
    let root = tree.root_node();
    let mut work = vec![(root, 0usize)];
    while let Some((n, depth)) = work.pop() {
        items::check(controls.0, controls.1).ok()?;
        if depth > 128 || n.is_error() || n.is_missing() {
            return None;
        }
        for i in 0..n.child_count() {
            work.push((n.child(i)?, depth + 1));
        }
    }
    if root.has_error() {
        return None;
    }
    let function = root.named_child(0)?;
    if root.named_child_count() != 1 || function.kind() != "function_item" {
        return None;
    }
    let block = function.child_by_field_name("body")?;
    let mut expression = block.named_child(0)?;
    if expression.kind() == "expression_statement" {
        expression = expression.named_child(0)?;
    }
    if expression.kind() != "match_expression" || block.named_child_count() != 1 {
        return None;
    }
    let value = expression.child_by_field_name("value")?;
    let body = expression.child_by_field_name("body")?;
    if body.named_child_count() != 2 {
        return None;
    }
    let arm = body.named_child(0)?;
    let pat = arm.child_by_field_name("pattern")?;
    let mut sites = Vec::new();
    expression_sites(value, &scratch, source, &expr_segment, &mut sites)?;
    pattern_sites(pat, &scratch, source, &pattern_segment, &mut sites)?;
    Some(sites)
}
fn expression_sites(
    node: Node<'_>,
    scratch: &str,
    source: &str,
    segment: &Segment,
    sites: &mut Vec<Site>,
) -> Option<()> {
    match node.kind() {
        "identifier" | "scoped_identifier" => {
            let range = segment.map(node, scratch, source)?;
            if !simple_path(&scratch[node.byte_range()]) {
                return None;
            }
            sites.push(Site {
                owner: range.clone(),
                range,
                variant: None,
            });
        }
        "parenthesized_expression" => {
            expression_sites(node.named_child(0)?, scratch, source, segment, sites)?
        }
        "integer_literal" | "float_literal" | "boolean_literal" | "string_literal"
        | "raw_string_literal" | "char_literal" => {
            segment.map(node, scratch, source)?;
        }
        _ => return None,
    }
    Some(())
}
fn pattern_sites(
    node: Node<'_>,
    scratch: &str,
    source: &str,
    segment: &Segment,
    sites: &mut Vec<Site>,
) -> Option<()> {
    match node.kind() {
        "match_pattern" => {
            if node.named_child_count() != 1 {
                return None;
            }
            pattern_sites(node.named_child(0)?, scratch, source, segment, sites)?;
        }
        "or_pattern" => {
            for i in 0..node.named_child_count() {
                pattern_sites(node.named_child(i as u32)?, scratch, source, segment, sites)?;
            }
        }
        "scoped_identifier" => {
            let text = &scratch[node.byte_range()];
            if !simple_path(text) {
                return None;
            }
            let (owner, variant) = text.rsplit_once("::")?;
            if matches!(owner, "crate" | "self" | "super" | "Self") {
                return None;
            }
            let range = segment.map(node, scratch, source)?;
            sites.push(Site {
                owner: span(range.start_byte, range.start_byte + owner.len()),
                range,
                variant: Some(variant.into()),
            });
        }
        "_" | "integer_literal" | "boolean_literal" | "char_literal" => {
            segment.map(node, scratch, source)?;
        }
        _ => return None,
    }
    Some(())
}
#[cfg(test)]
#[path = "matches_tests.rs"]
mod tests;
impl Analyzer<'_> {
    fn matches_shadow(&self, path: &str, node: Node<'_>) -> Option<SourceAnchor> {
        let source = &self.files[path].source;
        let module = self.lexical_module(path, node, false)?;
        let mut scopes = Vec::new();
        let mut parent = node.parent();
        while let Some(p) = parent {
            if matches!(
                p.kind(),
                "macro_invocation" | "macro_definition" | "token_tree"
            ) || p.has_error()
                || items::cfg::attributed(p, source, None)
            {
                return Some(anchor(
                    self.files,
                    path,
                    &span(p.start_byte(), p.end_byte()),
                ));
            }
            if matches!(p.kind(), "block" | "source_file" | "declaration_list") {
                scopes.push((path.to_owned(), p));
            }
            parent = p.parent();
        }
        for (p, context) in self.contexts {
            if p != path
                && module.starts_with(&context.module_segments)
                && context.module_segments.len() < module.len()
            {
                scopes.push((p.clone(), self.parsed[p].tree.root_node()));
            }
        }
        let consumer = self.consumer(path, node);
        if consumer != path {
            let final_module = &self.final_contexts[&consumer].module_segments;
            for (p, context) in self.contexts {
                if final_module.starts_with(&context.module_segments) {
                    scopes.push((p.clone(), self.parsed[p].tree.root_node()));
                }
            }
            for (p, item, destination) in self.selected {
                if destination == &consumer
                    && item.name.as_deref() == Some("matches")
                    && item.kind == "macro_definition"
                {
                    return Some(anchor(self.files, p, &item.span.range));
                }
            }
        }
        for (p, scope) in scopes {
            let source = &self.files[&p].source;
            for i in 0..scope.named_child_count() {
                if items::check(self.controls.0, self.controls.1).is_err() {
                    return Some(anchor(
                        self.files,
                        &p,
                        &span(scope.start_byte(), scope.end_byte()),
                    ));
                }
                let candidate = scope.named_child(i as u32).expect("child");
                let shadow = match candidate.kind() {
                    "macro_definition" => candidate.child_by_field_name("name").is_some_and(|n| {
                        source[n.byte_range()].trim_start_matches("r#") == "matches"
                    }),
                    "use_declaration" => {
                        items::use_facts(candidate, source, self.controls).map_or(true, |f| f.0)
                            || items::use_leaves(candidate, source, self.controls).map_or(
                                true,
                                |ls| {
                                    ls.iter()
                                        .any(|l| l.binding.trim_start_matches("r#") == "matches")
                                },
                            )
                    }
                    "attribute_item" | "inner_attribute_item" => {
                        !items::context_independent_attribute(&source[candidate.byte_range()])
                    }
                    "extern_crate_declaration" => attributed_use(candidate),
                    "macro_invocation" => candidate != node,
                    _ => false,
                };
                if shadow {
                    return Some(anchor(
                        self.files,
                        &p,
                        &span(candidate.start_byte(), candidate.end_byte()),
                    ));
                }
            }
        }
        None
    }
    fn matches_owner(&self, need: &Need, node: Node<'_>, text: &str) -> Option<String> {
        let module = self.lexical_module(&need.path, node, false)?;
        let first = text.split("::").next()?;
        let scoped = self.scoped_imports(&need.path, &module, node, first);
        if !scoped.is_empty() {
            let nearest = scoped
                .iter()
                .map(|b| {
                    b.scope_range
                        .as_ref()
                        .map(|r| r.end_byte - r.start_byte)
                        .unwrap_or(usize::MAX)
                })
                .min()?;
            let candidates: Vec<_> = scoped
                .iter()
                .filter(|b| {
                    b.scope_range
                        .as_ref()
                        .map(|r| r.end_byte - r.start_byte)
                        .unwrap_or(usize::MAX)
                        == nearest
                })
                .collect();
            if candidates.len() != 1 || candidates[0].conditioned || candidates[0].leaf.public {
                return None;
            }
            let base = self.imported_target(candidates[0])?;
            return Some(format!("{base}{}", text.strip_prefix(first)?));
        }
        if !text.contains("::") {
            let imports = self.imported(&need.path, &module, text);
            let direct = self
                .binding(&need.path, text)
                .filter(|_| self.contexts[&need.path].module_segments == module);
            if imports.len() > 1 || (direct.is_some() && !imports.is_empty()) {
                return None;
            }
            if direct.is_some() {
                return Some(canonical(&self.contexts[&need.path], text));
            }
            if imports.len() == 1 && !imports[0].conditioned {
                return self.imported_target(&imports[0]);
            }
            return None;
        }
        self.resolve_in(&need.path, &module, text, true)
    }
    pub(super) fn matches_repair(&mut self, need: &mut Need) -> bool {
        let Some(invocation) = self
            .matches_ledger
            .iter()
            .find(|v| v.anchor.path == need.path && v.anchor.range == need.range)
            .cloned()
        else {
            return false;
        };
        let data = &self.parsed[&need.path];
        let source = &self.files[&need.path].source;
        let mut node = data
            .tree
            .root_node()
            .named_descendant_for_byte_range(need.range.start_byte, need.range.end_byte)
            .expect("validated invocation");
        while node.kind() != "macro_invocation" {
            let Some(parent) = node.parent() else {
                return false;
            };
            node = parent;
        }
        let Some(sites) = shape(node, source, self.controls) else {
            return false;
        };
        if self.lexical_module(&need.path, node, false).is_none() {
            return false;
        }
        if let Some(witness) = self.matches_shadow(&need.path, node) {
            need.message = "asserted standard matches! has a known shadow or unexamined macro-provider context".into();
            need.refusal_basis.push(items::RefusalBasis::new(
                "chain_macro_statement",
                &witness.path,
                Some(witness.range),
            ));
            return false;
        }
        // Full clones preserve merged/deduplicated repairs as well as appended ones.
        let repairs = self.repairs.clone();
        let regions = self.visibility_regions.clone();
        let observations = self.visibility_observations.clone();
        let proofs = self.matches_proofs.clone();
        self.active_matches = Some(invocation.anchor.range.clone());
        let moved_owner = self.consumer(&need.path, node) != need.path;
        let mut ok = true;
        for site in sites {
            *self.reference_candidates += 1;
            if *self.reference_candidates > 100_000
                || items::check(self.controls.0, self.controls.1).is_err()
            {
                self.exceeded = true;
                ok = false;
                break;
            }
            let text = &source[site.owner.start_byte..site.owner.end_byte];
            let relevant = need.item_ids.iter().any(|id| {
                self.selected.iter().any(|(_, i, _)| {
                    &i.id == id
                        && i.name
                            .as_deref()
                            .is_some_and(|n| text.split("::").any(|s| s == n))
                })
            }) || self.imports.iter().any(|b| {
                b.path == need.path
                    && b.leaf.binding == text.split("::").next().unwrap_or("")
                    && self
                        .imported_target(b)
                        .is_some_and(|t| self.mapped(&t) != t)
            });
            let Some(context) = data
                .tree
                .root_node()
                .named_descendant_for_byte_range(site.owner.start_byte, site.owner.start_byte + 1)
            else {
                ok = false;
                break;
            };
            let mut child = need.clone();
            child.range = site.owner.clone();
            child.reason = DecisionReason::LexicalContextUnproved;
            child.category = "unsupported_dependency_form";
            let first = text.split("::").next().expect("simple path");
            let module = self
                .lexical_module(&need.path, context, false)
                .expect("checked module");
            let aliases = self.scoped_imports(&need.path, &module, context, first);
            let lexical = items::lexical_assessment_with_matches(
                &need.path,
                context,
                source,
                first,
                self.controls,
                aliases.len() == 1,
                self.cfg.as_ref(),
                Some(&self.pattern_routes),
                self.active_matches.as_ref(),
            )
            .map(|a| a.binding)
            .unwrap_or(items::LexicalBinding::Uncertain);
            if site.variant.is_some()
                && !matches!(first, "crate" | "self" | "super")
                && lexical != items::LexicalBinding::Absent
            {
                ok = false;
                break;
            }
            let declaration =
                if site.variant.is_none() && lexical == items::LexicalBinding::Independent {
                    None
                } else {
                    self.matches_owner(&child, context, text).and_then(|old| {
                        let using = self.lexical_module(&child.path, context, true)?;
                        let resolved = self.written_target(&mut child, &old, &using)?;
                        let (p, i) = self.declaration(&resolved.terminal)?;
                        Some((p, i, resolved.route))
                    })
                };
            let mut variant_anchor = None;
            if let Some(variant) = &site.variant {
                let Some((p, item, _)) = &declaration else {
                    ok = false;
                    break;
                };
                if item.kind != "enum_item" {
                    ok = false;
                    break;
                }
                let Some(enum_node) = self.parsed[p]
                    .tree
                    .root_node()
                    .named_descendant_for_byte_range(
                        item.span.range.start_byte,
                        item.span.range.end_byte,
                    )
                else {
                    ok = false;
                    break;
                };
                if enum_node.child_by_field_name("type_parameters").is_some() {
                    ok = false;
                    break;
                }
                let Some(body) = enum_node.child_by_field_name("body") else {
                    ok = false;
                    break;
                };
                let variants: Vec<_> = (0..body.named_child_count())
                    .filter_map(|i| body.named_child(i as u32))
                    .filter(|n| {
                        n.kind() == "enum_variant"
                            && n.child_by_field_name("name")
                                .is_some_and(|n| &self.files[p].source[n.byte_range()] == variant)
                    })
                    .collect();
                if variants.len() != 1
                    || variants[0].has_error()
                    || variants[0].child_by_field_name("body").is_some()
                    || attributed_use(variants[0])
                {
                    ok = false;
                    break;
                }
                variant_anchor = Some(anchor(
                    self.files,
                    p,
                    &span(variants[0].start_byte(), variants[0].end_byte()),
                ));
            }
            let repaired = if text.contains("::") {
                self.path_site_repair(&mut child, context, site.owner.clone())
            } else {
                self.bare_site_repair(&mut child, context)
            };
            if !repaired {
                need.message = format!(
                    "asserted matches! ordinary check failed at {}..{}: {}",
                    child.range.start_byte, child.range.end_byte, child.message
                );
                need.lexical_uncertainty = child.lexical_uncertainty;
                ok = false;
                break;
            }
            if relevant || (moved_owner && declaration.is_some()) {
                let Some((p, item, route)) = declaration else {
                    ok = false;
                    break;
                };
                let proof = MatchesProof { class: MatchesProofClass::AssumedStandardMatches,
                    anchor: anchor(self.files, &need.path, &site.range), invocation: invocation.anchor.clone(), item_ids: need.item_ids.clone(),
                    destination_path: self.consumer(&need.path, context), declaration: anchor(self.files, &p, &item.span.range), variant: variant_anchor, written_route: route,
                    basis: "caller-asserted standard matches! contract; exact grammar roles, known-shadow audit and ordinary written binding/access checks; no expansion, hygiene, compilation or equivalence proof".into() };
                if let Some(existing) = self
                    .matches_proofs
                    .iter_mut()
                    .find(|p| p.anchor == proof.anchor)
                {
                    for id in &proof.item_ids {
                        if !existing.item_ids.contains(id) {
                            existing.item_ids.push(id.clone());
                        }
                    }
                } else {
                    self.descriptor_bytes +=
                        serde_json::to_vec(&proof).expect("proof JSON").len() + 128;
                    self.matches_proofs.push(proof);
                }
            }
        }
        self.active_matches = None;
        if !ok {
            self.repairs = repairs;
            self.visibility_regions = regions;
            self.visibility_observations = observations;
            self.matches_proofs = proofs;
        }
        if self.descriptor_bytes > 128 * 1024 * 1024 {
            self.exceeded = true;
            return false;
        }
        ok
    }
    pub(super) fn matches_final_shadows(&mut self) -> Vec<Need> {
        let mut failures = Vec::new();
        self.matches_proofs.retain(|proof| {
            let witness = self.repairs.iter().find(|r| r.kind == "import_insert" && r.path == proof.destination_path
                && matches!(&r.target, RewriteTarget::Synthesis { binding: Some(name), .. } if name == "matches"));
            let Some(witness) = witness else { return true; };
            failures.push(Need {
                reason: DecisionReason::MacroContextUnexamined, category: "macro_dependency",
                path: proof.invocation.path.clone(), range: proof.invocation.range.clone(),
                attribute_range: None, choice_target: None, lexical_uncertainty: None,
                refusal_basis: witness.anchors.iter().map(|a| items::RefusalBasis::new("chain_macro_statement", &a.path, Some(a.range.clone()))).collect(),
                message: "planned import introduces a competing matches macro-provider binding".into(),
                item_ids: proof.item_ids.clone(),
            });
            false
        });
        failures
    }
    pub(super) fn matches_alias_repair(
        &mut self,
        need: &Need,
        node: Node<'_>,
        alias: &str,
    ) -> bool {
        if node.kind() != "macro_invocation" {
            return false;
        }
        let source = &self.files[&need.path].source;
        let Some(sites) = shape(node, source, self.controls) else {
            return false;
        };
        let mut stack = vec![node];
        while let Some(n) = stack.pop() {
            if items::check(self.controls.0, self.controls.1).is_err() {
                return false;
            }
            if matches!(
                n.kind(),
                "string_literal"
                    | "raw_string_literal"
                    | "char_literal"
                    | "line_comment"
                    | "block_comment"
            ) {
                continue;
            }
            if n.child_count() == 0
                && &source[n.byte_range()] == alias
                && !sites.iter().any(|s| {
                    s.owner.start_byte <= n.start_byte() && s.owner.end_byte >= n.end_byte()
                })
            {
                return false;
            }
            for i in 0..n.child_count() {
                stack.push(n.child(i).expect("child"));
            }
        }
        let mut child = need.clone();
        child.range = span(node.start_byte(), node.end_byte());
        child.reason = DecisionReason::MacroContextUnexamined;
        self.matches_repair(&mut child)
    }
}
