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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(rename_all = "snake_case")]
pub enum WitnessRelation {
    StatementMacroBeforeReference,
    BlockMacroMayIntroduceItems,
}
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct LexicalUncertainty {
    pub spelling: String,
    pub reason: LexicalReason,
    pub scope: LexicalLocation,
    pub pattern: Option<LexicalLocation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub witness_relation: Option<WitnessRelation>,
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
    pub definite_binding: Option<LexicalLocation>,
}
impl LexicalAssessment {
    fn definite(binding: LexicalBinding) -> Self {
        Self {
            binding,
            uncertainty: None,
            definite_binding: None,
        }
    }
    fn independent(path: &str, binding: Node<'_>) -> Self {
        Self {
            binding: LexicalBinding::Independent,
            uncertainty: None,
            definite_binding: Some(location(path, binding)),
        }
    }
}
impl Need {
    pub(crate) fn lexical(&mut self, assessment: LexicalAssessment) {
        self.lexical_uncertainty = assessment.uncertainty;
    }
}
enum PatternProof<'a> {
    Binds(Node<'a>, Vec<crate::plan::SourceAnchor>),
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
        definite_binding: None,
        uncertainty: Some(LexicalUncertainty {
            spelling: name.into(),
            reason,
            scope: location(path, scope),
            pattern: pattern.map(|p| location(path, p)),
            witness_relation: None,
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
        LexicalAssessment::independent(path, binding)
    }
}
fn matches(node: Node<'_>, source: &str, name: &str) -> bool {
    source[node.byte_range()].trim_start_matches("r#") == name
}
fn attributed(node: Node<'_>, source: &str, cfg: Option<&super::DeclaredCfg>) -> bool {
    if cfg.is_some() {
        super::cfg::attributed(node, source, cfg)
    } else {
        node.prev_named_sibling()
            .is_some_and(|p| p.kind() == "attribute_item")
    }
}
#[derive(Clone, Copy)]
enum IdentifierPosition {
    Explicit,
    Argument,
    Ambiguous,
}
/// Only binding positions are visited. Every child must be admitted, even after a binder.
#[allow(clippy::too_many_arguments)]
fn pattern_proof<'a>(
    pattern: Node<'a>,
    source: &str,
    name: &str,
    simple: bool,
    controls: (Instant, &AtomicBool),
    path: &str,
    globs: Option<&super::GlobRoutes<'_>>,
) -> Result<PatternProof<'a>, DomainError> {
    let mut stack = vec![(
        pattern,
        if simple {
            IdentifierPosition::Explicit
        } else {
            IdentifierPosition::Ambiguous
        },
    )];
    let mut competitor = None;
    let mut provenance = Vec::new();
    let mut binder = None;
    let mut binds = false;
    let mut unknown = None;
    while let Some((node, position)) = stack.pop() {
        check(controls.0, controls.1)?;
        let mut children = Vec::new();
        if node.has_error() || node.is_missing() {
            unknown.get_or_insert((LexicalReason::SyntaxRecovery, node));
            continue;
        }
        match node.kind() {
            "identifier" | "shorthand_field_identifier" => {
                if matches(node, source, name) {
                    let definite = match position {
                        IdentifierPosition::Explicit => true,
                        IdentifierPosition::Ambiguous => false,
                        IdentifierPosition::Argument => {
                            let competing = if let Some(competing) = competitor {
                                competing
                            } else {
                                let original =
                                    super::pattern_name_competes(pattern, source, name, controls)?;
                                let competing = if original && globs.is_some() {
                                    let (competing, trace) = super::pattern_competition(
                                        pattern, source, name, controls, path, globs,
                                    )?;
                                    provenance = trace;
                                    competing
                                } else {
                                    original
                                };
                                competitor = Some(competing);
                                competing
                            };
                            !competing
                        }
                    };
                    if definite {
                        binds = true;
                        if binder.replace(node).is_some() && !provenance.is_empty() {
                            unknown.get_or_insert((LexicalReason::UnsupportedPattern, node));
                        }
                    } else {
                        unknown.get_or_insert((
                            LexicalReason::IdentifierPatternBindingOrConstant,
                            node,
                        ));
                    }
                }
            }
            // A written path denotes a constructor/constant, never a binder.
            // The dependency scanner still visits the path independently.
            "scoped_identifier" | "scoped_type_identifier" => {}
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
                            if node.kind() == "reference_pattern" {
                                position
                            } else {
                                IdentifierPosition::Explicit
                            },
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
                children.push((capture, IdentifierPosition::Explicit));
                for i in 1..node.named_child_count() {
                    check(controls.0, controls.1)?;
                    let child = node.named_child(i as u32).expect("capture subpattern");
                    children.push((child, IdentifierPosition::Ambiguous));
                }
            }
            "or_pattern" => {
                // Admit only written non-binding alternatives, never a binder
                // from one arm or an unresolved bare constant/binder alternative.
                for i in 0..node.named_child_count() {
                    check(controls.0, controls.1)?;
                    let child = node.named_child(i as u32).expect("alternative pattern");
                    if matches!(
                        child.kind(),
                        "or_pattern"
                            | "string_literal"
                            | "raw_string_literal"
                            | "integer_literal"
                            | "float_literal"
                            | "negative_literal"
                            | "boolean_literal"
                            | "char_literal"
                            | "scoped_identifier"
                            | "scoped_type_identifier"
                            | "line_comment"
                            | "block_comment"
                    ) {
                        children.push((child, IdentifierPosition::Ambiguous));
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
                        children.push((child, IdentifierPosition::Argument));
                    }
                }
            }
            "field_pattern" => {
                if let Some(child) = node.child_by_field_name("pattern") {
                    children.push((child, IdentifierPosition::Argument));
                } else if let Some(child) = node.child_by_field_name("name") {
                    // Struct shorthand always declares a local; ref/mut are explicit too.
                    children.push((child, IdentifierPosition::Explicit));
                } else {
                    unknown.get_or_insert((LexicalReason::UnsupportedPattern, node));
                }
            }
            "match_pattern" => {
                for i in 0..node.named_child_count() {
                    check(controls.0, controls.1)?;
                    let child = node.named_child(i as u32).expect("match pattern child");
                    if Some(child) != node.child_by_field_name("condition") {
                        children.push((child, IdentifierPosition::Ambiguous));
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
        PatternProof::Binds(binder.expect("binding witness"), provenance)
    } else {
        PatternProof::Disjoint
    })
}
/// Potential constants in moved patterns remain dependencies even when the arm
/// never reads the name. Reuse the binder proof; do not treat forced binders or
/// constructor paths as ambiguous pattern arguments.
pub(crate) fn pattern_uncertainty(
    path: &str,
    node: Node<'_>,
    source: &str,
    controls: (Instant, &AtomicBool),
    globs: Option<&super::GlobRoutes<'_>>,
) -> Result<Option<LexicalAssessment>, DomainError> {
    let name = source[node.byte_range()].trim_start_matches("r#");
    let mut child = node;
    while let Some(parent) = child.parent() {
        check(controls.0, controls.1)?;
        if (matches!(
            parent.kind(),
            "let_declaration" | "parameter" | "let_condition" | "match_arm" | "for_expression"
        ) && parent.child_by_field_name("pattern") == Some(child))
            || parent.kind() == "closure_parameters"
        {
            if let PatternProof::Unproved(
                LexicalReason::IdentifierPatternBindingOrConstant,
                witness,
            ) = pattern_proof(
                child,
                source,
                name,
                matches!(
                    parent.kind(),
                    "let_declaration" | "parameter" | "closure_parameters"
                ),
                controls,
                path,
                globs,
            )? && witness == node
                && super::pattern_competition(child, source, name, controls, path, globs)?.0
            {
                return Ok(Some(uncertain(
                    path,
                    name,
                    parent,
                    LexicalReason::IdentifierPatternBindingOrConstant,
                    Some(witness),
                )));
            }
            return Ok(None);
        }
        if matches!(parent.kind(), "block" | "source_file") {
            break;
        }
        child = parent;
    }
    Ok(None)
}
#[allow(clippy::too_many_arguments)] // The cfg evidence belongs to the same lexical audit.
fn assess_pattern(
    path: &str,
    scope: Node<'_>,
    pattern: Node<'_>,
    source: &str,
    name: &str,
    reference: Node<'_>,
    controls: (Instant, &AtomicBool),
    cfg: Option<&super::DeclaredCfg>,
    globs: Option<&super::GlobRoutes<'_>>,
) -> Result<Option<LexicalAssessment>, DomainError> {
    if attributed(scope, source, cfg) || attributed(pattern, source, cfg) {
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
        match pattern_proof(pattern, source, name, simple, controls, path, globs)? {
            PatternProof::Binds(_, _) => Some(value_binding(path, name, scope, pattern, reference)),
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
/// Only chain CST nodes are flattened; expressions (including parentheses,
/// closures and nested conditions) do not contribute binders to this chain.
#[allow(clippy::too_many_arguments)]
fn assess_chain(
    path: &str,
    scope: Node<'_>,
    chain: Node<'_>,
    in_body: bool,
    source: &str,
    name: &str,
    reference: Node<'_>,
    controls: (Instant, &AtomicBool),
    cfg: Option<&super::DeclaredCfg>,
    globs: Option<&super::GlobRoutes<'_>>,
) -> Result<Option<LexicalAssessment>, DomainError> {
    let mut stack = vec![chain];
    let mut preceding = Vec::new();
    while let Some(operand) = stack.pop() {
        check(controls.0, controls.1)?;
        if operand.kind() == "let_chain" {
            for i in (0..operand.named_child_count()).rev() {
                check(controls.0, controls.1)?;
                stack.push(operand.named_child(i as u32).expect("chain operand"));
            }
        } else if operand.kind() == "let_condition"
            && (in_body || operand.end_byte() <= reference.start_byte())
        {
            let Some(pattern) = operand.child_by_field_name("pattern") else {
                return Ok(Some(uncertain(
                    path,
                    name,
                    scope,
                    LexicalReason::UnsupportedPattern,
                    Some(operand),
                )));
            };
            preceding.push(pattern);
        }
    }
    // Later successful bindings shadow earlier ones. The containing operand's
    // own pattern is excluded, so its initializer still sees the outer binding.
    for pattern in preceding.into_iter().rev() {
        check(controls.0, controls.1)?;
        if let Some(proof) = assess_pattern(
            path, scope, pattern, source, name, reference, controls, cfg, globs,
        )? {
            return Ok(Some(proof));
        }
    }
    Ok(None)
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
    lexical_assessment_with_cfg(path, node, source, name, controls, proven_import, None)
}
pub(crate) fn lexical_assessment_with_cfg(
    path: &str,
    node: Node<'_>,
    source: &str,
    name: &str,
    controls: (Instant, &AtomicBool),
    proven_import: bool,
    cfg: Option<&super::DeclaredCfg>,
) -> Result<LexicalAssessment, DomainError> {
    lexical_assessment_with_globs(path, node, source, name, controls, proven_import, cfg, None)
}
#[allow(clippy::too_many_arguments)]
pub(crate) fn lexical_assessment_with_globs(
    path: &str,
    node: Node<'_>,
    source: &str,
    name: &str,
    controls: (Instant, &AtomicBool),
    proven_import: bool,
    cfg: Option<&super::DeclaredCfg>,
    globs: Option<&super::GlobRoutes<'_>>,
) -> Result<LexicalAssessment, DomainError> {
    lexical_context(
        path,
        node,
        source,
        name.trim_start_matches("r#"),
        controls,
        proven_import,
        None,
        true,
        cfg,
        globs,
    )
}
/// Audit written bindings only for test-risk acknowledgment, never as a binding proof.
/// The ordinary assessment still reports potentially item-producing block macros.
pub(crate) fn test_consumer_binding(
    path: &str,
    node: Node<'_>,
    source: &str,
    name: &str,
    controls: (Instant, &AtomicBool),
) -> Result<LexicalBinding, DomainError> {
    lexical_context(
        path,
        node,
        source,
        name.trim_start_matches("r#"),
        controls,
        false,
        None,
        false,
        None,
        None,
    )
    .map(|a| a.binding)
}
#[allow(clippy::too_many_arguments)] // Explicit audit mode cannot change ordinary scan policy.
fn lexical_context(
    path: &str,
    node: Node<'_>,
    source: &str,
    name: &str,
    controls: (Instant, &AtomicBool),
    proven_import: bool,
    boundary: Option<Node<'_>>,
    statement_macros: bool,
    cfg: Option<&super::DeclaredCfg>,
    globs: Option<&super::GlobRoutes<'_>>,
) -> Result<LexicalAssessment, DomainError> {
    // Direct invocations, macro expression statements and unexamined outer
    // attributes can introduce block items, visible even before the expansion.
    // Audit enclosing blocks before any local proof or nested-item boundary.
    // The test-risk scan ignores only standalone macros, not attribute uncertainty.
    let mut child = node;
    while let Some(parent) = child.parent() {
        check(controls.0, controls.1)?;
        if matches!(parent.kind(), "source_file" | "mod_item") {
            break;
        }
        if parent.kind() == "block" {
            for i in 0..parent.named_child_count() {
                check(controls.0, controls.1)?;
                let statement = parent.named_child(i as u32).expect("statement");
                if statement.kind() == "attribute_item"
                    && (statement.has_error()
                        || statement.is_missing()
                        || (!super::context_independent_attribute(&source[statement.byte_range()])
                            && super::cfg::attribute(&source[statement.byte_range()], cfg, false)
                                .is_err()))
                {
                    // Doc comments are inert comment nodes; derive spellings alone
                    // do not establish built-in macro identity in this lexical scope.
                    return Ok(uncertain(
                        path,
                        name,
                        parent,
                        LexicalReason::ConditionalLocalContext,
                        Some(statement),
                    ));
                }
                if statement_macros
                    && (statement.kind() == "macro_invocation"
                        || (statement.kind() == "expression_statement"
                            && statement
                                .named_child(0)
                                .is_some_and(|n| n.kind() == "macro_invocation")))
                {
                    let mut assessment = uncertain(
                        path,
                        name,
                        parent,
                        LexicalReason::UnsupportedPattern,
                        Some(statement),
                    );
                    assessment
                        .uncertainty
                        .as_mut()
                        .expect("uncertainty")
                        .witness_relation = Some(if statement.start_byte() <= child.start_byte() {
                        WitnessRelation::StatementMacroBeforeReference
                    } else {
                        WitnessRelation::BlockMacroMayIntroduceItems
                    });
                    return Ok(assessment);
                }
            }
        }
        if Some(parent) == boundary {
            break;
        }
        child = parent;
    }
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
            // Arm bindings are visible in the guard and value, not in the
            // constructor path or other occurrences inside the pattern itself.
            "match_arm" => parent.child_by_field_name("pattern").filter(|pattern| {
                contains(parent.child_by_field_name("value"), node)
                    || contains(pattern.child_by_field_name("condition"), node)
            }),
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
                    if let Some(proof) = assess_chain(
                        path,
                        parent,
                        condition.expect("chain condition"),
                        contains(body, node),
                        source,
                        name,
                        node,
                        controls,
                        cfg,
                        globs,
                    )? {
                        return Ok(proof);
                    }
                    // A disjoint chain leaves the enclosing scope available.
                    None
                } else if contains(body, node) {
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
                            && super::cfg::attribute(&source[n.byte_range()], cfg, true).is_err()
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
            if let Some(proof) = assess_pattern(
                path, parent, pattern, source, name, node, controls, cfg, globs,
            )? {
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
                    && let Some(proof) = assess_pattern(
                        path, parent, pattern, source, name, node, controls, cfg, globs,
                    )?
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
                        LexicalAssessment::independent(path, parameter)
                    });
                }
            }
        }
        if parent.kind() == "block" {
            // Let bindings begin after the whole declaration, including a
            // let-else failure block. Neither initializer nor else sees them.
            // Later and nested patterns do not compete.
            for i in (0..parent.named_child_count()).rev() {
                check(controls.0, controls.1)?;
                let statement = parent.named_child(i as u32).expect("statement");
                if statement.end_byte() > child.start_byte() {
                    continue;
                }
                if statement.kind() == "let_declaration"
                    && let Some(pattern) = statement.child_by_field_name("pattern")
                    && let Some(proof) = assess_pattern(
                        path, statement, pattern, source, name, node, controls, cfg, globs,
                    )?
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
                    if attributed(statement, source, cfg) {
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
                            LexicalAssessment::independent(path, statement)
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
/// Private provenance is limited to bindings admitted by spelling-specific glob proof.
#[derive(Serialize)]
pub(crate) struct GlobBinding {
    pub item_id: String,
    pub destination: String,
    pub identifier: LexicalLocation,
    pub pattern: LexicalLocation,
    pub scope: LexicalLocation,
    pub spelling: String,
    pub routes: Vec<crate::plan::SourceAnchor>,
    pub references: Vec<LexicalLocation>,
}
fn pattern_site<'a>(
    node: Node<'a>,
    controls: (Instant, &AtomicBool),
) -> Result<Option<(Node<'a>, Node<'a>, bool)>, DomainError> {
    let mut child = node;
    while let Some(parent) = child.parent() {
        check(controls.0, controls.1)?;
        if (matches!(
            parent.kind(),
            "let_declaration" | "parameter" | "let_condition" | "match_arm" | "for_expression"
        ) && parent.child_by_field_name("pattern") == Some(child))
            || parent.kind() == "closure_parameters"
        {
            return Ok(Some((
                parent,
                child,
                matches!(
                    parent.kind(),
                    "let_declaration" | "parameter" | "closure_parameters"
                ),
            )));
        }
        if matches!(parent.kind(), "block" | "source_file") {
            break;
        }
        child = parent;
    }
    Ok(None)
}
pub(crate) fn glob_bindings(
    files: &std::collections::BTreeMap<String, super::FileSnapshot>,
    parsed: &std::collections::BTreeMap<String, super::ParsedFile>,
    selected: &[(String, super::Item, String)],
    globs: &super::GlobRoutes<'_>,
    controls: (Instant, &AtomicBool),
) -> Result<Vec<GlobBinding>, DomainError> {
    let mut ledger = Vec::new();
    let mut bytes = 0;
    let mut work = 0;
    for (path, item, destination) in selected {
        check(controls.0, controls.1)?;
        let source = &files[path].source;
        let root = parsed[path]
            .tree
            .root_node()
            .named_descendant_for_byte_range(item.span.range.start_byte, item.span.range.end_byte)
            .expect("selected item");
        let mut stack = vec![root];
        let mut references = Vec::new();
        let start = ledger.len();
        while let Some(node) = stack.pop() {
            check(controls.0, controls.1)?;
            if matches!(
                node.kind(),
                "token_tree"
                    | "macro_definition"
                    | "macro_invocation"
                    | "attribute_item"
                    | "inner_attribute_item"
            ) {
                continue;
            }
            if node.kind() == "identifier" {
                work += 1;
                if work > 100_000 {
                    return Err(DomainError::new(
                        "reference_work_limit",
                        "glob pattern ledger work guard reached",
                    ));
                }
                if super::reference_role(node) {
                    references.push(node);
                } else if let Some((scope, pattern, simple)) = pattern_site(node, controls)? {
                    let name = source[node.byte_range()].trim_start_matches("r#");
                    if let PatternProof::Binds(identifier, routes) =
                        pattern_proof(pattern, source, name, simple, controls, path, Some(globs))?
                        && identifier == node
                        && !routes.is_empty()
                    {
                        let entry = GlobBinding {
                            item_id: item.id.clone(),
                            destination: destination.clone(),
                            identifier: location(path, identifier),
                            pattern: location(path, pattern),
                            scope: location(path, scope),
                            spelling: name.into(),
                            routes,
                            references: Vec::new(),
                        };
                        bytes += serde_json::to_vec(&entry)
                            .expect("private provenance")
                            .len()
                            + 256;
                        if bytes > 128 * 1024 * 1024 {
                            return Err(DomainError::new(
                                "analysis_descriptor_bytes",
                                "glob pattern ledger guard reached",
                            ));
                        }
                        ledger.push(entry);
                    }
                }
            }
            for i in (0..node.named_child_count()).rev() {
                check(controls.0, controls.1)?;
                stack.push(node.named_child(i as u32).expect("child"));
            }
        }
        for node in references {
            check(controls.0, controls.1)?;
            let name = source[node.byte_range()].trim_start_matches("r#");
            if !ledger[start..].iter().any(|b| b.spelling == name) {
                continue;
            }
            let assessment = lexical_assessment_with_globs(
                path,
                node,
                source,
                name,
                controls,
                false,
                None,
                Some(globs),
            )?;
            if assessment.binding == LexicalBinding::Independent
                && let Some(binding) = assessment.definite_binding
            {
                for entry in &mut ledger[start..] {
                    check(controls.0, controls.1)?;
                    if entry.spelling == name && entry.pattern.range == binding.range {
                        bytes += path.len() + 256;
                        if bytes > 128 * 1024 * 1024 {
                            return Err(DomainError::new(
                                "analysis_descriptor_bytes",
                                "glob reference ledger guard reached",
                            ));
                        }
                        entry.references.push(location(path, node));
                    }
                }
            }
        }
    }
    Ok(ledger)
}
pub(crate) fn final_glob_binding(
    path: &str,
    identifier: Node<'_>,
    pattern: Node<'_>,
    scope: Node<'_>,
    source: &str,
    globs: &super::GlobRoutes<'_>,
    controls: (Instant, &AtomicBool),
) -> Result<bool, DomainError> {
    let Some((actual_scope, actual_pattern, simple)) = pattern_site(identifier, controls)? else {
        return Ok(false);
    };
    if actual_scope != scope
        || actual_pattern != pattern
        || !globs.pattern_context(path, identifier)?
    {
        return Ok(false);
    }
    let name = source[identifier.byte_range()].trim_start_matches("r#");
    // Recheck all final ancestors even when the source glob did not move.
    if super::pattern_competition(pattern, source, name, controls, path, Some(globs))?.0 {
        return Ok(false);
    }
    Ok(
        matches!(pattern_proof(pattern, source, name, simple, controls, path, Some(globs))?, PatternProof::Binds(binder, _) if binder == identifier),
    )
}
pub(crate) fn local(
    path: &str,
    node: Node<'_>,
    item: Node<'_>,
    source: &str,
    name: &str,
    controls: (Instant, &AtomicBool),
    globs: Option<&super::GlobRoutes<'_>>,
) -> bool {
    lexical_context(
        path,
        node,
        source,
        name,
        controls,
        false,
        Some(item),
        true,
        None,
        globs,
    )
    .is_ok_and(|a| a.binding == LexicalBinding::Independent)
}
