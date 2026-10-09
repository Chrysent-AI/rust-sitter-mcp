//! Borrowed preallocation bounds and owned charges for historical advice.
use super::*;

pub(super) fn evidence_allocation(evidence: &Evidence) -> usize {
    allocation(&evidence.normalized_scope)
        + allocation(&evidence.source_manifest)
        + evidence
            .buffers
            .iter()
            .map(|(p, b)| 256 + p.capacity() + b.capacity())
            .sum::<usize>()
        + evidence.inputs.ignores.capacity() * size_of::<scope::IgnoreInput>()
        + evidence
            .inputs
            .ignores
            .iter()
            .map(|i| i.path.capacity() + i.bytes.as_ref().map_or(0, Vec::capacity))
            .sum::<usize>()
        + evidence.inputs.identities.capacity() * size_of::<(String, u64, u64, u32)>()
        + evidence
            .inputs
            .identities
            .iter()
            .map(|i| i.0.capacity())
            .sum::<usize>()
        + evidence.scope_input_digest.capacity()
}

pub(super) fn retention_allocation(retention: &Retention) -> usize {
    retention.state.capacity()
        + [
            &retention.analysis_handle,
            &retention.analysis_id,
            &retention.snapshot_id,
            &retention.scope_input_digest,
            &retention.expires_at,
            &retention.reason,
        ]
        .into_iter()
        .map(|s| s.as_ref().map_or(0, String::capacity))
        .sum::<usize>()
        + retention.normalized_request.as_ref().map_or(0, allocation)
        + allocation(&retention.provenance)
}

pub(super) fn preparation_allocation(
    canonical: &SuggestSplitEnvelope,
    evidence: &Evidence,
    request: &SuggestSplitRequest,
    limit: usize,
    controls: Controls<'_>,
) -> Result<usize, &'static str> {
    // Metadata/node/Arc/registry reserves include both fixed-width prepared and published
    // identities. Replaced normalized options are counted alongside the original request,
    // and source text is counted before stripping, so the whole materialization is covered.
    let base = size_of::<Record>()
        .saturating_add(evidence_allocation(evidence))
        .saturating_add(4096)
        .saturating_add(canonical.snapshot_id.as_ref().map_or(0, String::len))
        .saturating_add(evidence.scope_input_digest.len())
        .saturating_add(canonical.root.as_ref().map_or(0, String::len))
        .saturating_add(env!("CARGO_PKG_VERSION").len())
        .saturating_add(env!("GIT_SHA").len())
        .saturating_add(allocation(&evidence.normalized_scope["paths"]))
        .saturating_add(allocation(&evidence.normalized_scope["globs"]));
    let remaining = limit.checked_sub(base).ok_or("record_too_large")?;
    let canonical = estimate(canonical, remaining, controls)?;
    let request = estimate(request, remaining - canonical, controls)?;
    Ok(base + canonical + request)
}

// serde_json's installed Value serializer copies strings, grows Vecs for unknown-length
// sequences and uses BTree maps. Count escaped bytes (twice, conservatively), map-entry
// reserves and spare array capacity without retaining JSON or a second Value tree.
// Commas in objects overcount array slots deliberately. Quote/escape state spans writes.
struct Estimate<'a> {
    bytes: usize,
    limit: usize,
    quoted: bool,
    escaped: bool,
    controls: Controls<'a>,
    failure: Option<&'static str>,
}
impl Write for Estimate<'_> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.controls.check().is_err() {
            self.failure = Some("cancelled_before_publication");
            return Err(io::Error::other("retention cancelled"));
        }
        for &byte in buf {
            let mut cost = 2;
            if self.quoted {
                if self.escaped {
                    self.escaped = false;
                } else if byte == b'\\' {
                    self.escaped = true;
                } else if byte == b'"' {
                    self.quoted = false;
                }
            } else {
                cost += match byte {
                    b'"' => {
                        self.quoted = true;
                        8
                    }
                    b':' => 256,
                    b'[' => 4 * size_of::<Value>(),
                    b',' => 2 * size_of::<Value>(),
                    _ => 0,
                };
            }
            self.bytes = match self.bytes.checked_add(cost).filter(|&n| n <= self.limit) {
                Some(bytes) => bytes,
                None => {
                    self.failure = Some("record_too_large");
                    return Err(io::Error::other("retention allocation bound exceeded"));
                }
            };
        }
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
fn estimate(
    value: &impl Serialize,
    limit: usize,
    controls: Controls<'_>,
) -> Result<usize, &'static str> {
    let mut counter = Estimate {
        bytes: 0,
        limit,
        quoted: false,
        escaped: false,
        controls,
        failure: None,
    };
    serde_json::to_writer(&mut counter, value)
        .map_err(|_| counter.failure.unwrap_or("record_too_large"))?;
    Ok(counter.bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn streaming_bound_covers_nested_escaped_values_and_unknown_length_arrays() {
        let cancelled = AtomicBool::new(false);
        let controls = Controls {
            deadline: Instant::now() + Duration::from_secs(60),
            cancelled: &cancelled,
        };
        for value in [
            json!([]),
            json!([null]),
            json!([1, 2, 3, 4, 5]),
            json!({"punctuation:,[": ["λ\\\"", {"empty": [], "text": "x".repeat(8192)}]}),
        ] {
            let bytes = estimate(&value, usize::MAX, controls).unwrap();
            let owned = serde_json::to_value(&value).unwrap();
            assert!(bytes >= allocation(&owned));
            assert_eq!(
                estimate(&value, bytes - 1, controls),
                Err("record_too_large")
            );
        }
        struct UnknownLength;
        impl Serialize for UnknownLength {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                use serde::ser::SerializeSeq;
                let mut sequence = serializer.serialize_seq(None)?;
                for _ in 0..65 {
                    sequence.serialize_element(&"x")?;
                }
                sequence.end()
            }
        }
        assert!(
            estimate(&UnknownLength, usize::MAX, controls).unwrap()
                >= allocation(&serde_json::to_value(UnknownLength).unwrap())
        );
    }
}
