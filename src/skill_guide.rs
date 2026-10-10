//! Build-time bundled Markdown; serving it never consults the filesystem.
use crate::result::DomainError;
use rmcp::schemars::JsonSchema;
use serde::{Deserialize, Serialize};

struct EmbeddedDocument {
    id: &'static str,
    path: &'static str,
    digest: &'static str,
    content: &'static str,
}
include!(concat!(env!("OUT_DIR"), "/skill_guide.rs"));

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(default, deny_unknown_fields)]
pub struct GuideRequest {
    /// Omit for the full concatenation; otherwise use an exact documents[].id.
    pub document: Option<String>,
    pub limits: GuideLimits,
}
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(default, deny_unknown_fields)]
pub struct GuideLimits {
    /// Duplicated structured/text wire budget, 64 KiB–16 MiB; default 2 MiB.
    pub response_bytes: usize,
}
impl Default for GuideLimits {
    fn default() -> Self {
        Self {
            response_bytes: 2 * 1024 * 1024,
        }
    }
}
#[derive(Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct DocumentIndex {
    pub id: &'static str,
    pub path: &'static str,
    pub digest: &'static str,
    pub bytes: usize,
}
#[derive(Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct GuideIntegrity {
    pub semantic: &'static str,
}
#[derive(Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct GuideEnvelope {
    pub schema_version: u32,
    pub tool: &'static str,
    pub status: &'static str,
    pub documents: Vec<DocumentIndex>,
    /// Exact full concatenation size, even for a selected slice or omitted content.
    pub bytes: usize,
    pub digest: &'static str,
    pub document: Option<String>,
    /// Selected content's full size/digest (concatenation when document is absent).
    pub content_bytes: usize,
    pub content_digest: Option<&'static str>,
    pub content: Option<&'static str>,
    pub omitted_bytes: usize,
    pub omission_reason: Option<&'static str>,
    pub limits: GuideLimits,
    pub integrity: GuideIntegrity,
    pub error: Option<DomainError>,
}
impl GuideEnvelope {
    pub fn failed(error: DomainError) -> Self {
        let mut result = Self::base(GuideRequest::default());
        result.status = "failed";
        result.error = Some(error);
        result
    }
    fn base(request: GuideRequest) -> Self {
        Self {
            schema_version: 1,
            tool: "read_skill_guide",
            status: "complete",
            documents: DOCUMENTS
                .iter()
                .map(|d| DocumentIndex {
                    id: d.id,
                    path: d.path,
                    digest: d.digest,
                    bytes: d.content.len(),
                })
                .collect(),
            bytes: CONCATENATED.len(),
            digest: CONCATENATED_DIGEST,
            document: request.document,
            content_bytes: 0,
            content_digest: None,
            content: None,
            omitted_bytes: 0,
            omission_reason: None,
            limits: request.limits,
            integrity: GuideIntegrity {
                semantic: "not_performed",
            },
            error: None,
        }
    }
    pub fn wire_bytes(&self) -> usize {
        let value = serde_json::to_value(self).expect("serializable guide");
        serde_json::to_vec(&serde_json::json!({"content":[{"type":"text","text":value.to_string()}],"structuredContent":value,"isError":self.error.is_some()})).expect("serializable guide wire").len() + 4096
    }
}

pub fn read(request: GuideRequest) -> GuideEnvelope {
    let mut result = GuideEnvelope::base(request);
    let error = if !(64 * 1024..=16 * 1024 * 1024).contains(&result.limits.response_bytes) {
        Some(DomainError::new(
            "INVALID_PARAMS",
            "limits.response_bytes must be 65536–16777216",
        ))
    } else {
        None
    };
    let selected = match result.document.as_deref() {
        None => Some((CONCATENATED, CONCATENATED_DIGEST)),
        Some(id) => DOCUMENTS
            .iter()
            .find(|d| d.id == id)
            .map(|d| (d.content, d.digest)),
    };
    // Never echo unbounded caller input into the mandatory index/error envelope.
    if error.is_some() || selected.is_none() {
        result.document = None;
        result.limits = GuideLimits::default();
        result.status = "failed";
        result.error = error.or_else(|| {
            Some(DomainError::new(
                "INVALID_PARAMS",
                "unknown document; use an exact documents[].id",
            ))
        });
        return result;
    }
    let (content, digest) = selected.expect("validated document");
    result.content = Some(content);
    result.content_bytes = content.len();
    result.content_digest = Some(digest);
    if result.wire_bytes() > result.limits.response_bytes {
        result.content = None;
        result.status = "partial";
        result.omitted_bytes = content.len();
        result.omission_reason = Some("response_bytes");
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::path::Path;
    use std::process::{Command, Stdio};

    fn digest(bytes: &[u8]) -> String {
        let mut child = Command::new("git")
            .args(["hash-object", "--stdin"])
            .current_dir(std::env::temp_dir())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(bytes).unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(output.status.success());
        format!(
            "git-blob-sha1:{}",
            String::from_utf8(output.stdout).unwrap().trim()
        )
    }
    fn reference_paths(root: &Path, paths: &mut Vec<String>) {
        for entry in std::fs::read_dir(root).unwrap() {
            let entry = entry.unwrap();
            if entry.file_type().unwrap().is_dir() {
                reference_paths(&entry.path(), paths);
            } else {
                paths.push(
                    entry
                        .path()
                        .strip_prefix(env!("CARGO_MANIFEST_DIR"))
                        .unwrap()
                        .to_str()
                        .unwrap()
                        .replace('\\', "/"),
                );
            }
        }
    }
    fn assert_document_matches(d: &EmbeddedDocument, bytes: &[u8]) {
        assert_eq!(
            digest(bytes),
            d.digest,
            "embedded skill digest drift for {}; rebuild",
            d.path
        );
        assert_eq!(bytes, d.content.as_bytes());
    }
    #[test]
    fn drift_guard_rejects_an_unrebuilt_local_edit_and_passes_after_restore() {
        let d = &DOCUMENTS[0];
        let directory =
            std::env::temp_dir().join(format!("embedded-guide-drift-{}", std::process::id()));
        std::fs::create_dir(&directory).unwrap();
        let path = directory.join("guide.md");
        std::fs::write(&path, d.content).unwrap();
        assert_document_matches(d, &std::fs::read(&path).unwrap());
        std::fs::write(
            &path,
            format!("{}\nTemporary drift demonstration.\n", d.content),
        )
        .unwrap();
        let stale =
            std::panic::catch_unwind(|| assert_document_matches(d, &std::fs::read(&path).unwrap()));
        assert!(
            stale.is_err(),
            "an unrebuilt embedded guide must reject edited bytes"
        );
        std::fs::write(&path, d.content).unwrap();
        assert_document_matches(d, &std::fs::read(&path).unwrap());
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(directory).unwrap();
    }
    #[test]
    fn embedded_skill_matches_working_tree() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut paths = Vec::new();
        reference_paths(
            &root.join(".pi/skills/split-rust-code/references"),
            &mut paths,
        );
        paths.sort();
        paths.insert(0, ".pi/skills/split-rust-code/SKILL.md".into());
        assert_eq!(
            paths,
            DOCUMENTS
                .iter()
                .map(|d| d.path.to_owned())
                .collect::<Vec<_>>(),
            "embedded skill file inventory drift; rebuild"
        );
        let mut concatenated = String::new();
        for d in DOCUMENTS {
            let bytes = std::fs::read(root.join(d.path)).unwrap();
            assert_document_matches(d, &bytes);
            if !concatenated.is_empty() {
                concatenated.push_str("\n\n");
            }
            concatenated.push_str(&format!(
                "# Bundled document: {}\n\nSource: `{}`\n\nDigest: `{}`\n\n{}",
                d.id, d.path, d.digest, d.content
            ));
        }
        assert_eq!(concatenated, CONCATENATED);
        assert_eq!(digest(concatenated.as_bytes()), CONCATENATED_DIGEST);
    }
    #[test]
    fn full_and_selected_content_are_exact() {
        let full = read(GuideRequest::default());
        assert_eq!(full.content, Some(CONCATENATED));
        assert_eq!(full.bytes, full.content_bytes);
        assert_eq!(full.digest, full.content_digest.unwrap());
        assert_eq!(full.omitted_bytes, 0);
        for d in DOCUMENTS {
            let slice = read(GuideRequest {
                document: Some(d.id.into()),
                ..Default::default()
            });
            assert_eq!(slice.content, Some(d.content));
            assert_eq!(slice.content_digest, Some(d.digest));
            assert_eq!(slice.content_bytes, d.content.len());
            assert_eq!(slice.bytes, full.bytes);
            assert_eq!(slice.documents.len(), full.documents.len());
        }
    }
    #[test]
    fn omission_keeps_index_and_exact_sizes() {
        let limited = read(GuideRequest {
            limits: GuideLimits {
                response_bytes: 65536,
            },
            ..Default::default()
        });
        assert_eq!(limited.status, "partial");
        assert!(limited.content.is_none());
        assert_eq!(limited.omitted_bytes, CONCATENATED.len());
        assert_eq!(limited.omission_reason, Some("response_bytes"));
        assert_eq!(limited.documents.len(), DOCUMENTS.len());
        assert!(limited.wire_bytes() <= 65536);
        let mut full = read(GuideRequest::default());
        full.limits.response_bytes = full.wire_bytes();
        let exact = full.wire_bytes();
        assert!(
            read(GuideRequest {
                limits: GuideLimits {
                    response_bytes: exact
                },
                ..Default::default()
            })
            .content
            .is_some()
        );
        assert!(
            read(GuideRequest {
                limits: GuideLimits {
                    response_bytes: exact - 1
                },
                ..Default::default()
            })
            .content
            .is_none()
        );
    }
    #[test]
    fn invalid_selection_and_limits_fail_with_index() {
        for request in [
            GuideRequest {
                document: Some("unknown".repeat(100000)),
                ..Default::default()
            },
            GuideRequest {
                limits: GuideLimits { response_bytes: 1 },
                ..Default::default()
            },
        ] {
            let result = read(request);
            assert_eq!(result.status, "failed");
            assert_eq!(result.error.unwrap().code, "INVALID_PARAMS");
            assert_eq!(result.documents.len(), DOCUMENTS.len());
            assert!(result.content.is_none());
        }
        assert!(
            serde_json::from_str::<GuideRequest>(r#"{"limits":{"time_budget_ms":1}}"#).is_err()
        );
    }
}
