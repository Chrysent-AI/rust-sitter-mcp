use crate::result::DomainError;
use tree_sitter::{CaptureQuantifier, Query, QueryPredicateArg};

pub struct CompiledQuery {
    pub query: Query,
    pub root: u32,
    pub arity: Vec<Vec<(u32, u32)>>,
}
impl CompiledQuery {
    pub fn new(source: &str) -> Result<Self, DomainError> {
        if source.len() > 64 * 1024 {
            return Err(DomainError::new("INVALID_QUERY", "query exceeds 64 KiB"));
        }
        let query = Query::new(&tree_sitter_rust::LANGUAGE.into(), source)
            .map_err(|e| DomainError::new("INVALID_QUERY", e.to_string()))?;
        if query.pattern_count() == 0
            || query.pattern_count() > 64
            || query.capture_names().len() > 64
        {
            return Err(DomainError::new(
                "INVALID_QUERY",
                "query needs 1–64 patterns and at most 64 capture names",
            ));
        }
        let root = query.capture_index_for_name("match").ok_or_else(|| {
            DomainError::new(
                "INVALID_QUERY_ROOT",
                "every pattern needs exactly one @match",
            )
        })?;
        let mut arity = Vec::new();
        for index in 0..query.pattern_count() {
            if !query.is_pattern_rooted(index)
                || query.capture_quantifiers(index)[root as usize] != CaptureQuantifier::One
            {
                return Err(DomainError::new(
                    "INVALID_QUERY_ROOT",
                    "patterns must be rooted and capture exactly one @match",
                ));
            }
            if !query.property_settings(index).is_empty()
                || !query.property_predicates(index).is_empty()
            {
                return Err(DomainError::new(
                    "UNSUPPORTED_QUERY_OPERATION",
                    "property predicates and directives are unsupported",
                ));
            }
            let mut checks = Vec::new();
            for predicate in query.general_predicates(index) {
                if predicate.operator.as_ref() != "rust-arity?" {
                    return Err(DomainError::new(
                        "UNSUPPORTED_QUERY_OPERATION",
                        format!("unsupported predicate #{}", predicate.operator),
                    ));
                }
                let [
                    QueryPredicateArg::Capture(capture),
                    QueryPredicateArg::String(number),
                ] = predicate.args.as_ref()
                else {
                    return Err(DomainError::new(
                        "INVALID_QUERY",
                        "#rust-arity? requires @capture and a quoted decimal u32",
                    ));
                };
                if query.capture_quantifiers(index)[*capture as usize] != CaptureQuantifier::One
                    || number.is_empty()
                    || !number.bytes().all(|b| b.is_ascii_digit())
                {
                    return Err(DomainError::new(
                        "INVALID_QUERY",
                        "#rust-arity? capture must occur once and N must be decimal u32",
                    ));
                }
                let count = number
                    .parse()
                    .map_err(|_| DomainError::new("INVALID_QUERY", "#rust-arity? N exceeds u32"))?;
                checks.push((*capture, count));
            }
            arity.push(checks);
        }
        Ok(Self { query, root, arity })
    }
}

pub fn rust_arity(node: tree_sitter::Node<'_>, expected: u32) -> bool {
    if node.kind() != "arguments" || node.has_error() {
        return false;
    }
    let language = node.language();
    let expressions = language.id_for_node_kind("_expression", true);
    let subtypes = language.subtypes_for_supertype(expressions);
    let mut cursor = node.walk();
    let children: Vec<_> = node
        .children(&mut cursor)
        .filter(|n| !matches!(n.kind(), "line_comment" | "block_comment"))
        .collect();
    if children.first().is_none_or(|n| n.kind() != "(")
        || children.last().is_none_or(|n| n.kind() != ")")
    {
        return false;
    }
    let mut count = 0u32;
    let mut wants_expression = true;
    for child in children
        .iter()
        .skip(1)
        .take(children.len().saturating_sub(2))
    {
        if wants_expression {
            if !child.is_named()
                || !subtypes.contains(&child.kind_id())
                || child.is_error()
                || child.is_missing()
            {
                return false;
            }
            count += 1;
            wants_expression = false;
        } else if child.kind() == "," {
            wants_expression = true;
        } else {
            return false;
        }
    }
    count == expected
}
