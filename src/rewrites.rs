//! Bounded written-binding repairs. Evidence is syntactic, never symbol resolution.
use crate::{
    items::{self, Item, ModuleEvidence, Need, ParsedFile},
    move_plan::{MoveRequest, RewriteTarget},
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
    pub target: RewriteTarget,
    pub item_ids: Vec<String>,
    pub anchors: Vec<SourceAnchor>,
    pub rationale: String,
    /// A visibility operation on the declaration linking this proposed file.
    pub declaration_for: Option<String>,
}
pub(crate) struct Analysis {
    pub repairs: Vec<Repair>,
    pub needs: Vec<Need>,
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
fn canonical(context: &ModuleEvidence, name: &str) -> String {
    let mut parts = vec!["crate".to_owned()];
    parts.extend(context.module_segments.clone());
    parts.push(name.into());
    parts.join("::")
}
#[derive(Clone)]
struct ImportBinding {
    path: String,
    module: Vec<String>,
    leaf: items::UseLeaf,
    conditioned: bool,
}
struct Analyzer<'a> {
    request: &'a MoveRequest,
    files: &'a BTreeMap<String, FileSnapshot>,
    parsed: &'a BTreeMap<String, ParsedFile>,
    selected: &'a [(String, Item, String)],
    contexts: &'a BTreeMap<String, ModuleEvidence>,
    final_contexts: &'a BTreeMap<String, ModuleEvidence>,
    repairs: Vec<Repair>,
    imports: Vec<ImportBinding>,
    controls: (Instant, &'a AtomicBool),
}
impl Analyzer<'_> {
    fn add(&mut self, repair: Repair) {
        if let Some(existing) = self
            .repairs
            .iter_mut()
            .find(|r| r.path == repair.path && r.range == repair.range && r.after == repair.after)
        {
            for id in repair.item_ids {
                if !existing.item_ids.contains(&id) {
                    existing.item_ids.push(id);
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
        let mut module = context.module_segments.clone();
        let mut inline = Vec::new();
        let mut parent = node.parent();
        while let Some(p) = parent {
            if p.kind() == "mod_item" && p.child_by_field_name("body").is_some() {
                if p.prev_named_sibling()
                    .is_some_and(|n| n.kind() == "attribute_item")
                {
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
            if let Some(context) = self.contexts.get(path) {
                for item in &data.items {
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
            .filter(|b| b.path == path && b.module == module && b.leaf.binding == name)
            .cloned()
            .collect()
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
    fn visibility(&mut self, path: &str, item: &Item, consumer: &[String], ids: &[String]) -> bool {
        let final_path = self.final_path(path, item).to_owned();
        let Some(defining) = self.final_contexts.get(&final_path).cloned() else {
            return false;
        };
        let using = consumer;
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
                        if parent_path == &declaration.path
                            && self.final_contexts[path].declaration_anchors.last()
                                == Some(declaration) =>
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
        )
    }
    fn widen(
        &mut self,
        path: &str,
        item: Option<&Item>,
        defining: &[String],
        using: &[String],
        ids: &[String],
        declaration_for: Option<String>,
    ) -> bool {
        let visibility = item.map(|i| i.visibility_key).unwrap_or("private");
        if using.starts_with(defining) || matches!(visibility, "pub" | "pub(crate)") {
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
        self.add(Repair {
            path: path.into(), range: span(at, at), after: "pub(crate) ".into(), kind: "visibility",
            target: RewriteTarget::Synthesis { path: path.into(), slot: "visibility_insert".into(), items: self.contributors(ids), boundary_role: None, parent_path: None, binding: Some(parts.join("::")) },
            item_ids: ids.to_vec(), anchors: item.map(|i| vec![anchor(self.files, path, &i.span.range)]).unwrap_or_default(),
            rationale: "a proven written access is outside the declaration's parent lexical scope and descendants".into(),
            declaration_for,
        });
        true
    }
    fn import(
        &mut self,
        path: &str,
        binding: &str,
        target: &str,
        ids: &[String],
        evidence: SourceAnchor,
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
                    .resolve_in(path, module, &existing[0].leaf.path, false)
                    .is_some_and(|old| self.mapped(&old) == target);
        }
        if self.repairs.iter().any(|r| r.kind == "import_insert" && r.path == path && matches!(&r.target, RewriteTarget::Synthesis { binding:Some(name), .. } if name == binding) && r.after != import_text(target, binding)) { return false; }
        let source = self
            .files
            .get(path)
            .map(|f| f.source.as_str())
            .unwrap_or("");
        let at = self
            .parsed
            .get(path)
            .and_then(|data| {
                data.items.first().map(|first| {
                    data.trivia
                        .iter()
                        .filter(|t| {
                            t.owned_by(first.span.range.start_byte..first.span.range.end_byte)
                                && t.range.end <= first.span.range.start_byte
                        })
                        .map(|t| t.range.start)
                        .min()
                        .unwrap_or(first.span.range.start_byte)
                })
            })
            .unwrap_or(source.len());
        let eol = if source.contains("\r\n") {
            "\r\n"
        } else {
            "\n"
        };
        let text = import_text(target, binding);
        self.add(Repair {
            path: path.into(), range: span(at, at), after: text, kind: "import_insert",
            target: RewriteTarget::Synthesis { path: path.into(), slot: "import".into(), items: self.contributors(ids), boundary_role: None, parent_path: None, binding: Some(binding.into()) },
            item_ids: ids.to_vec(), anchors: vec![evidence], rationale: format!("preserve the unique written binding {binding} through explicit {target}; boundary ending {eol:?} is separately audited"), declaration_for: None,
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
                if self.declaration(&candidate).is_some() {
                    return Some(candidate);
                }
                // External imports preserve written spelling; never follow another use binding.
                if !aliases && imports.is_empty() && !matches!(first.as_str(), "Self") {
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
            target: RewriteTarget::Source { anchor: a.clone() },
            item_ids: need.item_ids.clone(),
            anchors: vec![a],
            rationale: "complete written binding in a verified ordinary lexical context".into(),
            declaration_for: None,
        });
    }
    fn use_repair(&mut self, need: &Need, node: Node<'_>) -> bool {
        let source = &self.files[&need.path].source;
        if node
            .prev_named_sibling()
            .is_some_and(|n| n.kind() == "attribute_item")
        {
            return false;
        }
        let Some(module) = self.lexical_module(&need.path, node, false) else {
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
        let mut mappings = Vec::new();
        for leaf in &leaves {
            let Some(old) = self.resolve_in(&need.path, &module, &leaf.path, false) else {
                return false;
            };
            let new = self.mapped(&old);
            mappings.push((old, new));
        }
        // One common changed prefix can be replaced without touching any delimiter or alias.
        if leaves
            .iter()
            .all(|l| l.prefix_range == leaves[0].prefix_range && !l.prefix.is_empty())
            && mappings.iter().all(|(a, b)| a != b)
            && let Some(prefix_range) = &leaves[0].prefix_range
        {
            let new_prefix = mappings[0]
                .1
                .rsplit_once("::")
                .map(|(p, _)| p)
                .unwrap_or("");
            if mappings.iter().zip(&leaves).all(|((_, new), leaf)| {
                new.rsplit_once("::").is_some_and(|(p, suffix)| {
                    p == new_prefix && suffix == leaf.path.rsplit("::").next().unwrap_or("")
                })
            }) {
                let written_prefix = &source[prefix_range.start_byte..prefix_range.end_byte];
                // Only a full prefix is replaceable here; nested prefixes use leaf surgery below.
                if written_prefix == leaves[0].prefix {
                    self.source_repair(need, prefix_range.clone(), new_prefix.into(), "use_path");
                    for (old, _) in &mappings {
                        if let Some((p, i)) = self.declaration(old)
                            && !self.visibility(&p, &i, &using, &need.item_ids)
                        {
                            return false;
                        }
                    }
                    return true;
                }
            }
        }
        for (leaf, (old, new)) in leaves.iter().zip(mappings) {
            if old == new && module == using {
                continue;
            }
            if leaf.public {
                return false;
            }
            if leaf.prefix.is_empty() {
                self.source_repair(need, leaf.path_range.clone(), new.clone(), "use_path");
            } else {
                let Some(prefix) = self.resolve_in(&need.path, &module, &leaf.prefix, false) else {
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
                    if let Some(at) = following.find(',')
                        && following[..at].trim().is_empty()
                    {
                        removal.end_byte += at + 1;
                    } else {
                        let preceding = &source[list.start_byte + 1..removal.start_byte];
                        let Some(at) = preceding.rfind(',') else {
                            return false;
                        };
                        if !preceding[at + 1..].trim().is_empty() {
                            return false;
                        }
                        removal.start_byte = list.start_byte + 1 + at;
                    }
                    let bytes = &source[removal.start_byte..removal.end_byte];
                    if bytes.contains("//") || bytes.contains("/*") {
                        return false;
                    }
                    self.source_repair(need, removal, String::new(), "import_leaf_extract");
                    let consumer = self.consumer(&need.path, node);
                    if self.final_contexts[&consumer].module_segments != using {
                        return false;
                    }
                    if !self.import(
                        &consumer,
                        &leaf.binding,
                        &new,
                        &need.item_ids,
                        anchor(self.files, &need.path, &leaf.leaf_range),
                    ) {
                        return false;
                    }
                }
            }
            if let Some((p, i)) = self.declaration(&old)
                && !self.visibility(&p, &i, &using, &need.item_ids)
            {
                return false;
            }
        }
        true
    }
    fn path_repair(&mut self, need: &Need, node: Node<'_>) -> bool {
        if items::check(self.controls.0, self.controls.1).is_err() {
            return false;
        }
        let text = &self.files[&need.path].source[node.byte_range()];
        let Some(old_module) = self.lexical_module(&need.path, node, false) else {
            return false;
        };
        let Some(using) = self.lexical_module(&need.path, node, true) else {
            return false;
        };
        let first = text.split("::").next().unwrap_or("");
        if !matches!(first, "crate" | "self" | "super") {
            match items::lexical_binding(node, &self.files[&need.path].source, first, self.controls)
            {
                items::LexicalBinding::Independent => return true,
                items::LexicalBinding::Uncertain => return false,
                items::LexicalBinding::Absent => {}
            }
        }
        let Some(old) = self.resolve_in(&need.path, &old_module, text, true) else {
            return false;
        };
        let Some((path, binding)) = self.declaration(&old) else {
            return false;
        };
        let after = self.mapped(&old);
        if after != old || old_module != using {
            self.source_repair(
                need,
                span(node.start_byte(), node.end_byte()),
                after,
                "path",
            );
        }
        self.visibility(&path, &binding, &using, &need.item_ids)
    }
    fn repair(&mut self, need: &mut Need) -> bool {
        if need.category != "unsupported_dependency_form" {
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
            return false;
        };
        let Some(using) = self.lexical_module(&need.path, node, true) else {
            return false;
        };
        match items::lexical_binding(node, source, name, self.controls) {
            items::LexicalBinding::Independent => return true,
            items::LexicalBinding::Uncertain => {
                need.category = "binding_collision";
                need.message = "containing lexical binding context is uncertain".into();
                return false;
            }
            items::LexicalBinding::Absent => {}
        }
        let imports = self.imported(&need.path, &module, name);
        let direct = self
            .binding(&need.path, name)
            .filter(|_| self.contexts[&need.path].module_segments == module);
        if imports.len() > 1 || (direct.is_some() && !imports.is_empty()) {
            return false;
        }
        let (old, evidence) = if let Some(binding) = direct {
            (
                canonical(&self.contexts[&need.path], name),
                anchor(self.files, &need.path, &binding.span.range),
            )
        } else if imports.len() == 1 && !imports[0].conditioned && !imports[0].leaf.public {
            let Some(target) = self.resolve_in(&need.path, &module, &imports[0].leaf.path, false)
            else {
                return false;
            };
            (
                target,
                anchor(self.files, &need.path, &imports[0].leaf.declaration),
            )
        } else {
            return false;
        };
        let target = self.mapped(&old);
        let same_module = self.declaration(&old).is_some_and(|(p, i)| {
            self.final_contexts[self.final_path(&p, &i)].module_segments == using
        });
        if !same_module {
            if self.final_contexts[&consumer].module_segments != using {
                self.source_repair(
                    need,
                    span(node.start_byte(), node.end_byte()),
                    target.clone(),
                    "path",
                );
            } else if !self.import(&consumer, name, &target, &need.item_ids, evidence) {
                return false;
            }
        }
        if let Some((p, i)) = self.declaration(&old) {
            self.visibility(&p, &i, &using, &need.item_ids)
        } else {
            !old.starts_with("crate::")
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
) -> Result<Analysis, DomainError> {
    let mut analyzer = Analyzer {
        request,
        files,
        parsed,
        selected,
        contexts,
        final_contexts,
        repairs: Vec::new(),
        imports: Vec::new(),
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
                        analyzer.imports.push(ImportBinding {
                            path: path.clone(),
                            module: module.clone(),
                            leaf,
                            conditioned: node
                                .prev_named_sibling()
                                .is_some_and(|n| n.kind() == "attribute_item"),
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
    let mut remaining = Vec::new();
    for mut need in needs {
        items::check(controls.0, controls.1)?;
        if !analyzer.repair(&mut need) {
            remaining.push(need);
        }
    }
    Ok(Analysis {
        repairs: analyzer.repairs,
        needs: remaining,
    })
}
