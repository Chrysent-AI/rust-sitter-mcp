pub use crate::result::SearchRequest;
use crate::{
    cursor::Cursors,
    matching,
    query::CompiledQuery,
    result::*,
    scope::{self, Scope},
};
use std::{
    path::{Path, PathBuf},
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};

pub struct Engine {
    launch: PathBuf,
    cursors: Mutex<Cursors>,
}
impl Engine {
    pub fn new(launch: PathBuf) -> Result<Self, DomainError> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_rust::LANGUAGE.into())
            .map_err(|_| DomainError::new("INTERNAL", "Rust grammar ABI setup failed"))?;
        Ok(Self {
            launch,
            cursors: Mutex::new(Cursors::default()),
        })
    }
    pub fn search(&self, request: SearchRequest, cancelled: &AtomicBool) -> SearchEnvelope {
        let mut result = SearchEnvelope::empty(request.limits.clone());
        let start = Instant::now();
        let outcome = self.run(request, cancelled, &mut result);
        if let Err(error) = outcome {
            result.status = "failed".into();
            result.error = Some(error);
            result.matches.clear();
            result.counts.returned_matches = 0;
            result.next_cursor = None;
            result.has_more = None;
            result.coverage.scope_exhaustive = false;
        }
        tracing::info!(tool = "search_query", root = ?result.root, elapsed_ms = start.elapsed().as_millis(), status = %result.status, returned = result.counts.returned_matches, skipped = result.skipped.values().map(|r|r.count).sum::<u64>(), truncation = ?result.truncation_reasons, error = ?result.error.as_ref().map(|e| &e.code), "search finished");
        result
    }
    fn run(
        &self,
        mut request: SearchRequest,
        cancelled: &AtomicBool,
        result: &mut SearchEnvelope,
    ) -> Result<(), DomainError> {
        request.limits.validate()?;
        let deadline =
            Instant::now() + std::time::Duration::from_millis(request.limits.time_budget_ms);
        if request.context.before_lines > 20
            || request.context.after_lines > 20
            || !(1..=1000).contains(&request.page_size)
        {
            return Err(DomainError::new(
                "INVALID_PARAMS",
                "context is 0–20 lines and page_size is 1–1000",
            ));
        }
        let compiled = CompiledQuery::new(&request.query)?;
        let root = scope::resolve(&request.repo_path, &self.launch)?;
        result.root = Some(root.to_str().expect("validated root").into());
        let scope = Scope::new(root, &request)?;
        request.repo_path = result.root.clone().expect("resolved root");
        request.paths = scope.paths.clone();
        request.globs = scope.globs.clone();
        let token = request.cursor.take();
        let normalized = serde_json::to_string(&request).expect("request JSON");
        let resume = token
            .as_ref()
            .map(|token| {
                self.cursors
                    .lock()
                    .expect("cursor lock")
                    .lookup(token, &normalized)
            })
            .transpose()?;
        let (files, snapshot) = scope::discover(&scope, result, deadline, cancelled)?;
        result.snapshot_id = snapshot.clone();
        if let Some((_, series, _, _)) = &resume
            && snapshot
                .as_ref()
                .is_some_and(|snapshot| snapshot != &series.snapshot)
        {
            return Err(DomainError::new(
                "STALE_CURSOR",
                "scope manifest or source bytes changed; restart without cursor",
            ));
        }
        let (start_file, start_match) = resume
            .as_ref()
            .map(|(_, _, f, m)| (*f, *m))
            .unwrap_or((0, 0));
        if start_file >= files.len() && resume.is_some() && snapshot.is_some() {
            return Err(DomainError::new(
                "INVALID_CURSOR",
                "cursor file position is out of bounds",
            ));
        }
        let mut next = None;
        let mut work_stopped = false;
        let workers = std::thread::available_parallelism()
            .map(usize::from)
            .unwrap_or(1)
            .min(4);
        let mut file_index = start_file;
        'batches: while file_index < files.len() {
            if cancelled.load(Ordering::Relaxed) {
                return Err(DomainError::new("CANCELLED", "request cancelled"));
            }
            if Instant::now() >= deadline {
                next = Some((file_index, 0));
                work_stopped = true;
                result.truncation_reasons.push("matching_deadline".into());
                break;
            }
            let end = (file_index + workers).min(files.len());
            let batch = std::thread::scope(|threads| {
                let handles: Vec<_> = files[file_index..end]
                    .iter()
                    .map(|file| {
                        let compiled = &compiled;
                        let limits = &request.limits;
                        threads.spawn(move || {
                            matching::execute(file, compiled, limits, deadline, cancelled)
                        })
                    })
                    .collect();
                handles
                    .into_iter()
                    .map(|handle| {
                        handle.join().unwrap_or_else(|_| {
                            Err(DomainError::new("INTERNAL", "parser worker failed"))
                        })
                    })
                    .collect::<Vec<_>>()
            });
            for (offset, data) in batch.into_iter().enumerate() {
                let ordinal = file_index + offset;
                let data = data?;
                let remaining = request.limits.diagnostic_count.saturating_sub(
                    result.diagnostics.len()
                        + result
                            .skipped
                            .values()
                            .map(|r| r.examples.len())
                            .sum::<usize>(),
                );
                let emitted = data.diagnostics.len().min(remaining);
                result
                    .diagnostics
                    .extend(data.diagnostics[..emitted].iter().cloned());
                result.diagnostics_omitted += data.diagnostics_count - emitted;
                if let Some(reason) = &data.stopped {
                    result.counts.observed_matches += data.observed_count;
                    next = Some((ordinal, 0));
                    work_stopped = true;
                    result.truncation_reasons.push(reason.clone());
                    break 'batches;
                }
                result.counts.matched_files += 1;
                let skip = if ordinal == start_file {
                    start_match
                } else {
                    0
                };
                if skip > data.matches.len() {
                    return Err(DomainError::new(
                        "INVALID_CURSOR",
                        "cursor match position is out of bounds",
                    ));
                }
                result.counts.observed_matches += data.matches.len().saturating_sub(skip);
                let lines = matching::Lines::new(&files[ordinal].source);
                for (index, candidate) in data.matches.iter().enumerate().skip(skip) {
                    if Instant::now() >= deadline {
                        next = Some((ordinal, index));
                        work_stopped = true;
                        result.truncation_reasons.push("render_deadline".into());
                        break 'batches;
                    }
                    if result.matches.len() == request.page_size {
                        next = Some((ordinal, index));
                        result.truncation_reasons.push("page_size".into());
                        break 'batches;
                    }
                    let record = matching::render(
                        &files[ordinal],
                        &data,
                        candidate,
                        (ordinal, index),
                        &request.context,
                        request.limits.text_bytes,
                        &lines,
                    );
                    result.matches.push(record);
                    trim_metadata(result);
                    if result.wire_bytes() > request.limits.response_bytes {
                        let mut record = result.matches.pop().expect("just pushed");
                        if result.matches.is_empty() {
                            record.descriptor();
                            result.matches.push(record);
                            if result.wire_bytes() > request.limits.response_bytes {
                                return Err(DomainError::new(
                                    "INTERNAL",
                                    "mandatory match descriptor exceeds wire budget",
                                ));
                            }
                            result.truncation_reasons.push("field_omissions".into());
                        } else {
                            next = Some((ordinal, index));
                            result.truncation_reasons.push("response_bytes".into());
                            break 'batches;
                        }
                    }
                }
            }
            file_index = end;
        }
        if cancelled.load(Ordering::Relaxed) {
            return Err(DomainError::new("CANCELLED", "request cancelled"));
        }
        if let Some((file, position)) = next {
            result.has_more = if work_stopped { None } else { Some(true) };
            if let Some(snapshot) = snapshot {
                let mut cursors = self.cursors.lock().expect("cursor lock");
                let id = if let Some((id, _, _, _)) = resume {
                    id
                } else {
                    cursors.insert(normalized, snapshot)?
                };
                result.next_cursor = Some(cursors.token(id, file, position));
            } else {
                result
                    .truncation_reasons
                    .push("scan_incomplete: narrow paths/globs; cursor unavailable".into());
                result.has_more = None;
            }
        }
        let complete_matching = next.is_none() && result.coverage.eligible_scan_complete;
        result.coverage.scope_exhaustive &= complete_matching;
        if complete_matching && token.is_none() {
            result.counts.total_matches = Some(result.counts.observed_matches);
            result.counts.total_is_exact = true;
        }
        if !result.coverage.scope_exhaustive || next.is_some() {
            result.status = "partial".into();
        }
        if !result.coverage.eligible_scan_complete {
            result.has_more = None;
            result.next_cursor = None;
        }
        result.counts.returned_matches = result.matches.len();
        trim_metadata(result);
        if result.diagnostics_omitted > 0 {
            result.coverage.scope_exhaustive = false;
            result.status = "partial".into();
            result.truncation_reasons.push("diagnostics_omitted".into());
        }
        Ok(())
    }
    pub fn launch_directory(&self) -> &Path {
        &self.launch
    }
}
fn trim_metadata(result: &mut SearchEnvelope) {
    while result.wire_bytes() > result.limits.response_bytes && !result.diagnostics.is_empty() {
        result.diagnostics.pop();
        result.diagnostics_omitted += 1;
    }
    if result.wire_bytes() > result.limits.response_bytes {
        for reason in result.skipped.values_mut() {
            reason.examples_omitted += reason.examples.len() as u64;
            reason.examples.clear();
        }
    }
}
