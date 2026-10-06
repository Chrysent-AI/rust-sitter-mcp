//! Caller-assumed prelude evidence, separate from written-binding repairs.
use super::RewriteTarget;
use crate::{
    items::{self, DecisionReason, Item, ModuleEvidence, Need, ParsedFile},
    plan::SourceAnchor,
    result::{Coverage, DomainError},
    rewrites::Repair,
    scope::FileSnapshot,
};
use rmcp::schemars::JsonSchema;
use serde::Serialize;
use std::{collections::BTreeMap, sync::atomic::AtomicBool, time::Instant};
use tree_sitter::Node;

// Edition-independent subset of https://doc.rust-lang.org/std/prelude/v1/index.html.
// This is an explicit caller assumption, not Cargo/build-target discovery.
const STANDARD_PRELUDE: [(&str, &str); 5] = [
    ("Option", "std::option::Option"),
    ("Result", "std::result::Result"),
    ("Box", "std::boxed::Box"),
    ("Vec", "std::vec::Vec"),
    ("String", "std::string::String"),
];

#[derive(Debug, Clone, Default, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct MoveCoverage {
    #[serde(flatten)]
    pub scan: Coverage,
    /// Number of discharged occurrences, including omitted proof records.
    #[serde(skip_serializing_if = "is_zero")]
    pub standard_prelude: usize,
}
fn is_zero(value: &usize) -> bool {
    *value == 0
}
impl std::ops::Deref for MoveCoverage {
    type Target = Coverage;
    fn deref(&self) -> &Coverage {
        &self.scan
    }
}
impl std::ops::DerefMut for MoveCoverage {
    fn deref_mut(&mut self) -> &mut Coverage {
        &mut self.scan
    }
}
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(rename_all = "snake_case")]
pub enum BindingProofClass {
    StandardPrelude,
}
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct BindingProof {
    pub class: BindingProofClass,
    pub anchor: SourceAnchor,
    pub item_ids: Vec<String>,
    pub destination_path: String,
    pub standard_path: String,
    pub basis: String,
}

#[derive(Clone, Default)]
struct Shadows {
    context_unproved: bool,
    names: [bool; 5],
}
impl Shadows {
    fn merge(&mut self, other: &Self) {
        self.context_unproved |= other.context_unproved;
        for (name, other) in self.names.iter_mut().zip(other.names) {
            *name |= other;
        }
    }
    fn name(&mut self, text: &str) {
        if let Some(index) = STANDARD_PRELUDE
            .iter()
            .position(|(name, _)| *name == text.trim_start_matches("r#"))
        {
            self.names[index] = true;
        }
    }
    fn refuses(&self, index: usize) -> bool {
        self.context_unproved || self.names[index]
    }
}

/// Module scans visit immediate statements only; item scans include their lexical
/// body conservatively. Inline modules are always separate binding scopes.
fn shadows(
    node: Node<'_>,
    source: &str,
    descend: bool,
    controls: (Instant, &AtomicBool),
) -> Result<Shadows, DomainError> {
    let mut found = Shadows::default();
    let mut stack = vec![(node, false)];
    while let Some((current, in_pattern)) = stack.pop() {
        items::check(controls.0, controls.1)?;
        if current.has_error() || current.is_missing() {
            found.context_unproved = true;
        }
        match current.kind() {
            "line_comment" | "block_comment" | "string_literal" | "raw_string_literal"
            | "char_literal" | "token_tree" => continue,
            "macro_invocation" | "macro_definition" => {
                found.context_unproved = true;
                continue;
            }
            "attribute_item" | "inner_attribute_item" => {
                // An outer attribute on an inline module belongs to that child
                // scope, not to unrelated references in its parent.
                let mut next = current.next_named_sibling();
                while let Some(node) = next {
                    if !matches!(
                        node.kind(),
                        "attribute_item" | "line_comment" | "block_comment"
                    ) {
                        break;
                    }
                    next = node.next_named_sibling();
                }
                let child_attribute = current.kind() == "attribute_item"
                    && next.is_some_and(|n| {
                        n.kind() == "mod_item" && n.child_by_field_name("body").is_some()
                    });
                if !child_attribute {
                    // Includes no_std/no_core/no_implicit_prelude, cfg and derives.
                    found.context_unproved |=
                        !items::context_independent_attribute(&source[current.byte_range()]);
                }
                continue;
            }
            "use_declaration" => {
                found.context_unproved |= items::use_facts(current, source, controls)?.0;
                for leaf in items::use_leaves(current, source, controls)? {
                    found.name(&leaf.binding);
                }
                continue;
            }
            "function_item"
            | "function_signature_item"
            | "struct_item"
            | "enum_item"
            | "trait_item"
            | "union_item"
            | "type_item"
            | "associated_type"
            | "const_item"
            | "static_item"
            | "type_parameter"
            | "const_parameter"
            | "mod_item"
            | "extern_crate_declaration" => {
                if let Some(name) = current.child_by_field_name("name") {
                    found.name(&source[name.byte_range()]);
                }
            }
            "identifier" | "shorthand_field_identifier"
                if in_pattern
                    || current
                        .parent()
                        .is_some_and(|p| p.kind() == "extern_crate_declaration") =>
            {
                found.name(&source[current.byte_range()]);
            }
            _ => {}
        }
        if current.kind() == "mod_item" {
            // Its name binds here, but its declarations/imports do not.
            continue;
        }
        if !descend
            && current != node
            && matches!(
                current.kind(),
                "function_item"
                    | "struct_item"
                    | "enum_item"
                    | "trait_item"
                    | "union_item"
                    | "type_item"
                    | "const_item"
                    | "static_item"
                    | "mod_item"
                    | "impl_item"
            )
        {
            continue;
        }
        // Traverse statement/list wrappers: item macros may be wrapped in an
        // expression_statement, and foreign declarations in a declaration_list.
        for i in (0..current.named_child_count()).rev() {
            let child = current.named_child(i as u32).expect("child");
            stack.push((
                child,
                in_pattern
                    || current.child_by_field_name("pattern") == Some(child)
                    || current.kind() == "closure_parameters",
            ));
        }
    }
    Ok(found)
}

fn type_reference(node: Node<'_>) -> bool {
    if node.kind() != "type_identifier" || !items::reference_role(node) {
        return false;
    }
    let mut parent = node.parent();
    while let Some(current) = parent {
        if matches!(
            current.kind(),
            "scoped_identifier"
                | "scoped_type_identifier"
                | "qualified_type"
                | "generic_function"
                | "struct_expression"
                | "tuple_struct_pattern"
                | "struct_pattern"
                | "use_declaration"
                | "macro_invocation"
        ) {
            return false;
        }
        parent = current.parent();
    }
    true
}

/// Each entry holds only its own module's evidence. A need sees its ancestors,
/// never a sibling or a descendant (including conditional test modules).
#[derive(Clone, Default)]
struct ScopedShadows {
    root: Shadows,
    inline: BTreeMap<(usize, usize), Shadows>,
}
impl ScopedShadows {
    fn collect(
        node: Node<'_>,
        source: &str,
        descend: bool,
        controls: (Instant, &AtomicBool),
    ) -> Result<Self, DomainError> {
        let mut result = Self {
            root: shadows(node, source, descend, controls)?,
            inline: BTreeMap::new(),
        };
        let mut stack = vec![node];
        while let Some(current) = stack.pop() {
            items::check(controls.0, controls.1)?;
            if let Some(body) = (current.kind() == "mod_item")
                .then(|| current.child_by_field_name("body"))
                .flatten()
            {
                let mut found = shadows(body, source, descend, controls)?;
                let mut previous = current.prev_named_sibling();
                while let Some(attribute) = previous {
                    match attribute.kind() {
                        "attribute_item" => {
                            found.context_unproved |= !items::context_independent_attribute(
                                &source[attribute.byte_range()],
                            );
                        }
                        "line_comment" | "block_comment" => {}
                        _ => break,
                    }
                    previous = attribute.prev_named_sibling();
                }
                result
                    .inline
                    .insert((current.start_byte(), current.end_byte()), found);
            }
            if matches!(
                current.kind(),
                "token_tree"
                    | "line_comment"
                    | "block_comment"
                    | "string_literal"
                    | "raw_string_literal"
                    | "char_literal"
            ) {
                continue;
            }
            for i in (0..current.named_child_count()).rev() {
                stack.push(current.named_child(i as u32).expect("child"));
            }
        }
        Ok(result)
    }

    fn visible(&self, node: Option<Node<'_>>) -> Shadows {
        let mut result = self.root.clone();
        let mut parent = node.and_then(|n| n.parent());
        while let Some(current) = parent {
            if let Some(found) = self.inline.get(&(current.start_byte(), current.end_byte())) {
                result.merge(found);
            }
            parent = current.parent();
        }
        result
    }
}

fn chain_shadows(
    path: &str,
    node: Option<Node<'_>>,
    contexts: &BTreeMap<String, ModuleEvidence>,
    modules: &BTreeMap<String, ScopedShadows>,
    absent_destination: bool,
) -> Shadows {
    let mut result = Shadows::default();
    let Some(context) = contexts.get(path) else {
        result.context_unproved = true;
        return result;
    };
    result.context_unproved = !context.unresolved.is_empty()
        || context.filesystem_paths.first() != Some(&context.crate_root)
        || context.filesystem_paths.last().map(String::as_str) != Some(path);
    for file in &context.filesystem_paths {
        if let Some(shadows) = modules.get(file) {
            result.merge(&shadows.visible(if file == path { node } else { None }));
        } else if !(absent_destination && file == path) {
            result.context_unproved = true;
        }
    }
    result
}

#[allow(clippy::too_many_arguments)] // The fallback consumes the same immutable batch evidence.
pub(super) fn discharge(
    files: &BTreeMap<String, FileSnapshot>,
    parsed: &BTreeMap<String, ParsedFile>,
    selected: &[(String, Item, String)],
    contexts: &BTreeMap<String, ModuleEvidence>,
    final_contexts: &BTreeMap<String, ModuleEvidence>,
    repairs: &[Repair],
    needs: &mut Vec<Need>,
    controls: (Instant, &AtomicBool),
) -> Result<Vec<BindingProof>, DomainError> {
    let mut modules = BTreeMap::new();
    for (path, data) in parsed {
        items::check(controls.0, controls.1)?;
        modules.insert(
            path.clone(),
            ScopedShadows::collect(data.tree.root_node(), &files[path].source, false, controls)?,
        );
    }
    // Proposed imports are also visible bindings. Never let a fallback race a
    // written-binding repair from another member of the simultaneous batch.
    let mut final_modules = modules.clone();
    for repair in repairs {
        items::check(controls.0, controls.1)?;
        if repair.kind.starts_with("import") {
            let path = match &repair.target {
                RewriteTarget::Synthesis { path, .. } => path,
                RewriteTarget::Source { anchor } => &anchor.path,
            };
            let tree = crate::trivia::parse(&repair.after, controls.0, controls.1)?
                .ok_or_else(|| DomainError::new("planning_deadline", "import audit stopped"))?;
            final_modules
                .entry(path.clone())
                .or_default()
                .root
                .merge(&shadows(tree.root_node(), &repair.after, true, controls)?);
        }
    }
    // New sibling declarations bind names in their written parent too.
    for (path, context) in final_contexts {
        items::check(controls.0, controls.1)?;
        if !files.contains_key(path)
            && let (Some(parent), Some(name)) = (
                context.filesystem_paths.iter().rev().nth(1),
                context.module_segments.last(),
            )
        {
            final_modules
                .entry(parent.clone())
                .or_default()
                .root
                .name(name);
        }
    }
    let mut item_shadows = BTreeMap::new();
    let mut arrivals: BTreeMap<String, Shadows> = BTreeMap::new();
    for (path, item, destination) in selected {
        items::check(controls.0, controls.1)?;
        let node = parsed[path]
            .tree
            .root_node()
            .named_descendant_for_byte_range(item.span.range.start_byte, item.span.range.end_byte)
            .expect("selected item");
        let found = ScopedShadows::collect(node, &files[path].source, true, controls)?;
        arrivals
            .entry(destination.clone())
            .or_default()
            .merge(&found.root);
        item_shadows.insert(item.id.clone(), found);
    }
    for (path, found) in &arrivals {
        final_modules
            .entry(path.clone())
            .or_default()
            .root
            .merge(found);
    }
    let mut proofs = Vec::new();
    let mut remaining = Vec::new();
    let mut proof_bytes = 0;
    for need in needs.drain(..) {
        items::check(controls.0, controls.1)?;
        let source = &files[&need.path].source;
        let node = parsed[&need.path]
            .tree
            .root_node()
            .named_descendant_for_byte_range(need.range.start_byte, need.range.end_byte);
        let candidate = node.filter(|n| type_reference(*n)).and_then(|_| {
            STANDARD_PRELUDE.iter().position(|(name, _)| {
                *name == source[need.range.start_byte..need.range.end_byte].trim_start_matches("r#")
            })
        });
        let selection = selected.iter().find(|(path, item, _)| {
            path == &need.path
                && need.item_ids.contains(&item.id)
                && item.span.range.start_byte <= need.range.start_byte
                && item.span.range.end_byte >= need.range.end_byte
        });
        if need.reason == DecisionReason::ExternalOrMissingBinding
            && let (Some(index), Some((path, item, destination))) = (candidate, selection)
            && !chain_shadows(path, node, contexts, &modules, false).refuses(index)
            && !chain_shadows(
                destination,
                None,
                final_contexts,
                &final_modules,
                !files.contains_key(destination),
            )
            .refuses(index)
            && !item_shadows[&item.id].visible(node).refuses(index)
            && !arrivals[destination].refuses(index)
        {
            let proof = BindingProof {
                class: BindingProofClass::StandardPrelude,
                anchor: SourceAnchor {
                    path: need.path,
                    expected_text: source[need.range.start_byte..need.range.end_byte].into(),
                    range: need.range,
                },
                item_ids: need.item_ids,
                destination_path: destination.clone(),
                standard_path: STANDARD_PRELUDE[index].1.into(),
                basis: "caller enabled assume_standard_prelude; edition-independent std::prelude::v1 type; complete ordinary source/destination chains and written batch scopes examined without competing imports, globs, declarations, generic/local/pattern bindings, prelude-disabling attributes, conditional/recovered context or macros; no import synthesized; semantic checking not performed".into(),
            };
            proof_bytes += serde_json::to_vec(&proof).expect("proof JSON").len();
            if proof_bytes > 128 * 1024 * 1024 || proofs.len() >= 100_000 {
                return Err(DomainError::new(
                    "analysis_descriptor_bytes",
                    "prelude proof guard reached",
                ));
            }
            proofs.push(proof);
        } else {
            remaining.push(need);
        }
    }
    *needs = remaining;
    Ok(proofs)
}
