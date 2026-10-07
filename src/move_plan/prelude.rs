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
const STANDARD_PRELUDE: [(&str, &str); 7] = [
    ("Option", "std::option::Option"),
    ("Result", "std::result::Result"),
    ("Box", "std::boxed::Box"),
    ("Vec", "std::vec::Vec"),
    ("String", "std::string::String"),
    ("Some", "std::option::Option::Some"),
    ("None", "std::option::Option::None"),
];
const TYPE_COUNT: usize = 5;
const STD_ROOT: usize = STANDARD_PRELUDE.len() + items::BUILTIN_DERIVES.len();
const STD_ROOT_NAME: (&str, &str) = ("std", "std");
const SHADOW_COUNT: usize = STD_ROOT + 1;

#[derive(Debug, Clone, Default, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct MoveCoverage {
    #[serde(flatten)]
    pub scan: Coverage,
    /// Number of discharged occurrences, including omitted proof records.
    #[serde(skip_serializing_if = "is_zero")]
    pub standard_prelude: usize,
    /// Bare Option constructors discharged under the same caller assumption.
    #[serde(skip_serializing_if = "is_zero")]
    pub standard_prelude_constructor: usize,
    /// Number of unshadowed compiler-built-in derive names discharged.
    #[serde(skip_serializing_if = "is_zero")]
    pub standard_builtin_derive: usize,
    /// Occurrences discharged by pure RA resolution, independent of caller assumptions.
    #[serde(skip_serializing_if = "is_zero")]
    pub ra_resolved: usize,
    /// Conditional identities relying on the caller's registered-helper assertion.
    #[serde(skip_serializing_if = "is_zero")]
    pub assumed_declared_identity: usize,
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
#[allow(clippy::enum_variant_names)] // Variant names mirror the published wire classes.
pub enum BindingProofClass {
    StandardPrelude,
    StandardPreludeConstructor,
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
    names: [bool; SHADOW_COUNT],
    conditional_derives: [bool; 9],
    derives: BTreeMap<(String, usize, usize), usize>,
    refusal_basis: Vec<RefusalBasis>,
    scoped_prelude_controls: Vec<RefusalBasis>,
}
impl Shadows {
    fn merge(&mut self, other: &Self) {
        self.context_unproved |= other.context_unproved;
        for (conditional, other) in self
            .conditional_derives
            .iter_mut()
            .zip(other.conditional_derives)
        {
            *conditional |= other;
        }
        self.derives.extend(other.derives.clone());
        for basis in &other.refusal_basis {
            self.record(basis.clone());
        }
        for basis in &other.scoped_prelude_controls {
            if !self.scoped_prelude_controls.contains(basis) {
                self.scoped_prelude_controls.push(basis.clone());
            }
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
        } else if index == STD_ROOT {
            STD_ROOT_NAME.0
        } else {
            STANDARD_PRELUDE[index].0
        };
        self.refusal_basis
            .iter()
            .filter(|basis| {
                if matches!(basis.class.as_str(), "shadow" | "macro_shadow")
                    || (basis.class == "conditional_context" && basis.name.is_some())
                {
                    basis.name.as_deref() == Some(name)
                        || (!derive
                            && self.derives.values().any(|i| {
                                basis.name.as_deref() == Some(items::BUILTIN_DERIVES[*i].0)
                            }))
                } else {
                    basis.class != "derive_veto"
                }
            })
            .cloned()
            .collect()
    }
    fn name(&mut self, text: &str, path: &str, range: Option<crate::result::ByteRange>) {
        if let Some(index) = STANDARD_PRELUDE
            .iter()
            .chain(items::BUILTIN_DERIVES.iter())
            .chain([&STD_ROOT_NAME])
            .position(|(name, _)| *name == text.trim_start_matches("r#"))
        {
            self.names[index] = true;
            self.record(
                RefusalBasis::new("shadow", path, range).named(text.trim_start_matches("r#")),
            );
        }
    }
    fn refuses(&self, index: usize) -> bool {
        self.context_unproved
            || self.names[index]
            || self.derives.values().any(|i| self.refuses_derive(*i))
    }
    fn refuses_derive(&self, index: usize) -> bool {
        self.context_unproved
            || self.names[STANDARD_PRELUDE.len() + index]
            || self.conditional_derives[index]
    }
    /// Keep unbounded context; filesystem inheritance also keeps scoped prelude controls.
    /// Inline-to-parent promotion must not carry no_implicit_prelude upward.
    /// Terminal names stay in their declaring module; competing std roots stay
    /// relevant throughout the module identity chain.
    fn chain_context(&self, inherit_scoped_prelude: bool) -> Self {
        let mut result = Self::default();
        for basis in &self.refusal_basis {
            if (basis.class == "macro_shadow"
                || (basis.class == "shadow" && basis.name.as_deref() == Some("std")))
                && let Some(name) = &basis.name
            {
                result.name(name, &basis.anchor.path, basis.anchor.range.clone());
                result.record(basis.clone());
            }
            if basis.name.is_none()
                && (inherit_scoped_prelude || !self.scoped_prelude_controls.contains(basis))
                && matches!(
                    basis.class.as_str(),
                    "chain_macro_statement"
                        | "syntax_recovery"
                        | "prelude_disabled"
                        | "module_attribute"
                        | "unparseable_attribute"
                        | "conditional_context"
                )
            {
                result.context_unproved = true;
                result.record(basis.clone());
                if self.scoped_prelude_controls.contains(basis) {
                    result.scoped_prelude_controls.push(basis.clone());
                }
            }
        }
        result
    }
    fn remove_departure(&mut self, path: &str, item: &Item) {
        let departs = |range: &crate::result::ByteRange| {
            (item.span.range.start_byte <= range.start_byte
                && item.span.range.end_byte >= range.end_byte)
                || item.attributes.iter().any(|a| {
                    a.range.start_byte <= range.start_byte && a.range.end_byte >= range.end_byte
                })
        };
        self.refusal_basis.retain(|basis| {
            basis.anchor.path != path || basis.anchor.range.as_ref().is_none_or(|r| !departs(r))
        });
        self.scoped_prelude_controls.retain(|basis| {
            basis.anchor.path != path || basis.anchor.range.as_ref().is_none_or(|r| !departs(r))
        });
        self.derives.retain(|(file, start, end), _| {
            file != path
                || !departs(&crate::result::ByteRange {
                    start_byte: *start,
                    end_byte: *end,
                })
        });
        self.names = [false; SHADOW_COUNT];
        self.conditional_derives = [false; 9];
        self.context_unproved = self
            .refusal_basis
            .iter()
            .any(|b| b.name.is_none() && b.class != "derive_veto");
        for basis in self.refusal_basis.clone() {
            if matches!(basis.class.as_str(), "shadow" | "macro_shadow")
                && let Some(name) = &basis.name
            {
                self.name(name, &basis.anchor.path, basis.anchor.range.clone());
            }
            if basis.class == "conditional_context"
                && let Some(index) = items::BUILTIN_DERIVES
                    .iter()
                    .position(|(name, _)| basis.name.as_deref() == Some(name))
            {
                self.conditional_derives[index] = true;
            }
        }
    }
    fn attribute(&mut self, path: &str, node: Node<'_>, source: &str) {
        if node.has_error() || node.is_missing() {
            self.context("unparseable_attribute", path, Some(node));
            return;
        }
        if items::context_independent_attribute(&source[node.byte_range()]) {
            return;
        }
        if let Some(names) = items::derive_names(node, source) {
            for (index, range) in names {
                if let Some(index) = index {
                    self.derives
                        .insert((path.into(), range.start_byte, range.end_byte), index);
                    // A condition/unexamined companion on this declaration must
                    // not be erased by discharging its unconditional derive need.
                    let mut first = node;
                    while let Some(previous) = first.prev_named_sibling().filter(|n| {
                        matches!(
                            n.kind(),
                            "attribute_item" | "line_comment" | "block_comment"
                        )
                    }) {
                        first = previous;
                    }
                    let mut sibling = Some(first);
                    while let Some(attribute) = sibling.filter(|n| {
                        matches!(
                            n.kind(),
                            "attribute_item" | "line_comment" | "block_comment"
                        )
                    }) {
                        if attribute.kind() == "attribute_item"
                            && !items::context_independent_attribute(
                                &source[attribute.byte_range()],
                            )
                            && items::derive_names(attribute, source).is_none()
                        {
                            self.conditional_derives[index] = true;
                            self.record(
                                RefusalBasis::new(
                                    "conditional_context",
                                    path,
                                    Some(crate::result::ByteRange {
                                        start_byte: attribute.start_byte(),
                                        end_byte: attribute.end_byte(),
                                    }),
                                )
                                .named(items::BUILTIN_DERIVES[index].0),
                            );
                        }
                        sibling = attribute.next_named_sibling();
                    }
                } else {
                    // Unknown derives keep their own attribute need, not a
                    // blanket veto on unrelated type spellings in this scope.
                    self.record(RefusalBasis::new("derive_veto", path, Some(range)));
                }
            }
        } else {
            let name = node.named_child(0).and_then(|a| a.named_child(0));
            let Some(name) = name else {
                self.context("unparseable_attribute", path, Some(node));
                return;
            };
            if matches!(
                &source[name.byte_range()],
                "no_std" | "no_core" | "no_implicit_prelude" | "prelude_import"
            ) {
                self.context("prelude_disabled", path, Some(node));
                if &source[name.byte_range()] == "no_implicit_prelude" {
                    self.scoped_prelude_controls.push(RefusalBasis::new(
                        "prelude_disabled",
                        path,
                        Some(crate::result::ByteRange {
                            start_byte: node.start_byte(),
                            end_byte: node.end_byte(),
                        }),
                    ));
                }
                return;
            }
            let mut next = node.next_named_sibling();
            while next.is_some_and(|n| {
                matches!(
                    n.kind(),
                    "attribute_item" | "line_comment" | "block_comment"
                )
            }) {
                next = next.and_then(|n| n.next_named_sibling());
            }
            if node.kind() == "inner_attribute_item" {
                self.context("conditional_context", path, Some(node));
            } else if next.is_some_and(|n| n.kind() == "mod_item")
                && !matches!(&source[name.byte_range()], "cfg" | "path")
            {
                self.context("module_attribute", path, Some(node));
            }
            // Otherwise the attributed declaration's written name is already in
            // the superset shadow set, even when its presence is conditional.
            // The attribute's own move/context need remains independently blocked.
        }
    }
}

/// Module scans visit immediate declarations only. Lexical bodies are audited
/// at each occurrence through the same position-aware assessment as written needs.
fn shadows(
    path: &str,
    node: Node<'_>,
    source: &str,
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
            "macro_invocation" => {
                found.context("chain_macro_statement", path, Some(current));
                continue;
            }
            "block" => continue,
            "attribute_item" | "inner_attribute_item" => {
                found.attribute(path, current, source);
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
            "extern_crate_declaration" => {
                // A renamed crate binds only its alias, not its original name.
                if let Some(name) = current
                    .child_by_field_name("alias")
                    .or_else(|| current.child_by_field_name("name"))
                {
                    found.name(
                        &source[name.byte_range()],
                        path,
                        Some(crate::result::ByteRange {
                            start_byte: name.start_byte(),
                            end_byte: name.end_byte(),
                        }),
                    );
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
            | "macro_definition" => {
                if let Some(name) = current.child_by_field_name("name") {
                    if current.kind() == "macro_definition" {
                        found.record(
                            RefusalBasis::new(
                                "macro_shadow",
                                path,
                                Some(crate::result::ByteRange {
                                    start_byte: name.start_byte(),
                                    end_byte: name.end_byte(),
                                }),
                            )
                            .named(source[name.byte_range()].trim_start_matches("r#")),
                        );
                    }
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
            "identifier" | "shorthand_field_identifier" if in_pattern => {
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
        if matches!(
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
        ) {
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
    if !matches!(node.kind(), "type_identifier" | "scoped_type_identifier")
        || !items::reference_role(node)
    {
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

/// Bare value-position constructors only; qualified paths and patterns remain
/// written/semantic dependencies, not assumed prelude identities.
fn constructor_reference(node: Node<'_>) -> bool {
    if node.kind() != "identifier" || !items::reference_role(node) {
        return false;
    }
    let mut child = node;
    while let Some(parent) = child.parent() {
        if parent.kind().ends_with("_pattern")
            || parent.child_by_field_name("pattern") == Some(child)
            || matches!(
                parent.kind(),
                "scoped_identifier"
                    | "scoped_type_identifier"
                    | "generic_function"
                    | "use_declaration"
                    | "macro_invocation"
            )
        {
            return false;
        }
        child = parent;
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
        controls: (Instant, &AtomicBool),
    ) -> Result<Self, DomainError> {
        let mut result = Self {
            root: shadows(path, node, source, controls)?,
            inline: BTreeMap::new(),
        };
        let mut stack = vec![node];
        while let Some(current) = stack.pop() {
            items::check(controls.0, controls.1)?;
            if let Some(body) = (current.kind() == "mod_item")
                .then(|| current.child_by_field_name("body"))
                .flatten()
            {
                let mut found = shadows(path, body, source, controls)?;
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
                // Unbounded item/module expansion and malformed context remain
                // chain-wide; a sibling's finite written names/derives do not.
                // no_implicit_prelude is inherited downward, not promoted out
                // of an unrelated inline module into its parent or siblings.
                let mut unbounded = found.chain_context(false);
                // Textual macro definitions inherit downward, never upward out
                // of a child module like an unbounded expansion veto.
                unbounded.names = [false; SHADOW_COUNT];
                unbounded
                    .refusal_basis
                    .retain(|b| !matches!(b.class.as_str(), "shadow" | "macro_shadow"));
                result.root.merge(&unbounded);
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
            result.merge(&if file == path {
                shadows.visible(node)
            } else {
                shadows.root.chain_context(true)
            });
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
            ScopedShadows::collect(path, data.tree.root_node(), &files[path].source, controls)?,
        );
    }
    // Proposed imports are also visible bindings. Never let a fallback race a
    // written-binding repair from another member of the simultaneous batch.
    let mut final_modules = modules.clone();
    for (path, item, _) in selected {
        if let Some(module) = final_modules.get_mut(path) {
            module.root.remove_departure(path, item);
            for inline in module.inline.values_mut() {
                inline.remove_departure(path, item);
            }
        }
    }
    for repair in repairs {
        items::check(controls.0, controls.1)?;
        if repair.kind.starts_with("import") {
            let path = match &repair.target {
                RewriteTarget::Synthesis { path, .. } => path,
                RewriteTarget::Source { anchor } => &anchor.path,
            };
            let tree = crate::trivia::parse(&repair.after, controls.0, controls.1)?
                .ok_or_else(|| DomainError::new("planning_deadline", "import audit stopped"))?;
            let mut proposed = shadows(path, tree.root_node(), &repair.after, controls)?;
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
        let mut found = ScopedShadows::collect(path, node, &files[path].source, controls)?;
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
        let spelling = &source[need.range.start_byte..need.range.end_byte];
        let candidate = node.and_then(|node| {
            STANDARD_PRELUDE
                .iter()
                .enumerate()
                .position(|(index, (name, standard_path))| {
                    if index < TYPE_COUNT {
                        type_reference(node)
                            && ((node.kind() == "type_identifier"
                                && *name == spelling.trim_start_matches("r#"))
                                || (node.kind() == "scoped_type_identifier"
                                    && *standard_path == spelling))
                    } else {
                        *name == spelling.trim_start_matches("r#") && constructor_reference(node)
                    }
                })
        });
        let qualified =
            candidate.is_some() && node.is_some_and(|node| node.kind() == "scoped_type_identifier");
        // Qualification bypasses the terminal binding, not the std root.
        let audited_candidate = candidate.map(|index| if qualified { STD_ROOT } else { index });
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
        if qualified && let Some(node) = node {
            // Block-local extern-crate aliases are hoisted too, but are not
            // included in the ordinary lexical binding assessment.
            let mut parent = node.parent();
            while let Some(scope) = parent {
                items::check(controls.0, controls.1)?;
                if matches!(scope.kind(), "source_file" | "mod_item") {
                    break;
                }
                if scope.kind() == "block" {
                    for i in 0..scope.named_child_count() {
                        items::check(controls.0, controls.1)?;
                        let declaration = scope.named_child(i as u32).expect("statement");
                        if declaration.kind() == "extern_crate_declaration"
                            && let Some(name) = declaration
                                .child_by_field_name("alias")
                                .or_else(|| declaration.child_by_field_name("name"))
                            && source[name.byte_range()].trim_start_matches("r#") == "std"
                        {
                            visible.name(
                                "std",
                                path,
                                Some(crate::result::ByteRange {
                                    start_byte: name.start_byte(),
                                    end_byte: name.end_byte(),
                                }),
                            );
                        }
                    }
                }
                parent = scope.parent();
            }
        }
        if let Some(index) = audited_candidate
            && let Some(node) = node
        {
            // Constructors require both their own spelling and the assumed
            // Option type to be unshadowed in exactly the same lexical context.
            // Qualified types audit only the root, never terminal imports.
            for audited in
                std::iter::once(index).chain((!qualified && index >= TYPE_COUNT).then_some(0))
            {
                let name = if audited == STD_ROOT {
                    STD_ROOT_NAME.0
                } else {
                    STANDARD_PRELUDE[audited].0
                };
                let assessment =
                    items::lexical_assessment(path, node, source, name, controls, false)?;
                if assessment.binding != items::LexicalBinding::Absent {
                    if let Some(witness) = assessment.uncertainty {
                        let anchor = witness.pattern.as_ref().unwrap_or(&witness.scope);
                        visible.context_unproved = true;
                        visible.record(
                            RefusalBasis::new(
                                if witness.witness_relation.is_some() {
                                    "chain_macro_statement"
                                } else {
                                    "lexical_uncertainty"
                                },
                                path,
                                Some(anchor.range.clone()),
                            )
                            .named(name),
                        );
                    } else {
                        visible.name(
                            name,
                            path,
                            assessment.definite_binding.map(|binding| binding.range),
                        );
                    }
                }
            }
        }
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
                        basis: "caller enabled assume_standard_prelude; bare compiler-built-in derive name; complete ordinary module identity chains; own/enclosing inline source scopes and final destination batch scopes examined without same-spelling macro/item/use-leaf bindings or globs; chain-wide prelude controls and unbounded item/module-attribute context examined; written token-tree not expanded; built-in identity assumed, not macro hygiene proved; other derive names retain their own needs; no import synthesized; semantic checking not performed".into(),
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
            && !visible.refuses(audited_candidate.expect("prelude candidate"))
            && (index < TYPE_COUNT || !visible.refuses(0))
            && !derive_identity_unproved
        {
            record_proof(
                BindingProof {
                    class: if index < TYPE_COUNT {
                        BindingProofClass::StandardPrelude
                    } else {
                        BindingProofClass::StandardPreludeConstructor
                    },
                    anchor: SourceAnchor {
                        path: need.path,
                        expected_text: source[need.range.start_byte..need.range.end_byte].into(),
                        range: need.range,
                    },
                    item_ids: need.item_ids,
                    destination_path: destination.clone(),
                    standard_path: STANDARD_PRELUDE[index].1.into(),
                    basis: if index >= TYPE_COUNT {
                        "caller enabled assume_standard_prelude; bare value-position Option constructor; constructor and Option type spellings unshadowed under the same original/final module, batch, lexical, derive and prelude-control audits as standard-prelude types; constructor identity assumed with the type; derive-generated imports are not modeled; not macro hygiene proved; no import synthesized; semantic checking not performed".into()
                    } else {
                        let form = if qualified {
                            format!(
                                "fully-qualified type-position {}; terminal binding bypassed (same-name imports do not shadow this path); std root audited for competing written mod/extern-crate declarations in the module/block scope chain; ",
                                STANDARD_PRELUDE[index].1
                            )
                        } else {
                            String::new()
                        };
                        format!(
                            "caller enabled assume_standard_prelude; {form}edition-independent std::prelude::v1 type; complete ordinary module identity chains; own/enclosing inline source scopes and final destination batch scopes examined for competing written names and globs; written lexical bindings checked; direct block macro invocations, macro expression-statement wrappers and outer attribute siblings in the reference's block and ancestor blocks up to the nearest module audited regardless of source order, only strict context-independent attributes exempted; signatures outside those blocks ignore body expansion sites; scoped no_implicit_prelude and chain-wide crate-root prelude controls and unbounded item/module-attribute context examined; relevant local derives audited for same-spelling competition; written token-tree not expanded; no macro hygiene claim; no import synthesized; semantic checking not performed"
                        )
                    },
                },
                &mut proofs,
                &mut proof_bytes,
            )?;
        } else if !derive_discharged {
            if matches!(
                need.reason,
                DecisionReason::ExternalOrMissingBinding
                    | DecisionReason::GlobBindingUnproved
                    | DecisionReason::ConditionalOrInheritedContext
            ) && let Some(index) = candidate
            {
                need.refusal_basis.extend(
                    visible.basis_for(audited_candidate.expect("prelude candidate"), false),
                );
                if index >= TYPE_COUNT {
                    need.refusal_basis.extend(visible.basis_for(0, false));
                }
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
