//! Original-coordinate splices; artifacts are never written by the engine.
use crate::result::{ByteRange, DomainError};
use rmcp::schemars::JsonSchema;
use serde::Serialize;

#[derive(Debug, Clone, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct Edit {
    pub path: String,
    pub range: ByteRange,
    pub original_text: String,
    pub replacement_text: String,
    pub match_ids: Vec<String>,
    pub trivia_ids: Vec<String>,
}
impl Edit {
    pub fn new(
        path: &str,
        source: &str,
        start: usize,
        end: usize,
        replacement: String,
        match_id: &str,
    ) -> Self {
        Self {
            path: path.into(),
            range: ByteRange {
                start_byte: start,
                end_byte: end,
            },
            original_text: source[start..end].into(),
            replacement_text: replacement,
            match_ids: vec![match_id.into()],
            trivia_ids: Vec::new(),
        }
    }
}
pub fn sort_validate(edits: &mut [Edit]) -> Result<(), DomainError> {
    edits.sort_by(|a, b| (&a.path, &a.range).cmp(&(&b.path, &b.range)));
    for pair in edits.windows(2) {
        let (a, b) = (&pair[0], &pair[1]);
        if a.path == b.path
            && (a.range.end_byte > b.range.start_byte
                || (a.range.start_byte == b.range.start_byte
                    && a.range.start_byte == a.range.end_byte))
        {
            return Err(DomainError::new(
                "OVERLAPPING_EDITS",
                "selected ranges overlap/nest or inserts share a boundary; narrow selection",
            ));
        }
    }
    Ok(())
}
pub fn reconstruct(source: &str, edits: &[Edit]) -> Result<String, DomainError> {
    let mut proposed = source.to_owned();
    for edit in edits.iter().rev() {
        let range = edit.range.start_byte..edit.range.end_byte;
        if source.get(range.clone()) != Some(&edit.original_text) {
            return Err(DomainError::new(
                "SOURCE_CHANGED",
                "edit range/original bytes are invalid",
            ));
        }
        proposed.replace_range(range, &edit.replacement_text);
    }
    Ok(proposed)
}
