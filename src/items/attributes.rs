//! One strict attribute classifier for advice and written-binding/move guards.
use crate::result::ByteRange;
use tree_sitter::Node;

/// These existing forms require no relaxed proof. Derives deliberately remain
/// outside this predicate: macro identity needs scoped, opt-in evidence.
pub(crate) fn context_independent_attribute(text: &str) -> bool {
    text.starts_with("#[allow(")
        || text.starts_with("#![allow(")
        || text == "#[inline]"
        || text == "#[inline(always)]"
        || text == "#[inline(never)]"
        || text.starts_with("#[repr(")
}

// Compiler-provided derive macros, not arbitrary traits with these spellings.
pub(crate) const BUILTIN_DERIVES: [(&str, &str); 9] = [
    ("Debug", "std::fmt::Debug"),
    ("Clone", "std::clone::Clone"),
    ("Copy", "std::marker::Copy"),
    ("PartialEq", "std::cmp::PartialEq"),
    ("Eq", "std::cmp::Eq"),
    ("PartialOrd", "std::cmp::PartialOrd"),
    ("Ord", "std::cmp::Ord"),
    ("Hash", "std::hash::Hash"),
    ("Default", "std::default::Default"),
];

/// Recognize an outer, bare `derive(...)` attribute. Each comma-delimited name
/// retains its original coordinates; path forms and unknown names stay vetoes.
/// Grammar tokens, not source splitting, keep comments from disguising a path.
pub(crate) fn derive_names(
    node: Node<'_>,
    source: &str,
) -> Option<Vec<(Option<usize>, ByteRange)>> {
    if node.kind() != "attribute_item" || node.has_error() || node.is_missing() {
        return None;
    }
    let attribute = node.named_child(0)?;
    let name = attribute.named_child(0)?;
    if name.kind() != "identifier" || &source[name.byte_range()] != "derive" {
        return None;
    }
    let arguments = attribute.child_by_field_name("arguments")?;
    if arguments.child(0)?.kind() != "("
        || arguments.child(arguments.child_count() - 1)?.kind() != ")"
    {
        return None;
    }
    let mut result = Vec::new();
    let mut tokens = Vec::new();
    for i in 1..arguments.child_count() - 1 {
        let token = arguments.child(i)?;
        if matches!(token.kind(), "line_comment" | "block_comment") {
            continue;
        }
        if token.kind() == "," {
            if tokens.is_empty() {
                return None;
            }
            result.push(derive_name(&tokens, source));
            tokens.clear();
        } else {
            tokens.push(token);
        }
    }
    if !tokens.is_empty() {
        result.push(derive_name(&tokens, source));
    }
    (!result.is_empty()).then_some(result)
}

fn derive_name(tokens: &[Node<'_>], source: &str) -> (Option<usize>, ByteRange) {
    let first = tokens[0];
    let index = (tokens.len() == 1 && first.kind() == "identifier")
        .then(|| {
            BUILTIN_DERIVES
                .iter()
                .position(|(name, _)| *name == source[first.byte_range()].trim_start_matches("r#"))
        })
        .flatten();
    (
        index,
        ByteRange {
            start_byte: first.start_byte(),
            end_byte: tokens.last().expect("nonempty derive").end_byte(),
        },
    )
}
