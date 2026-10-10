//! Conditional admission of unchanged written derive invocations, never expansion proof.
use super::*;
use tree_sitter::Node;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(deny_unknown_fields)]
pub struct Assertion {
    pub invocation: SourceAnchor,
    pub helpers: Vec<Helper>,
    pub bindings: Vec<ProviderBinding>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(deny_unknown_fields)]
pub struct Helper {
    pub anchor: SourceAnchor,
    /// Bare derive name registering this helper, not an attribute-macro provider.
    pub derive_name: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(deny_unknown_fields)]
pub struct ProviderBinding {
    pub name: String,
    pub provider_path: String,
    pub original_import: SourceAnchor,
    pub destination_import: SourceAnchor,
}
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct Proof {
    pub class: ProofClass,
    pub invocation: SourceAnchor,
    pub helpers: Vec<Helper>,
    pub bindings: Vec<ProviderBinding>,
    pub item_ids: Vec<String>,
    pub destination_path: String,
    pub basis: String,
}
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(rename_all = "snake_case")]
pub enum ProofClass {
    AssumedDeriveRelocation,
}
const BASIS: &str = "caller asserts the exact derive invocation and registered helpers preserve all dependency interpretation (imports, relative paths and helper payloads) on relocation and introduce no conflicting namespace bindings, including generated imports; provider identity and helper registration are assumed, not expansion-inertness or resolved evidence; unchanged written import routes and attribute attachment checked; no generated methods, procedural expansion, hygiene, compilation or equivalence proof";
fn invalid(index: usize, message: &str) -> DomainError {
    let mut error = DomainError::new("INVALID_PARAMS", message);
    error.field = Some(format!("derived_attribute_relocations[{index}]"));
    error
}
fn exact<'a>(
    anchor: &SourceAnchor,
    files: &'a BTreeMap<String, FileSnapshot>,
    parsed: &'a BTreeMap<String, ParsedFile>,
    kind: &str,
) -> Option<Node<'a>> {
    let source = &files.get(&anchor.path)?.source;
    let range = &anchor.range;
    if range.start_byte >= range.end_byte
        || source.get(range.start_byte..range.end_byte) != Some(anchor.expected_text.as_str())
    {
        return None;
    }
    let mut node = parsed
        .get(&anchor.path)?
        .tree
        .root_node()
        .named_descendant_for_byte_range(range.start_byte, range.end_byte)?;
    loop {
        if node.start_byte() != range.start_byte || node.end_byte() != range.end_byte {
            return None;
        }
        if node.kind() == kind && !node.has_error() && !node.is_missing() {
            return Some(node);
        }
        node = node.parent()?;
    }
}
fn attached(anchor: &SourceAnchor, path: &str, item: &Item) -> bool {
    anchor.path == path
        && (item.attributes.iter().any(|a| a.range == anchor.range)
            || (item.span.range.start_byte <= anchor.range.start_byte
                && anchor.range.end_byte <= item.span.range.end_byte))
}
fn identity(text: &str) -> &str {
    text.strip_prefix("r#").unwrap_or(text)
}
fn bare_identifier(text: &str) -> bool {
    let text = identity(text);
    !text.is_empty()
        && text
            .chars()
            .enumerate()
            .all(|(i, c)| c == '_' || c.is_ascii_alphabetic() || (i > 0 && c.is_ascii_digit()))
}
fn helper_name(node: Node<'_>, source: &str) -> Option<String> {
    let attribute = node.named_child(0)?;
    let name = attribute.named_child(0)?;
    let text = &source[name.byte_range()];
    let normalized = identity(text);
    if name.kind() != "identifier"
        || !bare_identifier(text)
        || matches!(
            normalized,
            "derive"
                | "cfg"
                | "cfg_attr"
                | "path"
                | "macro_use"
                | "no_implicit_prelude"
                | "no_std"
                | "no_core"
                | "proc_macro"
                | "proc_macro_attribute"
                | "proc_macro_derive"
                | "recursion_limit"
                | "type_length_limit"
                | "feature"
                | "prelude_import"
                | "macro_export"
                | "crate_type"
                | "crate_name"
                | "repr"
                | "inline"
        )
        || items::identity_inert_metadata(Some(normalized))
        || items::context_independent_attribute(&source[node.byte_range()])
    {
        return None;
    }
    Some(text.to_owned())
}
fn import_matches(
    anchor: &SourceAnchor,
    path: &str,
    binding: &ProviderBinding,
    files: &BTreeMap<String, FileSnapshot>,
    parsed: &BTreeMap<String, ParsedFile>,
    controls: (Instant, &AtomicBool),
) -> bool {
    if anchor.path != path {
        return false;
    }
    let Some(node) = exact(anchor, files, parsed, "use_declaration") else {
        return false;
    };
    // Only unconditional file-root imports are admitted. Nested scopes and inherited
    // context must never be mistaken for the invocation's provider binding.
    if node.parent().is_none_or(|p| p.kind() != "source_file")
        || node
            .prev_named_sibling()
            .is_some_and(|p| p.kind() == "attribute_item")
    {
        return false;
    }
    items::use_leaves(node, &files[path].source, controls).is_ok_and(|leaves| {
        leaves.len() == 1
            && leaves[0].path == binding.provider_path
            && leaves[0].binding == binding.name
    })
}
/// Refuse visible expansion providers and competing written import routes rather
/// than interpreting their output. The caller assertion never clears these gates.
fn safe_namespace(
    path: &str,
    assertion: &Assertion,
    files: &BTreeMap<String, FileSnapshot>,
    parsed: &BTreeMap<String, ParsedFile>,
    controls: (Instant, &AtomicBool),
) -> Result<bool, DomainError> {
    let source = &files[path].source;
    let mut stack = vec![parsed[path].tree.root_node()];
    let mut bindings = BTreeSet::new();
    while let Some(node) = stack.pop() {
        items::check(controls.0, controls.1)?;
        if node.has_error() || node.is_missing() {
            return Ok(false);
        }
        match node.kind() {
            "macro_invocation" | "macro_definition" | "inner_attribute_item" => return Ok(false),
            "attribute_item" => {
                let range = ByteRange {
                    start_byte: node.start_byte(),
                    end_byte: node.end_byte(),
                };
                let asserted = (assertion.invocation.path == path
                    && assertion.invocation.range == range)
                    || assertion
                        .helpers
                        .iter()
                        .any(|h| h.anchor.path == path && h.anchor.range == range);
                let text = &source[node.byte_range()];
                if !asserted && !items::context_independent_attribute(text)
                    && !items::identity_inert_metadata(node.named_child(0)
                        .and_then(|n| n.named_child(0)).map(|n| &source[n.byte_range()]))
                    && items::derive_names(node, source).is_none_or(|names| names.iter().any(|(i, _)| i.is_none()))
                    // cfg predicates remain independent decisions, never discharged here.
                    && !text.starts_with("#[cfg(")
                {
                    return Ok(false);
                }
                continue;
            }
            "use_declaration" => {
                if !items::use_facts(node, source, controls).is_ok_and(|facts| !facts.0)
                    || node
                        .prev_named_sibling()
                        .is_some_and(|p| p.kind() == "attribute_item")
                {
                    return Ok(false);
                }
                let Ok(leaves) = items::use_leaves(node, source, controls) else {
                    return Ok(false);
                };
                for leaf in leaves {
                    if assertion.helpers.iter().any(|h| {
                        exact(&h.anchor, files, parsed, "attribute_item")
                            .and_then(|n| helper_name(n, &files[&h.anchor.path].source))
                            .is_some_and(|name| identity(&name) == identity(&leaf.binding))
                    }) || assertion.bindings.iter().any(|b| {
                        b.provider_path.split("::").next().map(identity)
                            == Some(identity(&leaf.binding))
                    }) || assertion.bindings.iter().any(|b| {
                        identity(&b.name) == identity(&leaf.binding) && b.provider_path != leaf.path
                    }) || leaf.path.split("::").any(|p| p == "*")
                        || !bindings.insert(identity(&leaf.binding).to_owned())
                    {
                        return Ok(false);
                    }
                }
                continue;
            }
            _ => {
                if let Some(name) = node.child_by_field_name("name")
                    && assertion.bindings.iter().any(|b| {
                        identity(&b.name) == identity(&source[name.byte_range()])
                            || b.provider_path.split("::").next().map(identity)
                                == Some(identity(&source[name.byte_range()]))
                    })
                {
                    return Ok(false);
                }
            }
        }
        for i in 0..node.named_child_count() {
            if let Some(child) = node.named_child(i as u32) {
                stack.push(child);
            }
        }
    }
    Ok(true)
}
#[allow(clippy::too_many_arguments)] // Exact admission consumes the existing immutable batch evidence.
pub(super) fn discharge(
    request: &MoveRequest,
    files: &BTreeMap<String, FileSnapshot>,
    parsed: &BTreeMap<String, ParsedFile>,
    selected: &[(String, Item, String)],
    contexts: &BTreeMap<String, ModuleEvidence>,
    written_proofs: &[BindingProof],
    repairs: &[Repair],
    needs: &mut Vec<items::Need>,
    controls: (Instant, &AtomicBool),
) -> Result<Vec<Proof>, DomainError> {
    let assertions = request
        .derived_attribute_relocations
        .as_deref()
        .unwrap_or_default();
    if assertions.len() > request.max_moves.saturating_mul(64) {
        return Err(invalid(0, "derive assertion cap exceeded"));
    }
    let mut seen = BTreeSet::new();
    let mut assertion_bytes = 0usize;
    for (index, assertion) in assertions.iter().enumerate() {
        assertion_bytes = assertion_bytes
            .saturating_add(assertion.invocation.expected_text.len())
            .saturating_add(
                assertion
                    .helpers
                    .iter()
                    .map(|h| h.anchor.expected_text.len() + h.derive_name.len() + 256)
                    .sum::<usize>(),
            )
            .saturating_add(
                assertion
                    .bindings
                    .iter()
                    .map(|b| {
                        b.name.len()
                            + b.provider_path.len()
                            + b.original_import.expected_text.len()
                            + b.destination_import.expected_text.len()
                            + 512
                    })
                    .sum::<usize>(),
            );
        if assertion.helpers.len() > 4096
            || assertion.bindings.len() > 1024
            || assertion_bytes > 128 * 1024 * 1024
        {
            return Err(invalid(index, "derive assertion descriptor cap exceeded"));
        }
    }
    let mut admitted = Vec::new();
    for (index, assertion) in assertions.iter().enumerate() {
        items::check(controls.0, controls.1)?;
        let a = &assertion.invocation;
        if !seen.insert((a.path.clone(), a.range.start_byte, a.range.end_byte)) {
            return Err(invalid(index, "duplicate derive invocation"));
        }
        let Some(node) = exact(a, files, parsed, "attribute_item") else {
            return Err(invalid(
                index,
                "derive invocation must be an exact captured attribute node",
            ));
        };
        let Some(names) = items::derive_names(node, &files[&a.path].source) else {
            return Err(invalid(
                index,
                "assertion must name a bare derive invocation",
            ));
        };
        let Some((path, item, destination)) = selected.iter().find(|(p, i, _)| {
            a.path == *p
                && i.attributes.iter().any(|attr| attr.range == a.range)
                && matches!(i.kind.as_str(), "struct_item" | "enum_item")
        }) else {
            return Err(invalid(
                index,
                "derive must be attached to a selected written struct or enum",
            ));
        };
        let mut helper_ranges = BTreeSet::new();
        for helper in &assertion.helpers {
            if !attached(&helper.anchor, path, item)
                || !helper_ranges
                    .insert((helper.anchor.range.start_byte, helper.anchor.range.end_byte))
                || exact(&helper.anchor, files, parsed, "attribute_item")
                    .and_then(|n| helper_name(n, &files[path].source))
                    .is_none()
                || !names.iter().any(|(_, r)| {
                    files[path].source.get(r.start_byte..r.end_byte)
                        == Some(helper.derive_name.as_str())
                })
            {
                return Err(invalid(
                    index,
                    "helper must be an exact attached registered-helper assertion naming a derive in this invocation",
                ));
            }
        }
        let mut eligible = true;
        let mut used_bindings = BTreeSet::new();
        let mut invocation_names = BTreeSet::new();
        for (_, range) in &names {
            let name = &files[path].source[range.start_byte..range.end_byte];
            if !bare_identifier(name) || !invocation_names.insert(identity(name)) {
                eligible = false;
                continue;
            }
            let bindings: Vec<_> = assertion
                .bindings
                .iter()
                .filter(|b| b.name == name)
                .collect();
            if bindings.is_empty()
                && written_proofs.iter().any(|proof| {
                    matches!(proof, BindingProof::Standard(p)
                    if p.class == prelude::BindingProofClass::StandardBuiltinDerive
                    && p.anchor.path == *path && p.anchor.range == *range
                    && p.destination_path == *destination)
                })
            {
                continue;
            }
            if bindings.len() != 1 {
                eligible = false;
                continue;
            }
            let binding = bindings[0];
            used_bindings.insert(name);
            if binding.provider_path.split("::").count() < 2
                || !binding.provider_path.split("::").all(bare_identifier)
                || matches!(
                    binding.provider_path.split("::").next(),
                    Some("self" | "super" | "crate")
                )
                || !import_matches(
                    &binding.original_import,
                    path,
                    binding,
                    files,
                    parsed,
                    controls,
                )
                || !import_matches(
                    &binding.destination_import,
                    destination,
                    binding,
                    files,
                    parsed,
                    controls,
                )
            {
                eligible = false;
            }
        }
        if used_bindings.len() != assertion.bindings.len() {
            eligible = false;
        }
        // Attribute/import edits would invalidate the asserted unchanged interpretation.
        let anchors = std::iter::once(a)
            .chain(assertion.helpers.iter().map(|h| &h.anchor))
            .chain(
                assertion
                    .bindings
                    .iter()
                    .flat_map(|b| [&b.original_import, &b.destination_import]),
            );
        for anchor in anchors {
            if request
                .trivia_overrides
                .as_deref()
                .unwrap_or_default()
                .iter()
                .any(|o| {
                    o.trivia.path == anchor.path
                        && o.trivia.range == anchor.range
                        && (matches!(o.disposition, MoveDisposition::KeepInPlace)
                            || o.target_item.as_ref().is_some_and(|target| {
                                target.path != *path || target.range != item.span.range
                            }))
                })
            {
                eligible = false;
            }
            if repairs.iter().any(|repair| {
                repair.path == anchor.path
                    && repair.range.start_byte < anchor.range.end_byte
                    && anchor.range.start_byte < repair.range.end_byte
            }) {
                eligible = false;
            }
        }
        for repair in repairs.iter().filter(|r| {
            r.kind == "import_insert"
                && (r.path == *path || r.path == *destination)
                && !r.after.is_empty()
        }) {
            let Some(tree) = trivia::parse(&repair.after, controls.0, controls.1)? else {
                eligible = false;
                continue;
            };
            let Some(import) = tree
                .root_node()
                .named_child(0)
                .filter(|n| n.kind() == "use_declaration")
            else {
                eligible = false;
                continue;
            };
            if !items::use_facts(import, &repair.after, controls).is_ok_and(|facts| !facts.0)
                || items::use_leaves(import, &repair.after, controls)?
                    .iter()
                    .any(|leaf| {
                        assertion.bindings.iter().any(|b| {
                            identity(&b.name) == identity(&leaf.binding)
                                || b.provider_path.split("::").next().map(identity)
                                    == Some(identity(&leaf.binding))
                        }) || assertion.helpers.iter().any(|h| {
                            exact(&h.anchor, files, parsed, "attribute_item")
                                .and_then(|n| helper_name(n, &files[path].source))
                                .is_some_and(|name| identity(&name) == identity(&leaf.binding))
                        })
                    })
            {
                eligible = false;
            }
        }
        if eligible {
            for (arrival_path, arrival, arrival_destination) in selected {
                if arrival_destination == destination
                    && arrival.id != item.id
                    && !safe_namespace(arrival_path, assertion, files, parsed, controls)?
                {
                    eligible = false;
                }
            }
            for context in [contexts.get(path), contexts.get(destination)]
                .into_iter()
                .flatten()
            {
                for ancestor in &context.filesystem_paths {
                    if files.contains_key(ancestor)
                        && !safe_namespace(ancestor, assertion, files, parsed, controls)?
                    {
                        eligible = false;
                    }
                }
            }
        }
        if !eligible
            || !safe_namespace(path, assertion, files, parsed, controls)?
            || !safe_namespace(destination, assertion, files, parsed, controls)?
        {
            continue;
        }
        admitted.push((
            path.clone(),
            item.id.clone(),
            destination.clone(),
            assertion.clone(),
        ));
    }
    let mut proofs = Vec::new();
    for (path, id, destination, assertion) in admitted {
        let mut discharged = false;
        needs.retain(|need| {
            if need.reason != DecisionReason::ConditionalOrInheritedContext || need.path != path {
                return true;
            }
            let Some(range) = &need.attribute_range else {
                return true;
            };
            let matches = *range == assertion.invocation.range
                || assertion.helpers.iter().any(|h| h.anchor.range == *range);
            if matches {
                discharged = true;
            }
            !matches
        });
        if discharged {
            proofs.push(Proof {
                class: ProofClass::AssumedDeriveRelocation,
                invocation: assertion.invocation,
                helpers: assertion.helpers,
                bindings: assertion.bindings,
                item_ids: vec![id],
                destination_path: destination,
                basis: BASIS.into(),
            });
        }
    }
    Ok(proofs)
}
