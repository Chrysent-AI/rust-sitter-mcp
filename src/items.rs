//! Written top-level inventory and ordinary module evidence. No manifest or semantic resolver.
pub(crate) mod associated;
mod attributes;
mod chain;
pub(crate) use attributes::{BUILTIN_DERIVES, context_independent_attribute, derive_names};
mod globs;
mod lexical;
mod refusal;
pub use refusal::RefusalBasis;
mod test_consumers;
use crate::{
    matching::Lines,
    result::{ByteRange, DomainError, SourceSlice, SyntaxFlags},
    scope::{FileSnapshot, Scope},
    trivia,
};
pub use chain::{
    ChainDiagnostic, ChainLocation, ChainOrigin, ChainReason, ChainRole, ModuleAnalysis,
};
pub(crate) use chain::{declaration_reasons, finalize_chain};
pub(crate) use globs::GlobRoutes;
pub use lexical::LexicalUncertainty;
pub(crate) use lexical::{
    LexicalBinding, lexical_assessment, lexical_binding, lexical_with_import_proof,
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
pub(crate) use test_consumers::{test_consumer, test_scope};
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
    #[serde(skip)]
    #[schemars(skip)]
    pub(crate) visibility_key: &'static str,
    pub attributes: Vec<SourceSlice>,
    pub trivia: Vec<SourceSlice>,
    pub bytes: usize,
    pub lines: usize,
    pub syntax: SyntaxFlags,
    pub eligibility: String,
    pub reasons: Vec<String>,
    pub signal_ids: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enclosing_impl: Option<associated::ImplIdentity>,
}
pub struct ParsedFile {
    pub tree: Tree,
    pub items: Vec<Item>,
    pub associated_items: Vec<Item>,
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
        let visibility_key = visibility_key(node, &file.source, deadline, cancelled)?;
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
            visibility_key,
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
            enclosing_impl: None,
        });
    }
    let associated_items = associated::inventory(
        file,
        &tree,
        &trivia,
        text_bytes,
        (deadline, cancelled),
        observed,
    )?;
    Ok(ParsedFile {
        tree,
        items,
        associated_items,
        trivia,
    })
}
/// Guard keys come from significant grammar tokens, never formatted source prefixes.
/// Written visibility remains raw in public descriptors and copied payloads.
fn visibility_key(
    node: Node<'_>,
    source: &str,
    deadline: Instant,
    cancelled: &AtomicBool,
) -> Result<&'static str, DomainError> {
    let visibility = (0..node.named_child_count())
        .filter_map(|i| node.named_child(i as u32))
        .find(|n| n.kind() == "visibility_modifier");
    let Some(visibility) = visibility else {
        return Ok("private");
    };
    let mut key = String::new();
    let mut stack = vec![visibility];
    while let Some(node) = stack.pop() {
        check(deadline, cancelled)?;
        if matches!(node.kind(), "line_comment" | "block_comment") {
            continue;
        }
        if node.child_count() == 0 {
            key.push_str(&source[node.byte_range()]);
        } else {
            for i in (0..node.child_count()).rev() {
                stack.push(node.child(i).expect("child"));
            }
        }
    }
    Ok(match key.as_str() {
        "pub" => "pub",
        "pub(crate)" | "crate" => "pub(crate)",
        _ => "restricted",
    })
}
/// Only a written absolute ancestor restriction has unchanged meaning after relocation.
pub(crate) fn absolute_visibility(
    node: Node<'_>,
    source: &str,
    controls: (Instant, &AtomicBool),
) -> Option<Vec<String>> {
    let visibility = (0..node.named_child_count())
        .filter_map(|i| node.named_child(i as u32))
        .find(|n| n.kind() == "visibility_modifier")?;
    let mut text = String::new();
    let mut stack = vec![visibility];
    while let Some(node) = stack.pop() {
        if check(controls.0, controls.1).is_err() {
            return None;
        }
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
    let path = text.strip_prefix("pub(incrate")?.strip_suffix(')')?;
    if path.is_empty() {
        return Some(Vec::new());
    }
    let segments: Vec<_> = path
        .strip_prefix("::")?
        .split("::")
        .map(str::to_owned)
        .collect();
    segments
        .iter()
        .all(|s| {
            !s.is_empty()
                && s.bytes().enumerate().all(|(i, b)| {
                    b == b'_' || b.is_ascii_alphabetic() || (i > 0 && b.is_ascii_digit())
                })
        })
        .then_some(segments)
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
pub fn public_chain(evidence: &ModuleEvidence, parsed: &BTreeMap<String, ParsedFile>) -> bool {
    evidence.declaration_anchors.iter().all(|a| {
        parsed.get(&a.path).is_some_and(|file| {
            file.items
                .iter()
                .any(|item| item.span.range == a.range && item.visibility_key == "pub")
        })
    })
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
    descriptor_bytes: &mut usize,
) -> Result<ModuleAnalysis, DomainError> {
    let mut analysis = ModuleAnalysis::default();
    let mut queue = vec![(root.to_owned(), ModuleEvidence {
        crate_root: root.into(), module_segments: Vec::new(), declaration_anchors: Vec::new(), filesystem_paths: vec![root.into()],
        assumptions: vec!["caller-selected root; not an active Cargo target; admitted-scope references only; build configuration/macros unexamined".into()], unresolved: Vec::new(),
    })];
    while let Some((path, mut evidence)) = queue.pop() {
        check(deadline, cancelled)?;
        if let Some(prior) = analysis.contexts.get_mut(&path) {
            prior
                .unresolved
                .push("multiple inclusion contexts/cycle".into());
            let mut failure = ChainDiagnostic::boundary(
                root,
                "",
                ChainRole::Source,
                ChainReason::InheritedUncertainty,
            );
            failure.evidenced_prefix_paths = evidence.filesystem_paths.clone();
            failure.at_file_path = path.clone();
            failure.declaration = evidence.declaration_anchors.last().map(|a| ChainLocation {
                path: a.path.clone(),
                range: a.range.clone(),
            });
            failure.candidate_paths.push(path.clone());
            failure
                .origin_reasons
                .push(ChainOrigin::MultipleInclusionContexts);
            analysis.record(failure, descriptor_bytes)?;
            continue;
        }
        let Some(data) = parsed.get(&path) else {
            continue;
        };
        let file = &files[&path];
        let mut origins = Vec::new();
        if data.tree.root_node().has_error() {
            evidence
                .unresolved
                .push("module evidence contains syntax recovery".into());
            origins.push(ChainOrigin::SyntaxRecovery);
        }
        if let Some(attribute) = data.trivia.iter().find(|t| {
            t.is_attribute()
                && t.classification == "scope"
                && !file.source[t.range.clone()].starts_with("#![allow(")
        }) {
            evidence
                .unresolved
                .push("inherited scope attributes require an explicit context choice".into());
            origins.push(ChainOrigin::ScopeAttributes);
            // Add the specific cause without changing the inherited-uncertainty veto.
            let mut failure = ChainDiagnostic::boundary(
                root,
                "",
                ChainRole::Source,
                ChainReason::RootAttributeChainUncertainty,
            );
            failure.at_file_path = path.clone();
            failure.evidenced_prefix_paths = evidence.filesystem_paths.clone();
            failure.candidate_paths.push(path.clone());
            failure.origin_reasons.push(ChainOrigin::ScopeAttributes);
            failure.declaration = Some(ChainLocation {
                path: path.clone(),
                range: ByteRange {
                    start_byte: attribute.range.start,
                    end_byte: attribute.range.end,
                },
            });
            analysis.record(failure, descriptor_bytes)?;
        }
        if !origins.is_empty() {
            let mut failure = ChainDiagnostic::boundary(
                root,
                "",
                ChainRole::Source,
                ChainReason::InheritedUncertainty,
            );
            failure.at_file_path = path.clone();
            failure.evidenced_prefix_paths = evidence.filesystem_paths.clone();
            failure.candidate_paths.push(path.clone());
            failure.origin_reasons = origins;
            analysis.record(failure, descriptor_bytes)?;
        }
        analysis.record_macro_declarations(
            file,
            data,
            &evidence,
            (deadline, cancelled),
            descriptor_bytes,
        )?;
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
            let flat = child_path(&path, root, name.trim_start_matches("r#"));
            let legacy = format!("{}/mod.rs", flat.trim_end_matches(".rs"));
            let mut reasons = declaration_reasons(data, file, item);
            if declarations.len() != 1 {
                reasons.push(ChainReason::CompetingDeclarations);
            }
            let mut failure = ChainDiagnostic::boundary(
                root,
                "",
                ChainRole::Source,
                ChainReason::ChainFileMissing,
            );
            failure.at_file_path = path.clone();
            failure.evidenced_prefix_paths = evidence.filesystem_paths.clone();
            failure.declaration = Some(ChainLocation {
                path: path.clone(),
                range: item.span.range.clone(),
            });
            failure.candidate_paths = vec![flat.clone(), legacy.clone()];
            if !reasons.is_empty() {
                for reason in reasons {
                    failure.reason = reason;
                    analysis.record(failure.clone(), descriptor_bytes)?;
                }
                continue;
            }
            let a = present(scope, &flat)?;
            let b = present(scope, &legacy)?;
            let child = if a && !b {
                flat
            } else if b && !a {
                legacy
            } else {
                failure.reason = if a {
                    ChainReason::CompetingFileLayout
                } else {
                    ChainReason::ChainFileMissing
                };
                analysis.record(failure, descriptor_bytes)?;
                continue;
            };
            if !files.contains_key(&child) {
                failure.reason = ChainReason::ChainFileUnadmitted;
                analysis.record(failure, descriptor_bytes)?;
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
        analysis.contexts.insert(path, evidence);
    }
    // Propagate observed competing contexts to descendants already traversed.
    let uncertain: Vec<_> = analysis
        .contexts
        .iter()
        .filter(|(_, e)| !e.unresolved.is_empty())
        .map(|(p, _)| p.clone())
        .collect();
    for evidence in analysis.contexts.values_mut() {
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
    Ok(analysis)
}

pub(crate) fn use_facts(
    node: Node<'_>,
    source: &str,
    controls: (Instant, &AtomicBool),
) -> Result<(bool, Vec<String>), DomainError> {
    let mut glob = false;
    let mut names = Vec::new();
    let mut stack = vec![node];
    while let Some(node) = stack.pop() {
        check(controls.0, controls.1)?;
        if matches!(node.kind(), "line_comment" | "block_comment") {
            continue;
        }
        glob |= node.kind() == "use_wildcard";
        if matches!(
            node.kind(),
            "identifier" | "type_identifier" | "crate" | "self" | "super"
        ) {
            names.push(source[node.byte_range()].trim_start_matches("r#").into());
        }
        for i in (0..node.named_child_count()).rev() {
            stack.push(node.named_child(i as u32).expect("child"));
        }
    }
    Ok((glob, names))
}
/// Conservative competitors for a bare pattern argument. This is not constant
/// resolution: visible unit variants and potentially item-producing macros
/// retain uncertainty without proving a same-named bare constant exists.
pub(crate) fn pattern_name_competes(
    pattern: Node<'_>,
    source: &str,
    name: &str,
    controls: (Instant, &AtomicBool),
) -> Result<bool, DomainError> {
    let same_name = |node: Node<'_>| {
        node.child_by_field_name("name")
            .is_some_and(|n| source[n.byte_range()].trim_start_matches("r#") == name)
    };
    let mut ancestor = pattern.parent();
    while let Some(scope) = ancestor {
        check(controls.0, controls.1)?;
        let module_body = scope.kind() == "declaration_list"
            && scope.parent().is_some_and(|p| p.kind() == "mod_item");
        let written_item_scope = matches!(scope.kind(), "block" | "source_file") || module_body;
        if written_item_scope || scope.kind() == "declaration_list" {
            for i in 0..scope.named_child_count() {
                check(controls.0, controls.1)?;
                let item = scope.named_child(i as u32).expect("scope item");
                // Item output is unknown in the containing module and every
                // intervening block/declaration scope, before or after the pattern.
                if item.kind() == "macro_invocation"
                    || (item.kind() == "expression_statement"
                        && item
                            .named_child(0)
                            .is_some_and(|n| n.kind() == "macro_invocation"))
                {
                    return Ok(true);
                }
                if !written_item_scope {
                    continue;
                }
                if matches!(item.kind(), "const_item" | "static_item") && same_name(item) {
                    return Ok(true);
                }
                // Unit structs are written value constructors too. Do not prove
                // a binder merely because the competitor is not an enum variant.
                if item.kind() == "struct_item"
                    && item.child_by_field_name("body").is_none()
                    && same_name(item)
                {
                    return Ok(true);
                }
                if item.kind() == "use_declaration"
                    && (use_facts(item, source, controls)?.0
                        || use_leaves(item, source, controls)?
                            .iter()
                            .any(|l| l.binding.trim_start_matches("r#") == name))
                {
                    return Ok(true);
                }
                if item.kind() == "enum_item"
                    && let Some(body) = item.child_by_field_name("body")
                {
                    for i in 0..body.named_child_count() {
                        check(controls.0, controls.1)?;
                        let variant = body.named_child(i as u32).expect("enum child");
                        if variant.kind() == "enum_variant"
                            && variant.child_by_field_name("body").is_none()
                            && same_name(variant)
                        {
                            return Ok(true);
                        }
                    }
                }
            }
            // A child module does not inherit its parent's imports or items.
            if scope.kind() == "source_file" || module_body {
                break;
            }
        }
        if let Some(parameters) = scope.child_by_field_name("type_parameters") {
            for i in 0..parameters.named_child_count() {
                check(controls.0, controls.1)?;
                let parameter = parameters.named_child(i as u32).expect("generic parameter");
                if parameter.kind() == "const_parameter" && same_name(parameter) {
                    return Ok(true);
                }
            }
        }
        ancestor = scope.parent();
    }
    Ok(false)
}
/// Written use leaves retain original ranges; lists are never regenerated.
#[derive(Clone)]
pub(crate) struct UseLeaf {
    pub path: String,
    pub binding: String,
    pub path_range: ByteRange,
    pub leaf_range: ByteRange,
    pub prefix: String,
    pub prefix_range: Option<ByteRange>,
    pub list_range: Option<ByteRange>,
    pub declaration: ByteRange,
    pub public: bool,
}
pub(crate) fn use_leaves(
    node: Node<'_>,
    source: &str,
    controls: (Instant, &AtomicBool),
) -> Result<Vec<UseLeaf>, DomainError> {
    let declaration = ByteRange {
        start_byte: node.start_byte(),
        end_byte: node.end_byte(),
    };
    let public = visibility_key(node, source, controls.0, controls.1)? != "private";
    let mut out = Vec::new();
    let mut bytes = 0usize;
    let Some(argument) = node.child_by_field_name("argument") else {
        return Ok(out);
    };
    let mut stack = vec![(argument, String::new(), None, None)];
    while let Some((node, prefix, prefix_range, list_range)) = stack.pop() {
        check(controls.0, controls.1)?;
        bytes += prefix.len() * 3 + node.end_byte() - node.start_byte() + 256;
        if bytes > 128 * 1024 * 1024 {
            return Err(DomainError::new(
                "analysis_descriptor_bytes",
                "owned use-leaf evidence guard reached",
            ));
        }
        match node.kind() {
            "scoped_use_list" => {
                let part = node.child_by_field_name("path");
                let next = part
                    .map(|n| source[n.byte_range()].to_owned())
                    .unwrap_or_default();
                let full = if prefix.is_empty() {
                    next
                } else {
                    format!("{prefix}::{next}")
                };
                stack.push((
                    node.child_by_field_name("list").expect("list"),
                    full,
                    part.map(|n| ByteRange {
                        start_byte: n.start_byte(),
                        end_byte: n.end_byte(),
                    }),
                    list_range,
                ));
            }
            "use_list" => {
                let range = ByteRange {
                    start_byte: node.start_byte(),
                    end_byte: node.end_byte(),
                };
                for i in (0..node.named_child_count()).rev() {
                    let child = node.named_child(i as u32).expect("child");
                    if !matches!(child.kind(), "line_comment" | "block_comment") {
                        bytes += prefix.len() + 256;
                        if bytes > 128 * 1024 * 1024 {
                            return Err(DomainError::new(
                                "analysis_descriptor_bytes",
                                "group prefix evidence guard reached",
                            ));
                        }
                        stack.push((
                            child,
                            prefix.clone(),
                            prefix_range.clone(),
                            Some(range.clone()),
                        ));
                    }
                }
            }
            "use_wildcard" => {}
            _ => {
                let path = node
                    .child_by_field_name("path")
                    .filter(|_| node.kind() == "use_as_clause")
                    .unwrap_or(node);
                let text = &source[path.byte_range()];
                let full = if prefix.is_empty() {
                    text.into()
                } else if text == "self" {
                    prefix.clone()
                } else {
                    format!("{prefix}::{text}")
                };
                let binding = node
                    .child_by_field_name("alias")
                    .map(|n| source[n.byte_range()].to_owned())
                    .unwrap_or_else(|| full.rsplit("::").next().unwrap_or("").into());
                out.push(UseLeaf {
                    path: full,
                    binding,
                    path_range: ByteRange {
                        start_byte: path.start_byte(),
                        end_byte: path.end_byte(),
                    },
                    leaf_range: ByteRange {
                        start_byte: node.start_byte(),
                        end_byte: node.end_byte(),
                    },
                    prefix,
                    prefix_range,
                    list_range,
                    declaration: declaration.clone(),
                    public,
                });
            }
        }
    }
    Ok(out)
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(rename_all = "snake_case")]
pub enum DecisionReason {
    ModuleChainFailure,
    RequiredRewriteRetained,
    OrdinaryTriviaChoice,
    RemovalGapChoice,
    CrossGroupReferenceReview,
    DestinationBindingConflict,
    SourceBindingAmbiguous,
    FinalAliasConflict,
    LexicalContextUnproved,
    PublicPathChange,
    MemberOrConstructorUnproved,
    VisibilityScopeUnproved,
    ConditionalOrInheritedContext,
    TestConsumerAcknowledged,
    MacroContextUnexamined,
    GlobBindingUnproved,
    GlobConsumerUnrepaired,
    ExternalOrMissingBinding,
    UnsupportedUnitKind,
    UnsupportedConstruct,
    TriviaPreservationUnproved,
}
#[derive(Clone, Serialize)]
pub struct Need {
    /// Original attribute veto, distinct from a repair that never completed.
    #[serde(skip)]
    pub(crate) attribute_range: Option<ByteRange>,
    pub reason: DecisionReason,
    pub choice_target: Option<crate::move_plan::RewriteTarget>,
    pub lexical_uncertainty: Option<LexicalUncertainty>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub refusal_basis: Vec<RefusalBasis>,
    pub category: &'static str,
    pub path: String,
    pub range: ByteRange,
    pub message: String,
    pub item_ids: Vec<String>,
}
fn need(
    reason: DecisionReason,
    category: &'static str,
    path: &str,
    node: Node<'_>,
    message: &str,
) -> Need {
    Need {
        attribute_range: None,
        reason,
        choice_target: None,
        lexical_uncertainty: None,
        refusal_basis: Vec::new(),
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
pub(crate) fn reference_role(node: Node<'_>) -> bool {
    if declaration_name(node) {
        return false;
    }
    // Bare closure parameters have no `pattern` field; only a direct
    // identifier is a binder, not types or constructors nested in parameters.
    if node.kind() == "identifier"
        && node
            .parent()
            .is_some_and(|p| p.kind() == "closure_parameters")
    {
        return false;
    }
    let mut child = node;
    while let Some(parent) = child.parent() {
        // The grammar wraps a match guard inside match_pattern, but its
        // condition is an expression reference, not a binding position.
        let in_match_guard = child.kind() == "match_pattern"
            && child.child_by_field_name("condition").is_some_and(|guard| {
                guard.start_byte() <= node.start_byte() && guard.end_byte() >= node.end_byte()
            });
        if (parent.child_by_field_name("pattern") == Some(child) && !in_match_guard)
            || matches!(parent.kind(), "lifetime" | "loop_label")
            || (parent.kind() == "field_expression"
                && parent.child_by_field_name("field") == Some(child))
        {
            return false;
        }
        if matches!(parent.kind(), "function_item" | "block" | "source_file") {
            break;
        }
        child = parent;
    }
    true
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
    evidence: (&mut usize, &mut BTreeMap<String, usize>),
) -> Result<(Vec<Need>, usize), DomainError> {
    let (deadline, cancelled) = controls;
    let (analysis_bytes, glob_exclusions) = evidence;
    let root = contexts
        .values()
        .next()
        .map(|c| c.crate_root.as_str())
        .unwrap_or("");
    let glob_routes = GlobRoutes::new(files, parsed, root, controls)?;
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
                    && item.enclosing_impl.is_none()
                    && other.enclosing_impl.is_none()
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
                        DecisionReason::DestinationBindingConflict,
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
                DecisionReason::UnsupportedUnitKind,
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
            && item.enclosing_impl.is_none()
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
                DecisionReason::SourceBindingAmbiguous,
                "binding_collision",
                source_path,
                node,
                "source has several same-named written declarations; binding identity is ambiguous",
            ));
        }
        if item.visibility_key == "restricted" {
            needs.push(need(DecisionReason::VisibilityScopeUnproved, "visibility_context", source_path, node, "restricted visibility changes lexical scope; explicit repair is outside dependency-free moves"));
        }
        let publicly_exposed = item.enclosing_impl.is_none()
            && item.visibility_key == "pub"
            && contexts
                .get(source_path)
                .is_some_and(|e| public_chain(e, parsed));
        if publicly_exposed {
            needs.push(need(DecisionReason::PublicPathChange, "reexport_dependency", source_path, node, "observed public path changes; choose a non-exposed item or an explicit API decision"));
        }
        for t in &data.trivia {
            check(deadline, cancelled)?;
            if t.is_attribute()
                && (t.owned_by(node.byte_range())
                    || t.classification == "scope"
                    || node.byte_range().contains(&t.range.start))
            {
                let text = &source[t.range.clone()];
                if !context_independent_attribute(text) {
                    let mut veto = need(
                        DecisionReason::ConditionalOrInheritedContext,
                        "scope_dependency",
                        source_path,
                        node,
                        "conditional, inherited or unexamined attribute context requires a supported explicit choice",
                    );
                    veto.attribute_range = Some(ByteRange {
                        start_byte: t.range.start,
                        end_byte: t.range.end,
                    });
                    needs.push(veto);
                }
            }
        }
        // A linear absence check avoids searching every lexical block for names that
        // cannot be local. A positive spelling still goes through the scoped checker.
        let mut possible_locals = std::collections::BTreeSet::new();
        if item.enclosing_impl.is_some()
            && let Some(parameters) = node
                .parent()
                .and_then(|body| body.parent())
                .and_then(|implementation| implementation.child_by_field_name("type_parameters"))
        {
            for i in 0..parameters.named_child_count() {
                if let Some(name) = parameters
                    .named_child(i as u32)
                    .and_then(|parameter| parameter.child_by_field_name("name"))
                {
                    possible_locals.insert(source[name.byte_range()].trim_start_matches("r#"));
                }
            }
        }
        let mut stack = vec![(node, false)];
        while let Some((current, in_pattern)) = stack.pop() {
            check(deadline, cancelled)?;
            if matches!(
                current.kind(),
                "identifier" | "type_identifier" | "shorthand_field_identifier"
            ) && (in_pattern || declaration_name(current))
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
                check(deadline, cancelled)?;
                let child = current.named_child(i as u32).expect("child");
                let binding_position = in_pattern
                    || current.child_by_field_name("pattern") == Some(child)
                    || current.kind() == "closure_parameters";
                stack.push((child, binding_position));
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
                needs.push(need(DecisionReason::MacroContextUnexamined, category, source_path, current, "macro expansion or file-relative include context is unexamined; no token-tree rewrite"));
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
                    DecisionReason::UnsupportedConstruct,
                    "unsupported_dependency_form",
                    source_path,
                    current,
                    "path/import context needs a repair or explicit binding evidence",
                ));
                continue;
            }
            if kind == "field_expression" {
                needs.push(need(DecisionReason::MemberOrConstructorUnproved, "visibility_context", source_path, current, "member/method access and trait-import context are not established syntactically"));
            }
            if kind == "identifier"
                && !reference_role(current)
                && let Some(assessment) =
                    lexical::pattern_uncertainty(source_path, current, source, controls)?
            {
                let mut value = need(
                    DecisionReason::LexicalContextUnproved,
                    "binding_collision",
                    source_path,
                    current,
                    "pattern argument may name a constant; no independent local binding proof",
                );
                value.lexical(assessment);
                needs.push(value);
            }
            let is_pattern = current
                .parent()
                .is_some_and(|p| p.child_by_field_name("pattern") == Some(current));
            if is_pattern && matches!(kind, "identifier" | "mut_pattern" | "reference_pattern") {
                continue;
            }
            if matches!(kind, "identifier" | "type_identifier") && reference_role(current) {
                *candidates += 1;
                if *candidates > 100_000 {
                    return Err(DomainError::new(
                        "reference_work_limit",
                        "reference candidate cap reached",
                    ));
                }
                let name = source[current.byte_range()].trim_start_matches("r#");
                let self_context = name == "Self"
                    && (item.enclosing_impl.is_some()
                        || matches!(item.kind.as_str(), "impl_item" | "trait_item"));
                let own = item.enclosing_impl.is_none()
                    && item.name.as_deref().map(|n| n.trim_start_matches("r#")) == Some(name);
                let bound = possible_locals.contains(name)
                    && lexical::local(current, node, source, name, deadline, cancelled);
                let co_moved = selected.iter().any(|(p, other, dest)| {
                    p == source_path
                        && dest == destination
                        && other.enclosing_impl.is_none()
                        && other.name.as_deref().map(|n| n.trim_start_matches("r#")) == Some(name)
                });
                if !self_context && !own && !bound && !co_moved {
                    let mut explicit = false;
                    let mut glob = false;
                    for i in data.items.iter().filter(|i| i.kind == "use_declaration") {
                        let use_node = data
                            .tree
                            .root_node()
                            .named_descendant_for_byte_range(
                                i.span.range.start_byte,
                                i.span.range.end_byte,
                            )
                            .expect("use");
                        glob |= use_facts(use_node, source, controls)?.0;
                        explicit |= use_leaves(use_node, source, controls)?
                            .iter()
                            .any(|leaf| leaf.binding == name);
                    }
                    let glob = glob && !explicit;
                    needs.push(need(if glob { DecisionReason::GlobBindingUnproved } else { DecisionReason::ExternalOrMissingBinding }, if glob { "glob_dependency" } else { "unsupported_dependency_form" }, source_path, current, "written bare dependency is not retained in the final scope; explicit import/path repair required"));
                }
            }
            for i in (0..current.named_child_count()).rev() {
                stack.push(current.named_child(i as u32).expect("child"));
            }
        }
        if let Some(name) = &item.name
            && item.enclosing_impl.is_none()
        {
            let name = name.trim_start_matches("r#");
            // Namespace distinction is deliberately conservative; collision is never silently renamed.
            if let Some(dest) = parsed.get(destination) {
                for other in &dest.items {
                    check(deadline, cancelled)?;
                    if other.kind == "use_declaration"
                        && use_facts(
                            dest.tree
                                .root_node()
                                .named_descendant_for_byte_range(
                                    other.span.range.start_byte,
                                    other.span.range.end_byte,
                                )
                                .expect("use"),
                            &files[destination].source,
                            controls,
                        )?
                        .0
                    {
                        needs.push(need(
                            DecisionReason::GlobBindingUnproved,
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
                        && use_leaves(
                            dest.tree
                                .root_node()
                                .named_descendant_for_byte_range(
                                    other.span.range.start_byte,
                                    other.span.range.end_byte,
                                )
                                .expect("use"),
                            &files[destination].source,
                            controls,
                        )?
                        .iter()
                        .any(|leaf| leaf.binding == name)
                    {
                        needs.push(need(
                            DecisionReason::DestinationBindingConflict,
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
                            DecisionReason::DestinationBindingConflict,
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
            module_spellings.insert(segment.unwrap_or("crate").to_owned());
            // Alias declarations alone are not affected consumers. Retain their spellings
            // for conservative glob relevance; written uses of the moved name are checked below.
            for (path, data) in parsed {
                let mut stack = vec![data.tree.root_node()];
                while let Some(node) = stack.pop() {
                    check(deadline, cancelled)?;
                    if node.kind() == "use_declaration" {
                        for leaf in use_leaves(node, &files[path].source, controls)? {
                            if leaf
                                .path
                                .split("::")
                                .any(|p| p == segment.unwrap_or("crate"))
                            {
                                module_spellings.insert(leaf.binding);
                            }
                        }
                    } else if !matches!(
                        node.kind(),
                        "macro_invocation" | "macro_definition" | "line_comment" | "block_comment"
                    ) {
                        for i in 0..node.named_child_count() {
                            stack.push(node.named_child(i as u32).expect("child"));
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
                    let use_evidence = if current.kind() == "use_declaration" {
                        Some(use_facts(current, other_source, controls)?)
                    } else {
                        None
                    };
                    let (glob_exclusion, glob_forwarding) = if use_evidence
                        .as_ref()
                        .is_some_and(|f| f.0)
                    {
                        glob_routes.exclusion(path, current, &[source_path, destination], name)?
                    } else {
                        (None, false)
                    };
                    if let Some(reason) = glob_exclusion {
                        *glob_exclusions.entry(reason.into()).or_default() += 1;
                    }
                    if let Some((glob, names)) = &use_evidence
                        && (names.iter().any(|s| s == name)
                            || (*glob
                                && glob_exclusion.is_none()
                                && (glob_forwarding
                                    || names.iter().any(|s| module_spellings.contains(s)))))
                    {
                        let public = visibility_key(current, other_source, deadline, cancelled)?
                            != "private";
                        needs.push(need(
                            if public {
                                DecisionReason::PublicPathChange
                            } else if *glob {
                                DecisionReason::GlobBindingUnproved
                            } else {
                                DecisionReason::UnsupportedConstruct
                            },
                            if public {
                                "reexport_dependency"
                            } else if *glob {
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
                                DecisionReason::MacroContextUnexamined,
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
                        && reference_role(current)
                    {
                        *candidates += 1;
                        if *candidates > 100_000 {
                            return Err(DomainError::new(
                                "reference_work_limit",
                                "reference candidate cap reached",
                            ));
                        }
                        // Same-file and explicit-path consumers keep the existing repair path.
                        // Other bare references need visible glob evidence, not module spelling.
                        let path_reference = current.parent().is_some_and(|p| {
                            matches!(p.kind(), "scoped_identifier" | "scoped_type_identifier")
                        });
                        if path == source_path || path_reference {
                            let assessment = if path_reference {
                                None
                            } else {
                                Some(lexical_assessment(
                                    path,
                                    current,
                                    other_source,
                                    name,
                                    controls,
                                    false,
                                )?)
                            };
                            match assessment
                                .as_ref()
                                .map_or(LexicalBinding::Absent, |a| a.binding)
                            {
                                LexicalBinding::Independent => {}
                                LexicalBinding::Uncertain => {
                                    let mut value = need(
                                        DecisionReason::LexicalContextUnproved,
                                        "binding_collision",
                                        path,
                                        current,
                                        "containing lexical binding context is unproved; no file-level reference proof",
                                    );
                                    value.lexical(assessment.expect("uncertain assessment"));
                                    needs.push(value);
                                }
                                LexicalBinding::Absent => needs.push(need(
                                    DecisionReason::ExternalOrMissingBinding,
                                    "unsupported_dependency_form",
                                    path,
                                    current,
                                    "remaining written consumer requires explicit repair/evidence",
                                )),
                            }
                        } else {
                            let imports =
                                glob_routes.consumer_imports(path, current, source_path, name)?;
                            if !imports.is_empty() {
                                let assessment = lexical_assessment(
                                    path,
                                    current,
                                    other_source,
                                    name,
                                    controls,
                                    false,
                                )?;
                                if assessment.binding != LexicalBinding::Independent {
                                    let mut value = need(
                                        DecisionReason::GlobConsumerUnrepaired,
                                        "glob_dependency",
                                        path,
                                        current,
                                        "bare consumer may reach the moved item through visible glob imports; third-file glob consumers are not repaired",
                                    );
                                    value.lexical(assessment);
                                    value.refusal_basis = imports
                                        .into_iter()
                                        .map(|range| {
                                            RefusalBasis::new("glob_import", path, Some(range))
                                                .named(name)
                                        })
                                        .collect();
                                    needs.push(value);
                                }
                            }
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
pub(crate) fn token_candidate(
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
