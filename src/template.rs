//! Exact expression-template expansion, independent of display bounds.
use crate::{matching::Candidate, pattern::Pattern, query::CompiledQuery, result::DomainError};
use std::{collections::BTreeSet, ops::Range};

pub struct Template {
    source: String,
    references: Vec<(String, Range<usize>)>,
}
#[derive(Clone)]
pub struct CopyOrigin {
    pub original: Range<usize>,
    pub output: Range<usize>,
}
pub struct Expansion {
    pub text: String,
    pub copies: Vec<CopyOrigin>,
}
impl Template {
    pub fn new(source: &str, pattern: &CompiledQuery) -> Result<Self, DomainError> {
        let compiled = Pattern::compile(source).map_err(|mut error| {
            error.code = "INVALID_TEMPLATE".into();
            error.field = Some("replacement".into());
            error
        })?;
        let names: BTreeSet<_> = pattern
            .sugar
            .as_ref()
            .expect("sugar pattern")
            .references()
            .into_iter()
            .map(|(name, _)| name)
            .collect();
        let references = compiled
            .sugar
            .as_ref()
            .expect("sugar template")
            .references();
        for (name, range) in &references {
            if !names.contains(name) {
                let mut error = DomainError::new(
                    "UNBOUND_METAVARIABLE",
                    format!("${name} is not bound by the pattern"),
                );
                error.field = Some("replacement".into());
                error.byte_offset = Some(range.start);
                return Err(error);
            }
        }
        Ok(Self {
            source: source.into(),
            references,
        })
    }
    pub fn expand(&self, candidate: &Candidate, source: &str) -> Expansion {
        let mut text = String::new();
        let mut copies = Vec::new();
        let mut last = 0;
        for (name, range) in &self.references {
            text.push_str(&self.source[last..range.start]);
            let (_, start, end) = candidate
                .captures
                .iter()
                .find(|(n, _, _)| n == name)
                .expect("validated binding");
            let output_start = text.len();
            text.push_str(&source[*start..*end]);
            copies.push(CopyOrigin {
                original: *start..*end,
                output: output_start..text.len(),
            });
            last = range.end;
        }
        text.push_str(&self.source[last..]);
        Expansion { text, copies }
    }
}
