//! Written top-level inventory and ordinary module evidence. No manifest or semantic resolver.
use crate::{
    matching::Lines,
    result::{ByteRange, DomainError, SourceSlice, SyntaxFlags},
    scope::{FileSnapshot, Scope},
    trivia,
};
use rmcp::schemars::JsonSchema;
use serde::Serialize;
use std::{
    collections::BTreeMap,
    fs,
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
    time::Instant,
};
use tree_sitter::{Node, Tree};

#[derive(Debug, Clone, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct Item {
    pub id: String,
    pub path: String,
    pub span: SourceSlice,
    pub kind: String,
    pub name: Option<String>,
    pub visibility: String,
    pub attributes: Vec<SourceSlice>,
    pub trivia: Vec<SourceSlice>,
    pub bytes: usize,
    pub lines: usize,
    pub syntax: SyntaxFlags,
    pub eligibility: String,
    pub reasons: Vec<String>,
    pub signal_ids: Vec<String>,
}
pub struct ParsedFile {
    pub tree: Tree,
    pub items: Vec<Item>,
    pub trivia: Vec<trivia::Trivia>,
}
pub fn check(deadline: Instant, cancelled: &AtomicBool) -> Result<(), DomainError> {
    if cancelled.load(Ordering::Relaxed) {
        Err(DomainError::new("CANCELLED", "request cancelled"))
    } else if Instant::now() >= deadline {
        Err(DomainError::new(
            "planning_deadline",
            "narrow the analysis scope",
        ))
    } else {
        Ok(())
    }
}
pub fn category(kind: &str) -> Option<&'static str> {
    match kind {
        "function_item" | "struct_item" | "enum_item" | "union_item" | "trait_item"
        | "impl_item" | "type_item" | "const_item" | "static_item" => None,
        "mod_item" => Some("module_context"),
        "use_declaration" | "extern_crate_declaration" | "foreign_mod_item" => {
            Some("scope_dependency")
        }
        "macro_definition" | "macro_invocation" | "expression_statement" => {
            Some("macro_dependency")
        }
        _ => Some("unsupported_dependency_form"),
    }
}
pub fn parse(
    file: &FileSnapshot,
    text_bytes: usize,
    deadline: Instant,
    cancelled: &AtomicBool,
    observed: &mut usize,
) -> Result<ParsedFile, DomainError> {
    let tree = trivia::parse(&file.source, deadline, cancelled)?
        .ok_or_else(|| DomainError::new("planning_deadline", "inventory parse stopped"))?;
    let trivia = trivia::move_inventory(&tree, &file.source, deadline, cancelled)?;
    let lines = Lines::new(&file.source);
    let mut items = Vec::new();
    for i in 0..tree.root_node().named_child_count() {
        check(deadline, cancelled)?;
        let node = tree.root_node().named_child(i as u32).expect("child");
        if matches!(
            node.kind(),
            "line_comment"
                | "block_comment"
                | "attribute_item"
                | "inner_attribute_item"
                | "shebang"
        ) {
            continue;
        }
        *observed += 1;
        if *observed > 100_000 {
            return Err(DomainError::new(
                "inventory_work_limit",
                "whole-call inventory descriptor cap reached",
            ));
        }
        let mut associated = Vec::new();
        for t in &trivia {
            check(deadline, cancelled)?;
            if t.owned_by(node.byte_range())
                || (t.range.start >= node.start_byte() && t.range.end <= node.end_byte())
            {
                associated.push(t);
            }
        }
        let span = lines.slice(node.start_byte(), node.end_byte(), text_bytes);
        let visibility = (0..node.named_child_count())
            .filter_map(|i| node.named_child(i as u32))
            .find(|n| n.kind() == "visibility_modifier")
            .map(|n| file.source[n.byte_range()].to_owned())
            .unwrap_or_else(|| "private".into());
        items.push(Item {
            id: format!("i/{}/{}/{}", file.path, node.start_byte(), node.end_byte()),
            path: file.path.clone(),
            bytes: node.end_byte() - node.start_byte(),
            lines: span.end.line - span.start.line + 1,
            span,
            kind: node.kind().into(),
            name: node
                .child_by_field_name("name")
                .map(|n| file.source[n.byte_range()].into()),
            visibility,
            attributes: associated
                .iter()
                .filter(|t| t.is_attribute())
                .map(|t| lines.slice(t.range.start, t.range.end, text_bytes))
                .collect(),
            trivia: associated
                .iter()
                .map(|t| lines.slice(t.range.start, t.range.end, text_bytes))
                .collect(),
            syntax: SyntaxFlags {
                file_has_recovery: tree.root_node().has_error(),
                subtree_has_recovery: node.has_error(),
                enclosing_has_recovery: tree.root_node().has_error(),
            },
            eligibility: if category(node.kind()).is_none() {
                "supported_unit"
            } else {
                "context_sensitive"
            }
            .into(),
            reasons: category(node.kind())
                .into_iter()
                .map(str::to_owned)
                .collect(),
            signal_ids: Vec::new(),
        });
    }
    Ok(ParsedFile {
        tree,
        items,
        trivia,
    })
}
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct ModuleEvidence {
    pub crate_root: String,
    pub module_segments: Vec<String>,
    pub declaration_anchors: Vec<crate::plan::SourceAnchor>,
    pub filesystem_paths: Vec<String>,
    pub assumptions: Vec<String>,
    pub unresolved: Vec<String>,
}
pub fn child_path(parent: &str, root: &str, name: &str) -> String {
    let parent = Path::new(parent);
    let directory = parent.parent().unwrap_or(Path::new(""));
    let base = if parent == Path::new(root) || parent.file_name().is_some_and(|n| n == "mod.rs") {
        directory.to_path_buf()
    } else {
        directory.join(parent.file_stem().expect("Rust file"))
    };
    base.join(format!("{name}.rs"))
        .to_str()
        .expect("UTF-8 path")
        .into()
}
pub fn module_name(path: &str) -> Result<&str, DomainError> {
    let name = Path::new(path)
        .file_stem()
        .and_then(|n| n.to_str())
        .unwrap_or("");
    const KEYWORDS: &str = "as async await break const continue crate dyn else enum extern false fn for if impl in let loop match mod move mut pub ref return self Self static struct super trait true type unsafe use where while abstract become box do final gen macro override priv try typeof unsized virtual yield macro_rules raw safe union";
    if name == "_"
        || name == "mod"
        || name.is_empty()
        || !name
            .bytes()
            .enumerate()
            .all(|(i, b)| b == b'_' || b.is_ascii_alphabetic() || (i > 0 && b.is_ascii_digit()))
        || KEYWORDS.split_whitespace().any(|k| k == name)
    {
        return Err(DomainError::new(
            "INVALID_NEW_FILE_NAME",
            "new basename must be a non-keyword ASCII Rust identifier, not _ or mod.rs",
        ));
    }
    Ok(name)
}
pub fn present(scope: &Scope, path: &str) -> Result<bool, DomainError> {
    match fs::symlink_metadata(scope.root.join(path)) {
        Ok(_) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(_) => Err(DomainError::new(
            "scan_incomplete",
            "cannot observe module layout",
        )),
    }
}
/// Follow only exact unconditioned declarations. A dangling declaration is recorded at its parent;
/// it does not manufacture a file/context and can be reused by an explicit creation.
pub fn modules(
    scope: &Scope,
    root: &str,
    files: &BTreeMap<String, FileSnapshot>,
    parsed: &BTreeMap<String, ParsedFile>,
    deadline: Instant,
    cancelled: &AtomicBool,
) -> Result<BTreeMap<String, ModuleEvidence>, DomainError> {
    let mut out = BTreeMap::new();
    let mut queue = vec![(root.to_owned(), ModuleEvidence {
        crate_root: root.into(), module_segments: Vec::new(), declaration_anchors: Vec::new(), filesystem_paths: vec![root.into()],
        assumptions: vec!["caller-selected root; not an active Cargo target; admitted-scope references only; build configuration/macros unexamined".into()], unresolved: Vec::new(),
    })];
    while let Some((path, mut evidence)) = queue.pop() {
        check(deadline, cancelled)?;
        if let Some(prior) = out.get_mut(&path) {
            let prior: &mut ModuleEvidence = prior;
            prior
                .unresolved
                .push("multiple inclusion contexts/cycle".into());
            continue;
        }
        let Some(data) = parsed.get(&path) else {
            continue;
        };
        let file = &files[&path];
        if data.tree.root_node().has_error() {
            evidence
                .unresolved
                .push("module evidence contains syntax recovery".into());
        }
        if data.trivia.iter().any(|t| {
            t.is_attribute()
                && t.classification == "scope"
                && !file.source[t.range.clone()].starts_with("#![allow(")
        }) {
            evidence
                .unresolved
                .push("inherited scope attributes require an explicit context choice".into());
        }
        let mut declarations: BTreeMap<String, Vec<&Item>> = BTreeMap::new();
        for item in &data.items {
            check(deadline, cancelled)?;
            if item.kind == "mod_item"
                && let Some(name) = &item.name
            {
                declarations.entry(name.clone()).or_default().push(item);
            }
        }
        for (name, declarations) in declarations {
            check(deadline, cancelled)?;
            let item = declarations[0];
            let node = data
                .tree
                .root_node()
                .named_descendant_for_byte_range(
                    item.span.range.start_byte,
                    item.span.range.end_byte,
                )
                .expect("item");
            if declarations.len() != 1
                || !item.attributes.is_empty()
                || node.child_by_field_name("body").is_some()
            {
                // Localized: unrelated attributed/inline modules don't veto other ordinary chains.
                continue;
            }
            let flat = child_path(&path, root, name.trim_start_matches("r#"));
            let legacy = format!("{}/mod.rs", flat.trim_end_matches(".rs"));
            let a = present(scope, &flat)?;
            let b = present(scope, &legacy)?;
            let child = if a && !b {
                flat
            } else if b && !a {
                legacy
            } else {
                continue;
            };
            if !files.contains_key(&child) {
                continue;
            }
            let mut next = evidence.clone();
            next.module_segments.push(name);
            next.declaration_anchors.push(crate::plan::SourceAnchor {
                path: path.clone(),
                range: item.span.range.clone(),
                expected_text: file.source[item.span.range.start_byte..item.span.range.end_byte]
                    .into(),
            });
            next.filesystem_paths.push(child.clone());
            queue.push((child, next));
        }
        out.insert(path, evidence);
    }
    // Propagate observed competing contexts to descendants already traversed.
    let uncertain: Vec<_> = out
        .iter()
        .filter(|(_, e)| !e.unresolved.is_empty())
        .map(|(p, _)| p.clone())
        .collect();
    for evidence in out.values_mut() {
        check(deadline, cancelled)?;
        if evidence
            .filesystem_paths
            .iter()
            .any(|p| uncertain.contains(p))
            && evidence.unresolved.is_empty()
        {
            evidence
                .unresolved
                .push("ancestor has competing/recovered context".into());
        }
    }
    Ok(out)
}

#[derive(Clone, Serialize)]
pub struct Need {
    pub category: &'static str,
    pub path: String,
    pub range: ByteRange,
    pub message: String,
    pub item_ids: Vec<String>,
}
fn need(category: &'static str, path: &str, node: Node<'_>, message: &str) -> Need {
    Need {
        category,
        path: path.into(),
        range: ByteRange {
            start_byte: node.start_byte(),
            end_byte: node.end_byte(),
        },
        message: message.into(),
        item_ids: Vec::new(),
    }
}
fn declaration_name(node: Node<'_>) -> bool {
    node.parent().is_some_and(|p| {
        p.child_by_field_name("name") == Some(node)
            && matches!(
                p.kind(),
                "function_item"
                    | "function_signature_item"
                    | "struct_item"
                    | "enum_item"
                    | "enum_variant"
                    | "trait_item"
                    | "union_item"
                    | "type_item"
                    | "associated_type"
                    | "const_item"
                    | "static_item"
                    | "type_parameter"
                    | "const_parameter"
                    | "mod_item"
                    | "extern_crate_declaration"
            )
    })
}
fn binding_pattern(node: Node<'_>, source: &str, name: &str) -> bool {
    if node.kind() == "identifier" {
        return source[node.byte_range()].trim_start_matches("r#") == name;
    }
    if matches!(node.kind(), "mut_pattern" | "reference_pattern") {
        return (0..node.named_child_count())
            .filter_map(|i| node.named_child(i as u32))
            .any(|n| binding_pattern(n, source, name));
    }
    false
}
/// Resolve only written lexical bindings in containing scopes, never a same-spelled name elsewhere.
fn local(
    node: Node<'_>,
    item: Node<'_>,
    source: &str,
    name: &str,
    deadline: Instant,
    cancelled: &AtomicBool,
) -> bool {
    let mut child = node;
    while let Some(parent) = child.parent() {
        if check(deadline, cancelled).is_err() {
            return false;
        }
        if let Some(params) = parent.child_by_field_name("parameters") {
            for i in 0..params.named_child_count() {
                if check(deadline, cancelled).is_err() {
                    return false;
                }
                if let Some(p) = params
                    .named_child(i as u32)
                    .and_then(|p| p.child_by_field_name("pattern"))
                    && binding_pattern(p, source, name)
                {
                    return true;
                }
            }
        }
        if let Some(params) = parent.child_by_field_name("type_parameters") {
            for i in 0..params.named_child_count() {
                if check(deadline, cancelled).is_err() {
                    return false;
                }
                if params
                    .named_child(i as u32)
                    .and_then(|p| p.child_by_field_name("name"))
                    .is_some_and(|n| source[n.byte_range()].trim_start_matches("r#") == name)
                {
                    return true;
                }
            }
        }
        if parent.kind() == "block" {
            for i in 0..parent.named_child_count() {
                if check(deadline, cancelled).is_err() {
                    return false;
                }
                let statement = parent.named_child(i as u32).expect("child");
                if statement.end_byte() <= child.start_byte()
                    && statement.kind() == "let_declaration"
                    && statement
                        .child_by_field_name("pattern")
                        .is_some_and(|p| binding_pattern(p, source, name))
                {
                    return true;
                }
                if statement.kind() == "function_item"
                    && statement
                        .child_by_field_name("name")
                        .is_some_and(|n| source[n.byte_range()].trim_start_matches("r#") == name)
                {
                    return true;
                }
            }
        }
        if parent == item {
            break;
        }
        child = parent;
    }
    false
}
/// Conservative relocation guard: dependencies requiring a repair are evidence, not edits.
/// `final_paths` is simultaneous membership; co-moved written declarations can remain bound.
pub fn dependencies(
    files: &BTreeMap<String, FileSnapshot>,
    parsed: &BTreeMap<String, ParsedFile>,
    selected: &[(String, Item, String)],
    contexts: &BTreeMap<String, ModuleEvidence>,
    controls: (Instant, &AtomicBool),
    candidates: &mut usize,
    analysis_bytes: &mut usize,
) -> Result<(Vec<Need>, usize), DomainError> {
    let (deadline, cancelled) = controls;
    let mut needs = Vec::new();
    let mut seen_needs = 0;
    for (index, (path, item, destination)) in selected.iter().enumerate() {
        check(deadline, cancelled)?;
        if let Some(name) = &item.name {
            let name = name.trim_start_matches("r#");
            for (_, other, other_destination) in &selected[..index] {
                check(deadline, cancelled)?;
                bound_needs(&needs, &mut seen_needs, analysis_bytes)?;
                if destination == other_destination
                    && other.name.as_deref().map(|n| n.trim_start_matches("r#")) == Some(name)
                {
                    let node = parsed[path]
                        .tree
                        .root_node()
                        .named_descendant_for_byte_range(
                            item.span.range.start_byte,
                            item.span.range.end_byte,
                        )
                        .expect("item");
                    let mut collision = need(
                        "binding_collision",
                        path,
                        node,
                        "simultaneous arrivals introduce the same written binding at one destination",
                    );
                    collision.item_ids = vec![item.id.clone(), other.id.clone()];
                    needs.push(collision);
                }
            }
        }
    }
    for (source_path, item, destination) in selected {
        check(deadline, cancelled)?;
        let need_start = needs.len();
        let data = &parsed[source_path];
        let source = &files[source_path].source;
        let node = data
            .tree
            .root_node()
            .named_descendant_for_byte_range(item.span.range.start_byte, item.span.range.end_byte)
            .expect("item");
        if let Some(category) = category(&item.kind) {
            needs.push(need(
                category,
                source_path,
                node,
                "this whole unit changes context; choose a supported declaration kind",
            ));
            needs
                .last_mut()
                .expect("need")
                .item_ids
                .push(item.id.clone());
            continue;
        }
        if let Some(name) = item.name.as_deref()
            && data
                .items
                .iter()
                .filter(|other| {
                    other.name.as_deref().map(|n| n.trim_start_matches("r#"))
                        == Some(name.trim_start_matches("r#"))
                })
                .count()
                > 1
        {
            needs.push(need(
                "binding_collision",
                source_path,
                node,
                "source has several same-named written declarations; binding identity is ambiguous",
            ));
        }
        if item.visibility.starts_with("pub(") && item.visibility != "pub(crate)" {
            needs.push(need("visibility_context", source_path, node, "restricted visibility changes lexical scope; explicit repair is outside dependency-free moves"));
        }
        let publicly_exposed = item.visibility == "pub"
            && contexts.get(source_path).is_some_and(|e| {
                e.declaration_anchors
                    .iter()
                    .all(|a| a.expected_text.starts_with("pub "))
            });
        if publicly_exposed {
            needs.push(need("reexport_dependency", source_path, node, "observed public path changes; choose a non-exposed item or an explicit API decision"));
        }
        for t in &data.trivia {
            check(deadline, cancelled)?;
            if t.is_attribute()
                && (t.owned_by(node.byte_range())
                    || t.classification == "scope"
                    || node.byte_range().contains(&t.range.start))
            {
                let text = &source[t.range.clone()];
                // Only built-in context-independent attributes are admitted; proc attributes/cfg block.
                if !(text.starts_with("#[allow(")
                    || text.starts_with("#![allow(")
                    || text == "#[inline]"
                    || text == "#[inline(always)]"
                    || text == "#[inline(never)]"
                    || text.starts_with("#[repr("))
                {
                    needs.push(need("scope_dependency", source_path, node, "conditional, inherited or unexamined attribute context requires a supported explicit choice"));
                }
            }
        }
        // A linear absence check avoids searching every lexical block for names that
        // cannot be local. A positive spelling still goes through the scoped checker.
        let mut possible_locals = std::collections::BTreeSet::new();
        let mut stack = vec![node];
        while let Some(current) = stack.pop() {
            check(deadline, cancelled)?;
            if matches!(current.kind(), "identifier" | "type_identifier")
                && (declaration_name(current)
                    || current.parent().is_some_and(|p| {
                        p.child_by_field_name("pattern") == Some(current)
                            || matches!(p.kind(), "mut_pattern" | "reference_pattern")
                    }))
            {
                possible_locals.insert(source[current.byte_range()].trim_start_matches("r#"));
            }
            if matches!(
                current.kind(),
                "line_comment"
                    | "block_comment"
                    | "string_literal"
                    | "raw_string_literal"
                    | "token_tree"
            ) {
                continue;
            }
            for i in (0..current.named_child_count()).rev() {
                stack.push(current.named_child(i as u32).expect("child"));
            }
        }
        *analysis_bytes += possible_locals.iter().map(|s| s.len() + 64).sum::<usize>();
        let mut stack = vec![node];
        while let Some(current) = stack.pop() {
            check(deadline, cancelled)?;
            bound_needs(&needs, &mut seen_needs, analysis_bytes)?;
            let kind = current.kind();
            if matches!(
                kind,
                "line_comment"
                    | "block_comment"
                    | "attribute_item"
                    | "inner_attribute_item"
                    | "string_literal"
                    | "raw_string_literal"
                    | "char_literal"
            ) {
                continue;
            }
            if matches!(kind, "macro_invocation" | "macro_definition") {
                let text = &source[current.byte_range()];
                let category = if ["include!", "include_str!", "include_bytes!"]
                    .iter()
                    .any(|prefix| text.starts_with(prefix))
                {
                    "scope_dependency"
                } else {
                    "macro_dependency"
                };
                needs.push(need(category, source_path, current, "macro expansion or file-relative include context is unexamined; no token-tree rewrite"));
                continue;
            }
            if matches!(
                kind,
                "scoped_identifier"
                    | "scoped_type_identifier"
                    | "qualified_type"
                    | "use_declaration"
            ) {
                needs.push(need(
                    "unsupported_dependency_form",
                    source_path,
                    current,
                    "path/import context needs a repair or explicit binding evidence",
                ));
                continue;
            }
            if kind == "field_expression" {
                needs.push(need("visibility_context", source_path, current, "member/method access and trait-import context are not established syntactically"));
            }
            if matches!(
                kind,
                "closure_expression"
                    | "for_expression"
                    | "match_expression"
                    | "if_let_expression"
                    | "while_let_expression"
            ) {
                needs.push(need(
                    "unsupported_dependency_form",
                    source_path,
                    current,
                    "lexical pattern context outside the supported binding checker",
                ));
            }
            let is_pattern = current
                .parent()
                .is_some_and(|p| p.child_by_field_name("pattern") == Some(current));
            if is_pattern && matches!(kind, "identifier" | "mut_pattern" | "reference_pattern") {
                continue;
            }
            if matches!(kind, "identifier" | "type_identifier") && !declaration_name(current) {
                *candidates += 1;
                if *candidates > 100_000 {
                    return Err(DomainError::new(
                        "reference_work_limit",
                        "reference candidate cap reached",
                    ));
                }
                let name = source[current.byte_range()].trim_start_matches("r#");
                let self_context =
                    name == "Self" && matches!(item.kind.as_str(), "impl_item" | "trait_item");
                let own = item.name.as_deref().map(|n| n.trim_start_matches("r#")) == Some(name);
                let bound = possible_locals.contains(name)
                    && local(current, node, source, name, deadline, cancelled);
                let co_moved = selected.iter().any(|(p, other, dest)| {
                    p == source_path
                        && dest == destination
                        && other.name.as_deref().map(|n| n.trim_start_matches("r#")) == Some(name)
                });
                if !self_context && !own && !bound && !co_moved {
                    let glob = data.items.iter().any(|i| {
                        i.kind == "use_declaration"
                            && source[i.span.range.start_byte..i.span.range.end_byte].contains('*')
                    });
                    needs.push(need(if glob { "glob_dependency" } else { "unsupported_dependency_form" }, source_path, current, "written bare dependency is not retained in the final scope; explicit import/path repair required"));
                }
            }
            for i in (0..current.named_child_count()).rev() {
                stack.push(current.named_child(i as u32).expect("child"));
            }
        }
        if let Some(name) = &item.name {
            let name = name.trim_start_matches("r#");
            // Namespace distinction is deliberately conservative; collision is never silently renamed.
            if let Some(dest) = parsed.get(destination) {
                for other in &dest.items {
                    check(deadline, cancelled)?;
                    if other.kind == "use_declaration"
                        && files[destination].source
                            [other.span.range.start_byte..other.span.range.end_byte]
                            .contains('*')
                    {
                        needs.push(need(
                            "glob_dependency",
                            destination,
                            dest.tree
                                .root_node()
                                .named_descendant_for_byte_range(
                                    other.span.range.start_byte,
                                    other.span.range.end_byte,
                                )
                                .expect("use"),
                            "destination glob may introduce a collision with the arriving binding",
                        ));
                    }
                    if other.kind == "use_declaration"
                        && files[destination].source
                            [other.span.range.start_byte..other.span.range.end_byte]
                            .split(|c: char| !c.is_alphanumeric() && c != '_')
                            .any(|s| s == name)
                    {
                        needs.push(need(
                            "binding_collision",
                            destination,
                            dest.tree
                                .root_node()
                                .named_descendant_for_byte_range(
                                    other.span.range.start_byte,
                                    other.span.range.end_byte,
                                )
                                .expect("use"),
                            "destination import/alias may bind the incoming name",
                        ));
                    }
                    if other.name.as_deref().map(|n| n.trim_start_matches("r#")) == Some(name)
                        && !selected
                            .iter()
                            .any(|(p, i, _)| p == destination && i.id == other.id)
                    {
                        needs.push(need(
                            "binding_collision",
                            destination,
                            dest.tree
                                .root_node()
                                .named_descendant_for_byte_range(
                                    other.span.range.start_byte,
                                    other.span.range.end_byte,
                                )
                                .expect("item"),
                            "destination has a same-named declaration",
                        ));
                    }
                }
            }
            let segment = contexts
                .get(source_path)
                .and_then(|e| e.module_segments.last())
                .map(String::as_str);
            let mut module_spellings = std::collections::BTreeSet::new();
            module_spellings.insert(segment.unwrap_or("crate"));
            // Alias spellings only establish conservative glob relevance, never rewrite authority.
            for (path, data) in parsed {
                for item in &data.items {
                    check(deadline, cancelled)?;
                    if item.kind == "use_declaration" {
                        let text = &files[path].source
                            [item.span.range.start_byte..item.span.range.end_byte];
                        let words: Vec<_> = text
                            .split(|c: char| !c.is_alphanumeric() && c != '_')
                            .filter(|s| !s.is_empty())
                            .collect();
                        if words.contains(&segment.unwrap_or("crate"))
                            && let Some(i) = words.iter().position(|s| *s == "as")
                            && let Some(alias) = words.get(i + 1)
                        {
                            module_spellings.insert(*alias);
                            let node = data
                                .tree
                                .root_node()
                                .named_descendant_for_byte_range(
                                    item.span.range.start_byte,
                                    item.span.range.end_byte,
                                )
                                .expect("use");
                            needs.push(need("unsupported_dependency_form", path, node, "alias of the changed module scope needs explicit consumer evidence; chained aliases are not resolved"));
                        }
                    }
                }
            }
            *analysis_bytes += module_spellings.iter().map(|s| s.len() + 64).sum::<usize>();
            for (path, other_data) in parsed {
                check(deadline, cancelled)?;
                let other_source = &files[path].source;
                let mut stack = vec![other_data.tree.root_node()];
                while let Some(current) = stack.pop() {
                    check(deadline, cancelled)?;
                    bound_needs(&needs, &mut seen_needs, analysis_bytes)?;
                    if selected.iter().any(|(p, i, dest)| {
                        p == path
                            && dest == destination
                            && current.start_byte() >= i.span.range.start_byte
                            && current.end_byte() <= i.span.range.end_byte
                    }) {
                        continue;
                    }
                    if matches!(
                        current.kind(),
                        "line_comment"
                            | "block_comment"
                            | "attribute_item"
                            | "inner_attribute_item"
                            | "string_literal"
                            | "raw_string_literal"
                            | "char_literal"
                    ) {
                        continue;
                    }
                    let text = &other_source[current.byte_range()];
                    if current.kind() == "use_declaration"
                        && (text
                            .split(|c: char| !c.is_alphanumeric() && c != '_')
                            .any(|s| s == name)
                            || (text.contains('*')
                                && text
                                    .split(|c: char| !c.is_alphanumeric() && c != '_')
                                    .any(|s| module_spellings.contains(s))))
                    {
                        needs.push(need(
                            if text.starts_with("pub ") {
                                "reexport_dependency"
                            } else if text.contains('*') {
                                "glob_dependency"
                            } else {
                                "unsupported_dependency_form"
                            },
                            path,
                            current,
                            "affected consumer import requires repair",
                        ));
                        continue;
                    }
                    if matches!(current.kind(), "macro_invocation" | "macro_definition") {
                        if token_candidate(current, other_source, name, deadline, cancelled)? {
                            needs.push(need(
                                "macro_dependency",
                                path,
                                current,
                                "affected token-tree candidate; expansion is unexamined",
                            ));
                        }
                        continue;
                    }
                    if matches!(current.kind(), "identifier" | "type_identifier")
                        && text.trim_start_matches("r#") == name
                        && !declaration_name(current)
                    {
                        *candidates += 1;
                        if *candidates > 100_000 {
                            return Err(DomainError::new(
                                "reference_work_limit",
                                "reference candidate cap reached",
                            ));
                        }
                        // Other ordinary bare scopes cannot refer to this binding without a path/use;
                        // same-file references and explicit paths are relevant conservative candidates.
                        let path_reference = current.parent().is_some_and(|p| {
                            matches!(p.kind(), "scoped_identifier" | "scoped_type_identifier")
                        });
                        if path == source_path || path_reference {
                            needs.push(need("unsupported_dependency_form", path, current, "remaining written consumer or shadow candidate requires explicit repair/evidence"));
                        }
                    }
                    for i in (0..current.named_child_count()).rev() {
                        stack.push(current.named_child(i as u32).expect("child"));
                    }
                }
            }
        }
        for need in &mut needs[need_start..] {
            check(deadline, cancelled)?;
            *analysis_bytes += item.id.len() + 32;
            if *analysis_bytes > 128 * 1024 * 1024 {
                return Err(DomainError::new(
                    "analysis_descriptor_bytes",
                    "affected-item evidence guard reached",
                ));
            }
            need.item_ids.push(item.id.clone());
        }
    }
    bound_needs(&needs, &mut seen_needs, analysis_bytes)?;
    Ok((needs, *candidates))
}
fn token_candidate(
    node: Node<'_>,
    source: &str,
    name: &str,
    deadline: Instant,
    cancelled: &AtomicBool,
) -> Result<bool, DomainError> {
    let mut stack = vec![node];
    while let Some(node) = stack.pop() {
        check(deadline, cancelled)?;
        if matches!(
            node.kind(),
            "string_literal"
                | "raw_string_literal"
                | "char_literal"
                | "line_comment"
                | "block_comment"
        ) {
            continue;
        }
        if matches!(node.kind(), "identifier" | "type_identifier")
            && source[node.byte_range()].trim_start_matches("r#") == name
        {
            return Ok(true);
        }
        for i in (0..node.named_child_count()).rev() {
            stack.push(node.named_child(i as u32).expect("child"));
        }
    }
    Ok(false)
}
fn bound_needs(needs: &[Need], seen: &mut usize, bytes: &mut usize) -> Result<(), DomainError> {
    for need in &needs[*seen..] {
        *bytes += serde_json::to_vec(need).expect("evidence JSON").len() + 32;
    }
    *seen = needs.len();
    if needs.len() > 100_000 || *bytes > 128 * 1024 * 1024 {
        return Err(DomainError::new(
            "analysis_descriptor_bytes",
            "dependency evidence guard reached",
        ));
    }
    Ok(())
}
