use crate::utils::display::{sanitize_error_diagnostic, sanitize_stop_reason};
use crate::pi::protocol::INVALID_TERMINAL_METADATA;

const TRANSCRIPT_ELISION: &str = "<accumulated content elided by the transcript sanitizer>";
const TRANSCRIPT_TERMINAL_METADATA_MAX_CHARS: usize = 240;
const TRANSCRIPT_ERROR_METADATA_MAX_CHARS: usize = 4096;

fn compact_transcript_message(value: &serde_json::Value) -> serde_json::Value {
    let Some(message) = value.as_object() else {
        return serde_json::json!({ "content": [{ "type": "text", "text": TRANSCRIPT_ELISION }] });
    };
    let mut compact = serde_json::Map::new();
    for key in ["role", "model", "usage"] {
        if let Some(v) = message.get(key) {
            compact.insert(key.to_string(), v.clone());
        }
    }
    for key in ["stopReason", "stop_reason"] {
        if let Some(value) = message.get(key) {
            compact.insert(key.to_string(), sanitized_transcript_stop_reason(value));
        }
    }
    // Preserve the failure reason even though content stays elided: the
    // error field is small, is not stream spam, and is the only record of
    // why a message ended in "error"/"aborted". Mirrors the TS sanitizer.
    for key in ["errorMessage", "error_message"] {
        if let Some(raw) = message.get(key).and_then(|v| v.as_str()) {
            compact.insert(
                key.to_string(),
                serde_json::Value::String(sanitize_error_diagnostic(
                    raw,
                    TRANSCRIPT_ERROR_METADATA_MAX_CHARS,
                )),
            );
        }
    }
    compact.insert(
        "content".to_string(),
        serde_json::json!([{ "type": "text", "text": TRANSCRIPT_ELISION }]),
    );
    serde_json::Value::Object(compact)
}

fn sanitized_transcript_stop_reason(value: &serde_json::Value) -> serde_json::Value {
    sanitized_transcript_terminal_value(value, sanitize_stop_reason)
}

fn sanitized_transcript_error(value: &serde_json::Value) -> serde_json::Value {
    sanitized_transcript_terminal_value(value, sanitize_error_diagnostic)
}

fn sanitized_transcript_terminal_value(
    value: &serde_json::Value,
    project: fn(&str, usize) -> String,
) -> serde_json::Value {
    serde_json::Value::String(
        value
            .as_str()
            .map(|raw| project(raw, TRANSCRIPT_TERMINAL_METADATA_MAX_CHARS))
            .unwrap_or_else(|| INVALID_TERMINAL_METADATA.to_string()),
    )
}
