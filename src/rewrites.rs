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
fn canonical(context: &ModuleEvidence, name: &str) -> String {
    let mut parts = vec!["crate".to_owned()];
    parts.extend(context.module_segments.clone());
    parts.push(name.into());
    parts.join("::")
}
struct Analyzer<'a> {
    request: &'a MoveRequest,
    files: &'a BTreeMap<String, FileSnapshot>,
    parsed: &'a BTreeMap<String, ParsedFile>,
    selected: &'a [(String, Item, String)],
    contexts: &'a BTreeMap<String, ModuleEvidence>,
    final_contexts: &'a BTreeMap<String, ModuleEvidence>,
    repairs: Vec<Repair>,
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
    fn visibility(&mut self, path: &str, item: &Item, consumer: &str, ids: &[String]) -> bool {
        let final_path = self.final_path(path, item).to_owned();
        let Some(defining) = self.final_contexts.get(&final_path).cloned() else {
            return false;
        };
        let Some(using) = self.final_contexts.get(consumer).cloned() else {
            return false;
        };
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
                &using.module_segments,
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
                &using.module_segments,
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
            &using.module_segments,
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
        let local = target.rsplit("::").next().unwrap_or(target);
        let text = if local == binding {
            format!("use {target};")
        } else {
            format!("use {target} as {binding};")
        };
        self.add(Repair {
            path: path.into(), range: span(at, at), after: text, kind: "import_insert",
            target: RewriteTarget::Synthesis { path: path.into(), slot: "import".into(), items: self.contributors(ids), boundary_role: None, parent_path: None, binding: Some(binding.into()) },
            item_ids: ids.to_vec(), anchors: vec![evidence], rationale: format!("preserve the unique written binding {binding} through explicit {target}; boundary ending {eol:?} is separately audited"), declaration_for: None,
        });
        true
    }
    fn resolve(&self, path: &str, text: &str) -> Option<String> {
        if !text.split("::").all(|s| {
            !s.is_empty()
                && s.bytes().enumerate().all(|(i, b)| {
                    b == b'_' || b.is_ascii_alphabetic() || (i > 0 && b.is_ascii_digit())
                })
        }) {
            return None;
        }
        let mut parts: Vec<_> = text.split("::").map(str::to_owned).collect();
        let context = self.contexts.get(path)?;
        let prefix = match parts.first()?.as_str() {
            "crate" => {
                parts.remove(0);
                Vec::new()
            }
            "self" => {
                parts.remove(0);
                context.module_segments.clone()
            }
            "super" => {
                let mut prefix = context.module_segments.clone();
                while parts.first().is_some_and(|p| p == "super") {
                    parts.remove(0);
                    prefix.pop()?;
                }
                prefix
            }
            _ => return None,
        };
        let mut result = vec!["crate".into()];
        result.extend(prefix);
        result.extend(parts);
        Some(result.join("::"))
    }
    fn path_repair(&mut self, need: &Need, node: Node<'_>) -> bool {
        if items::check(self.controls.0, self.controls.1).is_err() {
            return false;
        }
        let text = &self.files[&need.path].source[node.byte_range()];
        let Some(old) = self.resolve(&need.path, text) else {
            return false;
        };
        let consumer = self
            .selected
            .iter()
            .find(|(p, i, _)| {
                p == &need.path
                    && i.span.range.start_byte <= node.start_byte()
                    && i.span.range.end_byte >= node.end_byte()
            })
            .map(|(_, _, d)| d.clone())
            .unwrap_or_else(|| need.path.clone());
        let mut bindings = Vec::new();
        for (path, data) in self.parsed {
            let Some(context) = self.contexts.get(path) else {
                continue;
            };
            for item in &data.items {
                if item
                    .name
                    .as_deref()
                    .is_some_and(|name| canonical(context, name) == old)
                {
                    bindings.push((path.clone(), item.clone()));
                }
            }
        }
        if bindings.len() != 1 {
            return false;
        }
        let (path, binding) = &bindings[0];
        let destination = self.final_path(path, binding).to_owned();
        let Some(context) = self.final_contexts.get(&destination) else {
            return false;
        };
        let after = canonical(context, binding.name.as_deref().expect("named"));
        if after != text {
            let range = span(node.start_byte(), node.end_byte());
            let a = anchor(self.files, &need.path, &range);
            self.add(Repair { path: need.path.clone(), range, after, kind: "path", target: RewriteTarget::Source { anchor: a.clone() }, item_ids: need.item_ids.clone(), anchors: vec![a], rationale: "the complete simple path identifies one written declaration in its old ordinary context".into(), declaration_for: None });
        }
        self.visibility(path, binding, &consumer, &need.item_ids)
    }
    fn repair(&mut self, need: &Need) -> bool {
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
        let Some(binding) = self.binding(&need.path, name) else {
            return false;
        };
        let consumer = self
            .selected
            .iter()
            .find(|(p, i, _)| {
                p == &need.path
                    && i.span.range.start_byte <= node.start_byte()
                    && i.span.range.end_byte >= node.end_byte()
            })
            .map(|(_, _, d)| d.clone())
            .unwrap_or_else(|| need.path.clone());
        match items::lexical_binding(node, source, name, self.controls) {
            items::LexicalBinding::Independent => return true,
            items::LexicalBinding::Uncertain => return false,
            items::LexicalBinding::Absent => {}
        }
        let destination = self.final_path(&need.path, &binding).to_owned();
        if destination == consumer {
            return true;
        }
        let Some(context) = self.final_contexts.get(&destination) else {
            return false;
        };
        let target = canonical(context, name);
        let evidence = anchor(self.files, &need.path, &binding.span.range);
        if !self.import(&consumer, name, &target, &need.item_ids, evidence) {
            return false;
        }
        self.visibility(&need.path, &binding, &consumer, &need.item_ids)
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
        controls,
    };
    let mut remaining = Vec::new();
    for need in needs {
        items::check(controls.0, controls.1)?;
        if !analyzer.repair(&need) {
            remaining.push(need);
        }
    }
    Ok(Analysis {
        repairs: analyzer.repairs,
        needs: remaining,
    })
}
