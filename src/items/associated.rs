//! Whole written associated units; top-level binding inventory stays separate.
use super::*;
use crate::plan::SourceAnchor;
use serde::Serialize;

#[derive(Debug, Clone, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct ImplIdentity {
    /// Exact header anchor, ending immediately before the opening brace.
    pub anchor: SourceAnchor,
    /// Whole enclosing impl extent; never repeats its body text.
    pub range: ByteRange,
    pub header: String,
    pub written_type: String,
    pub exclusions: Vec<String>,
}

pub(crate) fn identity(
    file: &FileSnapshot,
    node: Node<'_>,
    trivia: &[trivia::Trivia],
) -> Option<ImplIdentity> {
    let body = node.child_by_field_name("body")?;
    let mut exclusions = Vec::new();
    if node.child_by_field_name("trait").is_some() {
        exclusions.push("trait_impl".into());
    }
    for i in 0..node.child_count() {
        if matches!(node.child(i)?.kind(), "unsafe" | "!") {
            exclusions.push("unsafe_negative_or_where_impl".into());
        }
    }
    if trivia.iter().any(|t| {
        t.is_attribute() && (t.owned_by(node.byte_range()) && t.range.start < body.start_byte())
    }) {
        exclusions.push("attributed_impl".into());
    }
    let ty = node.child_by_field_name("type")?;
    let nominal = if ty.kind() == "generic_type" {
        ty.child_by_field_name("type")?
    } else {
        ty
    };
    let written_type = file.source[nominal.byte_range()].to_owned();
    if !crate::rewrites::simple_path(&written_type) {
        exclusions.push("non_simple_impl_type".into());
    }
    Some(ImplIdentity {
        anchor: SourceAnchor {
            path: file.path.clone(),
            range: ByteRange {
                start_byte: node.start_byte(),
                end_byte: body.start_byte(),
            },
            expected_text: file.source[node.start_byte()..body.start_byte()].into(),
        },
        range: ByteRange {
            start_byte: node.start_byte(),
            end_byte: node.end_byte(),
        },
        header: file.source[node.start_byte()..body.start_byte()].into(),
        written_type,
        exclusions,
    })
}

pub(crate) fn inventory(
    file: &FileSnapshot,
    tree: &Tree,
    trivia: &[trivia::Trivia],
    text_bytes: usize,
    controls: (Instant, &AtomicBool),
    observed: &mut usize,
) -> Result<Vec<Item>, DomainError> {
    let lines = Lines::new(&file.source);
    let mut out = Vec::new();
    for i in 0..tree.root_node().named_child_count() {
        check(controls.0, controls.1)?;
        let implementation = tree.root_node().named_child(i as u32).expect("child");
        if implementation.kind() != "impl_item" {
            continue;
        }
        let Some(identity) = identity(file, implementation, trivia) else {
            continue;
        };
        let body = implementation
            .child_by_field_name("body")
            .expect("impl body");
        let macro_members = (0..body.named_child_count()).any(|i| {
            body.named_child(i as u32)
                .is_some_and(|n| n.kind() == "macro_invocation")
        });
        for j in 0..body.named_child_count() {
            check(controls.0, controls.1)?;
            let node = body.named_child(j as u32).expect("member");
            if matches!(
                node.kind(),
                "attribute_item" | "inner_attribute_item" | "line_comment" | "block_comment"
            ) {
                continue;
            }
            *observed += 1;
            if *observed > 100_000 {
                return Err(DomainError::new(
                    "inventory_work_limit",
                    "associated descriptor cap reached",
                ));
            }
            let mut reasons = identity.exclusions.clone();
            if !matches!(node.kind(), "function_item" | "const_item") {
                reasons.push("unsupported_associated_unit".into());
            }
            if macro_members {
                reasons.push("macro_generated_members_unexamined".into());
            }
            if node.child_by_field_name("type_parameters").is_some()
                || node.child_by_field_name("where_clause").is_some()
            {
                reasons.push("generic_member".into());
            }
            let attached: Vec<_> = trivia
                .iter()
                .filter(|t| {
                    t.owned_by(node.byte_range())
                        || (t.range.start >= node.start_byte() && t.range.end <= node.end_byte())
                })
                .collect();
            if attached.iter().any(|t| {
                t.is_attribute() && !context_independent_attribute(&file.source[t.range.clone()])
            }) {
                reasons.push("conditional_or_unexamined_member_attribute".into());
            }
            let span = lines.slice(node.start_byte(), node.end_byte(), text_bytes);
            out.push(Item {
                id: format!(
                    "a/{}/{}/{}/{}/{}",
                    file.path,
                    implementation.start_byte(),
                    implementation.end_byte(),
                    node.start_byte(),
                    node.end_byte()
                ),
                path: file.path.clone(),
                span: span.clone(),
                kind: node.kind().into(),
                name: node
                    .child_by_field_name("name")
                    .map(|n| file.source[n.byte_range()].into()),
                visibility: (0..node.named_child_count())
                    .filter_map(|i| node.named_child(i as u32))
                    .find(|n| n.kind() == "visibility_modifier")
                    .map(|n| file.source[n.byte_range()].into())
                    .unwrap_or_else(|| "private".into()),
                visibility_key: visibility_key(node, &file.source, controls.0, controls.1)?,
                attributes: attached
                    .iter()
                    .filter(|t| t.is_attribute())
                    .map(|t| lines.slice(t.range.start, t.range.end, text_bytes))
                    .collect(),
                trivia: attached
                    .iter()
                    .map(|t| lines.slice(t.range.start, t.range.end, text_bytes))
                    .collect(),
                bytes: node.end_byte() - node.start_byte(),
                lines: span.end.line - span.start.line + 1,
                syntax: SyntaxFlags {
                    file_has_recovery: tree.root_node().has_error(),
                    subtree_has_recovery: node.has_error(),
                    enclosing_has_recovery: implementation.has_error(),
                },
                // Even an otherwise supported member needs its enclosing header, so it
                // must not enter top-level-only advice selections or automatic partitions.
                eligibility: "context_sensitive".into(),
                reasons,
                signal_ids: Vec::new(),
                enclosing_impl: Some(identity.clone()),
            });
        }
    }
    Ok(out)
}
