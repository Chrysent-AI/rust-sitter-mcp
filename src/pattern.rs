//! Expression sugar compilation and full-source structural post-filters.
use crate::{
    query::{CompiledQuery, is_expression},
    result::DomainError,
};
use std::collections::{BTreeMap, BTreeSet};
use tree_sitter::{Node, Parser, Query};

const PREFIX: &str = "fn __ssr_wrapper() { let __ssr_value = (";
const SUFFIX: &str = "\n);\n}";

pub struct Pattern {
    nodes: Vec<PatternNode>,
    expression_slots: BTreeSet<(String, Option<String>)>,
}
type ParentField = Option<(usize, Option<String>)>;
struct PatternNode {
    kind: String,
    named: bool,
    leaf: Option<String>,
    variable: Option<String>,
    range: std::ops::Range<usize>,
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
        "arguments"
        | "parenthesized_expression"
        | "tuple_expression"
        | "array_expression"
        | "unary_expression"
        | "reference_expression"
        | "try_expression"
        | "await_expression"
        | "index_expression"
        | "range_expression" => true,
        "binary_expression" | "assignment_expression" | "compound_assignment_expr" => {
            matches!(field, Some("left" | "right"))
        }
        _ => false,
    }
}
fn supported(kind: &str) -> bool {
    matches!(
        kind,
        "call_expression"
            | "field_expression"
            | "identifier"
            | "self"
            | "scoped_identifier"
            | "super"
            | "crate"
            | "field_identifier"
            | "arguments"
            | "metavariable"
            | "integer_literal"
            | "float_literal"
            | "boolean_literal"
            | "char_literal"
            | "string_literal"
            | "raw_string_literal"
            | "parenthesized_expression"
            | "unit_expression"
            | "tuple_expression"
            | "array_expression"
            | "unary_expression"
            | "reference_expression"
            | "mutable_specifier"
            | "try_expression"
            | "await_expression"
            | "index_expression"
            | "binary_expression"
            | "assignment_expression"
            | "compound_assignment_expr"
            | "range_expression"
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
        let mut invalid_position = None;
        let mut previous_token = None;
        while let Some(node) = scan.pop() {
            if comment(node) {
                continue;
            }
            if literal(node.kind()) {
                if node.has_error() {
                    recovery.get_or_insert(offset(node, source.len()));
                }
                previous_token = Some(&wrapped[node.byte_range()]);
                continue;
            }
            if node.is_error() || node.is_missing() {
                recovery.get_or_insert(offset(node, source.len()));
            }
            if node.child_count() == 0 && wrapped[node.byte_range()].starts_with('$') {
                let mut ancestor = node.parent();
                while let Some(parent) = ancestor {
                    if matches!(parent.kind(), "token_tree" | "token_tree_pattern") {
                        return Err(error(
                            "UNSUPPORTED_PLACEHOLDER_POSITION",
                            "metavariables inside macro token trees are unsupported; use search_query",
                            offset(node, source.len()),
                            source.len(),
                        ));
                    }
                    ancestor = parent.parent();
                }
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
                let position = offset(node, source.len());
                if source[..position]
                    .chars()
                    .next_back()
                    .is_some_and(|c| c.is_alphanumeric() || c == '_')
                {
                    return Err(error(
                        "INVALID_PATTERN",
                        "metavariables cannot be part of a token",
                        position,
                        source.len(),
                    ));
                }
                if matches!(previous_token, Some("." | "::")) {
                    return Err(error(
                        "UNSUPPORTED_PLACEHOLDER_POSITION",
                        "field/method/path names must be concrete",
                        position,
                        source.len(),
                    ));
                }
                if !variable_position(node) {
                    invalid_position.get_or_insert(position);
                }
            }
            if node.child_count() == 0 {
                previous_token = Some(&wrapped[node.byte_range()]);
            }
            for i in (0..node.child_count()).rev() {
                scan.push(node.child(i).expect("child index"));
            }
        }
        if let Some(position) = invalid_position {
            return Err(error(
                "UNSUPPORTED_PLACEHOLDER_POSITION",
                "metavariables bind one expression, not field/method/path names, types, bindings or token trees",
                position,
                source.len(),
            ));
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
            || outer.end_byte() != PREFIX.len() + source.len() + 2
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
        if roots.len() != 1 || !is_expression(roots[0]) {
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
                range: offset(node, source.len())
                    ..node
                        .end_byte()
                        .saturating_sub(PREFIX.len())
                        .min(source.len()),
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
        let metadata: serde_json::Value = serde_json::from_str(tree_sitter_rust::NODE_TYPES)
            .map_err(|_| DomainError::new("INTERNAL", "invalid linked Rust grammar metadata"))?;
        let mut expression_slots = BTreeSet::new();
        for parent in metadata.as_array().expect("grammar node inventory") {
            let kind = parent["type"].as_str().expect("grammar node kind");
            let fields = parent["fields"]
                .as_object()
                .into_iter()
                .flat_map(|fields| {
                    fields
                        .iter()
                        .map(|(field, shape)| (Some(field.clone()), shape))
                })
                .chain(parent.get("children").map(|shape| (None, shape)));
            for (field, shape) in fields {
                if shape["types"]
                    .as_array()
                    .is_some_and(|types| types.iter().any(|t| t["type"] == "_expression"))
                {
                    expression_slots.insert((kind.to_owned(), field));
                }
            }
        }
        // Callees have an expanded _expression_except_range inventory rather than a supertype slot.
        expression_slots.insert(("call_expression".into(), Some("function".into())));
        let pattern = Self {
            nodes,
            expression_slots,
        };
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
    pub(crate) fn references(&self) -> Vec<(String, std::ops::Range<usize>)> {
        self.nodes
            .iter()
            .filter_map(|node| {
                node.variable
                    .as_ref()
                    .map(|name| (name.clone(), node.range.clone()))
            })
            .collect()
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
                        // Call callee slots use _expression_except_range, not the _expression supertype.
                        // The native query rejects that supertype there; the same expression check runs in captures().
                        let callee = self.nodes.iter().any(|parent| {
                            parent.kind == "call_expression"
                                && parent.children.iter().any(|(field, child)| {
                                    *child == index && field.as_deref() == Some("function")
                                })
                        });
                        let kind = if callee { "_" } else { "_expression" };
                        text.push_str(&format!("({kind}) @__ssr_v_{name}_{index}"));
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
        let parent = root.parent()?;
        let field = (0..parent.child_count()).find_map(|i| {
            (parent.child(i) == Some(root))
                .then(|| parent.field_name_for_child(i))
                .flatten()
        });
        if !parent.is_error()
            && !self
                .expression_slots
                .contains(&(parent.kind().into(), field.map(str::to_owned)))
        {
            return None;
        }
        let mut pending = vec![(0, root)];
        let mut bindings = BTreeMap::new();
        let mut captures = Vec::new();
        while let Some((index, node)) = pending.pop() {
            let expected = &self.nodes[index];
            if let Some(name) = &expected.variable {
                if !is_expression(node) || node.kind() == "metavariable" || node.is_missing() {
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
