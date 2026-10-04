use rmcp::schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tree_sitter::{Parser, Query, QueryCursor, StreamingIterator};

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(deny_unknown_fields)]
pub struct SearchRequest {
    /// Explicit repository directory. No current-directory fallback.
    pub repo_path: String,
    /// A Tree-sitter Rust query with a result capture named @match.
    pub query: String,
}

#[derive(Debug, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct SearchResult {
    pub matches: Vec<SmokeMatch>,
}

#[derive(Debug, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct SmokeMatch {
    pub path: String,
    pub start_byte: usize,
    pub end_byte: usize,
    pub text: String,
}

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct SearchError(pub String);

pub fn search(request: SearchRequest) -> Result<SearchResult, SearchError> {
    let language = tree_sitter_rust::LANGUAGE.into();
    let query = Query::new(&language, &request.query).map_err(|e| SearchError(e.to_string()))?;
    let root_capture = query
        .capture_index_for_name("match")
        .ok_or_else(|| SearchError("query requires @match".into()))?;
    let mut matches = Vec::new();
    for entry in ignore::WalkBuilder::new(&request.repo_path)
        .hidden(false)
        .build()
    {
        let entry = entry.map_err(|e| SearchError(e.to_string()))?;
        if entry.file_type().is_none_or(|t| !t.is_file())
            || entry.path().extension().is_none_or(|e| e != "rs")
        {
            continue;
        }
        let source =
            std::fs::read_to_string(entry.path()).map_err(|e| SearchError(e.to_string()))?;
        let mut parser = Parser::new();
        parser
            .set_language(&language)
            .map_err(|e| SearchError(e.to_string()))?;
        let tree = parser
            .parse(&source, None)
            .ok_or_else(|| SearchError("parser interrupted".into()))?;
        let mut cursor = QueryCursor::new();
        let mut found = cursor.matches(&query, tree.root_node(), source.as_bytes());
        while let Some(m) = found.next() {
            for capture in m.captures().iter().filter(|c| c.index == root_capture) {
                matches.push(SmokeMatch {
                    path: entry
                        .path()
                        .strip_prefix(&request.repo_path)
                        .unwrap_or(entry.path())
                        .to_string_lossy()
                        .into(),
                    start_byte: capture.node.start_byte(),
                    end_byte: capture.node.end_byte(),
                    text: source[capture.node.byte_range()].into(),
                });
            }
        }
    }
    Ok(SearchResult { matches })
}
