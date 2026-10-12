//! Request-local, read-only evidence from one explicitly audited installation.
#[path = "sysroot_identity.rs"]
mod claims;
#[path = "sysroot_graph.rs"]
mod graph;
#[path = "sysroot_routes.rs"]
mod routes;
#[cfg(test)]
#[path = "sysroot_tests.rs"]
mod tests;
use super::semantic::{CfgInput, Configuration};
use crate::{
    items,
    result::{DomainError, Limits},
};
pub(crate) use graph::Profile;
use rmcp::schemars::JsonSchema;
pub(crate) use routes::predicate;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

pub(crate) const PREFIX: &str = "@sysroot/0/library/";
const FILE_CAP: usize = 8 * 1024 * 1024;
const BYTE_CAP: usize = 128 * 1024 * 1024;
const ENTRY_CAP: usize = 20_000;
const OUTPUT_CAP: usize = 64 * 1024;
pub(crate) type Controls<'a> = (Instant, &'a AtomicBool);
type Identity = (u64, u64);

#[derive(Debug, Clone, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct Coverage {
    pub complete: bool,
    pub assurance: String,
    pub compiler: String,
    pub compiler_executable: String,
    pub selection: String,
    pub analyzer: String,
    pub request_deadline_ms: u64,
    pub toolchain: String,
    pub source_root: String,
    pub component_release: String,
    pub identity_inputs: Vec<String>,
    pub freshness: String,
    pub stage: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub corpus_digest: String,
    pub consulted_roots: Vec<graph::Root>,
    pub configuration: Option<Configuration>,
    pub cfg_recipe: Vec<CfgFact>,
    pub graph: Vec<graph::Edge>,
    pub omissions: Vec<String>,
    pub capture_usage: Usage,
}
#[derive(Debug, Clone, Default, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct Usage {
    pub max_entries: usize,
    pub max_file_bytes: usize,
    pub max_source_bytes: usize,
    pub entries: usize,
    pub input_files: usize,
    pub input_bytes: usize,
    pub comparison_input_files: usize,
    pub comparison_input_bytes: usize,
    pub files: usize,
    pub bytes: usize,
    pub raw_only_files: usize,
    pub source_files_per_overlay: usize,
    pub source_bytes_per_overlay: usize,
    pub comparison_entries: usize,
    pub comparison_files: usize,
    pub comparison_bytes: usize,
    pub graph_edges: usize,
    pub max_graph_crates: usize,
    pub max_graph_edges: usize,
    pub max_route_edges: usize,
    pub route_edges: usize,
    pub max_route_depth: usize,
    pub max_output_bytes_per_stream: usize,
    pub identification_subprocesses: usize,
    pub identification_stdout_bytes: usize,
    pub identification_stderr_bytes: usize,
    pub comparison_subprocesses: usize,
    pub comparison_stdout_bytes: usize,
    pub comparison_stderr_bytes: usize,
    pub overlays_materialized: usize,
}
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct CfgFact {
    pub crate_name: String,
    pub key: String,
    pub atom_value: Option<String>,
    pub enabled: bool,
    pub evidence: String,
    pub consequence: String,
}
impl Coverage {
    fn pending(limits: &Limits) -> Self {
        Self {
            complete: false,
            assurance: "trusted_local_rustup_installation; no independent release authentication".into(),
            compiler: String::new(), compiler_executable: String::new(), selection: String::new(), analyzer: "ra_ap@0.0.357".into(), request_deadline_ms: limits.time_budget_ms,
            toolchain: String::new(), source_root: String::new(),
            freshness: "not_performed".into(), stage: "identification".into(),
            component_release: String::new(), identity_inputs: Vec::new(),
            corpus_digest: String::new(), consulted_roots: Vec::new(), configuration: None, cfg_recipe: Vec::new(), graph: Vec::new(),
            omissions: vec!["one audited stable 1.97.1 aarch64-apple-darwin source-analysis profile; not the binary build recipe, target layout, compilation or equivalence".into(),
                "complete raw installed library boundary only; ten non-UTF-8 inputs remain raw evidence and cannot supply Rust/include_str facts; source totals are per overlay, not heap/RSS".into(),
                "separate bounded sysroot quotas share the request deadline with caller/external work; no downloads, state writes, build scripts, procedural macros, generated members, trait/generic facts or custom-built standard libraries".into()],
            capture_usage: Usage { max_entries: limits.max_files.min(ENTRY_CAP), max_file_bytes: FILE_CAP, max_source_bytes: limits.max_source_bytes.min(BYTE_CAP), max_route_depth: 32, max_route_edges: ENTRY_CAP, max_graph_crates: 32, max_graph_edges: 1024, max_output_bytes_per_stream: OUTPUT_CAP, ..Usage::default() },
        }
    }
}
#[cfg(test)]
pub(crate) fn test_coverage() -> Coverage {
    Coverage::pending(&Limits::default())
}
pub(crate) fn refuse(code: &str, message: &str) -> DomainError {
    DomainError::new(code, message)
}
pub(super) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
fn unreadable(_: std::io::Error) -> DomainError {
    refuse(
        "SYSROOT_INPUT_UNREADABLE",
        "installed input is absent or unreadable",
    )
}
fn identity(metadata: &fs::Metadata) -> Identity {
    use std::os::unix::fs::MetadataExt;
    (metadata.dev(), metadata.ino())
}
fn no_links(path: &Path) -> Result<(), DomainError> {
    for part in path.ancestors() {
        if fs::symlink_metadata(part)
            .map_err(unreadable)?
            .file_type()
            .is_symlink()
        {
            return Err(refuse(
                "SYSROOT_INPUT_UNSAFE",
                "installed input has a symbolic-link ancestor",
            ));
        }
    }
    Ok(())
}
fn read(
    path: &Path,
    cap: usize,
    controls: Controls<'_>,
) -> Result<(Vec<u8>, Identity), DomainError> {
    items::check(controls.0, controls.1)?;
    no_links(path)?;
    let before = fs::symlink_metadata(path).map_err(unreadable)?;
    if !before.is_file() {
        return Err(refuse(
            "SYSROOT_INPUT_UNSAFE",
            "installed input is not regular",
        ));
    }
    if before.len() > cap as u64 {
        return Err(refuse(
            "SYSROOT_CAPTURE_EXHAUSTED",
            "installed per-file cap exhausted",
        ));
    }
    let mut file = fs::File::open(path).map_err(unreadable)?;
    if identity(&file.metadata().map_err(unreadable)?) != identity(&before) {
        return Err(refuse(
            "SYSROOT_SOURCE_CHANGED",
            "input identity changed before capture",
        ));
    }
    let mut bytes = Vec::new();
    let mut chunk = [0; 16 * 1024];
    loop {
        items::check(controls.0, controls.1)?;
        let count = file.read(&mut chunk).map_err(unreadable)?;
        if count == 0 {
            break;
        }
        if bytes.len() + count > cap {
            return Err(refuse(
                "SYSROOT_CAPTURE_EXHAUSTED",
                "installed per-file cap exhausted",
            ));
        }
        bytes.extend_from_slice(&chunk[..count]);
    }
    let after = fs::symlink_metadata(path).map_err(unreadable)?;
    if !after.is_file()
        || after.file_type().is_symlink()
        || identity(&before) != identity(&after)
        || before.len() != after.len()
        || before.modified().ok() != after.modified().ok()
        || bytes.len() as u64 != before.len()
    {
        return Err(refuse(
            "SYSROOT_SOURCE_CHANGED",
            "installed input changed during capture",
        ));
    }
    Ok((bytes, identity(&before)))
}
fn output(mut reader: impl Read, exceeded: &AtomicBool) -> Result<Vec<u8>, std::io::Error> {
    let mut bytes = Vec::new();
    let mut chunk = [0; 4096];
    loop {
        let count = reader.read(&mut chunk)?;
        if count == 0 {
            return Ok(bytes);
        }
        if bytes.len() + count > OUTPUT_CAP {
            exceeded.store(true, Ordering::Relaxed);
            return Ok(bytes);
        }
        bytes.extend_from_slice(&chunk[..count]);
    }
}
#[derive(Clone)]
struct Compiler {
    executable: PathBuf,
    rustup_home: PathBuf,
    selection: std::ffi::OsString,
}
impl Compiler {
    fn selected(repository: &Path) -> Result<Self, DomainError> {
        let executable = std::env::var_os("PATH")
            .and_then(|value| {
                std::env::split_paths(&value)
                    .filter(|directory| directory.is_absolute())
                    .map(|directory| directory.join("rustc"))
                    .find(|path| path.is_file() && !path.starts_with(repository))
            })
            .ok_or_else(|| {
                refuse(
                    "SYSROOT_TOOLCHAIN_UNAVAILABLE",
                    "no server rustc on absolute PATH outside caller",
                )
            })?;
        let rustup_home = std::env::var_os("RUSTUP_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|value| PathBuf::from(value).join(".rustup")))
            .ok_or_else(|| {
                refuse(
                    "SYSROOT_TOOLCHAIN_UNAVAILABLE",
                    "server rustup installation unavailable",
                )
            })?;
        Ok(Self {
            executable,
            rustup_home,
            selection: std::env::var_os("RUSTUP_TOOLCHAIN").unwrap_or_else(|| "stable".into()),
        })
    }
    fn identify(
        &self,
        args: &[&str],
        controls: Controls<'_>,
        usage: &mut Usage,
        comparison: bool,
    ) -> Result<String, DomainError> {
        items::check(controls.0, controls.1)?;
        if comparison {
            usage.comparison_subprocesses += 1;
        } else {
            usage.identification_subprocesses += 1;
        }
        let mut command = Command::new(&self.executable);
        command
            .args(args)
            .current_dir("/")
            .env_clear()
            .env("RUSTUP_HOME", &self.rustup_home)
            .env("RUSTUP_AUTO_INSTALL", "0")
            .env("RUSTUP_TOOLCHAIN", &self.selection)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        // Identification only: expose the selected compiler's otherwise hidden
        // internal cfg atoms without compiling or executing any source.
        if args == ["-Z", "unstable-options", "--print", "cfg"] {
            command.env("RUSTC_BOOTSTRAP", "1");
        }
        let mut child = command.spawn().map_err(|_| {
            refuse(
                "SYSROOT_TOOLCHAIN_UNAVAILABLE",
                "cannot identify installed compiler; no installation attempted",
            )
        })?;
        let stdout = child.stdout.take().expect("piped stdout");
        let stderr = child.stderr.take().expect("piped stderr");
        let exceeded = AtomicBool::new(false);
        std::thread::scope(|threads| {
            let out = threads.spawn(|| output(stdout, &exceeded));
            let err = threads.spawn(|| output(stderr, &exceeded));
            let status = loop {
                if let Err(error) = items::check(controls.0, controls.1) {
                    let _ = child.kill();
                    let _ = child.wait();
                    break Err(error);
                }
                if exceeded.load(Ordering::Relaxed) {
                    let _ = child.kill();
                    let _ = child.wait();
                    break Err(refuse(
                        "SYSROOT_CAPTURE_EXHAUSTED",
                        "compiler output cap exhausted",
                    ));
                }
                match child.try_wait() {
                    Ok(Some(status)) => break Ok(status),
                    Ok(None) => std::thread::sleep(Duration::from_millis(5)),
                    Err(_) => {
                        let _ = child.kill();
                        let _ = child.wait();
                        break Err(refuse(
                            "SYSROOT_TOOLCHAIN_UNAVAILABLE",
                            "cannot reap identification subprocess",
                        ));
                    }
                }
            };
            let stdout = out
                .join()
                .expect("compiler stdout reader")
                .map_err(unreadable);
            let stderr = err
                .join()
                .expect("compiler stderr reader")
                .map_err(unreadable);
            if comparison {
                usage.comparison_stdout_bytes += stdout.as_ref().map_or(0, Vec::len);
                usage.comparison_stderr_bytes += stderr.as_ref().map_or(0, Vec::len);
            } else {
                usage.identification_stdout_bytes += stdout.as_ref().map_or(0, Vec::len);
                usage.identification_stderr_bytes += stderr.as_ref().map_or(0, Vec::len);
            }
            let status = status?;
            let bytes = stdout?;
            let _ = stderr?;
            if exceeded.load(Ordering::Relaxed) {
                return Err(refuse(
                    "SYSROOT_CAPTURE_EXHAUSTED",
                    "compiler output cap exhausted",
                ));
            }
            if !status.success() {
                return Err(refuse(
                    "SYSROOT_TOOLCHAIN_UNAVAILABLE",
                    "selected installed compiler unavailable; no replacement or installation attempted",
                ));
            }
            String::from_utf8(bytes).map_err(|_| {
                refuse(
                    "SYSROOT_IDENTITY_UNSUPPORTED",
                    "compiler identification is not UTF-8",
                )
            })
        })
    }
}
struct Observed {
    files: BTreeMap<PathBuf, (Vec<u8>, Identity)>,
    directories: BTreeMap<PathBuf, Identity>,
}
fn capture(
    root: &Path,
    usage: &mut Usage,
    controls: Controls<'_>,
    comparison: bool,
) -> Result<Observed, DomainError> {
    let mut observed = Observed {
        files: BTreeMap::new(),
        directories: BTreeMap::new(),
    };
    let mut pending = vec![(root.to_path_buf(), 0)];
    let mut entries = 0;
    let mut files = 0;
    let mut total = 0;
    while let Some((directory, depth)) = pending.pop() {
        items::check(controls.0, controls.1)?;
        if depth > 32 {
            return Err(refuse(
                "SYSROOT_CAPTURE_EXHAUSTED",
                "library enumeration depth cap exhausted",
            ));
        }
        no_links(&directory)?;
        let metadata = fs::symlink_metadata(&directory).map_err(unreadable)?;
        if !metadata.is_dir() {
            return Err(refuse(
                "SYSROOT_INPUT_UNSAFE",
                "library directory is not a directory",
            ));
        }
        observed
            .directories
            .insert(directory.clone(), identity(&metadata));
        let mut paths = Vec::new();
        for entry in fs::read_dir(&directory).map_err(unreadable)? {
            items::check(controls.0, controls.1)?;
            entries += 1;
            if comparison {
                usage.comparison_entries = entries;
            } else {
                usage.entries = entries;
            }
            if entries + usage.input_files > usage.max_entries {
                return Err(refuse(
                    "SYSROOT_CAPTURE_EXHAUSTED",
                    "library directory-entry cap exhausted",
                ));
            }
            paths.push(entry.map_err(unreadable)?.path());
        }
        paths.sort();
        for path in paths {
            items::check(controls.0, controls.1)?;
            let metadata = fs::symlink_metadata(&path).map_err(unreadable)?;
            if metadata.file_type().is_symlink() {
                return Err(refuse(
                    "SYSROOT_INPUT_UNSAFE",
                    "library contains a symbolic link",
                ));
            }
            if metadata.is_dir() {
                pending.push((path, depth + 1));
                continue;
            }
            let value = read(&path, usage.max_file_bytes, controls)?;
            files += 1;
            total += value.0.len();
            if comparison {
                usage.comparison_files = files;
                usage.comparison_bytes = total;
            } else {
                usage.files = files;
                usage.bytes = total;
            }
            if files + usage.input_files > usage.max_entries
                || total + usage.input_bytes > usage.max_source_bytes
            {
                return Err(refuse(
                    "SYSROOT_CAPTURE_EXHAUSTED",
                    "library file/aggregate raw-byte cap exhausted",
                ));
            }
            observed.files.insert(path, value);
        }
        if identity(&fs::symlink_metadata(&directory).map_err(unreadable)?) != identity(&metadata) {
            return Err(refuse(
                "SYSROOT_SOURCE_CHANGED",
                "library directory identity changed during enumeration",
            ));
        }
    }
    Ok(observed)
}
pub(crate) struct Corpus {
    pub texts: BTreeMap<String, String>,
    pub configuration: Configuration,
    pub coverage: Coverage,
    pub profile: graph::Profile,
    observed: Observed,
    compiler: Compiler,
    toolchain: PathBuf,
    toolchain_identity: Identity,
    compiler_cfg: String,
    claims: claims::Claims,
}
pub(crate) fn discover(
    repository: &Path,
    configuration: &Configuration,
    limits: &Limits,
    controls: Controls<'_>,
    disclosure: &mut Option<Coverage>,
) -> Result<Corpus, DomainError> {
    let coverage = disclosure.insert(Coverage::pending(limits));
    let outcome = (|| {
        let compiler = Compiler::selected(repository)?;
        coverage.compiler_executable = compiler.executable.to_string_lossy().into();
        coverage.selection = compiler.selection.to_string_lossy().into();
        coverage.compiler =
            compiler.identify(&["-vV"], controls, &mut coverage.capture_usage, false)?;
        let version: BTreeMap<_, _> = coverage
            .compiler
            .lines()
            .filter_map(|line| line.split_once(": "))
            .collect();
        if version.get("release") != Some(&"1.97.1")
            || version.get("commit-hash") != Some(&"8bab26f4f68e0e26f0bb7960be334d5b520ea452")
            || version.get("host") != Some(&"aarch64-apple-darwin")
        {
            return Err(refuse(
                "SYSROOT_IDENTITY_UNSUPPORTED",
                "only audited stable 1.97.1 aarch64-apple-darwin compiler profile supported",
            ));
        }
        let printed = compiler.identify(
            &["--print", "sysroot"],
            controls,
            &mut coverage.capture_usage,
            false,
        )?;
        if printed.lines().count() != 1 || !Path::new(printed.trim()).is_absolute() {
            return Err(refuse(
                "SYSROOT_IDENTITY_AMBIGUOUS",
                "compiler must identify one absolute sysroot",
            ));
        }
        let toolchain = PathBuf::from(printed.trim());
        no_links(&toolchain)?;
        let installation = compiler.rustup_home.join("toolchains");
        no_links(&installation)?;
        if toolchain.parent() != Some(installation.as_path()) {
            return Err(refuse(
                "SYSROOT_LAYOUT_UNSUPPORTED",
                "only conventional installed rustup toolchains supported",
            ));
        }
        let toolchain_identity = identity(&fs::symlink_metadata(&toolchain).map_err(unreadable)?);
        coverage.toolchain = toolchain.to_string_lossy().into();
        let library = toolchain.join("lib/rustlib/src/rust/library");
        coverage.source_root = library.to_string_lossy().into();
        if !library.exists() {
            return Err(refuse(
                "SYSROOT_SOURCES_UNAVAILABLE",
                "selected installation has no rust-src; install it outside the server",
            ));
        }
        no_links(&library)?;
        if library.starts_with(repository) || repository.starts_with(&library) {
            return Err(refuse(
                "SYSROOT_STATE_OVERLAP",
                "caller repository overlaps sysroot evidence",
            ));
        }
        let compiler_cfg = compiler.identify(
            &["-Z", "unstable-options", "--print", "cfg"],
            controls,
            &mut coverage.capture_usage,
            false,
        )?;
        let mut cfg = Vec::new();
        for line in compiler_cfg.lines() {
            if let Some((key, value)) = line.split_once('=') {
                let value: String = serde_json::from_str(value).map_err(|_| {
                    refuse(
                        "SYSROOT_CONFIGURATION_UNSUPPORTED",
                        "invalid compiler cfg identification",
                    )
                })?;
                cfg.push(CfgInput {
                    key: key.into(),
                    value: Some(value),
                });
            } else {
                cfg.push(CfgInput {
                    key: line.into(),
                    value: None,
                });
            }
        }
        coverage.stage = "component_identity".into();
        let claims = claims::capture(&toolchain, coverage, controls, false)?;
        coverage.stage = "raw_capture".into();
        let observed = capture(&library, &mut coverage.capture_usage, controls, false)?;
        claims::membership(&toolchain, &claims, &observed)?;
        let mut texts = BTreeMap::new();
        for (path, (raw, _)) in &observed.files {
            items::check(controls.0, controls.1)?;
            let relative = path
                .strip_prefix(&library)
                .expect("captured child")
                .to_str()
                .ok_or_else(|| refuse("SYSROOT_LAYOUT_UNSUPPORTED", "library path is not UTF-8"))?;
            match std::str::from_utf8(raw) {
                Ok(text) => {
                    texts.insert(format!("{PREFIX}{relative}"), text.to_owned());
                }
                Err(_) if path.extension().is_some_and(|ext| ext == "rs") => {
                    return Err(refuse(
                        "SYSROOT_LAYOUT_UNSUPPORTED",
                        "Rust source is not UTF-8",
                    ));
                }
                Err(_) => coverage.capture_usage.raw_only_files += 1,
            }
        }
        coverage.capture_usage.source_files_per_overlay = texts.len();
        coverage.capture_usage.source_bytes_per_overlay = texts.values().map(String::len).sum();
        coverage.stage = "graph_and_routes".into();
        let mut profile = graph::build(
            &library,
            &observed.files,
            &texts,
            configuration,
            &cfg,
            coverage,
        )?;
        routes::audit(&texts, &mut profile, coverage, controls)?;
        profile.validate_recipe(coverage)?;
        let configuration = profile.configuration.clone();
        coverage.configuration = Some(configuration.clone());
        let mut hash = Sha256::new();
        hash.update(coverage.compiler.as_bytes());
        hash.update(compiler_cfg.as_bytes());
        hash.update(
            serde_json::to_vec(&(
                &configuration,
                &coverage.cfg_recipe,
                &coverage.consulted_roots,
                &coverage.graph,
            ))
            .expect("serializable profile"),
        );
        hash.update(toolchain_identity.0.to_le_bytes());
        hash.update(toolchain_identity.1.to_le_bytes());
        for (path, (bytes, id)) in &claims {
            hash.update(path.to_string_lossy().as_bytes());
            hash.update((bytes.len() as u64).to_le_bytes());
            hash.update(bytes);
            hash.update(id.0.to_le_bytes());
            hash.update(id.1.to_le_bytes());
        }
        for (path, (bytes, id)) in &observed.files {
            items::check(controls.0, controls.1)?;
            let name = path
                .strip_prefix(&library)
                .expect("captured child")
                .to_str()
                .expect("checked UTF-8 path");
            hash.update((name.len() as u64).to_le_bytes());
            hash.update(name.as_bytes());
            hash.update((bytes.len() as u64).to_le_bytes());
            hash.update(bytes);
            hash.update(id.0.to_le_bytes());
            hash.update(id.1.to_le_bytes());
        }
        coverage.corpus_digest = format!("sha256:{}", hex(&hash.finalize()));
        coverage.complete = true;
        coverage.stage = "captured".into();
        Ok(Corpus {
            texts,
            configuration,
            profile,
            coverage: coverage.clone(),
            observed,
            compiler,
            toolchain,
            toolchain_identity,
            compiler_cfg,
            claims,
        })
    })();
    if outcome.is_err() {
        coverage.complete = false;
        coverage.corpus_digest.clear();
        coverage.omissions.push(
            "discovery/capture refused; no usable prefix corpus or resolution proof published"
                .into(),
        );
    }
    outcome
}
impl Corpus {
    pub(crate) fn recheck(
        &self,
        coverage: &mut Coverage,
        controls: Controls<'_>,
    ) -> Result<(), DomainError> {
        coverage.stage = "publication_recheck".into();
        let result = (|| {
            no_links(&self.toolchain)?;
            if identity(&fs::symlink_metadata(&self.toolchain).map_err(unreadable)?)
                != self.toolchain_identity
                || self
                    .compiler
                    .identify(&["-vV"], controls, &mut coverage.capture_usage, true)?
                    != self.coverage.compiler
                || self
                    .compiler
                    .identify(
                        &["--print", "sysroot"],
                        controls,
                        &mut coverage.capture_usage,
                        true,
                    )?
                    .trim()
                    != self.toolchain.to_str().expect("UTF-8 toolchain")
                || self.compiler.identify(
                    &["-Z", "unstable-options", "--print", "cfg"],
                    controls,
                    &mut coverage.capture_usage,
                    true,
                )? != self.compiler_cfg
            {
                return Err(refuse(
                    "SYSROOT_SOURCE_CHANGED",
                    "compiler/toolchain identity or cfg changed",
                ));
            }
            let claims = claims::capture(&self.toolchain, coverage, controls, true)?;
            if claims != self.claims {
                return Err(refuse(
                    "SYSROOT_SOURCE_CHANGED",
                    "installed component claims changed before publication",
                ));
            }
            let observed = capture(
                Path::new(&coverage.source_root),
                &mut coverage.capture_usage,
                controls,
                true,
            )?;
            if observed.directories != self.observed.directories
                || observed.files != self.observed.files
            {
                return Err(refuse(
                    "SYSROOT_SOURCE_CHANGED",
                    "source bytes, membership or identities changed before publication",
                ));
            }
            items::check(controls.0, controls.1)?;
            Ok(())
        })();
        coverage.freshness = if result.is_ok() {
            "observationally_unchanged"
        } else {
            "failed_or_interrupted"
        }
        .into();
        if result.is_ok() {
            coverage.stage = "publication_checked".into();
        }
        result
    }
}
