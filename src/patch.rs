//! Git framing belongs to this application, not the line-diff formatter.
use crate::result::DomainError;

fn label(prefix: &str, path: &str) -> String {
    let mut out = String::from("\"");
    for byte in prefix.bytes().chain(path.bytes()) {
        match byte {
            b'"' => out.push_str("\\\""),
            b'\\' => out.push_str("\\\\"),
            b'\n' => out.push_str("\\n"),
            b'\r' => out.push_str("\\r"),
            b'\t' => out.push_str("\\t"),
            32..=126 => out.push(char::from(byte)),
            _ => out.push_str(&format!("\\{byte:03o}")),
        }
    }
    out.push('"');
    out
}
pub fn section(path: &str, original: &str, proposed: &str) -> Result<String, DomainError> {
    if path.is_empty()
        || path.starts_with('/')
        || path.contains('\0')
        || path.split('/').any(|part| matches!(part, "" | "." | ".."))
    {
        return Err(DomainError::new(
            "PATCH_ENCODING_FAILED",
            "patch path must be normalized and root-relative",
        ));
    }
    if original == proposed {
        return Ok(String::new());
    }
    let old = label("a/", path);
    let new = label("b/", path);
    let diff = similar::TextDiff::from_lines(original, proposed);
    let hunks = diff
        .unified_diff()
        .context_radius(3)
        .missing_newline_hint(true)
        .header(&old, &new)
        .to_string();
    if hunks.is_empty() || !hunks.contains("@@ ") {
        return Err(DomainError::new(
            "PATCH_ENCODING_FAILED",
            "changed file has no encoded hunks",
        ));
    }
    Ok(format!("diff --git {old} {new}\n{hunks}"))
}
