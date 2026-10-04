use crate::{
    query::{CompiledQuery, rust_arity},
    result::*,
    scope::FileSnapshot,
};
use std::{
    collections::BTreeMap,
    ops::ControlFlow,
    sync::atomic::{AtomicBool, Ordering},
    time::Instant,
};
use tree_sitter::{ParseOptions, Parser, QueryCursor, QueryCursorOptions, StreamingIterator};

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Candidate {
    pub start: usize,
    pub end: usize,
    pub pattern: usize,
    pub captures: Vec<(String, usize, usize)>,
    pub subtree_recovery: bool,
    pub enclosing_recovery: bool,
    pub leading_start: usize,
    pub item_start: usize,
}
pub struct FileMatches {
    pub matches: Vec<Candidate>,
    pub observed_count: usize,
    pub diagnostics: Vec<Diagnostic>,
    pub diagnostics_count: usize,
    pub recovery: bool,
    pub comments: Vec<(usize, usize, String)>,
    pub stopped: Option<String>,
}
fn interrupted(cancelled: &AtomicBool, deadline: Instant) -> bool {
    cancelled.load(Ordering::Relaxed) || Instant::now() >= deadline
}
pub fn execute(
    file: &FileSnapshot,
    compiled: &CompiledQuery,
    limits: &Limits,
    deadline: Instant,
    cancelled: &AtomicBool,
) -> Result<FileMatches, DomainError> {
    let mut result = FileMatches {
        matches: Vec::new(),
        observed_count: 0,
        diagnostics: Vec::new(),
        diagnostics_count: 0,
        recovery: false,
        comments: Vec::new(),
        stopped: None,
    };
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_rust::LANGUAGE.into())
        .map_err(|_| DomainError::new("INTERNAL", "Rust grammar ABI setup failed"))?;
    let mut callback = |_: &tree_sitter::ParseState| {
        if interrupted(cancelled, deadline) {
            ControlFlow::Break(())
        } else {
            ControlFlow::Continue(())
        }
    };
    let mut source_callback = |offset: usize, _| &file.source.as_bytes()[offset..];
    let Some(tree) = parser.parse_with_options(
        &mut source_callback,
        None,
        Some(ParseOptions::new().progress_callback(&mut callback)),
    ) else {
        result.stopped = Some("parse_deadline".into());
        return Ok(result);
    };
    result.recovery = tree.root_node().has_error();
    let mut walker = tree.walk();
    loop {
        if interrupted(cancelled, deadline) {
            result.stopped = Some("syntax_deadline".into());
            return Ok(result);
        }
        let node = walker.node();
        if node.is_error() || node.is_missing() {
            result.diagnostics_count += 1;
            if result.diagnostics.len() < limits.diagnostic_count {
                result.diagnostics.push(Diagnostic {
                    path: file.path.clone(),
                    code: if node.is_missing() {
                        "MISSING"
                    } else {
                        "ERROR"
                    }
                    .into(),
                    range: ByteRange {
                        start_byte: node.start_byte(),
                        end_byte: node.end_byte(),
                    },
                });
            }
        }
        if matches!(node.kind(), "line_comment" | "block_comment") {
            // Bound trivia descriptors separately; exact omission counts are retained during rendering.
            let text = &file.source[node.byte_range()];
            let before = &file.source[..node.start_byte()];
            let content = text
                .strip_prefix("//")
                .or_else(|| text.strip_prefix("/*"))
                .unwrap_or(text)
                .trim_start();
            let reason = if ["---", "===", "***", "###"]
                .iter()
                .any(|prefix| content.starts_with(prefix))
            {
                "banner"
            } else if before.trim().is_empty() {
                "scope_prologue"
            } else if before.ends_with("\n\n") || before.ends_with("\r\n\r\n") {
                "blank_separator"
            } else {
                ""
            };
            if !reason.is_empty() {
                result
                    .comments
                    .push((node.start_byte(), node.end_byte(), reason.into()));
                if result.comments.len() > 100_000 {
                    result.stopped = Some("trivia_descriptor_limit".into());
                    return Ok(result);
                }
            }
        }
        if walker.goto_first_child() {
            continue;
        }
        loop {
            if walker.goto_next_sibling() {
                break;
            }
            if !walker.goto_parent() {
                break;
            }
        }
        if walker.node() == tree.root_node() {
            break;
        }
    }
    let mut cursor = QueryCursor::new();
    cursor.set_match_limit(limits.query_state_limit);
    let mut stopped = false;
    let mut progress = |_: &tree_sitter::QueryCursorState| {
        if interrupted(cancelled, deadline) {
            stopped = true;
            ControlFlow::Break(())
        } else {
            ControlFlow::Continue(())
        }
    };
    let mut descriptor_bytes = 0usize;
    let mut candidates = 0usize;
    {
        let mut matches = cursor.matches_with_options(
            &compiled.query,
            tree.root_node(),
            file.source.as_bytes(),
            QueryCursorOptions::new().progress_callback(&mut progress),
        );
        while let Some(found) = matches.next() {
            if interrupted(cancelled, deadline) {
                result.stopped = Some("query_deadline".into());
                break;
            }
            candidates += 1;
            if candidates > 100_000 {
                result.stopped = Some("candidate_match_limit".into());
                break;
            }
            let captures = found.captures();
            let roots: Vec<_> = captures
                .iter()
                .filter(|c| c.index == compiled.root)
                .collect();
            if roots.len() != 1 {
                return Err(DomainError::new(
                    "INVALID_QUERY_ROOT",
                    "runtime match must contain exactly one @match",
                ));
            }
            let pattern = found.pattern_index;
            let mut accepted = true;
            for (capture, arity) in &compiled.arity[pattern] {
                let nodes: Vec<_> = captures.iter().filter(|c| c.index == *capture).collect();
                if nodes.len() != 1 || nodes[0].node.kind() != "arguments" {
                    return Err(DomainError::new(
                        "INVALID_QUERY_ROOT",
                        "#rust-arity? must capture an arguments node",
                    ));
                }
                accepted &= rust_arity(nodes[0].node, *arity);
            }
            if !accepted {
                continue;
            }
            let node = roots[0].node;
            let mut capture_ranges = if let Some(sugar) = &compiled.sugar {
                let Some(bindings) = sugar.captures(node, &file.source) else {
                    continue;
                };
                bindings
            } else {
                Vec::new()
            };
            for capture in captures
                .iter()
                .filter(|c| c.index != compiled.root && compiled.sugar.is_none())
            {
                let range = capture.node.byte_range();
                if file.source.get(range.clone()).is_none() {
                    return Err(DomainError::new(
                        "INVALID_QUERY_ROOT",
                        "capture range is invalid",
                    ));
                }
                capture_ranges.push((
                    compiled.query.capture_names()[capture.index as usize].into(),
                    range.start,
                    range.end,
                ));
            }
            capture_ranges.sort();
            descriptor_bytes += 64
                + capture_ranges
                    .iter()
                    .map(|(name, _, _)| name.len() + 32)
                    .sum::<usize>();
            if descriptor_bytes > 128 * 1024 * 1024 {
                result.stopped = Some("capture_descriptor_limit".into());
                break;
            }
            let mut ancestor = node.parent();
            let mut item = node;
            let mut enclosing = false;
            while let Some(parent) = ancestor {
                if parent.kind() == "source_file" {
                    break;
                }
                enclosing |= parent.has_error();
                if parent.kind().ends_with("_item") {
                    item = parent;
                }
                ancestor = parent.parent();
            }
            let mut leading = item;
            while let Some(previous) = leading.prev_named_sibling() {
                if !matches!(
                    previous.kind(),
                    "line_comment" | "block_comment" | "attribute_item"
                ) {
                    break;
                }
                leading = previous;
            }
            result.observed_count += 1;
            result.matches.push(Candidate {
                start: node.start_byte(),
                end: node.end_byte(),
                pattern,
                captures: capture_ranges,
                subtree_recovery: node.has_error(),
                enclosing_recovery: enclosing,
                leading_start: leading.start_byte(),
                item_start: item.start_byte(),
            });
        }
    }
    if stopped || interrupted(cancelled, deadline) {
        result.stopped = Some("query_deadline".into());
    }
    if cursor.did_exceed_match_limit() {
        result.stopped = Some("query_state_limit".into());
    }
    if result.stopped.is_some() {
        result.matches.clear();
    } else {
        result.matches.sort();
        result.matches.dedup();
    }
    Ok(result)
}

pub struct Lines<'a> {
    source: &'a str,
    starts: Vec<usize>,
}
impl<'a> Lines<'a> {
    pub fn new(source: &'a str) -> Self {
        Self {
            source,
            starts: std::iter::once(0)
                .chain(
                    source
                        .bytes()
                        .enumerate()
                        .filter_map(|(i, b)| (b == b'\n').then_some(i + 1)),
                )
                .collect(),
        }
    }
    fn row(&self, byte: usize) -> usize {
        self.starts.partition_point(|start| *start <= byte) - 1
    }
    pub fn slice(&self, start: usize, end: usize, text_bytes: usize) -> SourceSlice {
        let first = self.row(start);
        let last = self.row(end);
        let len = end - start;
        SourceSlice {
            range: ByteRange {
                start_byte: start,
                end_byte: end,
            },
            start: Position {
                line: first + 1,
                byte_column: start - self.starts[first],
            },
            end: Position {
                line: last + 1,
                byte_column: end - self.starts[last],
            },
            text: (len <= text_bytes).then(|| self.source[start..end].into()),
            text_bytes: len,
            text_omitted: len > text_bytes,
        }
    }
    fn line(&self, row: usize, text_bytes: usize) -> SourceSlice {
        self.slice(
            self.starts[row],
            self.starts
                .get(row + 1)
                .copied()
                .unwrap_or(self.source.len()),
            text_bytes,
        )
    }
}
pub fn render(
    file: &FileSnapshot,
    data: &FileMatches,
    candidate: &Candidate,
    (file_ordinal, match_ordinal): (usize, usize),
    context: &Context,
    text_bytes: usize,
    lines: &Lines<'_>,
) -> MatchRecord {
    let first = lines.row(candidate.start);
    let last = lines.row(
        candidate
            .end
            .saturating_sub(usize::from(candidate.end > candidate.start)),
    );
    let physical_lines = lines.starts.len() - usize::from(file.source.ends_with('\n'));
    let before_start = first.saturating_sub(context.before_lines);
    let after_end = (last + 1 + context.after_lines).min(physical_lines);
    let mut captures: BTreeMap<String, Option<Vec<SourceSlice>>> = BTreeMap::new();
    for (name, start, end) in &candidate.captures {
        captures
            .entry(name.clone())
            .or_insert_with(|| Some(Vec::new()))
            .as_mut()
            .expect("capture vector")
            .push(lines.slice(*start, *end, text_bytes));
    }
    let trivia: Vec<_> = data
        .comments
        .iter()
        .filter(|(start, end, _)| {
            (*start >= candidate.start && *end <= candidate.end)
                || (*start >= candidate.leading_start && *end <= candidate.item_start)
                || (*end <= candidate.start && file.source[*end..candidate.start].trim().is_empty())
                || (*start >= candidate.end && file.source[candidate.end..*start].trim().is_empty())
        })
        .map(|(start, end, reason)| TriviaDecision {
            id: format!("t/{file_ordinal}/{start}/{end}"),
            path: file.path.clone(),
            span: lines.slice(*start, *end, text_bytes),
            reason: reason.clone(),
            default_disposition: "keep_in_place".into(),
        })
        .collect();
    MatchRecord {
        id: format!("m/{file_ordinal}/{match_ordinal}"),
        path: file.path.clone(),
        span: lines.slice(candidate.start, candidate.end, text_bytes),
        captures,
        captures_omitted: BTreeMap::new(),
        capture_occurrences: candidate.captures.len(),
        capture_occurrences_omitted: 0,
        pattern_index: candidate.pattern,
        syntax: SyntaxFlags {
            file_has_recovery: data.recovery,
            subtree_has_recovery: candidate.subtree_recovery,
            enclosing_has_recovery: candidate.enclosing_recovery,
        },
        context: ContextLines {
            before: (before_start..first)
                .map(|row| lines.line(row, text_bytes))
                .collect(),
            after: (last + 1..after_end)
                .map(|row| lines.line(row, text_bytes))
                .collect(),
            before_clipped: context.before_lines - (first - before_start),
            after_clipped: context.after_lines - after_end.saturating_sub(last + 1),
            before_omitted: 0,
            after_omitted: 0,
        },
        trivia_decisions: trivia,
        trivia_decisions_omitted: 0,
    }
}
