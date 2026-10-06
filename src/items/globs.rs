//! Written glob routes. Module identity is a file plus its inline module anchors,
//! not a globally shared identifier spelling. This is not semantic resolution.
use super::*;

#[derive(Clone, PartialEq, Eq)]
struct Module {
    path: String,
    inline: Vec<usize>,
}
#[derive(Clone)]
enum Target {
    Module(Module),
    Enum,
}
pub(crate) struct GlobRoutes<'a> {
    files: &'a BTreeMap<String, FileSnapshot>,
    parsed: &'a BTreeMap<String, ParsedFile>,
    root: &'a str,
    parents: BTreeMap<String, Vec<String>>,
    controls: (Instant, &'a AtomicBool),
}
impl<'a> GlobRoutes<'a> {
    pub(crate) fn new(
        files: &'a BTreeMap<String, FileSnapshot>,
        parsed: &'a BTreeMap<String, ParsedFile>,
        root: &'a str,
        controls: (Instant, &'a AtomicBool),
    ) -> Result<Self, DomainError> {
        let mut parents: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for (path, data) in parsed {
            for item in &data.items {
                check(controls.0, controls.1)?;
                if item.kind != "mod_item" {
                    continue;
                }
                let node = data
                    .tree
                    .root_node()
                    .named_descendant_for_byte_range(
                        item.span.range.start_byte,
                        item.span.range.end_byte,
                    )
                    .expect("module");
                // Track literal path inclusions as competing/uncertain parents;
                // they are never positive default-layout evidence.
                for attribute in &item.attributes {
                    let text =
                        &files[path].source[attribute.range.start_byte..attribute.range.end_byte];
                    if let Some(value) = text
                        .strip_prefix("#[path")
                        .and_then(|s| s.strip_suffix(']'))
                        .and_then(|s| s.trim().strip_prefix('='))
                        && let Ok(relative) = serde_json::from_str::<String>(value.trim())
                    {
                        let target = Path::new(path)
                            .parent()
                            .unwrap_or(Path::new(""))
                            .join(relative);
                        if let Some(target) = target.to_str() {
                            // Duplicate edges deliberately prevent parent proof.
                            parents
                                .entry(target.into())
                                .or_default()
                                .extend([path.clone(), path.clone()]);
                        }
                    }
                }
                if node.child_by_field_name("body").is_some()
                    || data.tree.root_node().has_error()
                    || item.attributes.iter().any(|a| {
                        let text = &files[path].source[a.range.start_byte..a.range.end_byte];
                        !text.starts_with("#[cfg(")
                    })
                {
                    continue;
                }
                let Some(name) = &item.name else { continue };
                // A written declaration fixes its default child location even if cfg is
                // unevaluated. Library/binary entry files use their directory. This
                // reverse edge proves only relative parent identity, never crate identity.
                let base = if Path::new(path)
                    .file_name()
                    .is_some_and(|n| n == "lib.rs" || n == "main.rs")
                {
                    path.as_str()
                } else {
                    root
                };
                let flat = child_path(path, base, name.trim_start_matches("r#"));
                let legacy = format!("{}/mod.rs", flat.trim_end_matches(".rs"));
                for child in [flat, legacy] {
                    if parsed.contains_key(&child) {
                        parents.entry(child).or_default().push(path.clone());
                    }
                }
            }
        }
        Ok(Self {
            files,
            parsed,
            root,
            parents,
            controls,
        })
    }
    fn scope<'b>(&self, module: &Module, tree: &'b Tree) -> Option<Node<'b>> {
        if let Some(start) = module.inline.last() {
            let node = tree
                .root_node()
                .named_descendant_for_byte_range(*start, *start + 1)?;
            let mut node = node;
            while node.kind() != "mod_item" {
                node = node.parent()?;
            }
            node.child_by_field_name("body")
        } else {
            Some(tree.root_node())
        }
    }
    fn parent(&self, module: &Module) -> Option<Module> {
        let mut parent = module.clone();
        if parent.inline.pop().is_some() {
            return Some(parent);
        }
        let parents = self.parents.get(&module.path)?;
        if parents.len() != 1 {
            return None;
        }
        parent.path = parents[0].clone();
        Some(parent)
    }
    fn resolve(
        &self,
        module: &Module,
        text: &str,
        depth: usize,
    ) -> Result<Option<Target>, DomainError> {
        check(self.controls.0, self.controls.1)?;
        if depth > 16 {
            return Ok(None);
        }
        let parts: Vec<_> = text
            .split("::")
            .map(|s| s.trim().trim_start_matches("r#"))
            .collect();
        let Some(first) = parts.first() else {
            return Ok(None);
        };
        let (mut target, mut index) = match *first {
            "crate" => (
                Target::Module(Module {
                    path: self.root.into(),
                    inline: Vec::new(),
                }),
                1,
            ),
            "self" => (Target::Module(module.clone()), 1),
            "super" => {
                let mut parent = module.clone();
                let mut index = 0;
                while parts.get(index) == Some(&"super") {
                    let Some(next) = self.parent(&parent) else {
                        return Ok(None);
                    };
                    parent = next;
                    index += 1;
                }
                (Target::Module(parent), index)
            }
            _ => (Target::Module(module.clone()), 0),
        };
        while index < parts.len() {
            check(self.controls.0, self.controls.1)?;
            let Target::Module(ref module) = target else {
                return Ok(None);
            };
            let Some(data) = self.parsed.get(&module.path) else {
                return Ok(None);
            };
            if data.tree.root_node().has_error() {
                return Ok(None);
            }
            let source = &self.files[&module.path].source;
            let Some(scope) = self.scope(module, &data.tree) else {
                return Ok(None);
            };
            let name = parts[index];
            let mut matches = Vec::new();
            for i in 0..scope.named_child_count() {
                check(self.controls.0, self.controls.1)?;
                let node = scope.named_child(i as u32).expect("scope child");
                if node.kind() == "use_declaration" {
                    for leaf in use_leaves(node, source, self.controls)? {
                        if leaf.binding.trim_start_matches("r#") == name {
                            matches.push(self.resolve(module, &leaf.path, depth + 1)?);
                        }
                    }
                } else if node
                    .child_by_field_name("name")
                    .is_some_and(|n| source[n.byte_range()].trim_start_matches("r#") == name)
                {
                    let value = match node.kind() {
                        "enum_item" => Some(Target::Enum),
                        "mod_item" if node.child_by_field_name("body").is_some() => {
                            let mut next = module.clone();
                            next.inline.push(node.start_byte());
                            Some(Target::Module(next))
                        }
                        "mod_item"
                            if node.prev_named_sibling().is_some_and(|p| {
                                p.kind() == "attribute_item"
                                    && !source[p.byte_range()].starts_with("#[cfg(")
                            }) =>
                        {
                            None
                        }
                        "mod_item" => {
                            let flat = child_path(&module.path, self.root, name);
                            let legacy = format!("{}/mod.rs", flat.trim_end_matches(".rs"));
                            let paths: Vec<_> = [flat, legacy]
                                .into_iter()
                                .filter(|p| self.parsed.contains_key(p))
                                .collect();
                            (paths.len() == 1).then(|| {
                                Target::Module(Module {
                                    path: paths[0].clone(),
                                    inline: Vec::new(),
                                })
                            })
                        }
                        _ => None,
                    };
                    matches.push(value);
                }
            }
            if matches.len() != 1 {
                return Ok(None);
            }
            let Some(next) = matches.pop().expect("one match") else {
                return Ok(None);
            };
            target = next;
            index += 1;
        }
        Ok(Some(target))
    }
    fn lexical(&self, path: &str, node: Node<'_>) -> Module {
        let mut inline = Vec::new();
        let mut parent = node.parent();
        while let Some(p) = parent {
            if p.kind() == "mod_item" {
                inline.push(p.start_byte());
            }
            parent = p.parent();
        }
        inline.reverse();
        Module {
            path: path.into(),
            inline,
        }
    }
    pub(crate) fn reaches_file(
        &self,
        path: &str,
        node: Node<'_>,
        target: &str,
    ) -> Result<bool, DomainError> {
        Ok(self.assess(path, node, &[target], None)?.1)
    }
    /// None means uncertain/reachable, and must not discharge a candidate.
    pub(crate) fn exclusion(
        &self,
        path: &str,
        node: Node<'_>,
        affected: &[&str],
        name: &str,
    ) -> Result<Option<&'static str>, DomainError> {
        Ok(self.assess(path, node, affected, Some(name))?.0)
    }
    fn assess(
        &self,
        path: &str,
        node: Node<'_>,
        affected: &[&str],
        name: Option<&str>,
    ) -> Result<(Option<&'static str>, bool), DomainError> {
        let source = &self.files[path].source;
        let module = self.lexical(path, node);
        let mut stack = vec![(node, String::new())];
        let mut reason = "different_written_module";
        let mut found = false;
        let mut uncertain = false;
        let mut reachable = false;
        while let Some((node, prefix)) = stack.pop() {
            check(self.controls.0, self.controls.1)?;
            if node.kind() == "scoped_use_list" {
                let part = node
                    .child_by_field_name("path")
                    .map(|n| &source[n.byte_range()])
                    .unwrap_or("");
                let next = if prefix.is_empty() {
                    part.into()
                } else {
                    format!("{prefix}::{part}")
                };
                stack.push((node.child_by_field_name("list").expect("list"), next));
            } else if node.kind() == "use_wildcard" {
                found = true;
                let text = source[node.byte_range()]
                    .trim()
                    .trim_end_matches('*')
                    .trim_end_matches("::");
                let text = if text.is_empty() {
                    prefix.clone()
                } else if prefix.is_empty() {
                    text.into()
                } else {
                    format!("{prefix}::{text}")
                };
                let first = text.split("::").next().unwrap_or("");
                let mut local_shadow = false;
                let mut parent = node.parent();
                while let Some(scope) = parent {
                    if matches!(scope.kind(), "block" | "declaration_list" | "source_file") {
                        if scope.kind() != "block" {
                            break;
                        }
                        for i in 0..scope.named_child_count() {
                            check(self.controls.0, self.controls.1)?;
                            let child = scope.named_child(i as u32).expect("block child");
                            local_shadow |= child
                                .child_by_field_name("name")
                                .is_some_and(|n| &source[n.byte_range()] == first);
                            if child.kind() == "use_declaration" {
                                local_shadow |= use_leaves(child, source, self.controls)?
                                    .iter()
                                    .any(|l| l.binding == first);
                            }
                        }
                    }
                    parent = scope.parent();
                }
                if local_shadow {
                    uncertain = true;
                    continue;
                }
                match self.resolve(&module, &text, 0)? {
                    Some(Target::Enum) => {
                        reason = "enum_variants";
                    }
                    Some(Target::Module(target)) => {
                        reachable |=
                            affected.contains(&target.path.as_str()) && target.inline.is_empty();
                        uncertain |= affected.contains(&target.path.as_str());
                        // A different module may forward the moved binding. Do not
                        // discharge wildcard/re-export or same-name import chains.
                        if let Some(data) = self.parsed.get(&target.path)
                            && let Some(scope) = self.scope(&target, &data.tree)
                        {
                            let text = &self.files[&target.path].source;
                            for i in 0..scope.named_child_count() {
                                check(self.controls.0, self.controls.1)?;
                                let import = scope.named_child(i as u32).expect("scope child");
                                if import.kind() == "use_declaration" {
                                    uncertain |= use_facts(import, text, self.controls)?.0
                                        || visibility_key(
                                            import,
                                            text,
                                            self.controls.0,
                                            self.controls.1,
                                        )? != "private"
                                        || use_leaves(import, text, self.controls)?
                                            .iter()
                                            .any(|l| name == Some(l.binding.as_str()));
                                }
                            }
                        }
                    }
                    None => {
                        // A caller-selected file root has no written child route for
                        // this absolute base. Do not turn all unrelated crate imports
                        // into aliases of that root. This is written coverage only.
                        let first = text
                            .strip_prefix("crate::")
                            .and_then(|s| s.split("::").next());
                        let mut absent_root_child = false;
                        if let (Some(first), Some(data)) = (first, self.parsed.get(self.root)) {
                            let root_source = &self.files[self.root].source;
                            let mut bound = data.tree.root_node().has_error();
                            for item in &data.items {
                                check(self.controls.0, self.controls.1)?;
                                bound |= item.name.as_deref() == Some(first)
                                    || item.kind == "macro_invocation";
                                if item.kind == "use_declaration" {
                                    let import = data
                                        .tree
                                        .root_node()
                                        .named_descendant_for_byte_range(
                                            item.span.range.start_byte,
                                            item.span.range.end_byte,
                                        )
                                        .expect("use");
                                    bound |= use_facts(import, root_source, self.controls)?.0
                                        || use_leaves(import, root_source, self.controls)?
                                            .iter()
                                            .any(|l| l.binding == first);
                                }
                            }
                            absent_root_child = !bound;
                        }
                        if absent_root_child {
                            reason = "no_written_route";
                        } else {
                            uncertain = true;
                        }
                    }
                }
            } else {
                for i in 0..node.named_child_count() {
                    stack.push((
                        node.named_child(i as u32).expect("use child"),
                        prefix.clone(),
                    ));
                }
            }
        }
        Ok(((found && !uncertain).then_some(reason), reachable))
    }
}
