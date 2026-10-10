//! Written glob routes. Module identity is a file plus its inline module anchors,
//! not a globally shared identifier spelling. This is not semantic resolution.
use super::*;

#[derive(Clone)]
pub(crate) enum PatternCompetition {
    Competing,
    Noncompeting(Vec<crate::plan::SourceAnchor>),
    Unknown,
}
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
    contexts: Option<&'a BTreeMap<String, ModuleEvidence>>,
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
            contexts: None,
        })
    }
    pub(crate) fn strict(
        files: &'a BTreeMap<String, FileSnapshot>,
        parsed: &'a BTreeMap<String, ParsedFile>,
        contexts: &'a BTreeMap<String, ModuleEvidence>,
        root: &'a str,
        controls: (Instant, &'a AtomicBool),
    ) -> Result<Self, DomainError> {
        let mut routes = Self::new(files, parsed, root, controls)?;
        routes.contexts = Some(contexts);
        Ok(routes)
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
        self.resolve_traced(module, text, depth, module, &mut Vec::new())
    }
    fn resolve_traced(
        &self,
        module: &Module,
        text: &str,
        depth: usize,
        using: &Module,
        trace: &mut Vec<crate::plan::SourceAnchor>,
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
            if self.contexts.is_some()
                && (!self.admitted(module)? || !self.safe_scope(scope, source, true)?)
            {
                return Ok(None);
            }
            let mut matches = Vec::new();
            for i in 0..scope.named_child_count() {
                check(self.controls.0, self.controls.1)?;
                let node = scope.named_child(i as u32).expect("scope child");
                if node.kind() == "use_declaration" {
                    for leaf in use_leaves(node, source, self.controls)? {
                        if leaf.binding.trim_start_matches("r#") == name {
                            if self.contexts.is_some() {
                                if !self.accessible(module, node, using, source)? {
                                    return Ok(None);
                                }
                                self.trace(module, node, trace)?;
                            }
                            matches.push(self.resolve_traced(
                                module,
                                &leaf.path,
                                depth + 1,
                                using,
                                trace,
                            )?);
                        }
                    }
                } else if node
                    .child_by_field_name("name")
                    .is_some_and(|n| source[n.byte_range()].trim_start_matches("r#") == name)
                {
                    if self.contexts.is_some() {
                        if !self.accessible(module, node, using, source)? {
                            return Ok(None);
                        }
                        self.trace(module, node, trace)?;
                    }
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
                            if paths.len() == 1 && self.contexts.is_some() {
                                let evidence = self.contexts.and_then(|c| c.get(&paths[0]));
                                if !evidence.is_some_and(|e| {
                                    e.unresolved.is_empty()
                                        && e.declaration_anchors.last().is_some_and(|a| {
                                            a.path == module.path
                                                && a.range.start_byte == node.start_byte()
                                                && a.range.end_byte == node.end_byte()
                                                && a.expected_text == source[node.byte_range()]
                                        })
                                }) {
                                    return Ok(None);
                                }
                            }
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
    /// Completeness is deliberately stricter than affected-consumer reachability.
    pub(crate) fn pattern_competition(
        &self,
        path: &str,
        declaration: Node<'_>,
        name: &str,
    ) -> Result<PatternCompetition, DomainError> {
        let module = self.lexical(path, declaration);
        if self.contexts.is_none() || !self.admitted(&module)? {
            return Ok(PatternCompetition::Unknown);
        }
        let source = &self.files[path].source;
        if !self.safe_attributes(declaration, source)? {
            return Ok(PatternCompetition::Unknown);
        }
        let mut trace = Vec::new();
        self.trace(&module, declaration, &mut trace)?;
        let mut stack = vec![(declaration, String::new())];
        let mut found = false;
        while let Some((node, prefix)) = stack.pop() {
            check(self.controls.0, self.controls.1)?;
            match node.kind() {
                "scoped_use_list" => {
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
                }
                "use_wildcard" => {
                    found = true;
                    let part = source[node.byte_range()]
                        .trim()
                        .trim_end_matches('*')
                        .trim_end_matches("::");
                    let route = if part.is_empty() {
                        prefix
                    } else if prefix.is_empty() {
                        part.into()
                    } else {
                        format!("{prefix}::{part}")
                    };
                    // Block-local aliases are outside the module traversal's evidence.
                    let first = route.split("::").next().unwrap_or("");
                    if !matches!(first, "crate" | "self" | "super") {
                        let mut parent = declaration.parent();
                        while let Some(scope) = parent {
                            check(self.controls.0, self.controls.1)?;
                            if scope.kind() == "block" {
                                for i in 0..scope.named_child_count() {
                                    check(self.controls.0, self.controls.1)?;
                                    let child = scope.named_child(i as u32).expect("child");
                                    if child.child_by_field_name("name").is_some_and(|n| {
                                        source[n.byte_range()].trim_start_matches("r#") == first
                                    }) || (child.kind() == "use_declaration"
                                        && use_leaves(child, source, self.controls)?
                                            .iter()
                                            .any(|l| l.binding.trim_start_matches("r#") == first))
                                    {
                                        return Ok(PatternCompetition::Unknown);
                                    }
                                }
                            }
                            if matches!(scope.kind(), "source_file" | "mod_item") {
                                break;
                            }
                            parent = scope.parent();
                        }
                    }
                    let Some(Target::Module(target)) =
                        self.resolve_traced(&module, &route, 0, &module, &mut trace)?
                    else {
                        return Ok(PatternCompetition::Unknown);
                    };
                    if !self.admitted(&target)? {
                        return Ok(PatternCompetition::Unknown);
                    }
                    let data = &self.parsed[&target.path];
                    let Some(scope) = self.scope(&target, &data.tree) else {
                        return Ok(PatternCompetition::Unknown);
                    };
                    let text = &self.files[&target.path].source;
                    if !self.safe_scope(scope, text, false)? {
                        return Ok(PatternCompetition::Unknown);
                    }
                    self.trace(&target, scope, &mut trace)?;
                    for i in 0..scope.named_child_count() {
                        check(self.controls.0, self.controls.1)?;
                        let item = scope.named_child(i as u32).expect("export");
                        let same = item
                            .child_by_field_name("name")
                            .is_some_and(|n| text[n.byte_range()].trim_start_matches("r#") == name);
                        if same
                            && (matches!(item.kind(), "const_item" | "static_item")
                                || (item.kind() == "struct_item"
                                    && item.child_by_field_name("body").is_none_or(|b| {
                                        b.kind() == "ordered_field_declaration_list"
                                    })))
                        {
                            return Ok(PatternCompetition::Competing);
                        }
                    }
                }
                "line_comment" | "block_comment" => {}
                _ => {
                    for i in 0..node.named_child_count() {
                        check(self.controls.0, self.controls.1)?;
                        stack.push((
                            node.named_child(i as u32).expect("use child"),
                            prefix.clone(),
                        ));
                    }
                }
            }
        }
        Ok(if found {
            PatternCompetition::Noncompeting(trace)
        } else {
            PatternCompetition::Unknown
        })
    }
    fn trace(
        &self,
        module: &Module,
        node: Node<'_>,
        trace: &mut Vec<crate::plan::SourceAnchor>,
    ) -> Result<(), DomainError> {
        check(self.controls.0, self.controls.1)?;
        let a = crate::plan::SourceAnchor {
            path: module.path.clone(),
            range: ByteRange {
                start_byte: node.start_byte(),
                end_byte: node.end_byte(),
            },
            expected_text: self.files[&module.path].source[node.byte_range()].into(),
        };
        if trace
            .iter()
            .map(|a| a.expected_text.len() + a.path.len() + 128)
            .sum::<usize>()
            + a.expected_text.len()
            > 128 * 1024 * 1024
            || trace.len() >= 100_000
        {
            return Err(DomainError::new(
                "analysis_descriptor_bytes",
                "glob pattern provenance guard reached",
            ));
        }
        trace.push(a);
        Ok(())
    }
    pub(crate) fn safe_attributes(
        &self,
        node: Node<'_>,
        source: &str,
    ) -> Result<bool, DomainError> {
        let mut prior = node.prev_named_sibling();
        while let Some(p) = prior {
            check(self.controls.0, self.controls.1)?;
            match p.kind() {
                "attribute_item" if !context_independent_attribute(&source[p.byte_range()]) => {
                    return Ok(false);
                }
                "attribute_item" | "line_comment" | "block_comment" => {
                    prior = p.prev_named_sibling()
                }
                _ => break,
            }
        }
        Ok(!node.has_error() && !node.is_missing())
    }
    pub(crate) fn safe_scope(
        &self,
        scope: Node<'_>,
        source: &str,
        aliases: bool,
    ) -> Result<bool, DomainError> {
        if scope.has_error() || scope.is_missing() {
            return Ok(false);
        }
        for i in 0..scope.named_child_count() {
            check(self.controls.0, self.controls.1)?;
            let node = scope.named_child(i as u32).expect("scope child");
            if matches!(node.kind(), "attribute_item" | "inner_attribute_item") {
                if !context_independent_attribute(&source[node.byte_range()]) {
                    return Ok(false);
                }
            } else if !matches!(
                node.kind(),
                "function_item"
                    | "struct_item"
                    | "enum_item"
                    | "union_item"
                    | "type_item"
                    | "trait_item"
                    | "impl_item"
                    | "const_item"
                    | "static_item"
                    | "mod_item"
                    | "line_comment"
                    | "block_comment"
                    | "shebang"
            ) && !(aliases && node.kind() == "use_declaration")
            {
                return Ok(false);
            }
        }
        Ok(true)
    }
    fn segments(&self, module: &Module) -> Option<Vec<String>> {
        let mut names = self.contexts?.get(&module.path)?.module_segments.clone();
        let data = self.parsed.get(&module.path)?;
        for start in &module.inline {
            let mut node = data
                .tree
                .root_node()
                .named_descendant_for_byte_range(*start, *start + 1)?;
            while node.kind() != "mod_item" {
                node = node.parent()?;
            }
            names.push(
                self.files[&module.path].source[node.child_by_field_name("name")?.byte_range()]
                    .trim_start_matches("r#")
                    .into(),
            );
        }
        Some(names)
    }
    fn accessible(
        &self,
        parent: &Module,
        node: Node<'_>,
        using: &Module,
        source: &str,
    ) -> Result<bool, DomainError> {
        let Some(defining) = self.segments(parent) else {
            return Ok(false);
        };
        let Some(using) = self.segments(using) else {
            return Ok(false);
        };
        let region = match visibility_key(node, source, self.controls.0, self.controls.1)? {
            "pub" | "pub(crate)" => return Ok(true),
            "private" => defining,
            _ => match absolute_visibility(node, source, self.controls) {
                Some(region) if defining.starts_with(&region) => region,
                _ => return Ok(false),
            },
        };
        Ok(using.starts_with(&region))
    }
    pub(crate) fn pattern_context(&self, path: &str, node: Node<'_>) -> Result<bool, DomainError> {
        self.admitted(&self.lexical(path, node))
    }
    fn admitted(&self, module: &Module) -> Result<bool, DomainError> {
        let Some(evidence) = self.contexts.and_then(|c| c.get(&module.path)) else {
            return Ok(false);
        };
        if !evidence.unresolved.is_empty() || evidence.crate_root != self.root {
            return Ok(false);
        }
        for a in &evidence.declaration_anchors {
            check(self.controls.0, self.controls.1)?;
            let Some(data) = self.parsed.get(&a.path) else {
                return Ok(false);
            };
            let source = &self.files[&a.path].source;
            let Some(node) = data
                .tree
                .root_node()
                .named_descendant_for_byte_range(a.range.start_byte, a.range.end_byte)
            else {
                return Ok(false);
            };
            if node.kind() != "mod_item"
                || node.child_by_field_name("body").is_some()
                || node.start_byte() != a.range.start_byte
                || node.end_byte() != a.range.end_byte
                || source.get(node.byte_range()) != Some(a.expected_text.as_str())
                || !self.safe_attributes(node, source)?
                || !self.safe_scope(data.tree.root_node(), source, true)?
            {
                return Ok(false);
            }
            let Some(name) = node
                .child_by_field_name("name")
                .map(|n| source[n.byte_range()].trim_start_matches("r#"))
            else {
                return Ok(false);
            };
            let mut count = 0;
            for i in 0..data.tree.root_node().named_child_count() {
                check(self.controls.0, self.controls.1)?;
                let item = data
                    .tree
                    .root_node()
                    .named_child(i as u32)
                    .expect("edge scope item");
                count += usize::from(
                    item.child_by_field_name("name")
                        .is_some_and(|n| source[n.byte_range()].trim_start_matches("r#") == name),
                );
                if item.kind() == "use_declaration" {
                    count += use_leaves(item, source, self.controls)?
                        .iter()
                        .filter(|l| l.binding.trim_start_matches("r#") == name)
                        .count();
                }
            }
            if count != 1 {
                return Ok(false);
            }
        }
        let Some(data) = self.parsed.get(&module.path) else {
            return Ok(false);
        };
        let source = &self.files[&module.path].source;
        if data.tree.root_node().has_error() {
            return Ok(false);
        }
        for i in 0..data.tree.root_node().named_child_count() {
            check(self.controls.0, self.controls.1)?;
            let n = data
                .tree
                .root_node()
                .named_child(i as u32)
                .expect("root child");
            if n.kind() == "inner_attribute_item"
                && !context_independent_attribute(&source[n.byte_range()])
            {
                return Ok(false);
            }
        }
        for start in &module.inline {
            check(self.controls.0, self.controls.1)?;
            let Some(mut node) = data
                .tree
                .root_node()
                .named_descendant_for_byte_range(*start, *start + 1)
            else {
                return Ok(false);
            };
            while node.kind() != "mod_item" {
                let Some(p) = node.parent() else {
                    return Ok(false);
                };
                node = p;
            }
            let Some(scope) = node.parent() else {
                return Ok(false);
            };
            if !self.safe_attributes(node, source)? || !self.safe_scope(scope, source, true)? {
                return Ok(false);
            }
            let Some(name) = node
                .child_by_field_name("name")
                .map(|n| source[n.byte_range()].trim_start_matches("r#"))
            else {
                return Ok(false);
            };
            let mut count = 0;
            for i in 0..scope.named_child_count() {
                check(self.controls.0, self.controls.1)?;
                let item = scope.named_child(i as u32).expect("inline scope item");
                count += usize::from(
                    item.child_by_field_name("name")
                        .is_some_and(|n| source[n.byte_range()].trim_start_matches("r#") == name),
                );
                if item.kind() == "use_declaration" {
                    count += use_leaves(item, source, self.controls)?
                        .iter()
                        .filter(|l| l.binding.trim_start_matches("r#") == name)
                        .count();
                }
            }
            if count != 1 {
                return Ok(false);
            }
        }
        Ok(true)
    }
    /// True includes unknown routes: only a written negative proof permits exclusion.
    fn may_forward_binding(
        &self,
        module: &Module,
        route: &str,
        affected: &[&str],
        name: &str,
        depth: usize,
    ) -> Result<bool, DomainError> {
        check(self.controls.0, self.controls.1)?;
        if depth > 16 {
            return Ok(true);
        }
        let (prefix, binding) = route.rsplit_once("::").unwrap_or(("self", route));
        let binding = binding.trim().trim_start_matches("r#");
        let target = match self.resolve(module, prefix, depth)? {
            Some(Target::Module(target)) => target,
            Some(Target::Enum) => return Ok(false),
            None => return Ok(true),
        };
        if affected.contains(&target.path.as_str()) && binding == name {
            return Ok(true);
        }
        let Some(data) = self.parsed.get(&target.path) else {
            return Ok(true);
        };
        let Some(scope) = self.scope(&target, &data.tree) else {
            return Ok(true);
        };
        if data.tree.root_node().has_error() {
            return Ok(true);
        }
        let source = &self.files[&target.path].source;
        for i in 0..scope.named_child_count() {
            check(self.controls.0, self.controls.1)?;
            let node = scope.named_child(i as u32).expect("scope child");
            if node.kind() == "macro_invocation" {
                return Ok(true);
            }
            if node.kind() == "use_declaration" {
                if use_facts(node, source, self.controls)?.0 {
                    return Ok(true);
                }
                for leaf in use_leaves(node, source, self.controls)? {
                    if leaf.binding.trim_start_matches("r#") == binding
                        && self.may_forward_binding(
                            &target,
                            &leaf.path,
                            affected,
                            name,
                            depth + 1,
                        )?
                    {
                        return Ok(true);
                    }
                }
            }
        }
        Ok(false)
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
    fn named_import_competes(
        &self,
        module: &Module,
        route: &str,
        reference: Node<'_>,
        depth: usize,
    ) -> Result<bool, DomainError> {
        check(self.controls.0, self.controls.1)?;
        if depth > 16 {
            return Ok(false);
        }
        let (prefix, name) = route.rsplit_once("::").unwrap_or(("self", route));
        let Some(Target::Module(target)) = self.resolve(module, prefix, 0)? else {
            return Ok(false);
        };
        let Some(data) = self.parsed.get(&target.path) else {
            return Ok(false);
        };
        let Some(scope) = self.scope(&target, &data.tree) else {
            return Ok(false);
        };
        let source = &self.files[&target.path].source;
        let mut bindings = Vec::new();
        for i in 0..scope.named_child_count() {
            check(self.controls.0, self.controls.1)?;
            let node = scope.named_child(i as u32).expect("scope child");
            if node.kind() == "use_declaration" {
                for leaf in use_leaves(node, source, self.controls)? {
                    if leaf.binding.trim_start_matches("r#") == name.trim_start_matches("r#") {
                        bindings.push(self.named_import_competes(
                            &target,
                            &leaf.path,
                            reference,
                            depth + 1,
                        )?);
                    }
                }
            } else if node.child_by_field_name("name").is_some_and(|n| {
                source[n.byte_range()].trim_start_matches("r#") == name.trim_start_matches("r#")
            }) {
                bindings.push(same_namespace(node, reference));
            }
        }
        Ok(bindings.len() == 1 && bindings[0])
    }
    /// Visible wildcard candidates for a bare consumer. Named bindings take
    /// precedence in their own scope; child modules do not inherit parent uses.
    /// Unknown/forwarded routes remain candidates, never identity proofs.
    pub(crate) fn consumer_imports(
        &self,
        path: &str,
        reference: Node<'_>,
        affected: &str,
        name: &str,
    ) -> Result<Vec<ByteRange>, DomainError> {
        let source = &self.files[path].source;
        let mut imports = Vec::new();
        let mut parent = reference.parent();
        while let Some(scope) = parent {
            check(self.controls.0, self.controls.1)?;
            let module_body = scope.kind() == "declaration_list"
                && scope.parent().is_some_and(|p| p.kind() == "mod_item");
            if matches!(scope.kind(), "block" | "source_file") || module_body {
                let mut globs = Vec::new();
                let mut named = false;
                for i in 0..scope.named_child_count() {
                    check(self.controls.0, self.controls.1)?;
                    let node = scope.named_child(i as u32).expect("scope child");
                    let mut previous = node.prev_named_sibling();
                    let mut attributed = false;
                    while let Some(sibling) = previous {
                        check(self.controls.0, self.controls.1)?;
                        if !matches!(
                            sibling.kind(),
                            "attribute_item" | "line_comment" | "block_comment"
                        ) {
                            break;
                        }
                        attributed |= sibling.kind() == "attribute_item";
                        previous = sibling.prev_named_sibling();
                    }
                    if node.kind() == "use_declaration" {
                        if !attributed {
                            for leaf in use_leaves(node, source, self.controls)? {
                                if leaf.binding.trim_start_matches("r#") == name {
                                    named |= self.named_import_competes(
                                        &self.lexical(path, node),
                                        &leaf.path,
                                        reference,
                                        0,
                                    )?;
                                }
                            }
                        }
                        if use_facts(node, source, self.controls)?.0
                            && self.exclusion(path, node, &[affected], name)?.0.is_none()
                        {
                            globs.push(ByteRange {
                                start_byte: node.start_byte(),
                                end_byte: node.end_byte(),
                            });
                        }
                    } else {
                        // A type-only declaration cannot hide a glob-resolved value.
                        // Attributed declarations cannot prove an active binding.
                        let namespace = same_namespace(node, reference);
                        named |= !attributed
                            && namespace
                            && node.child_by_field_name("name").is_some_and(|n| {
                                source[n.byte_range()].trim_start_matches("r#") == name
                            });
                    }
                }
                if named {
                    return Ok(imports);
                }
                imports.extend(globs);
                if imports.len() > 100_000 {
                    return Err(DomainError::new(
                        "reference_work_limit",
                        "consumer glob evidence cap reached",
                    ));
                }
                if scope.kind() == "source_file" || module_body {
                    break;
                }
            }
            parent = scope.parent();
        }
        Ok(imports)
    }
    pub(crate) fn reaches_file(
        &self,
        path: &str,
        node: Node<'_>,
        target: &str,
    ) -> Result<bool, DomainError> {
        Ok(self.assess(path, node, &[target], None)?.1)
    }
    /// No exclusion means uncertain/reachable. The boolean also retains forwarded
    /// binding candidates whose alias spelling is absent from the conservative seed.
    pub(crate) fn exclusion(
        &self,
        path: &str,
        node: Node<'_>,
        affected: &[&str],
        name: &str,
    ) -> Result<(Option<&'static str>, bool), DomainError> {
        let (reason, _, forwarded) = self.assess(path, node, affected, Some(name))?;
        Ok((reason, forwarded))
    }
    fn assess(
        &self,
        path: &str,
        node: Node<'_>,
        affected: &[&str],
        name: Option<&str>,
    ) -> Result<(Option<&'static str>, bool, bool), DomainError> {
        let source = &self.files[path].source;
        let module = self.lexical(path, node);
        let imported_names = use_facts(node, source, self.controls)?.1;
        let mut stack = vec![(node, String::new())];
        let mut reason = "different_written_module";
        let mut found = false;
        let mut uncertain = false;
        let mut reachable = false;
        let mut forwarded = false;
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
                        // discharge wildcard/re-export, same-name, or explicitly
                        // imported alias chains.
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
                                    if let Some(name) = name {
                                        for leaf in use_leaves(import, text, self.controls)? {
                                            if imported_names
                                                .iter()
                                                .any(|n| n == leaf.binding.trim_start_matches("r#"))
                                                && self.may_forward_binding(
                                                    &target, &leaf.path, affected, name, 0,
                                                )?
                                            {
                                                uncertain = true;
                                                forwarded = true;
                                            }
                                        }
                                    }
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
        Ok((
            (found && !uncertain).then_some(reason),
            reachable,
            forwarded,
        ))
    }
}

fn same_namespace(node: Node<'_>, reference: Node<'_>) -> bool {
    if reference.kind() == "type_identifier" {
        matches!(
            node.kind(),
            "struct_item" | "enum_item" | "union_item" | "type_item" | "trait_item"
        )
    } else {
        matches!(node.kind(), "function_item" | "const_item" | "static_item")
            || (node.kind() == "struct_item"
                && node
                    .child_by_field_name("body")
                    .is_none_or(|b| b.kind() == "ordered_field_declaration_list"))
    }
}

#[cfg(test)]
mod pattern_tests {
    use super::*;
    #[test]
    fn strict_query_never_turns_cancellation_or_depth_exhaustion_into_absence() {
        let cancelled = AtomicBool::new(false);
        let controls = (
            Instant::now() + std::time::Duration::from_secs(10),
            &cancelled,
        );
        let mut files: BTreeMap<String, FileSnapshot> = BTreeMap::new();
        files.insert("lib.rs".into(), FileSnapshot { path: "lib.rs".into(), source: "pub(crate) mod exports; use crate::exports::*; use crate::second as first; use crate::first as second; use crate::first::*;".into(), mode: 0o100644 });
        files.insert(
            "exports.rs".into(),
            FileSnapshot {
                path: "exports.rs".into(),
                source: "pub(crate) const other: u8 = 1;".into(),
                mode: 0o100644,
            },
        );
        let mut parsed = BTreeMap::new();
        let mut observed = 0;
        for (path, file) in &files {
            parsed.insert(
                path.clone(),
                parse(file, 0, controls.0, controls.1, &mut observed).unwrap(),
            );
        }
        let root = ModuleEvidence {
            crate_root: "lib.rs".into(),
            module_segments: Vec::new(),
            declaration_anchors: Vec::new(),
            filesystem_paths: vec!["lib.rs".into()],
            assumptions: Vec::new(),
            unresolved: Vec::new(),
        };
        let mut exporter = root.clone();
        exporter.module_segments.push("exports".into());
        exporter.filesystem_paths.push("exports.rs".into());
        exporter
            .declaration_anchors
            .push(crate::plan::SourceAnchor {
                path: "lib.rs".into(),
                range: parsed["lib.rs"].items[0].span.range.clone(),
                expected_text: "pub(crate) mod exports;".into(),
            });
        let contexts = BTreeMap::from([("lib.rs".into(), root), ("exports.rs".into(), exporter)]);
        let mut routes =
            GlobRoutes::strict(&files, &parsed, &contexts, "lib.rs", controls).unwrap();
        let tree = &parsed["lib.rs"].tree;
        let glob = tree.root_node().named_child(1).unwrap();
        assert!(matches!(
            routes
                .pattern_competition("lib.rs", glob, "binding")
                .unwrap(),
            PatternCompetition::Noncompeting(_)
        ));
        let cycle = tree.root_node().named_child(4).unwrap();
        assert!(matches!(
            routes
                .pattern_competition("lib.rs", cycle, "binding")
                .unwrap(),
            PatternCompetition::Unknown
        ));
        cancelled.store(true, Ordering::Relaxed);
        assert_eq!(
            routes
                .pattern_competition("lib.rs", glob, "binding")
                .err()
                .unwrap()
                .code,
            "CANCELLED"
        );
        cancelled.store(false, Ordering::Relaxed);
        let mut saturated = vec![
            crate::plan::SourceAnchor {
                path: String::new(),
                range: ByteRange {
                    start_byte: 0,
                    end_byte: 0
                },
                expected_text: String::new(),
            };
            100_000
        ];
        assert_eq!(
            routes
                .trace(
                    &Module {
                        path: "lib.rs".into(),
                        inline: Vec::new()
                    },
                    glob,
                    &mut saturated
                )
                .unwrap_err()
                .code,
            "analysis_descriptor_bytes"
        );
        routes.controls = (
            Instant::now() - std::time::Duration::from_secs(1),
            &cancelled,
        );
        assert_eq!(
            routes
                .pattern_competition("lib.rs", glob, "binding")
                .err()
                .unwrap()
                .code,
            "planning_deadline"
        );
    }
}
