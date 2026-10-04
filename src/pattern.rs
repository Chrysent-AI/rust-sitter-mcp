//! Expression sugar compilation and full-source structural post-filters.
use crate::{query::CompiledQuery, result::DomainError};
use std::collections::BTreeMap;
use tree_sitter::{Node, Parser, Query};

const PREFIX: &str = "fn __ssr_wrapper() { let __ssr_value = (";
const SUFFIX: &str = ");\n}";

pub struct Pattern {
    nodes: Vec<PatternNode>,
}
type ParentField = Option<(usize, Option<String>)>;
struct PatternNode {
    kind: String,
    named: bool,
    leaf: Option<String>,
    variable: Option<String>,
    children: Vec<(Option<String>, usize)>,
}
fn comment(node: Node<'_>) -> bool {
    matches!(node.kind(), "line_comment" | "block_comment")
}
fn literal(kind: &str) -> bool {
    matches!(
        kind,
        "integer_literal"
            | "float_literal"
            | "boolean_literal"
            | "char_literal"
            | "string_literal"
            | "raw_string_literal"
    )
}
fn error(code: &str, message: &str, offset: usize, length: usize) -> DomainError {
    DomainError::new(code, message).at_pattern(offset.min(length))
}
fn offset(node: Node<'_>, length: usize) -> usize {
    node.start_byte().saturating_sub(PREFIX.len()).min(length)
}
fn expression(node: Node<'_>) -> bool {
    let language = node.language();
    language
        .subtypes_for_supertype(language.id_for_node_kind("_expression", true))
        .contains(&node.kind_id())
}
fn variable_position(node: Node<'_>) -> bool {
    let Some(parent) = node.parent() else {
        return false;
    };
    let field = (0..parent.child_count()).find_map(|i| {
        (parent.child(i) == Some(node))
            .then(|| parent.field_name_for_child(i))
            .flatten()
    });
    match parent.kind() {
        "call_expression" => field == Some("function"),
        "field_expression" => field == Some("value"),
        "arguments" | "parenthesized_expression" => true,
        _ => false,
    }
}
fn supported(kind: &str) -> bool {
    matches!(
        kind,
        "call_expression"
            | "field_expression"
            | "identifier"
            | "field_identifier"
            | "integer_literal"
            | "arguments"
            | "metavariable"
    )
}

impl Pattern {
    pub fn compile(source: &str) -> Result<CompiledQuery, DomainError> {
        if source.len() > 64 * 1024 {
            return Err(error(
                "INVALID_PATTERN",
                "pattern exceeds 64 KiB",
                64 * 1024,
                source.len(),
            ));
        }
        let wrapped = format!("{PREFIX}{source}{SUFFIX}");
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_rust::LANGUAGE.into())
            .map_err(|_| DomainError::new("INTERNAL", "Rust grammar ABI setup failed"))?;
        let tree = parser
            .parse(&wrapped, None)
            .ok_or_else(|| DomainError::new("INTERNAL", "pattern parser failed"))?;
        // Validate native metavariables before reporting recovery or unsupported authored forms.
        let mut scan = vec![tree.root_node()];
        let mut recovery = None;
        while let Some(node) = scan.pop() {
            if comment(node) || literal(node.kind()) {
                continue;
            }
            if node.is_error() || node.is_missing() {
                recovery.get_or_insert(offset(node, source.len()));
            }
            if node.kind() == "metavariable" {
                let text = &wrapped[node.byte_range()];
                let name = &text[1..];
                if name.starts_with("__ssr_")
                    || !name.is_ascii()
                    || !name.bytes().enumerate().all(|(i, b)| {
                        b.is_ascii_alphabetic() || b == b'_' || (i > 0 && b.is_ascii_digit())
                    })
                {
                    return Err(error(
                        "INVALID_PATTERN",
                        "metavariable names must be ASCII identifiers without the __ssr_ prefix",
                        offset(node, source.len()),
                        source.len(),
                    ));
                }
                if !variable_position(node) {
                    return Err(error(
                        "UNSUPPORTED_PLACEHOLDER_POSITION",
                        "metavariables bind one expression, not field/method/path names, types, bindings or token trees",
                        offset(node, source.len()),
                        source.len(),
                    ));
                }
            }
            for i in (0..node.child_count()).rev() {
                scan.push(node.child(i).expect("child index"));
            }
        }
        if let Some(position) = recovery {
            return Err(error(
                "INVALID_PATTERN",
                "supply exactly one balanced expression; sequences, partial tokens, $$ escapes and sugar capture annotations are unsupported",
                position,
                source.len(),
            ));
        }
        let function = tree.root_node().named_child(0).ok_or_else(|| {
            error(
                "INVALID_PATTERN",
                "expected one expression",
                0,
                source.len(),
            )
        })?;
        let body = function.child_by_field_name("body").ok_or_else(|| {
            error(
                "INVALID_PATTERN",
                "expected expression wrapper",
                0,
                source.len(),
            )
        })?;
        let declaration = body.named_child(0).ok_or_else(|| {
            error(
                "INVALID_PATTERN",
                "expected one expression",
                0,
                source.len(),
            )
        })?;
        let outer = declaration.child_by_field_name("value").ok_or_else(|| {
            error(
                "INVALID_PATTERN",
                "expected one expression",
                0,
                source.len(),
            )
        })?;
        if tree.root_node().named_child_count() != 1
            || body.named_child_count() != 1
            || declaration.kind() != "let_declaration"
            || outer.kind() != "parenthesized_expression"
            || outer.start_byte() != PREFIX.len() - 1
            || outer.end_byte() != PREFIX.len() + source.len() + 1
        {
            return Err(error(
                "INVALID_PATTERN",
                "pattern must be one expression without a statement semicolon",
                0,
                source.len(),
            ));
        }
        let mut cursor = outer.walk();
        let roots: Vec<_> = outer
            .named_children(&mut cursor)
            .filter(|n| !comment(*n))
            .collect();
        if roots.len() != 1 || !expression(roots[0]) {
            return Err(error(
                "INVALID_PATTERN",
                "pattern must be one expression",
                0,
                source.len(),
            ));
        }
        let mut nodes: Vec<PatternNode> = Vec::new();
        let mut pending: Vec<(Node<'_>, ParentField)> = vec![(roots[0], None)];
        let mut occurrences = 0;
        while let Some((node, parent)) = pending.pop() {
            if comment(node) {
                continue;
            }
            if node.is_named() && !supported(node.kind()) {
                return Err(error(
                    "UNSUPPORTED_PATTERN_FORM",
                    "this authored form is unsupported; use search_query for broader syntax",
                    offset(node, source.len()),
                    source.len(),
                ));
            }
            if nodes.len() == 4096 {
                return Err(error(
                    "INVALID_PATTERN",
                    "pattern exceeds 4,096 significant nodes",
                    offset(node, source.len()),
                    source.len(),
                ));
            }
            let variable =
                (node.kind() == "metavariable").then(|| wrapped[node.byte_range()][1..].to_owned());
            occurrences += usize::from(variable.is_some());
            if occurrences > 64 {
                return Err(error(
                    "INVALID_PATTERN",
                    "pattern exceeds 64 metavariable occurrences",
                    offset(node, source.len()),
                    source.len(),
                ));
            }
            let index = nodes.len();
            if let Some((parent, field)) = parent {
                nodes[parent].children.push((field, index));
            }
            let atomic = variable.is_some() || literal(node.kind()) || node.child_count() == 0;
            nodes.push(PatternNode {
                kind: node.kind().into(),
                named: node.is_named(),
                leaf: (atomic && variable.is_none()).then(|| wrapped[node.byte_range()].to_owned()),
                variable,
                children: Vec::new(),
            });
            if !atomic {
                for i in (0..node.child_count()).rev() {
                    pending.push((
                        node.child(i).expect("child index"),
                        Some((index, node.field_name_for_child(i).map(str::to_owned))),
                    ));
                }
            }
        }
        let pattern = Self { nodes };
        let query_text = pattern.query_text();
        let query = Query::new(&tree_sitter_rust::LANGUAGE.into(), &query_text).map_err(|e| {
            DomainError::new("INTERNAL", format!("generated pattern query failed: {e}"))
        })?;
        let root = query
            .capture_index_for_name("match")
            .expect("generated match capture");
        Ok(CompiledQuery {
            query,
            root,
            arity: vec![Vec::new()],
            sugar: Some(pattern),
        })
    }
    fn query_text(&self) -> String {
        // An explicit stack keeps deeply nested, bounded input off the Rust call stack.
        enum Part {
            Node(usize),
            Text(String),
        }
        let mut parts = vec![Part::Node(0)];
        let mut text = String::from("(");
        let mut predicates = String::new();
        for (index, node) in self.nodes.iter().enumerate() {
            if matches!(node.kind.as_str(), "identifier" | "field_identifier") {
                predicates.push_str(&format!(
                    " (#eq? @__ssr_fixed_{index} {})",
                    serde_json::to_string(node.leaf.as_ref().expect("identifier leaf"))
                        .expect("JSON string")
                ));
            }
        }
        while let Some(part) = parts.pop() {
            match part {
                Part::Text(value) => text.push_str(&value),
                Part::Node(index) => {
                    let node = &self.nodes[index];
                    if let Some(name) = &node.variable {
                        text.push_str(&format!("(_expression) @__ssr_v_{name}_{index}"));
                    } else if !node.named {
                        text.push_str(
                            &serde_json::to_string(node.leaf.as_ref().expect("terminal leaf"))
                                .expect("JSON string"),
                        );
                    } else {
                        text.push('(');
                        text.push_str(&node.kind);
                        let capture =
                            if matches!(node.kind.as_str(), "identifier" | "field_identifier") {
                                format!(" @__ssr_fixed_{index}")
                            } else {
                                String::new()
                            };
                        parts.push(Part::Text(format!("){capture}")));
                        for (field, child) in node.children.iter().rev() {
                            parts.push(Part::Node(*child));
                            parts.push(Part::Text(
                                field
                                    .as_ref()
                                    .map_or_else(|| " ".into(), |f| format!(" {f}: ")),
                            ));
                        }
                    }
                }
            }
        }
        format!("{text} @match{predicates})")
    }
    pub fn captures(&self, root: Node<'_>, source: &str) -> Option<Vec<(String, usize, usize)>> {
        let mut pending = vec![(0, root)];
        let mut bindings = BTreeMap::new();
        let mut captures = Vec::new();
        while let Some((index, node)) = pending.pop() {
            let expected = &self.nodes[index];
            if let Some(name) = &expected.variable {
                if !expression(node) || node.kind() == "metavariable" || node.is_missing() {
                    return None;
                }
                let text = source.get(node.byte_range())?;
                if bindings
                    .insert(name.as_str(), text)
                    .is_some_and(|previous| previous != text)
                {
                    return None;
                }
                captures.push((name.clone(), node.start_byte(), node.end_byte()));
                continue;
            }
            if node.kind() != expected.kind
                || node.is_named() != expected.named
                || node.is_missing()
            {
                return None;
            }
            if let Some(leaf) = &expected.leaf {
                if source.get(node.byte_range())? != leaf {
                    return None;
                }
                continue;
            }
            let mut cursor = node.walk();
            let children: Vec<_> = node
                .children(&mut cursor)
                .enumerate()
                .filter(|(_, n)| !comment(*n))
                .collect();
            if children.len() != expected.children.len() {
                return None;
            }
            for ((field, child_index), (i, child)) in expected.children.iter().zip(children).rev() {
                if field.as_deref() != node.field_name_for_child(i as u32) {
                    return None;
                }
                pending.push((*child_index, child));
            }
        }
        Some(captures)
    }
}
