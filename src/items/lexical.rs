//! Spelling-specific written lexical evidence, not binding or constant resolution.
#[cfg(test)]
mod tests;
use super::{Need, check, use_facts, use_leaves};
use crate::result::{ByteRange, DomainError};
use rmcp::schemars::JsonSchema;
use serde::Serialize;
use std::{sync::atomic::AtomicBool, time::Instant};
use tree_sitter::Node;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(rename_all = "snake_case")]
pub enum LexicalReason {
    UnsupportedPattern,
    IdentifierPatternBindingOrConstant,
    ValueBindingInTypePosition,
    RelevantLocalImport,
    ConditionalLocalContext,
    SyntaxRecovery,
}
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct LexicalLocation {
    pub path: String,
    pub range: ByteRange,
    pub kind: String,
}
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct LexicalUncertainty {
    pub spelling: String,
    pub reason: LexicalReason,
    pub scope: LexicalLocation,
    pub pattern: Option<LexicalLocation>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LexicalBinding {
    Independent,
    Uncertain,
    Absent,
}
pub(crate) struct LexicalAssessment {
    pub binding: LexicalBinding,
    pub uncertainty: Option<LexicalUncertainty>,
}
impl LexicalAssessment {
    fn definite(binding: LexicalBinding) -> Self {
        Self {
            binding,
            uncertainty: None,
        }
    }
}
impl Need {
    pub(crate) fn lexical(&mut self, assessment: LexicalAssessment) {
        self.lexical_uncertainty = assessment.uncertainty;
    }
}
enum PatternProof<'a> {
    Binds,
    Disjoint,
    Unproved(LexicalReason, Node<'a>),
}
fn location(path: &str, node: Node<'_>) -> LexicalLocation {
    LexicalLocation {
        path: path.into(),
        range: ByteRange {
            start_byte: node.start_byte(),
            end_byte: node.end_byte(),
        },
        kind: node.kind().into(),
    }
}
fn uncertain(
    path: &str,
    name: &str,
    scope: Node<'_>,
    reason: LexicalReason,
    pattern: Option<Node<'_>>,
) -> LexicalAssessment {
    LexicalAssessment {
        binding: LexicalBinding::Uncertain,
        uncertainty: Some(LexicalUncertainty {
            spelling: name.into(),
            reason,
            scope: location(path, scope),
            pattern: pattern.map(|p| location(path, p)),
        }),
    }
}
/// A written value binding is not proof for a type-namespace occurrence.
fn value_binding(
    path: &str,
    name: &str,
    scope: Node<'_>,
    binding: Node<'_>,
    reference: Node<'_>,
) -> LexicalAssessment {
    if reference.kind() == "type_identifier" {
        uncertain(
            path,
            name,
            scope,
            LexicalReason::ValueBindingInTypePosition,
            Some(binding),
        )
    } else {
        LexicalAssessment::definite(LexicalBinding::Independent)
    }
}
fn matches(node: Node<'_>, source: &str, name: &str) -> bool {
    source[node.byte_range()].trim_start_matches("r#") == name
}
fn attributed(node: Node<'_>) -> bool {
    node.prev_named_sibling()
        .is_some_and(|p| p.kind() == "attribute_item")
}
/// Only binding positions are visited. Every child must be admitted, even after a binder.
fn pattern_proof<'a>(
    pattern: Node<'a>,
    source: &str,
    name: &str,
    simple: bool,
    controls: (Instant, &AtomicBool),
) -> Result<PatternProof<'a>, DomainError> {
    let mut stack = vec![(pattern, simple)];
    let mut binds = false;
    let mut unknown = None;
    while let Some((node, definite_identifier)) = stack.pop() {
        check(controls.0, controls.1)?;
        let mut children = Vec::new();
        if node.has_error() || node.is_missing() {
            unknown.get_or_insert((LexicalReason::SyntaxRecovery, node));
            continue;
        }
        match node.kind() {
            "identifier" | "shorthand_field_identifier" => {
                if matches(node, source, name) {
                    if definite_identifier {
                        binds = true;
                    } else {
                        unknown.get_or_insert((
                            LexicalReason::IdentifierPatternBindingOrConstant,
                            node,
                        ));
                    }
                }
            }
            "_"
            | "remaining_field_pattern"
            | "integer_literal"
            | "float_literal"
            | "negative_literal"
            | "boolean_literal"
            | "char_literal"
            | "string_literal"
            | "raw_string_literal" => {}
            "mut_pattern" | "ref_pattern" | "reference_pattern" => {
                for i in 0..node.named_child_count() {
                    check(controls.0, controls.1)?;
                    let child = node.named_child(i as u32).expect("pattern child");
                    if child.kind() != "mutable_specifier" {
                        children.push((
                            child,
                            definite_identifier || node.kind() != "reference_pattern",
                        ));
                    }
                }
            }
            "captured_pattern" => {
                let Some(capture) = node.named_child(0).filter(|c| c.kind() == "identifier") else {
                    unknown.get_or_insert((LexicalReason::UnsupportedPattern, node));
                    continue;
                };
                // The capture is a written binder, but its subpattern can still be unknown.
                children.push((capture, true));
                for i in 1..node.named_child_count() {
                    check(controls.0, controls.1)?;
                    let child = node.named_child(i as u32).expect("capture subpattern");
                    children.push((child, false));
                }
            }
            "or_pattern" => {
                // Admit only non-binding literal alternatives, never a binder from one arm.
                for i in 0..node.named_child_count() {
                    check(controls.0, controls.1)?;
                    let child = node.named_child(i as u32).expect("alternative pattern");
                    if matches!(
                        child.kind(),
                        "or_pattern"
                            | "string_literal"
                            | "raw_string_literal"
                            | "integer_literal"
                            | "line_comment"
                            | "block_comment"
                    ) {
                        children.push((child, false));
                    } else {
                        unknown.get_or_insert((LexicalReason::UnsupportedPattern, child));
                    }
                }
            }
            "tuple_pattern" | "slice_pattern" | "tuple_struct_pattern" | "struct_pattern" => {
                let constructor = node.child_by_field_name("type");
                for i in 0..node.named_child_count() {
                    check(controls.0, controls.1)?;
                    let child = node.named_child(i as u32).expect("pattern child");
                    if Some(child) != constructor {
                        children.push((child, false));
                    }
                }
            }
            "field_pattern" => {
                if let Some(child) = node.child_by_field_name("pattern") {
                    children.push((child, false));
                } else if let Some(child) = node.child_by_field_name("name") {
                    // Struct shorthand always declares a local; ref/mut are explicit too.
                    children.push((child, true));
                } else {
                    unknown.get_or_insert((LexicalReason::UnsupportedPattern, node));
                }
            }
            "match_pattern" => {
                for i in 0..node.named_child_count() {
                    check(controls.0, controls.1)?;
                    let child = node.named_child(i as u32).expect("match pattern child");
                    if Some(child) != node.child_by_field_name("condition") {
                        children.push((child, false));
                    }
                }
            }
            "line_comment" | "block_comment" => {}
            _ => {
                unknown.get_or_insert((LexicalReason::UnsupportedPattern, node));
            }
        }
        for child in children.into_iter().rev() {
            check(controls.0, controls.1)?;
            stack.push(child);
        }
    }
    Ok(if let Some((reason, node)) = unknown {
        PatternProof::Unproved(reason, node)
    } else if binds {
        PatternProof::Binds
    } else {
        PatternProof::Disjoint
    })
}
fn assess_pattern(
    path: &str,
    scope: Node<'_>,
    pattern: Node<'_>,
    source: &str,
    name: &str,
    reference: Node<'_>,
    controls: (Instant, &AtomicBool),
) -> Result<Option<LexicalAssessment>, DomainError> {
    if attributed(scope) || attributed(pattern) {
        return Ok(Some(uncertain(
            path,
            name,
            scope,
            LexicalReason::ConditionalLocalContext,
            Some(pattern),
        )));
    }
    let simple = matches!(
        scope.kind(),
        "let_declaration" | "function_item" | "closure_expression"
    );
    Ok(
        match pattern_proof(pattern, source, name, simple, controls)? {
            PatternProof::Binds => Some(value_binding(path, name, scope, pattern, reference)),
            PatternProof::Disjoint => None,
            PatternProof::Unproved(reason, p) => {
                Some(uncertain(path, name, scope, reason, Some(p)))
            }
        },
    )
}
fn contains(scope: Option<Node<'_>>, node: Node<'_>) -> bool {
    scope.is_some_and(|p| p.start_byte() <= node.start_byte() && p.end_byte() >= node.end_byte())
}
/// The first relevant inner uncertainty wins; a definite outer parameter cannot bypass it.
pub(crate) fn lexical_assessment(
    path: &str,
    node: Node<'_>,
    source: &str,
    name: &str,
    controls: (Instant, &AtomicBool),
    proven_import: bool,
) -> Result<LexicalAssessment, DomainError> {
    lexical_context(
        path,
        node,
        source,
        name.trim_start_matches("r#"),
        controls,
        proven_import,
        None,
    )
}
fn lexical_context(
    path: &str,
    node: Node<'_>,
    source: &str,
    name: &str,
    controls: (Instant, &AtomicBool),
    proven_import: bool,
    boundary: Option<Node<'_>>,
) -> Result<LexicalAssessment, DomainError> {
    let mut child = node;
    while let Some(parent) = child.parent() {
        check(controls.0, controls.1)?;
        if matches!(parent.kind(), "source_file" | "mod_item") {
            break;
        }
        if parent.has_error() || parent.is_missing() {
            return Ok(uncertain(
                path,
                name,
                parent,
                LexicalReason::SyntaxRecovery,
                None,
            ));
        }
        let pattern = match parent.kind() {
            "for_expression" if contains(parent.child_by_field_name("body"), node) => {
                parent.child_by_field_name("pattern")
            }
            "match_arm" => parent.child_by_field_name("pattern"),
            "if_expression" | "while_expression" => {
                let body = parent.child_by_field_name(if parent.kind() == "if_expression" {
                    "consequence"
                } else {
                    "body"
                });
                let condition = parent.child_by_field_name("condition");
                if condition.is_some_and(|c| c.kind() == "let_chain")
                    && (contains(body, node) || contains(condition, node))
                {
                    return Ok(uncertain(
                        path,
                        name,
                        parent,
                        LexicalReason::UnsupportedPattern,
                        condition,
                    ));
                }
                if contains(body, node) {
                    condition
                        .filter(|c| c.kind() == "let_condition")
                        .and_then(|c| c.child_by_field_name("pattern"))
                } else {
                    None
                }
            }
            _ => None,
        };
        if let Some(pattern) = pattern {
            if parent.kind() == "match_arm"
                && let Some(condition) = pattern.child_by_field_name("condition")
                && matches!(condition.kind(), "let_condition" | "let_chain")
            {
                return Ok(uncertain(
                    path,
                    name,
                    parent,
                    LexicalReason::UnsupportedPattern,
                    Some(condition),
                ));
            }
            if parent.kind() == "match_arm" {
                for i in 0..parent.named_child_count() {
                    check(controls.0, controls.1)?;
                    if parent.named_child(i as u32).is_some_and(|n| {
                        matches!(n.kind(), "attribute_item" | "inner_attribute_item")
                    }) {
                        return Ok(uncertain(
                            path,
                            name,
                            parent,
                            LexicalReason::ConditionalLocalContext,
                            Some(pattern),
                        ));
                    }
                }
            }
            if let Some(proof) =
                assess_pattern(path, parent, pattern, source, name, node, controls)?
            {
                return Ok(proof);
            }
        }
        if contains(parent.child_by_field_name("body"), node)
            && let Some(params) = parent.child_by_field_name("parameters")
        {
            for i in 0..params.named_child_count() {
                check(controls.0, controls.1)?;
                let parameter = params.named_child(i as u32).expect("parameter");
                if matches!(
                    parameter.kind(),
                    "line_comment" | "block_comment" | "self_parameter"
                ) {
                    continue;
                }
                if let Some(pattern) = parameter
                    .child_by_field_name("pattern")
                    .or_else(|| (parent.kind() == "closure_expression").then_some(parameter))
                    && let Some(proof) =
                        assess_pattern(path, parent, pattern, source, name, node, controls)?
                {
                    return Ok(proof);
                }
            }
        }
        if let Some(params) = parent.child_by_field_name("type_parameters") {
            for i in 0..params.named_child_count() {
                check(controls.0, controls.1)?;
                let parameter = params.named_child(i as u32).expect("generic parameter");
                if parameter
                    .child_by_field_name("name")
                    .is_some_and(|p| matches(p, source, name))
                {
                    return Ok(if parameter.kind() == "const_parameter" {
                        value_binding(path, name, parent, parameter, node)
                    } else {
                        LexicalAssessment::definite(LexicalBinding::Independent)
                    });
                }
            }
        }
        if parent.kind() == "block" {
            // Statement macros can introduce hoisted items as well as lets. A
            // definite written local cannot prove their expansion disjoint,
            // regardless of the macro's source order in this same block.
            for i in 0..parent.named_child_count() {
                check(controls.0, controls.1)?;
                let statement = parent.named_child(i as u32).expect("statement");
                if statement.kind() == "macro_invocation"
                    || (statement.kind() == "expression_statement"
                        && statement
                            .named_child(0)
                            .is_some_and(|n| n.kind() == "macro_invocation"))
                {
                    return Ok(uncertain(
                        path,
                        name,
                        parent,
                        LexicalReason::UnsupportedPattern,
                        Some(statement),
                    ));
                }
            }
            // Let bindings begin after their initializers. Later and nested patterns do not compete.
            for i in (0..parent.named_child_count()).rev() {
                check(controls.0, controls.1)?;
                let statement = parent.named_child(i as u32).expect("statement");
                if statement.end_byte() > child.start_byte() {
                    continue;
                }
                if statement.kind() == "let_declaration"
                    && let Some(pattern) = statement.child_by_field_name("pattern")
                    && let Some(proof) =
                        assess_pattern(path, statement, pattern, source, name, node, controls)?
                {
                    return Ok(proof);
                }
            }
            // Block items and imports are hoisted, unlike let bindings.
            for i in 0..parent.named_child_count() {
                check(controls.0, controls.1)?;
                let statement = parent.named_child(i as u32).expect("statement");
                if statement.kind() == "use_declaration" {
                    let relevant = use_facts(statement, source, controls)?.0
                        || (!proven_import
                            && use_leaves(statement, source, controls)?
                                .iter()
                                .any(|l| l.binding.trim_start_matches("r#") == name));
                    if relevant {
                        return Ok(uncertain(
                            path,
                            name,
                            parent,
                            LexicalReason::RelevantLocalImport,
                            Some(statement),
                        ));
                    }
                }
                if matches!(
                    statement.kind(),
                    "function_item"
                        | "struct_item"
                        | "enum_item"
                        | "type_item"
                        | "const_item"
                        | "static_item"
                        | "union_item"
                        | "trait_item"
                        | "mod_item"
                ) && statement
                    .child_by_field_name("name")
                    .is_some_and(|n| matches(n, source, name))
                {
                    if attributed(statement) {
                        return Ok(uncertain(
                            path,
                            name,
                            parent,
                            LexicalReason::ConditionalLocalContext,
                            Some(statement),
                        ));
                    }
                    return Ok(
                        if matches!(
                            statement.kind(),
                            "function_item" | "const_item" | "static_item"
                        ) {
                            value_binding(path, name, parent, statement, node)
                        } else {
                            LexicalAssessment::definite(LexicalBinding::Independent)
                        },
                    );
                }
            }
        }
        if Some(parent) == boundary {
            break;
        }
        // A nested item does not inherit the enclosing function's lexical locals.
        // Keep its unexamined block-item context explicit rather than treating an
        // outer parameter/let as a proven binding inside that item.
        if matches!(
            parent.kind(),
            "function_item" | "const_item" | "static_item"
        ) && parent.parent().is_some_and(|p| p.kind() == "block")
        {
            return Ok(uncertain(
                path,
                name,
                parent,
                LexicalReason::ConditionalLocalContext,
                None,
            ));
        }
        child = parent;
    }
    Ok(LexicalAssessment::definite(LexicalBinding::Absent))
}
pub(crate) fn local(
    node: Node<'_>,
    item: Node<'_>,
    source: &str,
    name: &str,
    deadline: Instant,
    cancelled: &AtomicBool,
) -> bool {
    lexical_context(
        "",
        node,
        source,
        name,
        (deadline, cancelled),
        false,
        Some(item),
    )
    .is_ok_and(|a| a.binding == LexicalBinding::Independent)
}
pub(crate) fn lexical_binding(
    node: Node<'_>,
    source: &str,
    name: &str,
    controls: (Instant, &AtomicBool),
) -> LexicalBinding {
    lexical_assessment("", node, source, name, controls, false)
        .map(|a| a.binding)
        .unwrap_or(LexicalBinding::Uncertain)
}
pub(crate) fn lexical_with_import_proof(
    node: Node<'_>,
    source: &str,
    name: &str,
    controls: (Instant, &AtomicBool),
) -> LexicalBinding {
    lexical_assessment("", node, source, name, controls, true)
        .map(|a| a.binding)
        .unwrap_or(LexicalBinding::Uncertain)
}
