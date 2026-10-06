//! Exact written test-module evidence shared by advice and move acknowledgment.
use super::*;
use std::collections::BTreeSet;

pub(crate) struct TestScope {
    pub aliases: BTreeMap<String, String>,
    pub shadowed: BTreeSet<String>,
    pub super_glob: bool,
}
/// Only a directly inventoried inline module with the written exact cfg(test) form.
/// This is consumer evidence, not cfg evaluation or inline-module move support.
pub(crate) fn test_scope(
    item: &Item,
    node: Node<'_>,
    source: &str,
    controls: (Instant, &AtomicBool),
    routes: &GlobRoutes<'_>,
) -> Result<Option<TestScope>, DomainError> {
    if item.kind != "mod_item"
        || !item.attributes.iter().any(|a| {
            a.range.start_byte < item.span.range.start_byte
                && source[a.range.start_byte..a.range.end_byte]
                    .chars()
                    .filter(|c| !c.is_whitespace())
                    .eq("#[cfg(test)]".chars())
        })
    {
        return Ok(None);
    }
    let Some(body) = node.child_by_field_name("body") else {
        return Ok(None);
    };
    let mut scope = TestScope {
        aliases: BTreeMap::new(),
        shadowed: BTreeSet::new(),
        super_glob: false,
    };
    for i in 0..body.named_child_count() {
        check(controls.0, controls.1)?;
        let child = body.named_child(i as u32).expect("module child");
        if let Some(name) = child.child_by_field_name("name") {
            scope
                .shadowed
                .insert(source[name.byte_range()].trim_start_matches("r#").into());
        }
        if child.kind() != "use_declaration" {
            continue;
        }
        for leaf in use_leaves(child, source, controls)? {
            let segments: Vec<_> = leaf.path.split("::").collect();
            let binding = leaf.binding.trim_start_matches("r#").to_owned();
            if segments.len() == 2 && segments[0] == "super" {
                let target = segments[1].trim_start_matches("r#").to_owned();
                if scope.aliases.insert(binding.clone(), target).is_some() {
                    scope.shadowed.insert(binding);
                }
            } else {
                scope.shadowed.insert(binding);
            }
        }
        scope.super_glob |= routes.reaches_file(&item.path, child, &item.path)?;
    }
    Ok(Some(scope))
}

/// Only written namespace compatibility, not binding identity or constructor access.
fn hoist_namespace_compatible(reference: Node<'_>, item: &Item, data: &ParsedFile) -> bool {
    match reference.kind() {
        "type_identifier" => matches!(
            item.kind.as_str(),
            "struct_item" | "enum_item" | "union_item" | "trait_item" | "type_item"
        ),
        "identifier" => match item.kind.as_str() {
            "function_item" | "const_item" | "static_item" => true,
            "struct_item" => data
                .tree
                .root_node()
                .named_descendant_for_byte_range(
                    item.span.range.start_byte,
                    item.span.range.end_byte,
                )
                .is_some_and(|declaration| {
                    declaration.kind() == "struct_item"
                        && declaration
                            .child_by_field_name("body")
                            .is_none_or(|body| body.kind() == "ordered_field_declaration_list")
                }),
            _ => false,
        },
        _ => false,
    }
}

/// A residual consumer need can be acknowledged only for its own selected file,
/// never for an attribute veto, caller repair failure, or another decision class.
pub(crate) fn test_consumer(
    need: &Need,
    data: &ParsedFile,
    source: &str,
    selected: &[(String, Item, String)],
    controls: (Instant, &AtomicBool),
    routes: &GlobRoutes<'_>,
) -> Result<bool, DomainError> {
    let hoist_risk = need.lexical_uncertainty.as_ref().is_some_and(|witness| {
        witness.reason == lexical::LexicalReason::UnsupportedPattern
            && matches!(
                witness.witness_relation,
                Some(
                    lexical::WitnessRelation::StatementMacroBeforeReference
                        | lexical::WitnessRelation::BlockMacroMayIntroduceItems
                )
            )
    });
    if need.attribute_range.is_some()
        || need.choice_target.is_some()
        || (need.lexical_uncertainty.is_some() && !hoist_risk)
        || (need.reason == DecisionReason::LexicalContextUnproved && !hoist_risk)
        || !matches!(
            (need.category, need.reason),
            ("binding_collision", DecisionReason::LexicalContextUnproved)
                | ("macro_dependency", DecisionReason::MacroContextUnexamined)
                | ("glob_dependency", DecisionReason::GlobBindingUnproved)
                | (
                    "module_context",
                    DecisionReason::ConditionalOrInheritedContext
                )
                | (
                    "unsupported_dependency_form",
                    DecisionReason::UnsupportedConstruct
                )
                | (
                    "unsupported_dependency_form",
                    DecisionReason::ExternalOrMissingBinding
                )
        )
        || need.item_ids.is_empty()
        || !need.item_ids.iter().all(|id| {
            selected
                .iter()
                .any(|(path, item, _)| path == &need.path && &item.id == id)
        })
    {
        return Ok(false);
    }
    let Some(mut node) = data
        .tree
        .root_node()
        .named_descendant_for_byte_range(need.range.start_byte, need.range.end_byte)
    else {
        return Ok(false);
    };
    if node.has_error() || node.is_missing() || node.kind() == "macro_definition" {
        return Ok(false);
    }
    // Check every enclosing written scope. #[test] is the test marker, not an
    // extra configuration. All other unexamined attributes retain their veto.
    loop {
        check(controls.0, controls.1)?;
        let direct_module =
            node.kind() == "mod_item" && node.parent().is_some_and(|p| p.kind() == "source_file");
        let mut previous = node.prev_named_sibling();
        while let Some(attribute) = previous {
            check(controls.0, controls.1)?;
            if matches!(attribute.kind(), "line_comment" | "block_comment") {
                previous = attribute.prev_named_sibling();
                continue;
            }
            if attribute.kind() != "attribute_item" {
                break;
            }
            let text: String = source[attribute.byte_range()]
                .chars()
                .filter(|c| !c.is_whitespace())
                .collect();
            if !context_independent_attribute(&text)
                && text != "#[test]"
                && !(direct_module && text == "#[cfg(test)]")
            {
                return Ok(false);
            }
            previous = attribute.prev_named_sibling();
        }
        for i in 0..node.named_child_count() {
            check(controls.0, controls.1)?;
            let child = node.named_child(i as u32).expect("scope child");
            if child.kind() == "inner_attribute_item"
                && !context_independent_attribute(&source[child.byte_range()])
            {
                return Ok(false);
            }
        }
        if node.kind() == "mod_item" {
            if !direct_module {
                return Ok(false); // No nested-module inference.
            }
            let Some(item) = data.items.iter().find(|item| {
                item.span.range.start_byte == node.start_byte()
                    && item.span.range.end_byte == node.end_byte()
            }) else {
                return Ok(false);
            };
            let Some(scope) = test_scope(item, node, source, controls, routes)? else {
                return Ok(false);
            };
            // Imports must actually reach the file or name its direct parent;
            // unknown forwarding/globs are not made safe by test placement.
            let consumer = data
                .tree
                .root_node()
                .named_descendant_for_byte_range(need.range.start_byte, need.range.end_byte)
                .expect("consumer node");
            if consumer.kind() == "use_declaration" {
                if use_facts(consumer, source, controls)?.0 {
                    return routes.reaches_file(&need.path, consumer, &need.path);
                }
                return Ok(use_leaves(consumer, source, controls)?.iter().all(|leaf| {
                    let parts: Vec<_> = leaf.path.split("::").collect();
                    parts.len() == 2
                        && parts[0] == "super"
                        && scope
                            .aliases
                            .contains_key(leaf.binding.trim_start_matches("r#"))
                }));
            }
            let targets: Vec<_> = selected
                .iter()
                .filter(|(_, item, _)| need.item_ids.contains(&item.id))
                .filter_map(|(_, item, _)| {
                    item.name
                        .as_deref()
                        .map(|name| (name.trim_start_matches("r#"), item))
                })
                .collect();
            let mut stack = vec![consumer];
            let mut observed = false;
            while let Some(reference) = stack.pop() {
                check(controls.0, controls.1)?;
                if matches!(
                    reference.kind(),
                    "string_literal"
                        | "raw_string_literal"
                        | "char_literal"
                        | "line_comment"
                        | "block_comment"
                ) {
                    continue;
                }
                let spelling = source[reference.byte_range()].trim_start_matches("r#");
                if matches!(reference.kind(), "identifier" | "type_identifier")
                    && targets.iter().any(|(name, _)| *name == spelling)
                {
                    if hoist_risk
                        && targets.iter().any(|(name, item)| {
                            *name == spelling && !hoist_namespace_compatible(reference, item, data)
                        })
                    {
                        return Ok(false);
                    }
                    observed = true;
                    let mut path = reference;
                    while let Some(parent) = path.parent().filter(|p| {
                        matches!(p.kind(), "scoped_identifier" | "scoped_type_identifier")
                    }) {
                        path = parent;
                    }
                    let direct_super = if path != reference {
                        let segments: Vec<_> = source[path.byte_range()].split("::").collect();
                        segments.len() >= 2 && segments[0] == "super" && segments[1] == spelling
                    } else {
                        // In token trees, :: is a sibling token rather than a CST path.
                        reference.prev_sibling().is_some_and(|separator| {
                            source[separator.byte_range()] == *"::"
                                && separator.prev_sibling().is_some_and(|base| {
                                    source[base.byte_range()] == *"super"
                                        && !base
                                            .prev_sibling()
                                            .is_some_and(|p| source[p.byte_range()] == *"::")
                                })
                        })
                    };
                    let qualified = path != reference
                        || reference
                            .prev_sibling()
                            .is_some_and(|p| source[p.byte_range()] == *"::");
                    let imported = !qualified
                        && !scope.shadowed.contains(spelling)
                        && (scope.super_glob
                            || scope
                                .aliases
                                .get(spelling)
                                .is_some_and(|target| target == spelling));
                    if !direct_super && !imported {
                        return Ok(false);
                    }
                    // A hoist witness must not mask a competing written local,
                    // namespace mismatch, pattern uncertainty, or recovery error.
                    if hoist_risk
                        && !direct_super
                        && lexical::test_consumer_binding(
                            &need.path, reference, source, spelling, controls,
                        )? != LexicalBinding::Absent
                    {
                        return Ok(false);
                    }
                }
                for i in 0..reference.named_child_count() {
                    stack.push(reference.named_child(i as u32).expect("consumer child"));
                }
            }
            // Macro tokens are only written candidates, never expanded or repaired.
            return Ok(observed);
        }
        let Some(parent) = node.parent() else {
            return Ok(false);
        };
        node = parent;
    }
}
