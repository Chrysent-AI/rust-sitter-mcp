//! Bounded written-binding repairs. Evidence is syntactic, never symbol resolution.
mod associated;
use crate::{
    items::{self, DecisionReason, Item, ModuleEvidence, Need, ParsedFile},
    move_plan::{MoveRequest, RewriteTarget, VisibilityConsumer, VisibilityExplanation},
    plan::SourceAnchor,
    result::{ByteRange, DomainError},
    scope::FileSnapshot,
};
use std::{collections::BTreeMap, sync::atomic::AtomicBool, time::Instant};
use tree_sitter::Node;

#[derive(Clone)]
pub(crate) struct Repair {
    pub path: String,
    pub range: ByteRange,
    pub after: String,
    pub kind: &'static str,
    pub written_reexport: bool,
    pub target: RewriteTarget,
    pub item_ids: Vec<String>,
    pub anchors: Vec<SourceAnchor>,
    pub rationale: String,
    pub visibility: Option<VisibilityExplanation>,
    /// A visibility operation on the declaration linking this proposed file.
    pub declaration_for: Option<String>,
    pub references: Vec<SourceAnchor>,
    pub caller_override: bool,
    pub import_module: Option<Vec<String>>,
    pub import_scope: Option<ByteRange>,
}
pub(crate) struct Analysis {
    pub repairs: Vec<Repair>,
    pub needs: Vec<Need>,
    pub descriptor_bytes: usize,
}
fn span(start: usize, end: usize) -> ByteRange {
    ByteRange {
        start_byte: start,
        end_byte: end,
    }
}
fn anchor(files: &BTreeMap<String, FileSnapshot>, path: &str, range: &ByteRange) -> SourceAnchor {
    SourceAnchor {
        path: path.into(),
        range: range.clone(),
        expected_text: files[path].source[range.start_byte..range.end_byte].into(),
    }
}
fn access(
    files: &BTreeMap<String, FileSnapshot>,
    path: &str,
    range: &ByteRange,
    reason: &str,
    original: &[String],
    final_region: &[String],
) -> VisibilityConsumer {
    VisibilityConsumer {
        anchor: anchor(files, path, range),
        access_reason: reason.into(),
        original_module_region: original.to_vec(),
        final_module_region: final_region.to_vec(),
    }
}
fn import_text(target: &str, binding: &str) -> String {
    if target.rsplit("::").next() == Some(binding) {
        format!("use {target};")
    } else {
        format!("use {target} as {binding};")
    }
}
pub(crate) fn simple_path(text: &str) -> bool {
    text.split("::").all(|s| {
        !s.is_empty()
            && s.bytes()
                .enumerate()
                .all(|(i, b)| b == b'_' || b.is_ascii_alphabetic() || (i > 0 && b.is_ascii_digit()))
    })
}
fn invalid_choice(message: &str) -> DomainError {
    let mut error = DomainError::new("INVALID_REWRITE_OVERRIDE", message);
    error.field = Some("rewrite_overrides".into());
    error
}
fn absolute_path(module: &[String], text: &str) -> Option<String> {
    if !simple_path(text) {
        return None;
    }
    let mut parts: Vec<_> = text.split("::").map(str::to_owned).collect();
    let mut prefix = match parts.first()?.as_str() {
        "crate" => {
            parts.remove(0);
            Vec::new()
        }
        "self" => {
            parts.remove(0);
            module.to_vec()
        }
        "super" => {
            let mut prefix = module.to_vec();
            while parts.first().is_some_and(|p| p == "super") {
                parts.remove(0);
                prefix.pop()?;
            }
            prefix
        }
        _ => return None,
    };
    prefix.splice(0..0, ["crate".into()]);
    prefix.extend(parts);
    Some(prefix.join("::"))
}
fn common_region(left: &[String], right: &[String]) -> Vec<String> {
    left.iter()
        .zip(right)
        .take_while(|(a, b)| a == b)
        .map(|(segment, _)| segment.clone())
        .collect()
}
struct VisibilityRegion {
    defining: Vec<String>,
    required: Vec<String>,
}
impl VisibilityRegion {
    fn text(&self) -> String {
        if self.required.is_empty() {
            "pub(crate)".into()
        } else if self.required.len() + 1 == self.defining.len() {
            "pub(super)".into()
        } else {
            format!("pub(in crate::{})", self.required.join("::"))
        }
    }
}
#[derive(Default)]
struct VisibilityObservations {
    consumers: Vec<VisibilityConsumer>,
    preserved_original_access_regions: Vec<Vec<String>>,
}
type VisibilitySubject = (String, usize, Option<String>);
type VisibilityKey = (String, usize, usize, Option<String>);
fn visibility_key(repair: &Repair) -> VisibilityKey {
    (
        repair.path.clone(),
        repair.range.start_byte,
        repair.range.end_byte,
        repair.declaration_for.clone(),
    )
}
fn restricted_region(
    modifier: Node<'_>,
    source: &str,
    defining: &[String],
    controls: (Instant, &AtomicBool),
) -> Option<Vec<String>> {
    // Read parsed grammar tokens; whitespace/comments do not alter the scope.
    let mut text = String::new();
    let mut stack = vec![modifier];
    while let Some(node) = stack.pop() {
        items::check(controls.0, controls.1).ok()?;
        if matches!(node.kind(), "line_comment" | "block_comment") {
            continue;
        }
        if node.child_count() == 0 {
            text.push_str(&source[node.byte_range()]);
        } else {
            for i in (0..node.child_count()).rev() {
                stack.push(node.child(i).expect("child"));
            }
        }
    }
    if text == "pub(crate)" {
        return Some(Vec::new());
    }
    let restriction = text.strip_prefix("pub(")?.strip_suffix(')')?;
    let restriction = match restriction {
        "self" | "super" => restriction,
        _ => restriction.strip_prefix("in")?,
    };
    let scope = absolute_path(defining, restriction)?;
    let scope: Vec<_> = scope.split("::").skip(1).map(str::to_owned).collect();
    defining.starts_with(&scope).then_some(scope)
}
fn chosen_visibility_region(
    text: &str,
    defining: &[String],
    controls: (Instant, &AtomicBool),
) -> Result<Vec<String>, DomainError> {
    if text.trim().is_empty() {
        return Ok(defining.to_vec());
    }
    if text.contains("//") || text.contains("/*") {
        return Err(invalid_choice(
            "visibility alternatives cannot contain comments",
        ));
    }
    let source = format!("{text} fn __visibility() {{}}");
    let tree = crate::trivia::parse(&source, controls.0, controls.1)?
        .ok_or_else(|| invalid_choice("alternative parse interrupted"))?;
    let root = tree.root_node();
    let node = root.named_child(0);
    let modifier = node.and_then(|n| n.named_child(0));
    if root.has_error()
        || root.named_child_count() != 1
        || node.is_none_or(|n| n.kind() != "function_item")
        || modifier.is_none_or(|n| {
            n.kind() != "visibility_modifier" || &source[n.byte_range()] != text.trim()
        })
    {
        return Err(invalid_choice(
            "alternative must be one internal visibility modifier",
        ));
    }
    restricted_region(modifier.expect("checked modifier"), &source, defining, controls)
        .ok_or_else(|| invalid_choice("visibility must be private, pub(crate), pub(self), pub(super), or pub(in an evidenced ancestor); no pub/API escalation"))
}
fn parsed_import(
    text: &str,
    controls: (Instant, &AtomicBool),
) -> Result<(String, String), DomainError> {
    let tree = crate::trivia::parse(text, controls.0, controls.1)?
        .ok_or_else(|| invalid_choice("alternative parse interrupted"))?;
    if tree.root_node().has_error() || tree.root_node().named_child_count() != 1 {
        return Err(invalid_choice(
            "alternative must be exactly one explicit non-glob use",
        ));
    }
    let node = tree.root_node().named_child(0).expect("child");
    let leaves = items::use_leaves(node, text, controls)?;
    if node.kind() != "use_declaration"
        || leaves.len() != 1
        || leaves[0].public
        || !leaves[0].prefix.is_empty()
        || !simple_path(&leaves[0].path)
        || !simple_path(&leaves[0].binding)
        || leaves[0].binding.contains("::")
        || text.contains("//")
        || text.contains("/*")
    {
        return Err(invalid_choice(
            "only one private explicit use with a simple path and optional alias is supported",
        ));
    }
    Ok((leaves[0].path.clone(), leaves[0].binding.clone()))
}
fn canonical(context: &ModuleEvidence, name: &str) -> String {
    let mut parts = vec!["crate".to_owned()];
    parts.extend(context.module_segments.clone());
    parts.push(name.into());
    parts.join("::")
}
fn attributed_use(node: Node<'_>) -> bool {
    let mut previous = node.prev_named_sibling();
    while let Some(comment) =
        previous.filter(|n| matches!(n.kind(), "line_comment" | "block_comment"))
    {
        previous = comment.prev_named_sibling();
    }
    previous.is_some_and(|n| n.kind() == "attribute_item")
}
#[derive(Clone)]
struct ImportBinding {
    path: String,
    module: Vec<String>,
    leaf: items::UseLeaf,
    conditioned: bool,
    written_attributes: bool,
    scope_range: Option<ByteRange>,
}
#[derive(Default, Clone)]
struct RouteEvidence {
    anchors: Vec<SourceAnchor>,
    fallback: Vec<String>,
}
impl RouteEvidence {
    fn merge(&mut self, other: Self) {
        for a in other.anchors {
            if !self.anchors.contains(&a) {
                self.anchors.push(a);
            }
        }
        for disclosure in other.fallback {
            if !self.fallback.contains(&disclosure) {
                self.fallback.push(disclosure);
            }
        }
    }
    fn disclose(&self, rationale: &mut String) -> usize {
        let before = rationale.len();
        for disclosure in &self.fallback {
            if !rationale.contains(disclosure) {
                rationale.push_str("; ");
                rationale.push_str(disclosure);
            }
        }
        rationale.len() - before
    }
}
struct WrittenTarget {
    terminal: String,
    route: String,
    evidence: RouteEvidence,
}
struct Analyzer<'a> {
    request: &'a MoveRequest,
    cfg: Option<items::DeclaredCfg>,
    files: &'a BTreeMap<String, FileSnapshot>,
    parsed: &'a BTreeMap<String, ParsedFile>,
    selected: &'a [(String, Item, String)],
    contexts: &'a BTreeMap<String, ModuleEvidence>,
    final_contexts: &'a BTreeMap<String, ModuleEvidence>,
    repairs: Vec<Repair>,
    visibility_regions: BTreeMap<VisibilityKey, VisibilityRegion>,
    visibility_observations: BTreeMap<VisibilitySubject, VisibilityObservations>,
    imports: Vec<ImportBinding>,
    descriptor_bytes: usize,
    reference_candidates: &'a mut usize,
    exceeded: bool,
    controls: (Instant, &'a AtomicBool),
    pattern_routes: items::GlobRoutes<'a>,
}
impl Analyzer<'_> {
    fn lexical_binding(
        &self,
        path: &str,
        node: Node<'_>,
        source: &str,
        name: &str,
        proven_import: bool,
    ) -> items::LexicalBinding {
        items::lexical_assessment_with_globs(
            path,
            node,
            source,
            name,
            self.controls,
            proven_import,
            self.cfg.as_ref(),
            Some(&self.pattern_routes),
        )
        .map(|a| a.binding)
        .unwrap_or(items::LexicalBinding::Uncertain)
    }
    fn account_lexical(&mut self, witness: &Option<items::LexicalUncertainty>) {
        if let Some(witness) = witness {
            self.descriptor_bytes += serde_json::to_vec(witness)
                .expect("lexical evidence JSON")
                .len()
                + 32;
            if self.descriptor_bytes > 128 * 1024 * 1024 {
                self.exceeded = true;
            }
        }
    }
    fn add(&mut self, repair: Repair) {
        self.descriptor_bytes += repair.path.len()
            + repair.after.len()
            + repair.rationale.len()
            + repair.visibility.as_ref().map_or(0, |v| {
                serde_json::to_vec(v)
                    .expect("visibility evidence JSON")
                    .len()
            })
            + repair
                .anchors
                .iter()
                .chain(&repair.references)
                .map(|a| a.path.len() + a.expected_text.len() + 128)
                .sum::<usize>()
            + 512;
        if self.descriptor_bytes > 128 * 1024 * 1024 || self.repairs.len() >= 100_000 {
            self.exceeded = true;
            return;
        }
        if let Some(existing) = self.repairs.iter_mut().find(|r| {
            r.path == repair.path
                && r.range == repair.range
                && r.after == repair.after
                && r.declaration_for == repair.declaration_for
        }) {
            existing.written_reexport |= repair.written_reexport;
            for id in repair.item_ids {
                if !existing.item_ids.contains(&id) {
                    existing.item_ids.push(id);
                }
            }
            for a in repair.references {
                if !existing.references.contains(&a) {
                    existing.references.push(a);
                }
            }
            for a in repair.anchors {
                if !existing.anchors.contains(&a) {
                    existing.anchors.push(a);
                }
            }
            if let RewriteTarget::Synthesis { items, .. } = &mut existing.target
                && let RewriteTarget::Synthesis {
                    items: incoming, ..
                } = repair.target
            {
                for a in incoming {
                    if !items.contains(&a) {
                        items.push(a);
                    }
                }
            }
        } else {
            self.repairs.push(repair);
        }
    }
    fn add_visibility(&mut self, mut repair: Repair, defining: &[String], required: &[String]) {
        let key = visibility_key(&repair);
        if !self.visibility_regions.contains_key(&key) {
            self.descriptor_bytes += repair.path.len()
                + repair.declaration_for.as_ref().map_or(0, String::len)
                + defining
                    .iter()
                    .chain(required)
                    .map(|s| s.len() + 32)
                    .sum::<usize>()
                + 256;
        }
        let region = self
            .visibility_regions
            .entry(key)
            .or_insert_with(|| VisibilityRegion {
                defining: defining.to_vec(),
                required: required.to_vec(),
            });
        region.required = common_region(&region.required, required);
        repair
            .visibility
            .as_mut()
            .expect("visibility evidence")
            .narrowest_covering_region = region.required.clone();
        repair.after = region.text();
        if repair.range.start_byte == repair.range.end_byte {
            repair.after.push(' ');
        }
        // All callers share one splice. Later callers can require a broader ancestor.
        if let Some(existing) = self.repairs.iter_mut().find(|r| {
            r.kind == "visibility"
                && r.path == repair.path
                && r.range == repair.range
                && r.declaration_for == repair.declaration_for
        }) {
            existing.after.clone_from(&repair.after);
            existing
                .visibility
                .as_mut()
                .expect("visibility evidence")
                .narrowest_covering_region = region.required.clone();
        }
        self.add(repair);
    }
    fn explain_visibility(&mut self) -> Result<(), DomainError> {
        for repair in &mut self.repairs {
            items::check(self.controls.0, self.controls.1)?;
            let Some(explanation) = &mut repair.visibility else {
                continue;
            };
            let subject = (
                repair.path.clone(),
                repair.range.start_byte,
                repair.declaration_for.clone(),
            );
            let observations = &self.visibility_observations[&subject];
            explanation.consumers = observations.consumers.clone();
            explanation.consumers.sort();
            explanation.preserved_original_access_regions =
                observations.preserved_original_access_regions.clone();
            explanation.preserved_original_access_regions.sort();
            explanation.basis = if explanation.consumers.is_empty() {
                "preserve original defining-region access; no new observed caller".into()
            } else {
                "cover the exact observed written accesses and preserved access requirements".into()
            };
            if !explanation.consumers.is_empty()
                && !explanation.preserved_original_access_regions.is_empty()
            {
                explanation.basis.push_str(
                    "; preserve original defining-region access without inferring outside callers",
                );
            }
            explanation.basis.push_str("; the narrowest covering region is their deepest common evidenced ancestor, not exhaustive caller discovery or semantic verification");
            repair.rationale.clone_from(&explanation.basis);
            repair.references = explanation
                .consumers
                .iter()
                .map(|c| c.anchor.clone())
                .collect();
            self.descriptor_bytes += serde_json::to_vec(explanation)
                .expect("visibility explanation JSON")
                .len()
                + repair.rationale.len()
                + repair
                    .references
                    .iter()
                    .map(|a| a.path.len() + a.expected_text.len() + 128)
                    .sum::<usize>();
            self.exceeded |= self.descriptor_bytes > 128 * 1024 * 1024;
        }
        Ok(())
    }
    fn contributors(&self, ids: &[String]) -> Vec<SourceAnchor> {
        self.request
            .moves
            .iter()
            .filter(|m| {
                self.selected.iter().any(|(_, i, _)| {
                    ids.contains(&i.id) && i.path == m.item.path && i.span.range == m.item.range
                })
            })
            .map(|m| m.item.clone())
            .collect()
    }
    fn final_path<'a>(&'a self, path: &'a str, item: &Item) -> &'a str {
        self.selected
            .iter()
            .find(|(p, i, _)| p == path && i.id == item.id)
            .map(|(_, _, d)| d.as_str())
            .unwrap_or(path)
    }
    fn lexical_module(
        &self,
        path: &str,
        node: Node<'_>,
        final_location: bool,
    ) -> Option<Vec<String>> {
        let final_path = self.consumer(path, node);
        let context = if final_location {
            self.final_contexts.get(&final_path)?
        } else {
            self.contexts.get(path)?
        };
        if !context.unresolved.is_empty() {
            return None;
        }
        let mut module = context.module_segments.clone();
        let mut inline = Vec::new();
        let mut parent = node.parent();
        while let Some(p) = parent {
            if p.kind() == "mod_item" && p.child_by_field_name("body").is_some() {
                if items::cfg::attributed(p, &self.files[path].source, self.cfg.as_ref()) {
                    return None;
                }
                inline.push(
                    self.files[path].source[p.child_by_field_name("name")?.byte_range()].to_owned(),
                );
            }
            parent = p.parent();
        }
        module.extend(inline.into_iter().rev());
        Some(module)
    }
    fn consumer(&self, path: &str, node: Node<'_>) -> String {
        self.selected
            .iter()
            .find(|(p, i, _)| {
                p == path
                    && i.span.range.start_byte <= node.start_byte()
                    && i.span.range.end_byte >= node.end_byte()
            })
            .map(|(_, _, d)| d.clone())
            .unwrap_or_else(|| path.into())
    }
    fn declaration(&self, target: &str) -> Option<(String, Item)> {
        let mut found = Vec::new();
        for (path, data) in self.parsed {
            if items::check(self.controls.0, self.controls.1).is_err() {
                return None;
            }
            if let Some(context) = self.contexts.get(path) {
                for item in &data.items {
                    if items::check(self.controls.0, self.controls.1).is_err() {
                        return None;
                    }
                    if item
                        .name
                        .as_deref()
                        .is_some_and(|name| canonical(context, name) == target)
                    {
                        found.push((path.clone(), item.clone()));
                    }
                }
            }
        }
        (found.len() == 1).then(|| found.remove(0))
    }
    fn imported(&self, path: &str, module: &[String], name: &str) -> Vec<ImportBinding> {
        self.imports
            .iter()
            .filter(|b| {
                b.path == path
                    && b.module == module
                    && b.leaf.binding == name
                    && b.scope_range.is_none()
            })
            .cloned()
            .collect()
    }
    fn imported_target(&self, binding: &ImportBinding) -> Option<String> {
        let node = self.parsed[&binding.path]
            .tree
            .root_node()
            .named_descendant_for_byte_range(
                binding.leaf.declaration.start_byte,
                binding.leaf.declaration.end_byte,
            )?;
        self.resolve_use(&binding.path, &binding.module, node, &binding.leaf.path)
    }
    fn reexports(&self, target: &str) -> Vec<&ImportBinding> {
        self.imports
            .iter()
            .filter(|b| {
                b.scope_range.is_none()
                    && self.contexts.get(&b.path).is_some_and(|c| {
                        c.unresolved.is_empty()
                            && b.module == c.module_segments
                            && canonical(c, &b.leaf.binding) == target
                    })
            })
            .collect()
    }
    // Consumer-side routing only: never discharge a moved declaration's API need.
    fn follow_reexport(
        &self,
        need: &mut Need,
        target: &str,
    ) -> Option<(String, Vec<SourceAnchor>)> {
        const MAX_REEXPORT_HOPS: usize = 8;
        let mut target = target.to_owned();
        let mut seen = std::collections::BTreeSet::new();
        let mut anchors = Vec::new();
        loop {
            if items::check(self.controls.0, self.controls.1).is_err() {
                return None;
            }
            let bindings = self.reexports(&target);
            if let Some((path, item)) = self.declaration(&target) {
                if !bindings.is_empty() || !self.contexts[&path].unresolved.is_empty() {
                    self.missing_target(need, &target);
                    return None;
                }
                if !anchors.is_empty() {
                    anchors.push(anchor(self.files, &path, &item.span.range));
                }
                return Some((target, anchors));
            }
            if !target.starts_with("crate::") && anchors.is_empty() {
                return Some((target, anchors));
            }
            if !seen.insert(target.clone()) {
                self.missing_target(need, &target);
                need.message =
                    "written re-export chain is cyclic; no terminal declaration evidence".into();
                return None;
            }
            if anchors.len() == MAX_REEXPORT_HOPS {
                self.missing_target(need, &target);
                need.message = "written re-export chain exceeds the eight-hop evidence cap".into();
                return None;
            }
            if bindings.len() != 1 {
                self.missing_target(need, &target);
                return None;
            }
            let binding = bindings[0];
            let item = self.parsed[&binding.path]
                .items
                .iter()
                .find(|i| i.span.range == binding.leaf.declaration)?;
            if binding.conditioned || !matches!(item.visibility_key, "pub" | "pub(crate)") {
                self.missing_target(need, &target);
                return None;
            }
            let Some(next) = self.imported_target(binding) else {
                self.missing_target(need, &target);
                return None;
            };
            // Each hop must also be written-accessible to its exporting module.
            // The final consumer need not see the hidden implementation edges.
            let exported = self.declaration(&next).or_else(|| {
                let leaves = self.reexports(&next);
                (leaves.len() == 1)
                    .then(|| {
                        let leaf = leaves[0];
                        self.parsed[&leaf.path]
                            .items
                            .iter()
                            .find(|i| i.span.range == leaf.leaf.declaration)
                            .map(|i| (leaf.path.clone(), i.clone()))
                    })
                    .flatten()
            });
            if let Some((path, exported)) = exported
                && !self.accessible_route(need, &path, &exported, &binding.module)
            {
                return None;
            }
            anchors.push(anchor(self.files, &binding.path, &binding.leaf.declaration));
            target = next;
        }
    }
    fn written_target(
        &self,
        need: &mut Need,
        target: &str,
        using: &[String],
    ) -> Option<WrittenTarget> {
        let (terminal, mut anchors) = self.follow_reexport(need, target)?;
        let mut route = self.mapped(&terminal);
        let mut fallback = Vec::new();
        if let Some((path, item)) = self.declaration(&terminal) {
            let mut rejected = need.clone();
            // Direct relocation keeps its explicit visibility repairs, including
            // synthesized edges; retained terminals must not widen hidden modules.
            if !self.accessible_route(&mut rejected, &path, &item, using)
                && (!anchors.is_empty() || self.final_path(&path, &item) == path)
            {
                // A facade proves access, not permission to expose a private terminal.
                if !matches!(item.visibility_key, "pub" | "pub(crate)")
                    || self.final_path(&path, &item) != path
                {
                    *need = rejected;
                    return None;
                }
                let mut candidates = Vec::new();
                for binding in &self.imports {
                    items::check(self.controls.0, self.controls.1).ok()?;
                    if binding.scope_range.is_some() || binding.conditioned || !binding.leaf.public
                    {
                        continue;
                    }
                    let Some(context) = self
                        .contexts
                        .get(&binding.path)
                        .filter(|c| c.unresolved.is_empty() && c.module_segments == binding.module)
                    else {
                        continue;
                    };
                    let candidate = canonical(context, &binding.leaf.binding);
                    let mut probe = need.clone();
                    let Some((end, mut hops)) = self.follow_reexport(&mut probe, &candidate) else {
                        continue;
                    };
                    if end != terminal || hops.is_empty() {
                        continue;
                    }
                    let Some(export) = self.parsed[&binding.path]
                        .items
                        .iter()
                        .find(|i| i.span.range == binding.leaf.declaration)
                    else {
                        continue;
                    };
                    if self.accessible_route(&mut probe, &binding.path, export, using) {
                        let hop_count = hops.len() - 1;
                        for a in &context.declaration_anchors {
                            if !hops.contains(a) {
                                hops.push(a.clone());
                            }
                        }
                        candidates.push((hop_count, candidate, hops));
                    }
                }
                candidates.sort_by(|a, b| (a.0, &a.1).cmp(&(b.0, &b.1)));
                candidates.dedup_by(|a, b| a.1 == b.1);
                let Some((hops, chosen, chosen_anchors)) = candidates.first() else {
                    *need = rejected;
                    return None;
                };
                let rejection = rejected.refusal_basis.last()?;
                if let Some(range) = &rejection.anchor.range {
                    anchors.push(anchor(self.files, &rejection.anchor.path, range));
                }
                for a in chosen_anchors {
                    if !anchors.contains(a) {
                        anchors.push(a.clone());
                    }
                }
                fallback.push(format!(
                    "written_reexport_route_fallback: canonical {route} rejected ({class}); choose {chosen} from {count} accessible written routes by shortest re-export hops, then lexicographic absolute path ({hops} hops); no canonical visibility widening",
                    class = rejection.class, count = candidates.len()
                ));
                route = chosen.clone();
            }
        }
        Some(WrittenTarget {
            terminal,
            route,
            evidence: RouteEvidence { anchors, fallback },
        })
    }
    // Check the chosen path's written edges; a facade does not authorize widening
    // the hidden canonical modules it intentionally avoids.
    fn accessible_route(&self, need: &mut Need, path: &str, item: &Item, using: &[String]) -> bool {
        let final_path = self.final_path(path, item);
        let Some(context) = self.final_contexts.get(final_path) else {
            return self.inaccessible_route(need, item.name.as_deref().unwrap_or("crate"), None);
        };
        for (index, segment) in context.module_segments.iter().enumerate() {
            if items::check(self.controls.0, self.controls.1).is_err() {
                return false;
            }
            let declaration = context.declaration_anchors.get(index);
            let parent = &context.module_segments[..index];
            let visible = declaration.and_then(|a| {
                let data = self.parsed.get(&a.path)?;
                let module = data.items.iter().find(|i| i.span.range == a.range)?;
                if !context.unresolved.is_empty()
                    || module.kind != "mod_item"
                    || module.name.as_deref() != Some(segment)
                    || !module.attributes.is_empty()
                    || module.syntax.file_has_recovery
                    || !self
                        .contexts
                        .get(&a.path)
                        .is_some_and(|c| c.unresolved.is_empty() && c.module_segments == parent)
                {
                    return None;
                }
                self.module_visible(&a.path, module, parent, using)
            });
            if visible != Some(true) {
                return self.inaccessible_route(need, segment, declaration);
            }
        }
        if !context.unresolved.is_empty() {
            return self.inaccessible_route(need, "crate", None);
        }
        true
    }
    fn module_visible(
        &self,
        path: &str,
        module: &Item,
        parent: &[String],
        using: &[String],
    ) -> Option<bool> {
        match module.visibility_key {
            "pub" | "pub(crate)" => return Some(true),
            "private" => return Some(using.starts_with(parent)),
            "restricted" => {}
            _ => return None,
        }
        let node = self.parsed[path]
            .tree
            .root_node()
            .named_descendant_for_byte_range(
                module.span.range.start_byte,
                module.span.range.end_byte,
            )?;
        let modifier = (0..node.named_child_count())
            .filter_map(|i| node.named_child(i as u32))
            .find(|n| n.kind() == "visibility_modifier")?;
        restricted_region(modifier, &self.files[path].source, parent, self.controls)
            .map(|scope| using.starts_with(&scope))
    }
    fn inaccessible_route(
        &self,
        need: &mut Need,
        segment: &str,
        declaration: Option<&SourceAnchor>,
    ) -> bool {
        need.reason = DecisionReason::VisibilityScopeUnproved;
        need.category = "visibility_context";
        need.message = format!(
            "written re-export route segment {segment} is inaccessible or its visibility from the destination is unproved; no accessible admitted route rewrite or module widening is offered"
        );
        let class = format!("inaccessible_route:{segment}");
        let basis = if let Some(a) = declaration {
            items::RefusalBasis::new(&class, &a.path, Some(a.range.clone()))
        } else {
            // Missing/synthesized edges have no written declaration coordinates.
            items::RefusalBasis::new(&class, &need.path, None)
        };
        need.refusal_basis.push(basis.named(segment));
        false
    }
    fn missing_target(&self, need: &mut Need, target: &str) -> bool {
        need.reason = DecisionReason::ExternalOrMissingBinding;
        need.message = "needed target has no unique directly evidenced declaration or admitted written re-export chain".into();
        if let Some(binding) = self.reexports(target).first() {
            need.reason = if binding.conditioned {
                DecisionReason::ConditionalOrInheritedContext
            } else {
                DecisionReason::ExternalOrMissingBinding
            };
            need.category = if binding.conditioned {
                "scope_dependency"
            } else {
                "unsupported_dependency_form"
            };
            need.path = binding.path.clone();
            need.range = binding.leaf.declaration.clone();
        }
        false
    }
    fn binding(&self, path: &str, name: &str) -> Option<Item> {
        let matching: Vec<_> = self
            .parsed
            .get(path)?
            .items
            .iter()
            .filter(|i| i.name.as_deref() == Some(name))
            .collect();
        (matching.len() == 1).then(|| matching[0].clone())
    }
    fn visibility(
        &mut self,
        path: &str,
        item: &Item,
        consumer: &[String],
        ids: &[String],
        access: &VisibilityConsumer,
    ) -> bool {
        let final_path = self.final_path(path, item).to_owned();
        let Some(defining) = self.final_contexts.get(&final_path).cloned() else {
            return false;
        };
        let using = consumer;
        // A private-child extraction cannot become a wider module-building operation.
        if self.request.moves.iter().any(|m| {
            matches!(&m.destination,
            crate::move_plan::Destination::NewChild { path, parent_path }
                if path == &final_path && self.final_contexts.get(parent_path)
                    .is_none_or(|parent| !using.starts_with(&parent.module_segments)))
        }) {
            return false;
        }
        // Each edge is declared in its parent scope, not inside the child it introduces.
        // Synthesized edges have no fictional source anchor and remain explicit evidence.
        for (index, declaration) in defining.declaration_anchors.iter().enumerate() {
            let Some(module) = self.parsed[&declaration.path]
                .items
                .iter()
                .find(|i| i.span.range == declaration.range)
                .cloned()
            else {
                return false;
            };
            let parent = &defining.module_segments[..index];
            let created = self
                .request
                .moves
                .iter()
                .find_map(|m| match &m.destination {
                    crate::move_plan::Destination::NewSibling { path, parent_path }
                    | crate::move_plan::Destination::NewChild { path, parent_path }
                        if parent_path == &declaration.path
                            && self.final_contexts.get(path).is_some_and(|c| {
                                c.declaration_anchors.last() == Some(declaration)
                            }) =>
                    {
                        Some(path.clone())
                    }
                    _ => None,
                });
            if !self.widen(
                &declaration.path,
                Some(&module),
                parent,
                using,
                ids,
                created,
                Some(access),
            ) {
                return false;
            }
        }
        if let Some(crate::move_plan::Destination::NewSibling { parent_path, .. }) = self
            .request
            .moves
            .iter()
            .find(|m| m.destination.path() == final_path)
            .map(|m| &m.destination)
            && defining.declaration_anchors.len() < defining.module_segments.len()
            && !self.widen(
                parent_path,
                None,
                &defining.module_segments[..defining.module_segments.len() - 1],
                using,
                ids,
                Some(final_path.clone()),
                Some(access),
            )
        {
            return false;
        }
        self.widen(
            path,
            Some(item),
            &defining.module_segments,
            using,
            ids,
            None,
            Some(access),
        )
    }
    fn visibility_need(
        &mut self,
        need: &mut Need,
        path: &str,
        item: &Item,
        using: &[String],
        facade_access: bool,
        access: VisibilityConsumer,
    ) -> bool {
        // Attribute-only semantic evidence cannot substitute for binding access.
        // Keep all required visibility/module repairs even if context is discharged later.
        if !facade_access
            && self.request.resolve_semantic
            && !self.visibility(path, item, using, &need.item_ids, &access)
        {
            need.reason = DecisionReason::VisibilityScopeUnproved;
            need.category = "visibility_context";
            need.message =
                "final access includes an uncertain or insufficient restricted declaration scope"
                    .into();
            return false;
        }
        for attribute in &item.attributes {
            let text =
                &self.files[path].source[attribute.range.start_byte..attribute.range.end_byte];
            // Inner attributes on a required binding remain an inherited-scope
            // boundary. Sharing the outer allowlist must not relax that guard.
            if (text.starts_with("#![") || !items::context_independent_attribute(text))
                && items::cfg::attribute(text, self.cfg.as_ref(), true).is_err()
            {
                need.reason = DecisionReason::ConditionalOrInheritedContext;
                need.category = "scope_dependency";
                need.path = path.into();
                need.range = attribute.range.clone();
                need.message =
                    "required written binding has conditional/unexamined attribute context".into();
                let derive = self.parsed[path]
                    .tree
                    .root_node()
                    .named_descendant_for_byte_range(
                        attribute.range.start_byte,
                        attribute.range.end_byte,
                    )
                    .and_then(|node| items::derive_names(node, &self.files[path].source))
                    .is_some();
                need.refusal_basis.push(items::RefusalBasis::new(
                    if derive {
                        "derive_veto"
                    } else {
                        "conditional_context"
                    },
                    path,
                    Some(attribute.range.clone()),
                ));
                return false;
            }
        }
        if facade_access
            || self.request.resolve_semantic
            || self.visibility(path, item, using, &need.item_ids, &access)
        {
            return true;
        }
        need.reason = DecisionReason::VisibilityScopeUnproved;
        need.category = "visibility_context";
        need.message =
            "final access includes an uncertain or insufficient restricted declaration scope"
                .into();
        false
    }
    #[allow(clippy::too_many_arguments)] // Access evidence accompanies the unchanged visibility calculation.
    fn widen(
        &mut self,
        path: &str,
        item: Option<&Item>,
        defining: &[String],
        using: &[String],
        ids: &[String],
        declaration_for: Option<String>,
        access: Option<&VisibilityConsumer>,
    ) -> bool {
        let at = item
            .map(|i| i.span.range.start_byte)
            .unwrap_or(self.files[path].source.len());
        let subject = (path.to_owned(), at, declaration_for.clone());
        let observations = self.visibility_observations.entry(subject).or_default();
        if let Some(access) = access {
            if !observations.consumers.contains(access) {
                self.descriptor_bytes += serde_json::to_vec(access)
                    .expect("visibility access JSON")
                    .len()
                    + path.len()
                    + 128;
                observations.consumers.push(access.clone());
            }
        } else if !observations
            .preserved_original_access_regions
            .iter()
            .any(|r| r == using)
        {
            self.descriptor_bytes +=
                using.iter().map(|s| s.len() + 32).sum::<usize>() + path.len() + 128;
            observations
                .preserved_original_access_regions
                .push(using.to_vec());
        }
        self.exceeded |= self.descriptor_bytes > 128 * 1024 * 1024;
        let explanation = |preserved: &[String]| VisibilityExplanation {
            consumers: Vec::new(),
            original_defining_region: item
                .and_then(|_| self.contexts.get(path).map(|c| c.module_segments.clone())),
            final_defining_region: defining.to_vec(),
            preserved_access_region: preserved.to_vec(),
            preserved_original_access_regions: Vec::new(),
            narrowest_covering_region: Vec::new(),
            basis: String::new(),
        };
        let visibility = item.map(|i| i.visibility_key).unwrap_or("private");
        if using.starts_with(defining) || matches!(visibility, "pub" | "pub(crate)") {
            return true;
        }
        if visibility == "restricted" {
            let Some(item) = item else {
                return false;
            };
            let node = self.parsed[path]
                .tree
                .root_node()
                .named_descendant_for_byte_range(
                    item.span.range.start_byte,
                    item.span.range.end_byte,
                )
                .expect("item");
            let modifier = (0..node.named_child_count())
                .filter_map(|i| node.named_child(i as u32))
                .find(|n| n.kind() == "visibility_modifier")
                .expect("restriction");
            // Relative restrictions are only evidence when their declaring scope stays put.
            let scope = if self.final_path(path, item) == path {
                restricted_region(modifier, &self.files[path].source, defining, self.controls)
            } else {
                items::absolute_visibility(node, &self.files[path].source, self.controls)
                    .filter(|scope| defining.starts_with(scope))
            };
            let Some(scope) = scope else {
                return false;
            };
            if using.starts_with(&scope) {
                return true;
            }
            let required = common_region(&scope, using);
            let bytes = &self.files[path].source[modifier.byte_range()];
            if bytes.contains("//") || bytes.contains("/*") {
                return false;
            }
            let range = span(modifier.start_byte(), modifier.end_byte());
            let a = anchor(self.files, path, &range);
            self.add_visibility(Repair {
                path: path.into(),
                range,
                after: String::new(),
                kind: "visibility",
                written_reexport: false,
                target: RewriteTarget::Source { anchor: a.clone() },
                item_ids: ids.to_vec(),
                anchors: vec![a],
                rationale: "known ancestor restriction is insufficient for a proven final access; preserve its existing region and cover all proven callers at their deepest common ancestor"
                    .into(),
                visibility: Some(explanation(&scope)),
                declaration_for,
                references: Vec::new(),
                caller_override: false,
                import_module: None,
                import_scope: None,
            }, defining, &required);
            return true;
        }
        if visibility != "private" {
            return false;
        }
        let at = item
            .map(|i| i.span.range.start_byte)
            .unwrap_or(self.files[path].source.len());
        let mut parts = vec!["crate".to_owned()];
        parts.extend(defining.iter().cloned());
        parts.push(item.and_then(|i| i.name.clone()).unwrap_or_else(|| {
            items::module_name(declaration_for.as_deref().expect("virtual declaration"))
                .expect("validated name")
                .into()
        }));
        self.add_visibility(Repair {
            path: path.into(), range: span(at, at), after: String::new(), kind: "visibility", written_reexport: false,
            target: RewriteTarget::Synthesis { path: path.into(), slot: "visibility_insert".into(), items: self.contributors(ids), boundary_role: None, parent_path: None, binding: Some(parts.join("::")) },
            item_ids: ids.to_vec(), anchors: item.map(|i| vec![anchor(self.files, path, &i.span.range)]).unwrap_or_default(),
            rationale: "proven access is outside the declaration's parent scope; cover all proven callers at their deepest common ancestor".into(),
            visibility: Some(explanation(defining)),
            declaration_for,
            references: Vec::new(), caller_override: false, import_module: None, import_scope: None,
        }, defining, &common_region(defining, using));
        true
    }
    fn import(
        &mut self,
        path: &str,
        binding: &str,
        target: &str,
        ids: &[String],
        evidence: (SourceAnchor, RouteEvidence),
        references: Vec<SourceAnchor>,
    ) -> bool {
        if let Some(data) = self.parsed.get(path)
            && data
                .items
                .iter()
                .any(|i| i.name.as_deref() == Some(binding) && self.final_path(path, i) == path)
        {
            return false;
        }
        if self
            .selected
            .iter()
            .any(|(_, i, d)| d == path && i.name.as_deref() == Some(binding))
        {
            return false;
        }
        let module = &self.final_contexts[path].module_segments;
        let prefix = target.split("::").next().unwrap_or("");
        if !matches!(prefix, "crate" | "self" | "super")
            && (self.binding(path, prefix).is_some()
                || !self.imported(path, module, prefix).is_empty()
                || self
                    .selected
                    .iter()
                    .any(|(_, i, d)| d == path && i.name.as_deref() == Some(prefix))
                || self.repairs.iter().any(|r| {
                    r.kind == "import_insert"
                        && r.path == path
                        && parsed_import(&r.after, self.controls)
                            .is_ok_and(|(_, binding)| binding == prefix)
                }))
        {
            return false;
        }
        let existing: Vec<_> = self
            .imported(path, module, binding)
            .into_iter()
            .filter(|b| {
                !self.repairs.iter().any(|r| {
                    r.path == path
                        && r.kind == "import_leaf_extract"
                        && r.range.start_byte <= b.leaf.leaf_range.start_byte
                        && r.range.end_byte >= b.leaf.leaf_range.end_byte
                })
            })
            .collect();
        if !existing.is_empty() {
            return existing.len() == 1
                && !existing[0].conditioned
                && !existing[0].leaf.public
                && self
                    .imported_target(&existing[0])
                    .is_some_and(|old| self.mapped(&old) == target);
        }
        if self.repairs.iter().any(|r| r.kind == "import_insert" && r.path == path && matches!(&r.target, RewriteTarget::Synthesis { binding:Some(name), .. } if name == binding) && r.after != import_text(target, binding)) { return false; }
        let source = self
            .files
            .get(path)
            .map(|f| f.source.as_str())
            .unwrap_or("");
        let (at, boundary_item) = self
            .parsed
            .get(path)
            .map(|data| crate::move_plan::ergonomics::import_boundary(data, source))
            .unwrap_or((source.len(), None));
        let written_reexport = !evidence.1.anchors.is_empty();
        let mut rationale = format!(
            "preserve the unique written binding {binding} through explicit {target}; insert after the last attached use, or before the first attached item"
        );
        evidence.1.disclose(&mut rationale);
        let mut anchors: Vec<_> = std::iter::once(evidence.0)
            .chain(evidence.1.anchors)
            .collect();
        if let Some(item) = boundary_item {
            anchors.push(anchor(self.files, path, &item.span.range));
        }
        if self.files.contains_key(path) {
            anchors.push(anchor(self.files, path, &span(at, at)));
        }
        let eol = if source.contains("\r\n") {
            "\r\n"
        } else {
            "\n"
        };
        let text = import_text(target, binding);
        self.add(Repair {
            path: path.into(),
            range: span(at, at),
            after: text,
            kind: "import_insert",
            written_reexport,
            target: RewriteTarget::Synthesis {
                path: path.into(),
                slot: "import".into(),
                items: self.contributors(ids),
                boundary_role: None,
                parent_path: None,
                binding: Some(binding.into()),
            },
            item_ids: ids.to_vec(),
            anchors,
            rationale: format!("{rationale}; boundary ending {eol:?} is separately audited"),
            visibility: None,
            declaration_for: None,
            references,
            caller_override: false,
            import_module: None,
            import_scope: None,
        });
        true
    }
    fn resolve_in(
        &self,
        path: &str,
        module: &[String],
        text: &str,
        aliases: bool,
    ) -> Option<String> {
        if !simple_path(text) {
            return None;
        }
        let mut parts: Vec<_> = text.split("::").map(str::to_owned).collect();
        let first = parts.first()?.clone();
        let prefix = match first.as_str() {
            "crate" => {
                parts.remove(0);
                Vec::new()
            }
            "self" => {
                parts.remove(0);
                module.to_vec()
            }
            "super" => {
                let mut prefix = module.to_vec();
                while parts.first().is_some_and(|p| p == "super") {
                    parts.remove(0);
                    prefix.pop()?;
                }
                prefix
            }
            _ => {
                let imports = self.imported(path, module, &first);
                if aliases
                    && imports.len() == 1
                    && !imports[0].conditioned
                    && !imports[0].leaf.public
                {
                    let base = self.resolve_in(path, module, &imports[0].leaf.path, false)?;
                    if base.starts_with("crate::")
                        && !self
                            .contexts
                            .values()
                            .any(|c| canonical(c, "").trim_end_matches("::") == base)
                    {
                        return None;
                    }
                    parts.remove(0);
                    return Some(if parts.is_empty() {
                        base
                    } else {
                        format!("{base}::{}", parts.join("::"))
                    });
                }
                let mut local = vec!["crate".to_owned()];
                local.extend(module.iter().cloned());
                local.extend(parts.clone());
                let candidate = local.join("::");
                if self.declaration(&candidate).is_some() || !self.reexports(&candidate).is_empty()
                {
                    return Some(candidate);
                }
                // External imports preserve written spelling; never follow another use binding.
                if !aliases
                    && imports.is_empty()
                    && self.binding(path, &first).is_none()
                    && !matches!(first.as_str(), "Self")
                {
                    return Some(text.into());
                }
                return None;
            }
        };
        let mut result = vec!["crate".into()];
        result.extend(prefix);
        result.extend(parts);
        Some(result.join("::"))
    }
    fn mapped(&self, target: &str) -> String {
        self.declaration(target)
            .and_then(|(p, i)| {
                self.final_contexts
                    .get(self.final_path(&p, &i))
                    .map(|c| canonical(c, i.name.as_deref().expect("named")))
            })
            .unwrap_or_else(|| target.into())
    }
    fn source_repair(&mut self, need: &Need, range: ByteRange, after: String, kind: &'static str) {
        let a = anchor(self.files, &need.path, &range);
        if a.expected_text == after {
            return;
        }
        self.add(Repair {
            path: need.path.clone(),
            range,
            after,
            kind,
            written_reexport: false,
            target: RewriteTarget::Source { anchor: a.clone() },
            item_ids: need.item_ids.clone(),
            anchors: vec![a],
            rationale: "complete written binding in a verified ordinary lexical context".into(),
            visibility: None,
            declaration_for: None,
            references: Vec::new(),
            caller_override: false,
            import_module: None,
            import_scope: None,
        });
    }
    fn path_evidence(&mut self, need: &Need, evidence: &RouteEvidence) {
        if evidence.anchors.is_empty() {
            return;
        }
        if let Some(repair) = self.repairs.iter_mut().find(|r| {
            r.kind == "path"
                && r.path == need.path
                && r.range.start_byte <= need.range.start_byte
                && r.range.end_byte >= need.range.end_byte
        }) {
            repair.written_reexport = true;
            self.descriptor_bytes += evidence.disclose(&mut repair.rationale);
            for a in &evidence.anchors {
                if !repair.anchors.contains(a) {
                    self.descriptor_bytes += a.path.len() + a.expected_text.len() + 128;
                    if self.descriptor_bytes > 128 * 1024 * 1024 {
                        self.exceeded = true;
                        return;
                    }
                    repair.anchors.push(a.clone());
                }
            }
            self.exceeded |= self.descriptor_bytes > 128 * 1024 * 1024;
        }
    }
    fn import_references(
        &mut self,
        path: &str,
        module: &[String],
        binding: &str,
        owner: Node<'_>,
    ) -> Option<Vec<SourceAnchor>> {
        let mut references = Vec::new();
        let mut reference_bytes = 0;
        let source = &self.files[path].source;
        let mut stack = vec![self.parsed[path].tree.root_node()];
        while let Some(node) = stack.pop() {
            if items::check(self.controls.0, self.controls.1).is_err() {
                return None;
            }
            if matches!(
                node.kind(),
                "use_declaration"
                    | "macro_invocation"
                    | "macro_definition"
                    | "line_comment"
                    | "block_comment"
                    | "attribute_item"
                    | "inner_attribute_item"
                    | "string_literal"
                    | "raw_string_literal"
                    | "char_literal"
            ) {
                continue;
            }
            if matches!(node.kind(), "identifier" | "type_identifier")
                && source[node.byte_range()] == *binding
                && items::reference_role(node)
                && self.consumer(path, node) == self.consumer(path, owner)
                && (owner.parent().is_some_and(|p| p.kind() == "source_file")
                    || owner.parent().is_some_and(|p| {
                        p.start_byte() <= node.start_byte() && p.end_byte() >= node.end_byte()
                    }))
                && self.lexical_module(path, node, false).as_deref() == Some(module)
            {
                *self.reference_candidates += 1;
                reference_bytes += path.len() + node.end_byte() - node.start_byte() + 128;
                if *self.reference_candidates > 100_000
                    || self.descriptor_bytes + reference_bytes > 128 * 1024 * 1024
                {
                    self.exceeded = true;
                    return None;
                }
                if node.parent().is_some_and(|p| {
                    matches!(p.kind(), "scoped_identifier" | "scoped_type_identifier")
                }) {
                    return None;
                }
                match self.lexical_binding(path, node, source, binding, true) {
                    items::LexicalBinding::Independent => {}
                    items::LexicalBinding::Uncertain => return None,
                    items::LexicalBinding::Absent => references.push(anchor(
                        self.files,
                        path,
                        &span(node.start_byte(), node.end_byte()),
                    )),
                }
            }
            if references.len() > 100_000 {
                return None;
            }
            for i in (0..node.named_child_count()).rev() {
                stack.push(node.named_child(i as u32).expect("child"));
            }
        }
        Some(references)
    }
    fn resolve_use(
        &self,
        path: &str,
        module: &[String],
        node: Node<'_>,
        text: &str,
    ) -> Option<String> {
        if !simple_path(text) {
            return None;
        }
        let first = text.split("::").next()?;
        if matches!(first, "crate" | "self" | "super") {
            return self.resolve_in(path, module, text, false);
        }
        let mut aliases = self.scoped_imports(path, module, node, first);
        if aliases.is_empty() {
            aliases = self.imported(path, module, first);
        }
        if aliases.len() == 1
            && aliases[0].leaf.public
            && !aliases[0].conditioned
            && text == first
            && self.lexical_binding(path, node, &self.files[path].source, first, true)
                == items::LexicalBinding::Absent
        {
            return self.resolve_in(path, module, text, false);
        }
        if !aliases.is_empty() {
            if aliases.len() != 1
                || aliases[0].conditioned
                || aliases[0].leaf.public
                || self.lexical_binding(path, node, &self.files[path].source, first, true)
                    != items::LexicalBinding::Absent
            {
                return None;
            }
            let base = self.resolve_in(path, module, &aliases[0].leaf.path, false)?;
            if !self
                .contexts
                .values()
                .any(|c| canonical(c, "").trim_end_matches("::") == base)
            {
                return None;
            }
            return Some(format!("{base}{}", text.strip_prefix(first)?));
        }
        if self.lexical_binding(path, node, &self.files[path].source, first, false)
            != items::LexicalBinding::Absent
        {
            return None;
        }
        self.resolve_in(path, module, text, false)
    }
    fn use_repair(&mut self, need: &mut Need, node: Node<'_>) -> bool {
        let source = &self.files[&need.path].source;
        if items::cfg::attributed(node, source, self.cfg.as_ref()) {
            need.reason = DecisionReason::ConditionalOrInheritedContext;
            need.category = "scope_dependency";
            need.message = "affected import has unexamined attribute/conditional context".into();
            return false;
        }
        let Some(module) = self.lexical_module(&need.path, node, false) else {
            need.reason = DecisionReason::ConditionalOrInheritedContext;
            need.category = "module_context";
            need.message = "affected import has uncertain inline/conditional module context".into();
            return false;
        };
        let Some(using) = self.lexical_module(&need.path, node, true) else {
            return false;
        };
        let Ok(leaves) = items::use_leaves(node, source, self.controls) else {
            return false;
        };
        if leaves.is_empty() {
            return false;
        }
        let repair_start = self.repairs.len();
        let mut mappings = Vec::new();
        let mut declarations = Vec::new();
        let mut reexports = RouteEvidence::default();
        for leaf in &leaves {
            let Some(old) = self.resolve_use(&need.path, &module, node, &leaf.path) else {
                let first = leaf.path.split("::").next().unwrap_or("");
                if !matches!(first, "crate" | "self" | "super")
                    && let Ok(assessment) = items::lexical_assessment_with_globs(
                        &need.path,
                        node,
                        source,
                        first,
                        self.controls,
                        true,
                        self.cfg.as_ref(),
                        Some(&self.pattern_routes),
                    )
                    && assessment.binding == items::LexicalBinding::Uncertain
                {
                    need.reason = DecisionReason::LexicalContextUnproved;
                    need.category = "binding_collision";
                    need.lexical(assessment);
                    need.message =
                        "import prefix has an unproved containing lexical context".into();
                }
                return false;
            };
            // Public exposures of moved items remain separate API decisions.
            let resolved = if leaf.public {
                WrittenTarget {
                    terminal: old.clone(),
                    route: self.mapped(&old),
                    evidence: RouteEvidence::default(),
                }
            } else {
                let Some(resolved) = self.written_target(need, &old, &using) else {
                    return false;
                };
                resolved
            };
            declarations.push((
                self.declaration(&resolved.terminal),
                !resolved.evidence.fallback.is_empty(),
            ));
            for a in &resolved.evidence.anchors {
                self.descriptor_bytes += a.path.len() + a.expected_text.len() + 128;
            }
            if self.descriptor_bytes > 128 * 1024 * 1024 {
                self.exceeded = true;
                return false;
            }
            reexports.merge(resolved.evidence);
            let new = resolved.route;
            if old != new {
                let parent = node.parent().expect("use scope");
                let scope = (parent.kind() == "block")
                    .then(|| span(parent.start_byte(), parent.end_byte()));
                if self
                    .imports
                    .iter()
                    .filter(|b| {
                        b.path == need.path
                            && b.module == module
                            && b.scope_range == scope
                            && b.leaf.binding == leaf.binding
                    })
                    .count()
                    != 1
                    || (0..parent.named_child_count())
                        .filter_map(|i| parent.named_child(i as u32))
                        .any(|n| {
                            n.child_by_field_name("name")
                                .is_some_and(|n| source[n.byte_range()] == leaf.binding)
                        })
                {
                    need.reason = DecisionReason::LexicalContextUnproved;
                    need.category = "binding_collision";
                    need.message =
                        "affected import alias has competing written bindings in its lexical scope"
                            .into();
                    return false;
                }
                let mut stack = vec![self.parsed[&need.path].tree.root_node()];
                while let Some(candidate) = stack.pop() {
                    if items::check(self.controls.0, self.controls.1).is_err() {
                        return false;
                    }
                    if matches!(candidate.kind(), "macro_invocation" | "macro_definition") {
                        if self.lexical_module(&need.path, candidate, false).as_deref()
                            == Some(module.as_slice())
                            && self.lexical_binding(
                                &need.path,
                                candidate,
                                source,
                                &leaf.binding,
                                false,
                            ) != items::LexicalBinding::Independent
                            && items::token_candidate(
                                candidate,
                                source,
                                &leaf.binding,
                                self.controls.0,
                                self.controls.1,
                            )
                            .unwrap_or(true)
                        {
                            need.reason = DecisionReason::MacroContextUnexamined;
                            need.category = "macro_dependency";
                            need.range = span(candidate.start_byte(), candidate.end_byte());
                            need.message =
                                "affected imported alias occurs in unexamined macro tokens".into();
                            return false;
                        }
                        continue;
                    }
                    for i in (0..candidate.named_child_count()).rev() {
                        stack.push(candidate.named_child(i as u32).expect("child"));
                    }
                }
            }
            mappings.push((old, new));
        }
        // One common changed prefix can be replaced without touching any delimiter or alias.
        if leaves
            .iter()
            .all(|l| l.prefix_range == leaves[0].prefix_range && !l.prefix.is_empty())
            && mappings.iter().all(|(a, b)| a != b || module != using)
            && let Some(prefix_range) = &leaves[0].prefix_range
        {
            let new_prefix = mappings[0]
                .1
                .rsplit_once("::")
                .map(|(p, _)| p)
                .unwrap_or("");
            if mappings.iter().zip(&leaves).all(|((_, new), leaf)| {
                new.rsplit_once("::").is_some_and(|(p, suffix)| {
                    p == new_prefix
                        && suffix == leaf.path.rsplit("::").next().unwrap_or("")
                        && source[leaf.path_range.start_byte..leaf.path_range.end_byte] == *suffix
                })
            }) {
                let written_prefix = &source[prefix_range.start_byte..prefix_range.end_byte];
                let inherited = leaves[0]
                    .prefix
                    .strip_suffix(written_prefix)
                    .unwrap_or("")
                    .trim_end_matches("::");
                let replacement = if inherited.is_empty() {
                    Some(new_prefix.to_owned())
                } else {
                    self.resolve_in(&need.path, &module, inherited, false)
                        .and_then(|prefix| {
                            new_prefix
                                .strip_prefix(&format!("{prefix}::"))
                                .map(str::to_owned)
                        })
                };
                if let Some(replacement) = replacement {
                    self.source_repair(need, prefix_range.clone(), replacement, "use_path");
                    for (leaf, (declaration, facade)) in leaves.iter().zip(&declarations) {
                        if let Some((p, i)) = declaration
                            && !self.visibility_need(
                                need,
                                p,
                                i,
                                &using,
                                *facade,
                                access(
                                    self.files,
                                    &need.path,
                                    &leaf.path_range,
                                    "written_import",
                                    &module,
                                    &using,
                                ),
                            )
                        {
                            return false;
                        }
                    }
                    self.use_evidence(repair_start, &reexports);
                    return true;
                }
            }
        }
        for ((leaf, (old, new)), (declaration, facade)) in
            leaves.iter().zip(mappings).zip(declarations)
        {
            if old == new && module == using {
                if let Some((p, i)) = declaration
                    && !self.visibility_need(
                        need,
                        &p,
                        &i,
                        &using,
                        facade,
                        access(
                            self.files,
                            &need.path,
                            &leaf.path_range,
                            "written_import",
                            &module,
                            &using,
                        ),
                    )
                {
                    return false;
                }
                continue;
            }
            if leaf.public {
                return false;
            }
            if leaf.prefix.is_empty() {
                self.source_repair(need, leaf.path_range.clone(), new.clone(), "use_path");
            } else {
                let Some(prefix) =
                    self.final_resolve(&self.consumer(&need.path, node), &using, &leaf.prefix)
                else {
                    return false;
                };
                if let Some(relative) = new.strip_prefix(&format!("{prefix}::")) {
                    self.source_repair(need, leaf.path_range.clone(), relative.into(), "use_path");
                } else {
                    let Some(list) = &leaf.list_range else {
                        return false;
                    };
                    let mut removal = leaf.leaf_range.clone();
                    let following = &source[removal.end_byte..list.end_byte - 1];
                    let preceding = &source[list.start_byte + 1..removal.start_byte];
                    if let Some(at) = following.find(',')
                        && following[..at].trim().is_empty()
                    {
                        removal.end_byte += at + 1;
                    } else if preceding.trim().is_empty() && following.trim().is_empty() {
                        // A singleton named list has no delimiter to consume. Leave
                        // its empty braces byte-exact and extract only the leaf.
                    } else {
                        let Some(at) = preceding.rfind(',') else {
                            return false;
                        };
                        if !preceding[at + 1..].trim().is_empty() {
                            return false;
                        }
                        let comma = list.start_byte + 1 + at;
                        if !self.repairs.iter().any(|r| {
                            r.path == need.path
                                && r.kind == "import_leaf_extract"
                                && r.range.start_byte <= comma
                                && r.range.end_byte > comma
                        }) {
                            removal.start_byte = comma;
                        }
                    }
                    let bytes = &source[removal.start_byte..removal.end_byte];
                    if bytes.contains("//") || bytes.contains("/*") {
                        need.reason = DecisionReason::TriviaPreservationUnproved;
                        need.category = "trivia_ownership";
                        need.message =
                            "leaf/delimiter trivia cannot be consumed during extraction".into();
                        return false;
                    }
                    self.source_repair(need, removal, String::new(), "import_leaf_extract");
                    let consumer = self.consumer(&need.path, node);
                    let references = self
                        .import_references(&need.path, &module, &leaf.binding, node)
                        .unwrap_or_default();
                    if !node.parent().is_some_and(|p| p.kind() == "source_file") {
                        let scope = node.parent().expect("use scope");
                        let repair = Repair { path:need.path.clone(),range:span(node.end_byte(), node.end_byte()),after:import_text(&new, &leaf.binding),kind:"import_insert",written_reexport:false,target:RewriteTarget::Synthesis {path:consumer.clone(),slot:"import".into(),items:self.contributors(&need.item_ids),boundary_role:None,parent_path:None,binding:Some(format!("{}@{}", leaf.binding, node.start_byte()))},item_ids:need.item_ids.clone(),anchors:vec![anchor(self.files, &need.path, &leaf.leaf_range)],rationale:"extract only the changed leaf into an explicit binding in its original lexical scope".into(),visibility:None,declaration_for:None,references,caller_override:false,import_module:Some(using.clone()),import_scope:Some(span(scope.start_byte(),scope.end_byte())) };
                        if self.binding_collision(&repair, &leaf.binding, &using) {
                            need.reason = DecisionReason::LexicalContextUnproved;
                            need.category = "binding_collision";
                            need.message =
                                "extracted leaf conflicts with a surviving lexical binding".into();
                            return false;
                        }
                        self.add(repair);
                    } else if !self.import(
                        &consumer,
                        &leaf.binding,
                        &new,
                        &need.item_ids,
                        (
                            anchor(self.files, &need.path, &leaf.leaf_range),
                            reexports.clone(),
                        ),
                        references,
                    ) {
                        return false;
                    }
                }
            }
            if let Some((p, i)) = declaration
                && !self.visibility_need(
                    need,
                    &p,
                    &i,
                    &using,
                    facade,
                    access(
                        self.files,
                        &need.path,
                        &leaf.path_range,
                        "written_import",
                        &module,
                        &using,
                    ),
                )
            {
                return false;
            }
        }
        self.use_evidence(repair_start, &reexports);
        true
    }
    fn use_evidence(&mut self, start: usize, evidence: &RouteEvidence) {
        if evidence.anchors.is_empty() {
            return;
        }
        for repair in &mut self.repairs[start..] {
            if !matches!(
                repair.kind,
                "use_path" | "import_insert" | "import_leaf_extract"
            ) {
                continue;
            }
            repair.written_reexport = true;
            self.descriptor_bytes += evidence.disclose(&mut repair.rationale);
            for a in &evidence.anchors {
                if !repair.anchors.contains(a) {
                    self.descriptor_bytes += a.path.len() + a.expected_text.len() + 128;
                    if self.descriptor_bytes > 128 * 1024 * 1024 {
                        self.exceeded = true;
                        return;
                    }
                    repair.anchors.push(a.clone());
                }
            }
            self.exceeded |= self.descriptor_bytes > 128 * 1024 * 1024;
        }
    }
    fn scoped_imports(
        &self,
        path: &str,
        module: &[String],
        node: Node<'_>,
        name: &str,
    ) -> Vec<ImportBinding> {
        let mut found: Vec<_> = self
            .imports
            .iter()
            .filter(|b| {
                b.path == path
                    && b.module == module
                    && b.leaf.binding == name
                    && b.scope_range.as_ref().is_some_and(|r| {
                        r.start_byte <= node.start_byte() && r.end_byte >= node.end_byte()
                    })
            })
            .cloned()
            .collect();
        if let Some(nearest) = found
            .iter()
            .map(|b| {
                b.scope_range.as_ref().expect("scope").end_byte
                    - b.scope_range.as_ref().expect("scope").start_byte
            })
            .min()
        {
            found.retain(|b| {
                b.scope_range.as_ref().expect("scope").end_byte
                    - b.scope_range.as_ref().expect("scope").start_byte
                    == nearest
            });
        }
        found
    }
    fn path_repair(&mut self, need: &mut Need, node: Node<'_>) -> bool {
        if items::check(self.controls.0, self.controls.1).is_err() {
            return false;
        }
        let text = &self.files[&need.path].source[node.byte_range()];
        let Some(old_module) = self.lexical_module(&need.path, node, false) else {
            need.reason = DecisionReason::ConditionalOrInheritedContext;
            need.category = "module_context";
            need.message =
                "containing inline/conditional lexical module is not uniquely evidenced".into();
            return false;
        };
        let Some(using) = self.lexical_module(&need.path, node, true) else {
            return false;
        };
        let first = text.split("::").next().unwrap_or("");
        let local_aliases = self.scoped_imports(&need.path, &old_module, node, first);
        if !matches!(first, "crate" | "self" | "super") {
            let Ok(assessment) = items::lexical_assessment_with_globs(
                &need.path,
                node,
                &self.files[&need.path].source,
                first,
                self.controls,
                local_aliases.len() == 1,
                self.cfg.as_ref(),
                Some(&self.pattern_routes),
            ) else {
                return false;
            };
            match assessment.binding {
                items::LexicalBinding::Independent | items::LexicalBinding::Uncertain => {
                    need.reason = DecisionReason::LexicalContextUnproved;
                    need.category = "binding_collision";
                    need.lexical(assessment);
                    need.message =
                        "path prefix has a local or unproved competing namespace binding".into();
                    return false;
                }
                items::LexicalBinding::Absent => {}
            }
        }
        let old = if local_aliases.len() == 1
            && !local_aliases[0].conditioned
            && !local_aliases[0].leaf.public
        {
            self.resolve_in(&need.path, &old_module, &local_aliases[0].leaf.path, false)
                .filter(|base| {
                    self.contexts
                        .values()
                        .any(|c| canonical(c, "").trim_end_matches("::") == base)
                })
                .map(|base| format!("{base}{}", text.strip_prefix(first).expect("prefix")))
        } else if local_aliases.is_empty() {
            self.resolve_in(&need.path, &old_module, text, true)
        } else {
            None
        };
        let Some(old) = old else {
            need.reason = DecisionReason::ExternalOrMissingBinding;
            return false;
        };
        let Some(resolved) = self.written_target(need, &old, &using) else {
            return false;
        };
        let Some((path, binding)) = self.declaration(&resolved.terminal) else {
            return self.missing_target(need, &resolved.terminal);
        };
        let after = resolved.route;
        if self.constructor_unknown(&binding, node) {
            need.reason = DecisionReason::MemberOrConstructorUnproved;
            need.category = "visibility_context";
            need.message =
                "constructor/field access is not established by top-level visibility evidence"
                    .into();
            return false;
        }
        if after != old || old_module != using {
            self.source_repair(
                need,
                span(node.start_byte(), node.end_byte()),
                after,
                "path",
            );
            self.path_evidence(need, &resolved.evidence);
        }
        self.visibility_need(
            need,
            &path,
            &binding,
            &using,
            !resolved.evidence.fallback.is_empty(),
            access(
                self.files,
                &need.path,
                &span(node.start_byte(), node.end_byte()),
                "written_path",
                &old_module,
                &using,
            ),
        )
    }
    fn constructor_unknown(&self, item: &Item, reference: Node<'_>) -> bool {
        if item.kind != "struct_item" {
            return false;
        }
        let declaration = self.parsed[&item.path]
            .tree
            .root_node()
            .named_descendant_for_byte_range(item.span.range.start_byte, item.span.range.end_byte)
            .expect("item");
        let tuple = declaration
            .child_by_field_name("body")
            .is_some_and(|n| n.kind() == "ordered_field_declaration_list");
        let mut parent = reference.parent();
        while let Some(p) = parent {
            if p.kind() == "struct_expression" {
                return true;
            }
            if p.kind() == "call_expression" {
                return tuple;
            }
            if !matches!(
                p.kind(),
                "scoped_identifier" | "scoped_type_identifier" | "generic_function"
            ) {
                break;
            }
            parent = p.parent();
        }
        false
    }
    fn final_resolve(&self, path: &str, module: &[String], text: &str) -> Option<String> {
        if let Some(absolute) = absolute_path(module, text) {
            return Some(absolute);
        }
        let first = text.split("::").next()?;
        let imports = self.imported(path, module, first);
        if imports.len() == 1 && !imports[0].conditioned && !imports[0].leaf.public {
            let base = self.resolve_in(path, module, &imports[0].leaf.path, false)?;
            let final_base = self.mapped(&base);
            let rest = text.strip_prefix(first)?;
            if !rest.is_empty() {
                return Some(format!("{final_base}{rest}"));
            }
        }
        let mut parts = vec!["crate".to_owned()];
        parts.extend(module.iter().cloned());
        parts.push(text.into());
        let local = parts.join("::");
        if self
            .final_contexts
            .values()
            .any(|c| canonical(c, "").trim_end_matches("::") == local)
            || self.selected.iter().any(|(_, i, d)| {
                i.name
                    .as_deref()
                    .is_some_and(|n| canonical(&self.final_contexts[d], n) == local)
            })
            || self.declaration(&local).is_some()
        {
            Some(local)
        } else if simple_path(text) && imports.is_empty() {
            Some(text.into())
        } else {
            None
        }
    }
    fn choice_target(&self, repair: &Repair, module: &[String], text: &str) -> Option<String> {
        if let Some(absolute) = absolute_path(module, text) {
            return Some(absolute);
        }
        if !simple_path(text) {
            return None;
        }
        let first = text.split("::").next()?;
        let node = self.parsed.get(&repair.path).and_then(|p| {
            p.tree
                .root_node()
                .named_descendant_for_byte_range(repair.range.start_byte, repair.range.end_byte)
        });
        let consumer = match &repair.target {
            RewriteTarget::Synthesis { path, .. } => path.clone(),
            RewriteTarget::Source { .. } => self.consumer(&repair.path, node?),
        };
        if let Some(node) = node
            && self.lexical_binding(
                &repair.path,
                node,
                &self.files[&repair.path].source,
                first,
                true,
            ) != items::LexicalBinding::Absent
        {
            return None;
        }
        let pending: Vec<_> = self
            .repairs
            .iter()
            .filter(|r| {
                if r.kind != "import_insert" || r.after.is_empty() {
                    return false;
                }
                let RewriteTarget::Synthesis { path, .. } = &r.target else {
                    return false;
                };
                path == &consumer
                    && r.import_module
                        .as_deref()
                        .unwrap_or(&self.final_contexts[path].module_segments)
                        == module
                    && r.import_scope.as_ref().is_none_or(|scope| {
                        r.path == repair.path
                            && scope.start_byte <= repair.range.start_byte
                            && scope.end_byte >= repair.range.end_byte
                    })
            })
            .filter_map(|r| parsed_import(&r.after, self.controls).ok())
            .filter(|(_, binding)| binding == first)
            .collect();
        if pending.len() > 1 {
            return None;
        }
        if let Some((path, _)) = pending.first() {
            let target = self.final_resolve(&consumer, module, path)?;
            let rest = text.strip_prefix(first)?;
            if !rest.is_empty()
                && !self
                    .final_contexts
                    .values()
                    .any(|c| canonical(c, "").trim_end_matches("::") == target)
            {
                return None;
            }
            return Some(format!("{target}{rest}"));
        }
        if let Some(node) = node
            && let Some(old_module) = self.lexical_module(&repair.path, node, false)
        {
            let aliases = self.scoped_imports(&repair.path, &old_module, node, first);
            if !aliases.is_empty() {
                if aliases.len() != 1 || aliases[0].conditioned || aliases[0].leaf.public {
                    return None;
                }
                let base = self.imported_target(&aliases[0])?;
                if !self
                    .final_contexts
                    .values()
                    .any(|c| canonical(c, "").trim_end_matches("::") == base)
                {
                    return None;
                }
                return Some(format!("{base}{}", text.strip_prefix(first)?));
            }
        }
        self.final_resolve(&consumer, module, text)
    }
    fn binding_collision(&self, repair: &Repair, name: &str, module: &[String]) -> bool {
        if let Some(scope) = &repair.import_scope {
            let node = self.parsed[&repair.path]
                .tree
                .root_node()
                .named_descendant_for_byte_range(scope.start_byte, scope.end_byte)
                .expect("scope");
            if (0..node.named_child_count())
                .filter_map(|i| node.named_child(i as u32))
                .any(|n| {
                    n.child_by_field_name("name")
                        .is_some_and(|n| self.files[&repair.path].source[n.byte_range()] == *name)
                })
            {
                return true;
            }
            return self.imports.iter().any(|b| {
                b.path == repair.path
                    && b.leaf.binding == name
                    && (b.scope_range.as_ref() == Some(scope)
                        || (node.kind() == "declaration_list"
                            && b.scope_range.is_none()
                            && b.module == module))
                    && !self.repairs.iter().any(|r| {
                        r.kind == "import_leaf_extract"
                            && r.path == b.path
                            && r.range.start_byte <= b.leaf.leaf_range.start_byte
                            && r.range.end_byte >= b.leaf.leaf_range.end_byte
                    })
            });
        }
        self.parsed.get(&repair.path).is_some_and(|p| {
            p.items.iter().any(|i| {
                i.name.as_deref() == Some(name) && self.final_path(&repair.path, i) == repair.path
            })
        }) || !self.imported(&repair.path, module, name).is_empty()
            || self
                .selected
                .iter()
                .any(|(_, i, d)| d == &repair.path && i.name.as_deref() == Some(name))
    }
    fn repair_need(
        &self,
        repair: &Repair,
        reason: DecisionReason,
        category: &'static str,
        message: &str,
    ) -> Need {
        let a = repair
            .references
            .first()
            .or(repair.anchors.first())
            .cloned()
            .or_else(|| self.contributors(&repair.item_ids).into_iter().next())
            .expect("repair has original evidence");
        let choice_target = self
            .request
            .rewrite_overrides
            .as_deref()
            .unwrap_or_default()
            .iter()
            .any(|c| {
                c.target == repair.target
                    && matches!(c.action, crate::move_plan::RewriteAction::Replace)
            })
            .then(|| repair.target.clone());
        Need {
            attribute_range: None,
            reason: if reason == DecisionReason::FinalAliasConflict && choice_target.is_none() {
                DecisionReason::DestinationBindingConflict
            } else {
                reason
            },
            choice_target,
            lexical_uncertainty: None,
            refusal_basis: Vec::new(),
            category,
            path: a.path,
            range: a.range,
            message: message.into(),
            item_ids: repair.item_ids.clone(),
        }
    }
    fn choices(&mut self) -> Result<Vec<Need>, DomainError> {
        let mut failures = Vec::new();
        // Final aliases must be chosen before any path validates against them, regardless of scan order.
        self.repairs.sort_by_key(|r| r.kind != "import_insert");
        let mut cursor = 0;
        while cursor < self.repairs.len() {
            let index = cursor;
            cursor += 1;
            items::check(self.controls.0, self.controls.1)?;
            let repair = self.repairs[index].clone();
            let Some(choice) = self
                .request
                .rewrite_overrides
                .as_deref()
                .unwrap_or_default()
                .iter()
                .find(|c| c.target == repair.target)
            else {
                continue;
            };
            if !matches!(choice.action, crate::move_plan::RewriteAction::Replace) {
                continue;
            }
            let text = choice
                .replacement_text
                .as_deref()
                .ok_or_else(|| invalid_choice("replace requires replacement_text"))?;
            if text.len() > 64 * 1024 {
                return Err(invalid_choice(
                    "replacement exceeds the 64-KiB alternative bound",
                ));
            }
            let module = if let Some(module) = &repair.import_module {
                module.clone()
            } else {
                match &repair.target {
                    RewriteTarget::Source { anchor } => {
                        let node = self.parsed[&anchor.path]
                            .tree
                            .root_node()
                            .named_descendant_for_byte_range(
                                anchor.range.start_byte,
                                anchor.range.end_byte,
                            )
                            .ok_or_else(|| invalid_choice("no source context for alternative"))?;
                        self.lexical_module(&anchor.path, node, true)
                            .ok_or_else(|| {
                                invalid_choice("uncertain lexical module for alternative")
                            })?
                    }
                    RewriteTarget::Synthesis { path, .. } => {
                        self.final_contexts[path].module_segments.clone()
                    }
                }
            };
            match repair.kind {
                "path" | "use_path" => {
                    if !simple_path(text) {
                        return Err(invalid_choice(
                            "only complete simple paths are supported; no comments, raw/absolute/qualified paths or code injection",
                        ));
                    }
                    let prefix = self
                        .imports
                        .iter()
                        .find(|b| {
                            b.path == repair.path
                                && (b.leaf.path_range == repair.range
                                    || b.leaf.prefix_range.as_ref() == Some(&repair.range))
                        })
                        .map(|b| {
                            if b.leaf.path_range == repair.range {
                                b.leaf.prefix.as_str()
                            } else {
                                b.leaf
                                    .prefix
                                    .strip_suffix(
                                        &self.files[&repair.path].source
                                            [repair.range.start_byte..repair.range.end_byte],
                                    )
                                    .unwrap_or("")
                                    .trim_end_matches("::")
                            }
                        })
                        .unwrap_or("");
                    let full = |t: &str| {
                        if prefix.is_empty() {
                            t.to_owned()
                        } else {
                            format!("{prefix}::{t}")
                        }
                    };
                    let expected = self.choice_target(&repair, &module, &full(&repair.after));
                    if expected.is_none()
                        || self.choice_target(&repair, &module, &full(text)) != expected
                    {
                        return Err(invalid_choice(
                            "path alternative must preserve the same evidenced final target",
                        ));
                    }
                }
                "visibility" => {
                    let region = &self.visibility_regions[&visibility_key(&repair)];
                    let chosen = chosen_visibility_region(text, &region.defining, self.controls)?;
                    if !region.required.starts_with(&chosen) {
                        failures.push(self.repair_need(&repair, DecisionReason::VisibilityScopeUnproved, "visibility_context", "selected visibility does not cover every proven caller and the declaration's preserved access region"));
                    }
                }
                "import_insert" => {
                    let (expected, binding) = parsed_import(&repair.after, self.controls)?;
                    let (alternative, new_binding) = if simple_path(text) {
                        (text.to_owned(), String::new())
                    } else {
                        parsed_import(text, self.controls)?
                    };
                    let expected = self.choice_target(&repair, &module, &expected);
                    if expected.is_none()
                        || self.choice_target(&repair, &module, &alternative) != expected
                    {
                        return Err(invalid_choice(
                            "import/path alternative must preserve the same evidenced final target",
                        ));
                    }
                    if new_binding != binding {
                        if repair.references.is_empty() {
                            return Err(invalid_choice(
                                "binding change requires anchored written reference evidence",
                            ));
                        }
                        let collision = !new_binding.is_empty()
                            && self.binding_collision(&repair, &new_binding, &module);
                        if collision {
                            failures.push(self.repair_need(&repair, DecisionReason::FinalAliasConflict, "binding_collision", "caller-selected import alias collides with a final written binding"));
                        }
                        for reference in &repair.references {
                            let node = self.parsed[&reference.path]
                                .tree
                                .root_node()
                                .named_descendant_for_byte_range(
                                    reference.range.start_byte,
                                    reference.range.end_byte,
                                )
                                .expect("reference");
                            if !new_binding.is_empty() {
                                let assessment = items::lexical_assessment_with_globs(
                                    &reference.path,
                                    node,
                                    &self.files[&reference.path].source,
                                    &new_binding,
                                    self.controls,
                                    false,
                                    self.cfg.as_ref(),
                                    Some(&self.pattern_routes),
                                )?;
                                if assessment.binding != items::LexicalBinding::Absent {
                                    self.account_lexical(&assessment.uncertainty);
                                    if self.exceeded {
                                        return Err(DomainError::new(
                                            "analysis_descriptor_bytes",
                                            "lexical alias evidence guard reached",
                                        ));
                                    }
                                    failures.push(Need {attribute_range:None,reason:DecisionReason::FinalAliasConflict,choice_target:Some(repair.target.clone()),lexical_uncertainty:assessment.uncertainty,refusal_basis:Vec::new(),category:"binding_collision",path:reference.path.clone(),range:reference.range.clone(),message:"caller-selected alias is shadowed or unproved at an anchored access".into(),item_ids:repair.item_ids.clone()});
                                }
                            }
                            let need = Need {
                                attribute_range: None,
                                reason: DecisionReason::UnsupportedConstruct,
                                choice_target: None,
                                lexical_uncertainty: None,
                                refusal_basis: Vec::new(),
                                category: "unsupported_dependency_form",
                                path: reference.path.clone(),
                                range: reference.range.clone(),
                                message: String::new(),
                                item_ids: repair.item_ids.clone(),
                            };
                            let start = self.repairs.len();
                            self.source_repair(
                                &need,
                                reference.range.clone(),
                                if new_binding.is_empty() {
                                    alternative.clone()
                                } else {
                                    new_binding.clone()
                                },
                                "path",
                            );
                            for authored in &mut self.repairs[start..] {
                                authored.caller_override = true;
                            }
                        }
                        if new_binding.is_empty() {
                            self.repairs[index].after.clear();
                            self.repairs[index].caller_override = true;
                            continue;
                        }
                    }
                }
                "import_leaf_extract" if text.is_empty() => {}
                _ if text == repair.after => {}
                _ => return Err(invalid_choice("unsupported rewrite alternative")),
            }
            self.repairs[index].after = text.into();
            self.repairs[index].caller_override = true;
        }
        // Recheck all chosen synthesized bindings as one simultaneous set.
        let imports: Vec<_> = self
            .repairs
            .iter()
            .filter(|r| r.kind == "import_insert" && !r.after.is_empty())
            .collect();
        for (index, left) in imports.iter().enumerate() {
            let (target, binding) = parsed_import(&left.after, self.controls)?;
            let prefix = target.split("::").next().unwrap_or("");
            let module = left.import_module.as_deref().unwrap_or(
                &self.final_contexts[match &left.target {
                    RewriteTarget::Synthesis { path, .. } => path,
                    _ => &left.path,
                }]
                .module_segments,
            );
            if !matches!(prefix, "crate" | "self" | "super")
                && self.binding_collision(left, prefix, module)
            {
                failures.push(self.repair_need(
                    left,
                    DecisionReason::FinalAliasConflict,
                    "binding_collision",
                    "external import prefix conflicts with a final written binding",
                ));
            }
            for right in &imports {
                if !matches!(prefix, "crate" | "self" | "super")
                    && left.path == right.path
                    && left.import_scope == right.import_scope
                    && left.import_module == right.import_module
                    && parsed_import(&right.after, self.controls)?.1 == prefix
                {
                    failures.push(self.repair_need(
                        left,
                        DecisionReason::FinalAliasConflict,
                        "binding_collision",
                        "external import prefix conflicts with a selected alias",
                    ));
                }
            }
            for right in &imports[..index] {
                if left.path == right.path
                    && left.import_scope == right.import_scope
                    && left.import_module == right.import_module
                    && parsed_import(&right.after, self.controls)?.1 == binding
                    && left.after != right.after
                {
                    failures.push(self.repair_need(
                        left,
                        DecisionReason::FinalAliasConflict,
                        "binding_collision",
                        "selected imports introduce competing final aliases",
                    ));
                }
            }
        }
        Ok(failures)
    }
    fn repair(&mut self, need: &mut Need) -> bool {
        // Preserve the semantic tier's original/final attribute evidence when
        // requested; written-only calls need no resolver to evaluate predicates.
        if !self.request.resolve_semantic
            && need.reason == DecisionReason::ConditionalOrInheritedContext
            && let Some(range) = &need.attribute_range
            && items::cfg::attribute(
                &self.files[&need.path].source[range.start_byte..range.end_byte],
                self.cfg.as_ref(),
                true,
            )
            .is_ok()
        {
            return true;
        }
        if matches!(need.category, "reexport_dependency" | "glob_dependency") {
            let node = self.parsed[&need.path]
                .tree
                .root_node()
                .named_descendant_for_byte_range(need.range.start_byte, need.range.end_byte);
            if let Some(node) = node.filter(|n| n.kind() == "use_declaration")
                && items::use_facts(node, &self.files[&need.path].source, self.controls)
                    .is_ok_and(|f| !f.0)
                && let Some(module) = self.lexical_module(&need.path, node, false)
                && let Ok(leaves) =
                    items::use_leaves(node, &self.files[&need.path].source, self.controls)
                && !leaves.is_empty()
                && leaves.iter().all(|leaf| {
                    self.resolve_in(&need.path, &module, &leaf.path, false)
                        .is_some_and(|target| {
                            !self.selected.iter().any(|(p, i, _)| {
                                need.item_ids.contains(&i.id)
                                    && i.name.as_deref().is_some_and(|name| {
                                        canonical(&self.contexts[p], name) == target
                                    })
                            })
                        })
                })
            {
                return true;
            }
        }
        if need.category == "visibility_context"
            && let Some((path, item, destination)) = self.selected.iter().find(|(p, i, _)| {
                p == &need.path && i.span.range == need.range && i.visibility_key == "restricted"
            })
        {
            let node = self.parsed[path]
                .tree
                .root_node()
                .named_descendant_for_byte_range(
                    item.span.range.start_byte,
                    item.span.range.end_byte,
                )
                .expect("item");
            return items::absolute_visibility(node, &self.files[path].source, self.controls)
                .is_some_and(|scope| {
                    self.contexts
                        .get(path)
                        .is_some_and(|c| c.module_segments.starts_with(&scope))
                        && self
                            .final_contexts
                            .get(destination)
                            .is_some_and(|c| c.module_segments.starts_with(&scope))
                });
        }
        if need.category != "unsupported_dependency_form"
            && need.reason != DecisionReason::LexicalContextUnproved
        {
            return false;
        }
        let data = &self.parsed[&need.path];
        let source = &self.files[&need.path].source;
        let Some(node) = data
            .tree
            .root_node()
            .named_descendant_for_byte_range(need.range.start_byte, need.range.end_byte)
        else {
            return false;
        };
        if node.kind() == "use_declaration" {
            return self.use_repair(need, node);
        }
        let mut path_node = node;
        while let Some(parent) = path_node.parent() {
            if !matches!(
                parent.kind(),
                "scoped_identifier" | "scoped_type_identifier"
            ) {
                break;
            }
            path_node = parent;
        }
        if matches!(
            path_node.kind(),
            "scoped_identifier" | "scoped_type_identifier"
        ) {
            return self.path_repair(need, path_node);
        }
        if !matches!(node.kind(), "identifier" | "type_identifier") {
            return false;
        }
        let name = &source[node.byte_range()];
        let consumer = self.consumer(&need.path, node);
        let Some(module) = self.lexical_module(&need.path, node, false) else {
            need.reason = DecisionReason::ConditionalOrInheritedContext;
            need.category = "module_context";
            need.message = "written consumer has uncertain lexical module context".into();
            return false;
        };
        let Some(using) = self.lexical_module(&need.path, node, true) else {
            return false;
        };
        let local_imports: Vec<_> = self
            .imports
            .iter()
            .filter(|b| {
                b.path == need.path
                    && b.module == module
                    && b.leaf.binding == name
                    && b.scope_range.as_ref().is_some_and(|r| {
                        r.start_byte <= node.start_byte() && r.end_byte >= node.end_byte()
                    })
            })
            .cloned()
            .collect();
        let Ok(assessment) = items::lexical_assessment_with_globs(
            &need.path,
            node,
            source,
            name,
            self.controls,
            !local_imports.is_empty(),
            self.cfg.as_ref(),
            Some(&self.pattern_routes),
        ) else {
            return false;
        };
        match assessment.binding {
            items::LexicalBinding::Independent => return true,
            items::LexicalBinding::Uncertain => {
                need.reason = DecisionReason::LexicalContextUnproved;
                need.category = "binding_collision";
                need.lexical(assessment);
                need.message = "containing lexical binding context is unproved".into();
                return false;
            }
            items::LexicalBinding::Absent => {}
        }
        if !local_imports.is_empty() {
            let nearest = local_imports
                .iter()
                .map(|b| {
                    b.scope_range.as_ref().expect("scope").end_byte
                        - b.scope_range.as_ref().expect("scope").start_byte
                })
                .min()
                .expect("import");
            let nearest: Vec<_> = local_imports
                .iter()
                .filter(|b| {
                    b.scope_range.as_ref().expect("scope").end_byte
                        - b.scope_range.as_ref().expect("scope").start_byte
                        == nearest
                })
                .collect();
            if nearest.len() != 1 || nearest[0].conditioned || nearest[0].leaf.public {
                need.reason = DecisionReason::LexicalContextUnproved;
                need.category = "binding_collision";
                return false;
            }
            let Some(old) = self.imported_target(nearest[0]) else {
                return false;
            };
            let Some(resolved) = self.written_target(need, &old, &using) else {
                return false;
            };
            return if let Some((p, i)) = self.declaration(&resolved.terminal) {
                self.visibility_need(
                    need,
                    &p,
                    &i,
                    &using,
                    !resolved.evidence.fallback.is_empty(),
                    access(
                        self.files,
                        &need.path,
                        &span(node.start_byte(), node.end_byte()),
                        "written_binding",
                        &module,
                        &using,
                    ),
                )
            } else if resolved.terminal.starts_with("crate::") {
                self.missing_target(need, &resolved.terminal)
            } else {
                true
            };
        }
        let imports = self.imported(&need.path, &module, name);
        let direct = self
            .binding(&need.path, name)
            .filter(|_| self.contexts[&need.path].module_segments == module);
        if imports.len() > 1 || (direct.is_some() && !imports.is_empty()) {
            need.reason = DecisionReason::LexicalContextUnproved;
            need.category = "binding_collision";
            need.message = "several written declarations/imports compete for this binding".into();
            return false;
        }
        if imports.len() == 1 && imports[0].conditioned {
            need.reason = DecisionReason::ConditionalOrInheritedContext;
            need.category = "scope_dependency";
            need.message =
                "needed import has conditional or public/chained exposure context".into();
            return false;
        }
        let (old, evidence) = if let Some(binding) = direct {
            (
                canonical(&self.contexts[&need.path], name),
                anchor(self.files, &need.path, &binding.span.range),
            )
        } else if imports.len() == 1 && !imports[0].conditioned {
            let target = if imports[0].leaf.public {
                Some(canonical(&self.contexts[&need.path], name))
            } else {
                self.imported_target(&imports[0])
            };
            let Some(target) = target else {
                return false;
            };
            (
                target,
                anchor(self.files, &need.path, &imports[0].leaf.declaration),
            )
        } else {
            return false;
        };
        let occurrence = need.clone();
        let Some(resolved) = self.written_target(need, &old, &using) else {
            // A generated declaration has no written terminal inventory item.
            // Carry only the caller's existing, written-accessible public facade
            // for a type-position import; never guess a canonical terminal. The
            // need remains until resolution proves identity/access at both ends.
            let leaves = self.reexports(&old);
            if self.request.resolve_semantic
                && need.reason == DecisionReason::ExternalOrMissingBinding
                && node.kind() == "type_identifier"
                && imports.len() == 1
                && !imports[0].leaf.public
                && leaves.len() == 1
                && !leaves[0].conditioned
                // Configured ordinary bindings do not relax the separate
                // provisional facade route for generated declarations.
                && !leaves[0].written_attributes
                && let Some(item) = self.parsed[&leaves[0].path]
                    .items
                    .iter()
                    .find(|i| i.span.range == leaves[0].leaf.declaration)
                && matches!(item.visibility_key, "pub" | "pub(crate)")
                && self.accessible_route(need, &leaves[0].path, item, &using)
                && self.final_contexts[&consumer].module_segments == using
            {
                // Failed written routing can retarget the need to a facade use.
                // Resolution must audit the original type occurrence, not that use.
                *need = occurrence;
                let route = RouteEvidence {
                    anchors: vec![anchor(self.files, &leaves[0].path, &leaves[0].leaf.declaration)],
                    fallback: vec!["preserve written public facade import pending both-overlay declaration identity; no generated terminal guessed".into()],
                };
                let _ = self.import(
                    &consumer,
                    name,
                    &old,
                    &need.item_ids,
                    (evidence, route),
                    vec![anchor(
                        self.files,
                        &need.path,
                        &span(node.start_byte(), node.end_byte()),
                    )],
                );
            }
            return false;
        };
        let facade = !resolved.evidence.fallback.is_empty();
        let old = resolved.terminal;
        if let Some((_, binding)) = self.declaration(&old)
            && self.constructor_unknown(&binding, node)
        {
            need.reason = DecisionReason::MemberOrConstructorUnproved;
            need.category = "visibility_context";
            need.message =
                "constructor/field access is not established by top-level visibility evidence"
                    .into();
            return false;
        }
        let target = resolved.route;
        let same_module = self.declaration(&old).is_some_and(|(p, i)| {
            self.final_contexts
                .get(self.final_path(&p, &i))
                .is_some_and(|c| c.module_segments == using)
        });
        if !same_module {
            if self.final_contexts[&consumer].module_segments != using {
                self.source_repair(
                    need,
                    span(node.start_byte(), node.end_byte()),
                    target.clone(),
                    "path",
                );
                self.path_evidence(need, &resolved.evidence);
            } else if !self.import(
                &consumer,
                name,
                &target,
                &need.item_ids,
                (evidence, resolved.evidence),
                vec![anchor(
                    self.files,
                    &need.path,
                    &span(node.start_byte(), node.end_byte()),
                )],
            ) {
                need.reason = DecisionReason::DestinationBindingConflict;
                need.category = "binding_collision";
                need.message =
                    "required destination import conflicts with a final declaration/import/alias"
                        .into();
                return false;
            }
        }
        if let Some((p, i)) = self.declaration(&old) {
            self.visibility_need(
                need,
                &p,
                &i,
                &using,
                facade,
                access(
                    self.files,
                    &need.path,
                    &span(node.start_byte(), node.end_byte()),
                    "written_binding",
                    &module,
                    &using,
                ),
            )
        } else if old.starts_with("crate::") {
            self.missing_target(need, &old)
        } else {
            true
        }
    }
}
#[allow(clippy::too_many_arguments)] // Immutable corpus/context inputs stay explicit at this seam.
pub(crate) fn analyze(
    request: &MoveRequest,
    files: &BTreeMap<String, FileSnapshot>,
    parsed: &BTreeMap<String, ParsedFile>,
    selected: &[(String, Item, String)],
    contexts: &BTreeMap<String, ModuleEvidence>,
    final_contexts: &BTreeMap<String, ModuleEvidence>,
    needs: Vec<Need>,
    controls: (Instant, &AtomicBool),
    reference_candidates: &mut usize,
) -> Result<Analysis, DomainError> {
    let mut analyzer = Analyzer {
        request,
        cfg: request.declared_cfg(),
        files,
        parsed,
        selected,
        contexts,
        final_contexts,
        repairs: Vec::new(),
        visibility_regions: BTreeMap::new(),
        visibility_observations: BTreeMap::new(),
        imports: Vec::new(),
        descriptor_bytes: 0,
        reference_candidates,
        exceeded: false,
        pattern_routes: items::GlobRoutes::strict(
            files,
            parsed,
            contexts,
            &request.crate_root,
            controls,
        )?,
        controls,
    };
    for (path, data) in parsed {
        let mut stack = vec![data.tree.root_node()];
        while let Some(node) = stack.pop() {
            items::check(controls.0, controls.1)?;
            if matches!(
                node.kind(),
                "token_tree" | "macro_definition" | "macro_invocation"
            ) {
                continue;
            }
            if node.kind() == "use_declaration" {
                if let Some(module) = analyzer.lexical_module(path, node, false) {
                    for leaf in items::use_leaves(node, &files[path].source, controls)? {
                        analyzer.descriptor_bytes += leaf.path.len()
                            + leaf.prefix.len()
                            + leaf.binding.len()
                            + path.len()
                            + module.iter().map(|s| s.len() + 32).sum::<usize>()
                            + 512;
                        if analyzer.descriptor_bytes > 128 * 1024 * 1024 {
                            return Err(DomainError::new(
                                "analysis_descriptor_bytes",
                                "written import index guard reached",
                            ));
                        }
                        analyzer.imports.push(ImportBinding {
                            path: path.clone(),
                            module: module.clone(),
                            leaf,
                            written_attributes: attributed_use(node),
                            conditioned: if analyzer.cfg.is_some() {
                                items::cfg::attributed(
                                    node,
                                    &files[path].source,
                                    analyzer.cfg.as_ref(),
                                )
                            } else {
                                attributed_use(node)
                            },
                            scope_range: {
                                let mut parent = node.parent();
                                let mut scope = None;
                                while let Some(p) = parent {
                                    if p.kind() == "block" {
                                        scope = Some(span(p.start_byte(), p.end_byte()));
                                        break;
                                    }
                                    if matches!(p.kind(), "source_file" | "mod_item") {
                                        break;
                                    }
                                    parent = p.parent();
                                }
                                scope
                            },
                        });
                        if analyzer.imports.len() > 100_000 {
                            return Err(DomainError::new(
                                "reference_work_limit",
                                "written import binding guard reached",
                            ));
                        }
                    }
                }
                continue;
            }
            for i in (0..node.named_child_count()).rev() {
                stack.push(node.named_child(i as u32).expect("child"));
            }
        }
    }
    let mut remaining = analyzer.associated()?;
    for mut need in needs {
        items::check(controls.0, controls.1)?;
        if !analyzer.repair(&mut need) {
            if analyzer.cfg.is_some()
                && (need.reason == DecisionReason::ConditionalOrInheritedContext
                    || need
                        .lexical_uncertainty
                        .as_ref()
                        .is_some_and(|w| w.reason == items::LexicalReason::ConditionalLocalContext))
            {
                need.message.push_str("; caller-declared configuration consulted; unknown atoms and unsupported context retain their veto");
            }
            analyzer.account_lexical(&need.lexical_uncertainty);
            if !analyzer.exceeded {
                remaining.push(need);
            }
        }
        if analyzer.exceeded {
            return Err(DomainError::new(
                if *analyzer.reference_candidates > 100_000 {
                    "reference_work_limit"
                } else {
                    "analysis_descriptor_bytes"
                },
                "rewrite evidence guard reached",
            ));
        }
    }
    analyzer.explain_visibility()?;
    remaining.extend(analyzer.choices()?);
    items::check(controls.0, controls.1)?;
    if analyzer.exceeded {
        return Err(DomainError::new(
            "analysis_descriptor_bytes",
            "caller-authored evidence guard reached",
        ));
    }
    Ok(Analysis {
        repairs: analyzer.repairs,
        needs: remaining,
        descriptor_bytes: analyzer.descriptor_bytes,
    })
}
