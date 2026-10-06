//! Caller-assumed prelude evidence, separate from written-binding repairs.
#[cfg(test)]
#[path = "prelude_tests.rs"]
mod tests;
use super::RewriteTarget;
use crate::{
    items::{self, DecisionReason, Item, ModuleEvidence, Need, ParsedFile, RefusalBasis},
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
    /// Number of unshadowed compiler-built-in derive names discharged.
    #[serde(skip_serializing_if = "is_zero")]
    pub standard_builtin_derive: usize,
    /// Occurrences discharged by pure RA resolution, independent of caller assumptions.
    #[serde(skip_serializing_if = "is_zero")]
    pub ra_resolved: usize,
    /// Residual test-consumer decisions acknowledged as risks, not binding proofs.
    #[serde(skip_serializing_if = "is_zero")]
    pub test_consumers_acknowledged: usize,
    /// Excluded item/glob pairs by written-route reason, not semantic absence proof.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub glob_exclusions: BTreeMap<String, usize>,
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(rename_all = "snake_case")]
pub enum BindingProofClass {
    StandardPrelude,
    StandardBuiltinDerive,
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
    names: [bool; 14],
    derive_veto: bool,
    derives: BTreeMap<(String, usize, usize), usize>,
    refusal_basis: Vec<RefusalBasis>,
}
impl Shadows {
    fn merge(&mut self, other: &Self) {
        self.context_unproved |= other.context_unproved;
        self.derive_veto |= other.derive_veto;
        self.derives.extend(other.derives.clone());
        for basis in &other.refusal_basis {
            self.record(basis.clone());
        }
        for (name, other) in self.names.iter_mut().zip(other.names) {
            *name |= other;
        }
    }
    fn record(&mut self, basis: RefusalBasis) {
        if !self.refusal_basis.contains(&basis) {
            self.refusal_basis.push(basis);
        }
    }
    fn context(&mut self, class: &str, path: &str, node: Option<Node<'_>>) {
        self.context_unproved = true;
        self.record(RefusalBasis::new(
            class,
            path,
            node.map(|n| crate::result::ByteRange {
                start_byte: n.start_byte(),
                end_byte: n.end_byte(),
            }),
        ));
    }
    fn basis_for(&self, index: usize, derive: bool) -> Vec<RefusalBasis> {
        let name = if derive {
            items::BUILTIN_DERIVES[index].0
        } else {
            STANDARD_PRELUDE[index].0
        };
        self.refusal_basis
            .iter()
            .filter(|basis| {
                if basis.class == "shadow" {
                    basis.name.as_deref() == Some(name)
                        || (!derive
                            && self.derives.values().any(|i| {
                                basis.name.as_deref() == Some(items::BUILTIN_DERIVES[*i].0)
                            }))
                } else {
                    !derive || basis.class != "derive_veto"
                }
            })
            .cloned()
            .collect()
    }
    fn name(&mut self, text: &str, path: &str, range: Option<crate::result::ByteRange>) {
        if let Some(index) = STANDARD_PRELUDE
            .iter()
            .chain(items::BUILTIN_DERIVES.iter())
            .position(|(name, _)| *name == text.trim_start_matches("r#"))
        {
            self.names[index] = true;
            self.record(RefusalBasis::new("shadow", path, range).named(text));
        }
    }
    fn refuses(&self, index: usize) -> bool {
        self.context_unproved
            || self.names[index]
            || self.derive_veto
            || self.derives.values().any(|i| self.refuses_derive(*i))
    }
    fn refuses_derive(&self, index: usize) -> bool {
        self.context_unproved || self.names[STANDARD_PRELUDE.len() + index]
    }
    fn attribute(&mut self, path: &str, node: Node<'_>, source: &str) {
        if items::context_independent_attribute(&source[node.byte_range()]) {
            return;
        }
        if let Some(names) = items::derive_names(node, source) {
            for (index, range) in names {
                if let Some(index) = index {
                    self.derives
                        .insert((path.into(), range.start_byte, range.end_byte), index);
                } else {
                    self.derive_veto = true;
                    self.record(RefusalBasis::new("derive_veto", path, Some(range)));
                }
            }
        } else {
            self.context("conditional_context", path, Some(node));
        }
    }
}

/// Module scans visit immediate statements only; item scans include their lexical
/// body conservatively. Inline modules are always separate binding scopes.
fn shadows(
    path: &str,
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
            found.context("syntax_recovery", path, Some(current));
        }
        match current.kind() {
            "line_comment" | "block_comment" | "string_literal" | "raw_string_literal"
            | "char_literal" | "token_tree" => continue,
            "macro_invocation" | "macro_definition" => {
                found.context("chain_macro_statement", path, Some(current));
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
                    // Derive identity is checked only after both chains merge.
                    found.attribute(path, current, source);
                }
                continue;
            }
            "use_declaration" => {
                if items::use_facts(current, source, controls)?.0 {
                    found.context("glob_import", path, Some(current));
                }
                for leaf in items::use_leaves(current, source, controls)? {
                    found.name(&leaf.binding, path, Some(leaf.leaf_range));
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
                    found.name(
                        &source[name.byte_range()],
                        path,
                        Some(crate::result::ByteRange {
                            start_byte: name.start_byte(),
                            end_byte: name.end_byte(),
                        }),
                    );
                }
            }
            "identifier" | "shorthand_field_identifier"
                if in_pattern
                    || current
                        .parent()
                        .is_some_and(|p| p.kind() == "extern_crate_declaration") =>
            {
                found.name(
                    &source[current.byte_range()],
                    path,
                    Some(crate::result::ByteRange {
                        start_byte: current.start_byte(),
                        end_byte: current.end_byte(),
                    }),
                );
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
        path: &str,
        node: Node<'_>,
        source: &str,
        descend: bool,
        controls: (Instant, &AtomicBool),
    ) -> Result<Self, DomainError> {
        let mut result = Self {
            root: shadows(path, node, source, descend, controls)?,
            inline: BTreeMap::new(),
        };
        let mut stack = vec![node];
        while let Some(current) = stack.pop() {
            items::check(controls.0, controls.1)?;
            if let Some(body) = (current.kind() == "mod_item")
                .then(|| current.child_by_field_name("body"))
                .flatten()
            {
                let mut found = shadows(path, body, source, descend, controls)?;
                let mut previous = current.prev_named_sibling();
                while let Some(attribute) = previous {
                    match attribute.kind() {
                        "attribute_item" => {
                            found.attribute(path, attribute, source);
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
        result.context("unresolved_chain", path, None);
        return result;
    };
    result.context_unproved = !context.unresolved.is_empty()
        || context.filesystem_paths.first() != Some(&context.crate_root)
        || context.filesystem_paths.last().map(String::as_str) != Some(path);
    if result.context_unproved {
        result.record(RefusalBasis::new("unresolved_chain", path, None));
    }
    for file in &context.filesystem_paths {
        if let Some(shadows) = modules.get(file) {
            result.merge(&shadows.visible(if file == path { node } else { None }));
        } else if !(absent_destination && file == path) {
            result.context("unresolved_chain", file, None);
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
            ScopedShadows::collect(
                path,
                data.tree.root_node(),
                &files[path].source,
                false,
                controls,
            )?,
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
            let mut proposed = shadows(path, tree.root_node(), &repair.after, true, controls)?;
            // Parsed replacement offsets are not original-source coordinates.
            for basis in &mut proposed.refusal_basis {
                basis.anchor.range = match &repair.target {
                    RewriteTarget::Source { anchor } => Some(anchor.range.clone()),
                    RewriteTarget::Synthesis { .. } => None,
                };
            }
            final_modules
                .entry(path.clone())
                .or_default()
                .root
                .merge(&proposed);
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
                .name(name, path, None);
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
        let mut found = ScopedShadows::collect(path, node, &files[path].source, true, controls)?;
        // Leading outer attributes lie outside the item anchor, but arrive with
        // it and must participate in the destination's expansion audit.
        for attribute in &item.attributes {
            items::check(controls.0, controls.1)?;
            if attribute.range.end_byte <= item.span.range.start_byte
                && let Some(node) = parsed[path]
                    .tree
                    .root_node()
                    .named_descendant_for_byte_range(
                        attribute.range.start_byte,
                        attribute.range.end_byte,
                    )
            {
                found.root.attribute(path, node, &files[path].source);
            }
        }
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
    let mut proofs = BTreeMap::new();
    let mut remaining = Vec::new();
    let mut proof_bytes = 0;
    for mut need in needs.drain(..) {
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
        let Some((path, item, destination)) = selection else {
            remaining.push(need);
            continue;
        };
        let context_node = need
            .attribute_range
            .as_ref()
            .and_then(|range| {
                parsed[path]
                    .tree
                    .root_node()
                    .named_descendant_for_byte_range(range.start_byte, range.end_byte)
            })
            .or(node);
        let mut visible = chain_shadows(path, context_node, contexts, &modules, false);
        visible.merge(&chain_shadows(
            destination,
            None,
            final_contexts,
            &final_modules,
            !files.contains_key(destination),
        ));
        visible.merge(&item_shadows[&item.id].visible(context_node));
        visible.merge(&arrivals[destination]);
        // Record contextual derives too: an attribute on a retained declaration
        // can be the veto that otherwise prevents a moved type reference.
        let mut derive_identity_unproved = false;
        if candidate.is_some() || need.attribute_range.is_some() {
            for ((path, start, end), index) in &visible.derives {
                items::check(controls.0, controls.1)?;
                let derive_node = parsed[path]
                    .tree
                    .root_node()
                    .named_descendant_for_byte_range(*start, *end);
                if visible.refuses_derive(*index)
                    || chain_shadows(path, derive_node, contexts, &modules, false)
                        .refuses_derive(*index)
                {
                    derive_identity_unproved = true;
                    if need.reason == DecisionReason::ExternalOrMissingBinding
                        && candidate.is_some()
                    {
                        need.refusal_basis.push(RefusalBasis::new(
                            "derive_veto",
                            path,
                            Some(crate::result::ByteRange {
                                start_byte: *start,
                                end_byte: *end,
                            }),
                        ));
                        need.refusal_basis.extend(visible.basis_for(*index, true));
                        need.refusal_basis.extend(
                            chain_shadows(path, derive_node, contexts, &modules, false)
                                .basis_for(*index, true),
                        );
                    }
                    continue;
                }
                record_proof(
                    BindingProof {
                        class: BindingProofClass::StandardBuiltinDerive,
                        anchor: SourceAnchor {
                            path: path.clone(),
                            expected_text: files[path].source[*start..*end].into(),
                            range: crate::result::ByteRange {
                                start_byte: *start,
                                end_byte: *end,
                            },
                        },
                        item_ids: need.item_ids.clone(),
                        destination_path: destination.clone(),
                        standard_path: items::BUILTIN_DERIVES[*index].1.into(),
                        basis: "caller enabled assume_standard_prelude; bare compiler-built-in derive name; complete ordinary source/destination visible chains and written batch scopes examined without competing same-spelling macro/item/use-leaf bindings or globs, prelude-disabling attributes, conditional/recovered context or unexamined macros; standard-defined expansion introduces no module-scope binding; other derive names remain vetoes; no import synthesized; semantic checking not performed".into(),
                    },
                    &mut proofs,
                    &mut proof_bytes,
                )?;
            }
        }
        let derive_discharged = need.reason == DecisionReason::ConditionalOrInheritedContext
            && need.attribute_range.as_ref().is_some_and(|range| {
                parsed[path]
                    .tree
                    .root_node()
                    .named_descendant_for_byte_range(range.start_byte, range.end_byte)
                    .and_then(|attribute| items::derive_names(attribute, source))
                    .is_some_and(|names| {
                        names
                            .iter()
                            .all(|(index, _)| index.is_some_and(|i| !visible.refuses_derive(i)))
                    })
            });
        if need.reason == DecisionReason::ExternalOrMissingBinding
            && let Some(index) = candidate
            && !visible.refuses(index)
            && !derive_identity_unproved
        {
            record_proof(
                BindingProof {
                    class: BindingProofClass::StandardPrelude,
                    anchor: SourceAnchor {
                        path: need.path,
                        expected_text: source[need.range.start_byte..need.range.end_byte].into(),
                        range: need.range,
                    },
                    item_ids: need.item_ids,
                    destination_path: destination.clone(),
                    standard_path: STANDARD_PRELUDE[index].1.into(),
                    basis: "caller enabled assume_standard_prelude; edition-independent std::prelude::v1 type; complete ordinary source/destination chains and written batch scopes examined without competing imports, globs, declarations, generic/local/pattern bindings, prelude-disabling attributes, conditional/recovered context or macros; any contextual derives separately audited as unshadowed compiler built-ins; no import synthesized; semantic checking not performed".into(),
                },
                &mut proofs,
                &mut proof_bytes,
            )?;
        } else if !derive_discharged {
            if matches!(
                need.reason,
                DecisionReason::ExternalOrMissingBinding | DecisionReason::GlobBindingUnproved
            ) && let Some(index) = candidate
            {
                need.refusal_basis.extend(visible.basis_for(index, false));
            }
            if let Some(attribute) = need.attribute_range.as_ref().and_then(|range| {
                parsed[path]
                    .tree
                    .root_node()
                    .named_descendant_for_byte_range(range.start_byte, range.end_byte)
            }) && let Some(names) = items::derive_names(attribute, source)
            {
                for (index, range) in names {
                    if let Some(index) = index {
                        if visible.refuses_derive(index) {
                            need.refusal_basis.extend(visible.basis_for(index, true));
                        }
                    } else {
                        need.refusal_basis.push(RefusalBasis::new(
                            "derive_veto",
                            path,
                            Some(range),
                        ));
                    }
                }
            }
            let mut unique = Vec::new();
            for basis in need.refusal_basis.drain(..) {
                if !unique.contains(&basis) {
                    unique.push(basis);
                }
            }
            need.refusal_basis = unique;
            remaining.push(need);
        }
    }
    *needs = remaining;
    Ok(proofs.into_values().collect())
}

fn record_proof(
    mut proof: BindingProof,
    proofs: &mut BTreeMap<(String, usize, usize, String), BindingProof>,
    bytes: &mut usize,
) -> Result<(), DomainError> {
    let key = (
        proof.anchor.path.clone(),
        proof.anchor.range.start_byte,
        proof.anchor.range.end_byte,
        proof.destination_path.clone(),
    );
    if let Some(prior) = proofs.get(&key) {
        *bytes -= serde_json::to_vec(prior).expect("proof JSON").len();
        for id in &prior.item_ids {
            if !proof.item_ids.contains(id) {
                proof.item_ids.push(id.clone());
            }
        }
        proof.item_ids.sort();
    }
    *bytes += serde_json::to_vec(&proof).expect("proof JSON").len();
    proofs.insert(key, proof);
    if *bytes > 128 * 1024 * 1024 || proofs.len() > 100_000 {
        return Err(DomainError::new(
            "analysis_descriptor_bytes",
            "prelude proof guard reached",
        ));
    }
    Ok(())
}
