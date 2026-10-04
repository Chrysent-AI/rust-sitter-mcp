//! Source-anchored ownership and byte-origin proofs, never text-equality guesses.
use crate::{
    edit::Edit,
    matching::{Candidate, Lines},
    plan::{Disposition, PlanEnvelope, SourceAnchor, TriviaOverride},
    result::{ByteRange, DomainError, TriviaDecision},
    template::{CopyOrigin, Expansion},
};
use std::{
    collections::BTreeSet,
    ops::{ControlFlow, Range},
    sync::atomic::{AtomicBool, Ordering},
    time::Instant,
};
use tree_sitter::{Node, ParseOptions, Parser, Tree};

pub struct Trivia {
    pub range: Range<usize>,
    kind: String,
    protected: bool,
    pub classification: String,
    pub reason: String,
    owner: Option<Owner>,
}
#[derive(Clone)]
struct Owner {
    kind: String,
    marker: Range<usize>,
}
pub struct Primary {
    pub candidate: usize,
    pub id: String,
    pub expansion: Expansion,
    trivia_ids: Vec<String>,
    before: Vec<(Range<usize>, String, String)>,
    after: Vec<(Range<usize>, String, String)>,
}
impl Primary {
    pub fn new(candidate: usize, id: String, expansion: Expansion) -> Self {
        Self {
            candidate,
            id,
            expansion,
            trivia_ids: Vec::new(),
            before: Vec::new(),
            after: Vec::new(),
        }
    }
}
pub fn parse(
    source: &str,
    deadline: Instant,
    cancelled: &AtomicBool,
) -> Result<Option<Tree>, DomainError> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_rust::LANGUAGE.into())
        .map_err(|_| DomainError::new("INTERNAL", "grammar ABI setup failed"))?;
    let mut progress = |_: &tree_sitter::ParseState| {
        if cancelled.load(Ordering::Relaxed) || Instant::now() >= deadline {
            ControlFlow::Break(())
        } else {
            ControlFlow::Continue(())
        }
    };
    let mut bytes = |offset: usize, _| &source.as_bytes()[offset..];
    Ok(parser.parse_with_options(
        &mut bytes,
        None,
        Some(ParseOptions::new().progress_callback(&mut progress)),
    ))
}
fn comment(node: Node<'_>) -> bool {
    matches!(node.kind(), "line_comment" | "block_comment")
}
fn trivia_node(node: Node<'_>) -> bool {
    comment(node)
        || matches!(
            node.kind(),
            "attribute_item" | "inner_attribute_item" | "shebang"
        )
}
fn marker(node: Node<'_>) -> Option<Range<usize>> {
    let mut stack = vec![node];
    while let Some(node) = stack.pop() {
        if trivia_node(node) {
            continue;
        }
        if node.child_count() == 0 && node.end_byte() > node.start_byte() {
            return Some(node.byte_range());
        }
        for i in (0..node.child_count()).rev() {
            stack.push(node.child(i).expect("child"));
        }
    }
    None
}
fn owner(node: Node<'_>) -> Option<Owner> {
    Some(Owner {
        kind: node.kind().into(),
        marker: marker(node)?,
    })
}
fn scope_owner(node: Node<'_>) -> Option<Owner> {
    let mut parent = node.parent();
    while let Some(node) = parent {
        if node.kind() == "source_file" {
            return Some(Owner {
                kind: "source_file".into(),
                marker: 0..0,
            });
        }
        if matches!(node.kind(), "block" | "declaration_list") {
            return owner(node);
        }
        parent = node.parent();
    }
    None
}
fn next_significant(mut node: Node<'_>) -> Option<Node<'_>> {
    while let Some(next) = node.next_named_sibling() {
        node = next;
        if !trivia_node(next) {
            return Some(next);
        }
    }
    None
}
fn previous_significant(mut node: Node<'_>) -> Option<Node<'_>> {
    while let Some(previous) = node.prev_named_sibling() {
        node = previous;
        if !trivia_node(previous) {
            return Some(previous);
        }
    }
    None
}
fn blank(gap: &str) -> bool {
    let normalized = gap.replace("\r\n", "\n");
    normalized
        .split('\n')
        .skip(1)
        .take(
            normalized
                .bytes()
                .filter(|b| *b == b'\n')
                .count()
                .saturating_sub(1),
        )
        .any(|line| line.trim().is_empty())
}
pub fn inventory(tree: &Tree, source: &str) -> Vec<Trivia> {
    let mut out = Vec::new();
    let mut stack = vec![tree.root_node()];
    while let Some(node) = stack.pop() {
        if !trivia_node(node) {
            for i in (0..node.child_count()).rev() {
                stack.push(node.child(i).expect("child"));
            }
            continue;
        }
        let text = &source[node.byte_range()];
        let inner = node.kind() == "inner_attribute_item"
            || node.kind() == "shebang"
            || node.child_by_field_name("inner").is_some();
        let outer = node.kind() == "attribute_item" || node.child_by_field_name("outer").is_some();
        let prev = previous_significant(node);
        let next = next_significant(node);
        let before = &source[..node.start_byte()];
        let same_line = before
            .rsplit('\n')
            .next()
            .is_some_and(|line| !line.trim().is_empty())
            && prev.is_some_and(|p| {
                source[p.end_byte()..node.start_byte()]
                    .bytes()
                    .all(|b| matches!(b, b' ' | b'\t' | b'\r'))
            });
        let content = text
            .strip_prefix("//")
            .or_else(|| text.strip_prefix("/*"))
            .unwrap_or(text)
            .trim_start();
        let banner = ["---", "===", "***", "###"]
            .iter()
            .any(|prefix| content.starts_with(prefix));
        let preceding = node
            .prev_named_sibling()
            .map(|p| &source[p.end_byte()..node.start_byte()])
            .unwrap_or(before);
        let following = node
            .next_named_sibling()
            .map(|p| &source[node.end_byte()..p.start_byte()])
            .unwrap_or("");
        let (classification, reason, attached) = if inner {
            ("scope", "scope_prologue", scope_owner(node))
        } else if outer {
            ("leading", "outer_attachment", next.and_then(owner))
        } else if same_line {
            ("trailing", "same_line", prev.and_then(owner))
        } else if banner {
            ("ambiguous", "banner", None)
        } else if blank(preceding) || blank(following) {
            ("ambiguous", "blank_separator", None)
        } else if prev.is_none()
            && matches!(
                node.parent().map(|p| p.kind()),
                Some("source_file" | "declaration_list")
            )
        {
            ("scope", "scope_prologue", scope_owner(node))
        } else if next.is_some() {
            ("leading", "contiguous_preceding", next.and_then(owner))
        } else {
            ("ambiguous", "competing_neighbors", None)
        };
        out.push(Trivia {
            range: node.byte_range(),
            kind: node.kind().into(),
            protected: inner || outer || classification == "scope",
            classification: classification.into(),
            reason: reason.into(),
            owner: attached,
        });
    }
    out.sort_by_key(|t| (t.range.start, t.range.end));
    out
}
fn internal(t: &Trivia, c: &Candidate) -> bool {
    t.range.start >= c.start && t.range.end <= c.end
}
fn copied(t: &Trivia, p: &Primary) -> bool {
    p.expansion
        .copies
        .iter()
        .any(|copy| copy.original.start <= t.range.start && copy.original.end >= t.range.end)
}
fn relevant(t: &Trivia, c: &Candidate, source: &str) -> bool {
    internal(t, c)
        || (t.range.start >= c.leading_start && t.range.end <= c.item_start)
        || (t.range.end <= c.start && source[t.range.end..c.start].trim().is_empty())
        || (t.range.start >= c.end && source[c.end..t.range.start].trim().is_empty())
}
fn anchor(path: &str, source: &str, range: Range<usize>) -> SourceAnchor {
    SourceAnchor {
        path: path.into(),
        range: ByteRange {
            start_byte: range.start,
            end_byte: range.end,
        },
        expected_text: source[range].into(),
    }
}
fn separator(source: &str, at: usize) -> &'static str {
    if let Some(end) = source[..at].rfind('\n') {
        if end > 0 && source.as_bytes()[end - 1] == b'\r' {
            return "\r\n";
        }
        return "\n";
    }
    if let Some(end) = source[at..].find('\n')
        && end > 0
        && source.as_bytes()[at + end - 1] == b'\r'
    {
        return "\r\n";
    }
    "\n"
}
pub fn account(
    (path, ordinal): (&str, usize),
    source: &str,
    candidates: &[Candidate],
    trivia: &[Trivia],
    primaries: &mut [Primary],
    overrides: &[TriviaOverride],
    result: &mut PlanEnvelope,
) -> Result<(Vec<Edit>, Vec<SourceAnchor>), DomainError> {
    let lines = Lines::new(source);
    let mut edits = Vec::new();
    let mut consumed = Vec::new();
    for t in trivia {
        let affected: Vec<_> = primaries
            .iter()
            .enumerate()
            .filter(|(_, p)| relevant(t, &candidates[p.candidate], source))
            .map(|(i, _)| i)
            .collect();
        if affected.is_empty() {
            continue;
        }
        let id = format!("t/{ordinal}/{}/{}", t.range.start, t.range.end);
        for index in &affected {
            primaries[*index].trivia_ids.push(id.clone());
        }
        let trivia_anchor = anchor(path, source, t.range.clone());
        let override_value = overrides.iter().find(|o| o.trivia == trivia_anchor);
        if override_value.is_some() {
            consumed.push(trivia_anchor);
        }
        let consuming: Vec<_> = affected
            .iter()
            .copied()
            .filter(|i| internal(t, &candidates[primaries[*i].candidate]))
            .collect();
        let retained = consuming.iter().all(|i| copied(t, &primaries[*i]));
        let mut decision = TriviaDecision {
            id: id.clone(),
            path: path.into(),
            span: lines.slice(t.range.start, t.range.end, result.limits.text_bytes),
            reason: if !retained {
                "internal_unretained".into()
            } else {
                t.reason.clone()
            },
            default_disposition: "keep_in_place".into(),
            classification: t.classification.clone(),
            owner: t.owner.as_ref().map(|o| ByteRange {
                start_byte: o.marker.start,
                end_byte: o.marker.end,
            }),
            suggested_dispositions: vec!["keep_in_place".into()],
            selected_disposition: "keep_in_place".into(),
            preservable: retained,
            match_ids: affected.iter().map(|i| primaries[*i].id.clone()).collect(),
        };
        if !t.protected && !consuming.iter().any(|i| copied(t, &primaries[*i])) {
            decision
                .suggested_dispositions
                .extend(["before_match".into(), "after_match".into()]);
        }
        let mut moved = false;
        if let Some(o) = override_value {
            match o.disposition {
                Disposition::KeepInPlace => {
                    if o.target_match.is_some() {
                        return Err(DomainError::new(
                            "INVALID_TRIVIA_OVERRIDE",
                            "keep_in_place forbids target_match",
                        ));
                    }
                }
                Disposition::BeforeMatch | Disposition::AfterMatch => {
                    let target = o.target_match.as_ref().ok_or_else(|| {
                        DomainError::new(
                            "INVALID_TRIVIA_OVERRIDE",
                            "movement requires selected same-file target_match",
                        )
                    })?;
                    let target_index = primaries.iter().position(|p| {
                        anchor(
                            path,
                            source,
                            candidates[p.candidate].start..candidates[p.candidate].end,
                        ) == *target
                    });
                    let valid_target = target_index.filter(|_| {
                        !t.protected
                            && !consuming.iter().any(|i| copied(t, &primaries[*i]))
                            && consuming.len() <= 1
                    });
                    if let Some(index) = valid_target {
                        let text = source[t.range.clone()].to_owned();
                        let before = matches!(o.disposition, Disposition::BeforeMatch);
                        let p = &mut primaries[index];
                        if before {
                            p.before.push((t.range.clone(), text, id.clone()));
                        } else {
                            p.after.push((t.range.clone(), text, id.clone()));
                        }
                        if consuming.is_empty() {
                            let mut deletion = Edit::new(
                                path,
                                source,
                                t.range.start,
                                t.range.end,
                                String::new(),
                                &p.id,
                            );
                            deletion.trivia_ids.push(id.clone());
                            edits.push(deletion);
                        }
                        decision.selected_disposition = if before {
                            "before_match"
                        } else {
                            "after_match"
                        }
                        .into();
                        decision.preservable = true;
                        moved = true;
                    } else {
                        result.blocker("UNSUPPORTED_TRIVIA_DISPOSITION","docs, attributes, scope prologues, copied trivia and unselected/cross-file targets cannot move",Some(path),Some(decision.span.range.clone()));
                    }
                }
            }
        }
        if !retained && !moved {
            result.blocker("UNRETAINED_TRIVIA","template consumes trivia without retained capture provenance; keep containing capture or use preserving ordinary-comment override",Some(path),Some(decision.span.range.clone()));
        }
        result.plan.trivia_decisions.push(decision);
    }
    for p in primaries {
        let c = &candidates[p.candidate];
        p.before.sort_by_key(|(r, _, _)| (r.start, r.end));
        p.after.sort_by_key(|(r, _, _)| (r.start, r.end));
        let mut prefix = String::new();
        let mut suffix = String::new();
        let mut moves = Vec::new();
        let mut ids = p.trivia_ids.clone();
        for (range, text, id) in &p.before {
            let start = prefix.len();
            prefix.push_str(text);
            moves.push(CopyOrigin {
                original: range.clone(),
                output: start..prefix.len(),
            });
            prefix.push_str(if text.starts_with("//") && !text.ends_with('\n') {
                separator(source, range.start)
            } else {
                " "
            });
            ids.push(id.clone());
        }
        let offset = prefix.len();
        for copy in &mut p.expansion.copies {
            copy.output.start += offset;
            copy.output.end += offset;
        }
        let body_end = offset + p.expansion.text.len();
        for (range, text, id) in &p.after {
            suffix.push_str(if text.starts_with("//") {
                separator(source, range.start)
            } else {
                " "
            });
            let start = body_end + suffix.len();
            suffix.push_str(text);
            moves.push(CopyOrigin {
                original: range.clone(),
                output: start..body_end + suffix.len(),
            });
            if text.starts_with("//") && !text.ends_with('\n') {
                suffix.push_str(separator(source, range.start));
            }
            ids.push(id.clone());
        }
        p.expansion.text = format!("{prefix}{}{suffix}", p.expansion.text);
        p.expansion.copies.extend(moves);
        let mut edit = Edit::new(
            path,
            source,
            c.start,
            c.end,
            p.expansion.text.clone(),
            &p.id,
        );
        ids.sort();
        ids.dedup();
        edit.trivia_ids = ids;
        edits.push(edit);
    }
    Ok((edits, consumed))
}
/// Map retained source intervals to virtual bytes, including explicit capture/comment copies.
pub fn origins(
    source_len: usize,
    edits: &[Edit],
    primaries: &[Primary],
    candidates: &[Candidate],
) -> Vec<CopyOrigin> {
    let mut copies = Vec::new();
    let mut original_at = 0;
    let mut output_at = 0;
    for edit in edits {
        let start = edit.range.start_byte;
        let end = edit.range.end_byte;
        copies.push(CopyOrigin {
            original: original_at..start,
            output: output_at..output_at + start - original_at,
        });
        output_at += start - original_at;
        if let Some(p) = primaries
            .iter()
            .find(|p| candidates[p.candidate].start == start && candidates[p.candidate].end == end)
        {
            copies.extend(p.expansion.copies.iter().map(|copy| CopyOrigin {
                original: copy.original.clone(),
                output: output_at + copy.output.start..output_at + copy.output.end,
            }));
        }
        output_at += edit.replacement_text.len();
        original_at = end;
    }
    copies.push(CopyOrigin {
        original: original_at..source_len,
        output: output_at..output_at + source_len - original_at,
    });
    copies
}
fn mapped(range: &Range<usize>, copies: &[CopyOrigin]) -> Vec<Range<usize>> {
    copies
        .iter()
        .filter(|copy| copy.original.start <= range.start && copy.original.end >= range.end)
        .map(|copy| {
            copy.output.start + range.start - copy.original.start
                ..copy.output.start + range.end - copy.original.start
        })
        .collect()
}
pub fn verify(original: &[Trivia], proposed: &[Trivia], copies: &[CopyOrigin]) -> bool {
    original.iter().all(|t| {
        let ranges = mapped(&t.range, copies);
        !ranges.is_empty()
            && ranges.iter().all(|range| {
                proposed
                    .iter()
                    .find(|new| new.range == *range && new.kind == t.kind)
                    .is_some_and(|new| {
                        if !t.protected {
                            return true;
                        }
                        match (&t.owner, &new.owner) {
                            (Some(old), Some(new)) => {
                                old.kind == new.kind
                                    && mapped(&old.marker, copies).contains(&new.marker)
                            }
                            _ => false,
                        }
                    })
            })
    })
}
pub fn validate_overrides(overrides: &[TriviaOverride]) -> Result<(), DomainError> {
    let mut seen = BTreeSet::new();
    for o in overrides {
        if !seen.insert(o.trivia.clone()) {
            return Err(DomainError::new(
                "INVALID_TRIVIA_OVERRIDE",
                "duplicate/conflicting trivia anchor",
            ));
        }
    }
    Ok(())
}
