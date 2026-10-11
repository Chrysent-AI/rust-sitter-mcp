//! Durable, greppable server-owned advice artifacts, independent of the live handle cache.
use super::*;
use std::{
    fs,
    io::BufWriter,
    path::{Path, PathBuf},
    sync::atomic::AtomicU64,
};

static SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct Spill {
    pub state: String,
    pub directory: Option<String>,
    pub index: Option<String>,
    pub files: BTreeMap<String, String>,
    pub reason: Option<String>,
}

fn invalid(message: &str) -> io::Error {
    io::Error::other(message)
}

fn resolved(path: &Path) -> io::Result<PathBuf> {
    if !path.is_absolute()
        || path
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(invalid(
            "data directory must be absolute without parent components",
        ));
    }
    if path.exists() {
        return path.canonicalize();
    }
    let parent = path
        .parent()
        .ok_or_else(|| invalid("invalid data directory"))?;
    Ok(resolved(parent)?.join(
        path.file_name()
            .ok_or_else(|| invalid("invalid data directory"))?,
    ))
}

fn data_tree() -> io::Result<PathBuf> {
    let base = match std::env::var_os("XDG_DATA_HOME") {
        Some(value) => PathBuf::from(value),
        None => PathBuf::from(std::env::var_os("HOME").ok_or_else(|| invalid("HOME is unset"))?)
            .join(".local/share"),
    };
    let base = resolved(&base)?;
    let server = base.join("rust-sitter-mcp");
    let tree = server.join("analyses");
    for path in [&server, &tree] {
        if fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
            return Err(invalid("server state subtree must not be a symbolic link"));
        }
    }
    Ok(tree)
}

struct CheckedWriter<'a> {
    inner: BufWriter<fs::File>,
    controls: Controls<'a>,
}
impl Write for CheckedWriter<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.controls
            .check()
            .map_err(|_| invalid("spill cancelled"))?;
        self.inner.write(bytes)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

fn write_json<T: Serialize>(path: &Path, value: &T, controls: Controls<'_>) -> io::Result<()> {
    controls.check().map_err(|_| invalid("spill cancelled"))?;
    let mut writer = CheckedWriter {
        inner: BufWriter::new(fs::File::create(path)?),
        controls,
    };
    serde_json::to_writer_pretty(&mut writer, value)?;
    writer.write_all(b"\n")?;
    writer.flush()?;
    writer.inner.get_ref().sync_all()
}

pub(super) fn persist_at(
    tree: &Path,
    full: &SuggestSplitEnvelope,
    evidence: Option<&Evidence>,
    request: &SuggestSplitRequest,
    controls: Controls<'_>,
) -> io::Result<Spill> {
    let tree = resolved(tree)?;
    if tree.to_str().is_none() {
        return Err(invalid(
            "spill directory must be valid UTF-8 for resolvable JSON pointers",
        ));
    }
    if let Some(root) = &full.root {
        let root = Path::new(root).canonicalize()?;
        if tree.starts_with(root) {
            return Err(invalid("spill directory is inside caller repository"));
        }
    } else {
        return Err(invalid("caller repository root unavailable"));
    }
    controls.check().map_err(|_| invalid("spill cancelled"))?;
    fs::create_dir_all(&tree)?;
    let id = format!(
        "analysis-{}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    );
    let destination = tree.join(&id);
    let temporary = tree.join(format!(".{id}.tmp"));
    let result = (|| {
        // Every post-creation failure path is protected by the cleanup below,
        // including permission updates that precede any file write.
        fs::create_dir(&temporary)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&temporary, fs::Permissions::from_mode(0o700))?;
        }
        let mut files = BTreeMap::new();
        macro_rules! topics {
            ($($field:ident),* $(,)?) => { $(
                let name = concat!(stringify!($field), ".json");
                write_json(&temporary.join(name), &full.$field, controls)?;
                files.insert(stringify!($field).to_owned(), destination.join(name).to_string_lossy().into_owned());
            )* };
        }
        // Every canonical envelope field is preserved before wire fitting, without a second Value allocation.
        topics!(
            schema_version,
            tool,
            advisory,
            root,
            snapshot_id,
            status,
            coverage,
            counts,
            limits,
            effective_work_limits,
            truncation_reasons,
            skipped,
            diagnostics,
            diagnostics_omitted,
            chain_diagnostics,
            error,
            source,
            scope_trivia,
            inventory,
            overlaps,
            item_contexts,
            impl_contexts,
            signals,
            decisions,
            advice_decisions,
            decision_groups,
            drafts,
            inspection_partitions,
            ownership_candidates,
            boundary_observations,
            test_observations,
            partition_outcome,
            extraction_payoff,
            draft_summaries,
            draft_eligibility,
            retention,
            integrity
        );
        write_json(&temporary.join("request.json"), request, controls)?;
        files.insert(
            "request".into(),
            destination
                .join("request.json")
                .to_string_lossy()
                .into_owned(),
        );
        if let Some(evidence) = evidence {
            write_json(
                &temporary.join("source_buffers.json"),
                &evidence.buffers,
                controls,
            )?;
            write_json(
                &temporary.join("scope_evidence.json"),
                &json!({"scope_input_digest":evidence.scope_input_digest,"normalized_scope":evidence.normalized_scope,"source_manifest":evidence.source_manifest,"inputs":evidence.inputs}),
                controls,
            )?;
            for name in ["source_buffers", "scope_evidence"] {
                files.insert(
                    name.into(),
                    destination
                        .join(format!("{name}.json"))
                        .to_string_lossy()
                        .into_owned(),
                );
            }
        }
        let spill = Spill {
            state: "persisted".into(),
            directory: Some(destination.to_string_lossy().into_owned()),
            index: Some(
                destination
                    .join("index.json")
                    .to_string_lossy()
                    .into_owned(),
            ),
            files,
            reason: None,
        };
        write_json(
            &temporary.join("index.json"),
            &json!({"schema_version":1,"snapshot_id":full.snapshot_id,"analysis_directory":id,"files":spill.files,"omissions":full.counts.omissions,"evidence_captured":evidence.is_some(),"semantic":"not_performed"}),
            controls,
        )?;
        controls.check().map_err(|_| invalid("spill cancelled"))?;
        fs::rename(&temporary, &destination)?;
        Ok(spill)
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&temporary);
    }
    result
}

pub(super) fn persist(
    full: &SuggestSplitEnvelope,
    evidence: Option<&Evidence>,
    request: &SuggestSplitRequest,
    controls: Controls<'_>,
) -> Spill {
    match data_tree().and_then(|tree| persist_at(&tree, full, evidence, request, controls)) {
        Ok(spill) => spill,
        Err(error) => Spill {
            state: "unavailable".into(),
            directory: None,
            index: None,
            files: BTreeMap::new(),
            reason: Some(error.to_string()),
        },
    }
}

/// Remove only the server's reserved analysis subtree. Live handles are unaffected.
pub fn flush() -> io::Result<String> {
    let tree = data_tree()?;
    flush_at(&tree)
}
fn flush_at(tree: &Path) -> io::Result<String> {
    if !tree.exists() {
        return Ok(format!("No spilled analyses at {}", tree.display()));
    }
    let entries = fs::read_dir(tree)?.count();
    fs::remove_dir_all(tree)?;
    Ok(format!(
        "Removed {entries} analysis entries from {}",
        tree.display()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Scratch(PathBuf);
    impl Scratch {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "rust-sitter-spill-test-{}-{}",
                std::process::id(),
                SEQUENCE.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
        fn path(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn request() -> SuggestSplitRequest {
        serde_json::from_value(
            json!({"repo_path":"/unused", "crate_root":"src/lib.rs", "source_path":"src/lib.rs"}),
        )
        .unwrap()
    }
    #[test]
    fn persisted_topics_resolve_and_flush_is_idempotent() {
        let scratch = Scratch::new();
        let root = Scratch::new();
        let tree = scratch.path().join("analyses");
        let mut full = SuggestSplitEnvelope::empty(Limits::default());
        full.root = Some(root.path().to_string_lossy().into_owned());
        let flag = AtomicBool::new(false);
        let controls = Controls {
            deadline: Instant::now() + Duration::from_secs(30),
            cancelled: &flag,
        };
        let request = request();
        let spill = persist_at(&tree, &full, None, &request, controls).unwrap();
        for path in spill.files.values() {
            assert!(Path::new(path).is_file());
        }
        let value: Value = serde_json::from_slice(
            &fs::read(spill.files.get("test_observations").unwrap()).unwrap(),
        )
        .unwrap();
        assert_eq!(
            value,
            serde_json::to_value(&full.test_observations).unwrap()
        );
        assert!(Path::new(spill.index.as_ref().unwrap()).is_file());
        assert!(flush_at(&tree).unwrap().starts_with("Removed 1"));
        assert!(flush_at(&tree).unwrap().starts_with("No spilled"));
        assert!(root.path().read_dir().unwrap().next().is_none());
    }
    #[test]
    fn cancellation_and_caller_owned_paths_never_publish() {
        let root = Scratch::new();
        let mut full = SuggestSplitEnvelope::empty(Limits::default());
        full.root = Some(root.path().to_string_lossy().into_owned());
        let flag = AtomicBool::new(false);
        let controls = Controls {
            deadline: Instant::now() + Duration::from_secs(30),
            cancelled: &flag,
        };
        let request = request();
        assert!(
            persist_at(
                &root.path().join("analyses"),
                &full,
                None,
                &request,
                controls
            )
            .is_err()
        );
        let scratch = Scratch::new();
        flag.store(true, Ordering::Relaxed);
        assert!(
            persist_at(
                &scratch.path().join("analyses"),
                &full,
                None,
                &request,
                controls
            )
            .is_err()
        );
        assert!(scratch.path().read_dir().unwrap().next().is_none());
    }
}
