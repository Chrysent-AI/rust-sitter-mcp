//! Request-local, read-only library evidence. Never part of repository edit admission.
#[cfg(test)]
#[path = "external_registry_tests.rs"]
mod registry_tests;
#[path = "external_sources.rs"]
mod sources;
#[cfg(test)]
#[path = "external_tests.rs"]
mod tests;
use super::semantic::{Configuration, CrateInput, DependencyInput};
use crate::{
    items,
    result::{DomainError, Limits},
    scope,
};
use rmcp::schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

pub(crate) const PREFIX: &str = "@external/";
pub(crate) fn disabled(value: &bool) -> bool {
    !*value
}
const OUTPUT_CAP: usize = 8 * 1024 * 1024;
// Ancestor searches and candidate-edge walks each stop at 32 hops. All captured
// manifests/locks share the request's file and aggregate-byte preflight budget.
const ANCESTOR_CAP: usize = 32;
static SEQUENCE: AtomicU64 = AtomicU64::new(0);
type Controls<'a> = (Instant, &'a AtomicBool);

#[derive(Debug, Clone, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct ConsultedRoot {
    pub package_id: String,
    pub package: String,
    pub version: String,
    pub source: String,
    pub provenance: String,
    pub root: String,
    pub namespace: String,
    pub features: Vec<String>,
    pub edition: String,
    pub files: usize,
    pub bytes: usize,
}
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct Coverage {
    pub complete: bool,
    pub consulted_roots: Vec<ConsultedRoot>,
    pub corpus_digest: String,
    pub omissions: Vec<String>,
}
pub(crate) struct Corpus {
    pub texts: BTreeMap<String, String>,
    pub configuration: Configuration,
    pub coverage: Coverage,
    observed: BTreeMap<PathBuf, Vec<u8>>,
    roots: Vec<PathBuf>,
    identities: BTreeMap<PathBuf, (u64, u64)>,
    limits: Limits,
    repository: PathBuf,
    crate_root: String,
    caller_configuration: Configuration,
    cargo_home: PathBuf,
    discovery_tree: PathBuf,
}
fn refusal(code: &str, message: &str) -> DomainError {
    DomainError::new(code, message)
}
fn io_error(_: std::io::Error) -> DomainError {
    refusal(
        "EXTERNAL_DEPENDENCY_UNREADABLE",
        "dependency discovery/capture input is absent or unreadable",
    )
}

// Resolve existing ancestors before creating state; do not follow server-subtree links.
fn resolved(path: &Path) -> Result<PathBuf, DomainError> {
    if !path.is_absolute()
        || path
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(refusal(
            "EXTERNAL_STATE_UNAVAILABLE",
            "server state must be absolute without parent components",
        ));
    }
    if path.exists() {
        return path.canonicalize().map_err(io_error);
    }
    Ok(resolved(
        path.parent()
            .ok_or_else(|| refusal("EXTERNAL_STATE_UNAVAILABLE", "invalid state directory"))?,
    )?
    .join(
        path.file_name()
            .ok_or_else(|| refusal("EXTERNAL_STATE_UNAVAILABLE", "invalid state directory"))?,
    ))
}
pub(crate) fn disjoint(
    tree: &Path,
    roots: impl IntoIterator<Item = PathBuf>,
) -> Result<(), DomainError> {
    let tree = resolved(tree)?;
    for root in roots {
        let root = root.canonicalize().map_err(io_error)?;
        if tree.starts_with(&root) || root.starts_with(&tree) {
            return Err(refusal(
                "EXTERNAL_STATE_OVERLAP",
                "server state overlaps caller repository or consulted dependency root",
            ));
        }
    }
    Ok(())
}
fn state_tree() -> Result<PathBuf, DomainError> {
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".local/share")))
        .ok_or_else(|| refusal("EXTERNAL_STATE_UNAVAILABLE", "server data home unavailable"))?;
    let base = resolved(&base)?;
    let server = base.join("rust-sitter-mcp");
    let tree = server.join("discovery");
    for path in [&server, &tree] {
        if fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink()) {
            return Err(refusal(
                "EXTERNAL_STATE_UNAVAILABLE",
                "server state subtree must not be a symbolic link",
            ));
        }
    }
    Ok(tree)
}
struct Scratch(PathBuf);
impl Scratch {
    fn new(tree: &Path) -> Result<Self, DomainError> {
        fs::create_dir_all(tree).map_err(|_| {
            refusal(
                "EXTERNAL_STATE_UNAVAILABLE",
                "cannot create server discovery state",
            )
        })?;
        let path = tree.join(format!(
            "request-{}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).map_err(|_| {
            refusal(
                "EXTERNAL_STATE_UNAVAILABLE",
                "cannot create request Cargo home",
            )
        })?;
        let scratch = Self(path);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&scratch.0, fs::Permissions::from_mode(0o700)).map_err(io_error)?;
        }
        Ok(scratch)
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[derive(Deserialize)]
struct Metadata {
    packages: Vec<Package>,
    resolve: Option<Resolve>,
    workspace_root: PathBuf,
}
#[derive(Deserialize)]
struct Package {
    id: String,
    name: String,
    version: String,
    source: Option<String>,
    manifest_path: PathBuf,
    targets: Vec<Target>,
    dependencies: Vec<DeclaredDependency>,
}
#[derive(Deserialize)]
struct Target {
    name: String,
    kind: Vec<String>,
    src_path: PathBuf,
    edition: String,
}
#[derive(Deserialize)]
struct DeclaredDependency {
    name: String,
    rename: Option<String>,
    path: Option<PathBuf>,
    kind: Option<String>,
    source: Option<String>,
}
#[derive(Deserialize)]
struct Resolve {
    nodes: Vec<Node>,
}
#[derive(Deserialize)]
struct Node {
    id: String,
    deps: Vec<Edge>,
    features: Vec<String>,
}
#[derive(Deserialize)]
struct Edge {
    name: String,
    pkg: String,
    dep_kinds: Vec<EdgeKind>,
}
#[derive(Deserialize)]
struct EdgeKind {
    kind: Option<String>,
    target: Option<String>,
}

fn config_guard(directory: &Path) -> Result<(), DomainError> {
    // Cargo reads ancestor configuration even with an empty home. Refuse it instead
    // of allowing wrappers, source replacements or environment injection.
    for ancestor in directory.ancestors() {
        for name in [".cargo/config", ".cargo/config.toml"] {
            match fs::symlink_metadata(ancestor.join(name)) {
                Ok(_) => {
                    return Err(refusal(
                        "EXTERNAL_CONFIGURATION_UNSUPPORTED",
                        "Cargo ancestor configuration is not admitted for path discovery",
                    ));
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(io_error(e)),
            }
        }
    }
    Ok(())
}
fn bounded_output(mut reader: impl Read, exceeded: &AtomicBool) -> Result<Vec<u8>, std::io::Error> {
    let mut bytes = Vec::new();
    let mut chunk = [0; 16 * 1024];
    loop {
        let n = reader.read(&mut chunk)?;
        if n == 0 {
            return Ok(bytes);
        }
        if bytes.len() + n > OUTPUT_CAP {
            exceeded.store(true, Ordering::Relaxed);
            return Ok(bytes);
        }
        bytes.extend_from_slice(&chunk[..n]);
    }
}
#[cfg(test)]
fn metadata(
    directory: &Path,
    tree: &Path,
    controls: Controls<'_>,
) -> Result<Metadata, DomainError> {
    metadata_with_sources(directory, tree, None, &Limits::default(), controls)
}
fn metadata_with_sources(
    directory: &Path,
    tree: &Path,
    sources: Option<&mut sources::Sources>,
    limits: &Limits,
    controls: Controls<'_>,
) -> Result<Metadata, DomainError> {
    items::check(controls.0, controls.1)?;
    if sources.is_none() {
        config_guard(directory)?;
    }
    let scratch = Scratch::new(tree)?;
    if let Some(sources) = sources.as_deref() {
        sources.stage(&scratch.0, controls)?;
    }
    // Keep only server tool lookup and rustup selection, not caller Cargo/wrapper env.
    // Force a server-selected installed toolchain, overriding caller rust-toolchain files.
    let cargo = std::env::var_os("PATH")
        .and_then(|path| {
            std::env::split_paths(&path)
                .filter(|directory| directory.is_absolute())
                .map(|directory| directory.join("cargo"))
                .find(|path| path.is_file() && !path.starts_with(directory))
        })
        .ok_or_else(|| {
            refusal(
                "EXTERNAL_DISCOVERY_FAILED",
                "no server-owned Cargo executable on absolute PATH",
            )
        })?;
    let mut command = Command::new(cargo);
    command
        .args(["metadata", "--format-version", "1", "--offline", "--locked"])
        .current_dir(directory)
        .env_clear()
        .env("CARGO_HOME", &scratch.0)
        .env("HOME", &scratch.0)
        .env("RUSTUP_AUTO_INSTALL", "0")
        .env(
            "RUSTUP_TOOLCHAIN",
            std::env::var_os("RUSTUP_TOOLCHAIN").unwrap_or_else(|| "stable".into()),
        )
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for key in ["PATH", "RUSTUP_HOME"] {
        if let Some(value) = std::env::var_os(key) {
            command.env(key, value);
        }
    }
    if std::env::var_os("RUSTUP_HOME").is_none()
        && let Some(home) = std::env::var_os("HOME")
    {
        command.env("RUSTUP_HOME", PathBuf::from(home).join(".rustup"));
    }
    let mut child = command.spawn().map_err(|_| {
        refusal(
            "EXTERNAL_DISCOVERY_FAILED",
            "cannot start offline locked Cargo metadata",
        )
    })?;
    let stdout = child.stdout.take().expect("piped stdout");
    let stderr = child.stderr.take().expect("piped stderr");
    let exceeded = AtomicBool::new(false);
    let output = std::thread::scope(|threads| {
        let out = threads.spawn(|| bounded_output(stdout, &exceeded));
        let err = threads.spawn(|| bounded_output(stderr, &exceeded));
        let status = loop {
            if let Err(error) = items::check(controls.0, controls.1) {
                let _ = child.kill();
                let _ = child.wait();
                break Err(error);
            }
            if exceeded.load(Ordering::Relaxed) {
                let _ = child.kill();
                let _ = child.wait();
                break Err(refusal(
                    "EXTERNAL_CORPUS_INCOMPLETE",
                    "Cargo metadata output exceeded cap",
                ));
            }
            match child.try_wait() {
                Ok(Some(status)) => break Ok(status),
                Ok(None) => std::thread::sleep(Duration::from_millis(5)),
                Err(_) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    break Err(refusal(
                        "EXTERNAL_DISCOVERY_FAILED",
                        "cannot reap metadata subprocess",
                    ));
                }
            }
        };
        let out = out
            .join()
            .expect("metadata stdout reader")
            .map_err(io_error);
        let err = err
            .join()
            .expect("metadata stderr reader")
            .map_err(io_error);
        status.and_then(|status| { let bytes = out?; let _ = err?;
            if exceeded.load(Ordering::Relaxed) { return Err(refusal("EXTERNAL_CORPUS_INCOMPLETE", "Cargo metadata output exceeded cap")); }
            if !status.success() { return Err(refusal("EXTERNAL_DISCOVERY_FAILED", "offline locked Cargo discovery failed; required local package/index, configuration or lock is unavailable or inconsistent; no downloads permitted")); }
            serde_json::from_slice(&bytes).map_err(|_| refusal("EXTERNAL_DISCOVERY_FAILED", "invalid Cargo metadata JSON"))
        })
    });
    let mut output = output;
    if let (Ok(metadata), Some(sources)) = (&mut output, sources) {
        sources.comparison = sources.verify_staged(&scratch.0, limits, controls)?;
        sources.restore_paths(metadata, &scratch.0)?;
    }
    fs::remove_dir_all(&scratch.0).map_err(|_| {
        refusal(
            "EXTERNAL_STATE_UNAVAILABLE",
            "cannot remove ephemeral request Cargo home",
        )
    })?;
    items::check(controls.0, controls.1)?;
    output
}

// Lexical tokens only: this does not resolve Cargo tables, features or packages.
// Keeping string tokens distinct avoids treating quoted content as punctuation.
fn manifest_tokens(source: &str) -> Result<Vec<(String, bool)>, DomainError> {
    let mut chars = source.chars().peekable();
    let mut tokens = Vec::new();
    let unsupported = || {
        refusal(
            "EXTERNAL_ROOT_UNSUPPORTED",
            "manifest path pre-check cannot decode a string literal",
        )
    };
    while let Some(c) = chars.next() {
        let mut token = c.to_string();
        let quoted = matches!(c, '\'' | '"');
        if c.is_whitespace() {
            continue;
        }
        if c == '#' {
            for c in chars.by_ref() {
                if c == '\n' {
                    break;
                }
            }
            continue;
        }
        if quoted {
            token.clear();
            let mut look = chars.clone();
            let multiline = look.next() == Some(c) && look.next() == Some(c);
            if multiline {
                chars.next();
                chars.next();
                if chars.peek() == Some(&'\r') {
                    chars.next();
                }
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
            }
            loop {
                let next = chars.next().ok_or_else(unsupported)?;
                if next == c {
                    let mut look = chars.clone();
                    if !multiline || (look.next() == Some(c) && look.next() == Some(c)) {
                        if multiline {
                            chars.next();
                            chars.next();
                            // TOML permits one/two content quotes before the closing triple.
                            for _ in 0..2 {
                                if chars.peek() == Some(&c) {
                                    token.push(chars.next().expect("peeked quote"));
                                }
                            }
                        }
                        break;
                    }
                }
                if multiline && next == '\r' && chars.peek() == Some(&'\n') {
                    chars.next();
                    token.push('\n');
                    continue;
                }
                if c == '"' && next == '\\' {
                    let escape = chars.next().ok_or_else(unsupported)?;
                    let decoded = match escape {
                        'b' => '\u{8}',
                        't' => '\t',
                        'n' => '\n',
                        'f' => '\u{c}',
                        'r' => '\r',
                        '"' => '"',
                        '\\' => '\\',
                        'u' | 'U' => {
                            let mut value = 0;
                            for _ in 0..if escape == 'u' { 4 } else { 8 } {
                                value = value * 16
                                    + chars
                                        .next()
                                        .and_then(|c| c.to_digit(16))
                                        .ok_or_else(unsupported)?;
                            }
                            char::from_u32(value).ok_or_else(unsupported)?
                        }
                        c if multiline && c.is_whitespace() => {
                            while chars.peek().is_some_and(|c| c.is_whitespace()) {
                                chars.next();
                            }
                            continue;
                        }
                        _ => return Err(unsupported()),
                    };
                    token.push(decoded);
                } else {
                    token.push(next);
                }
            }
        } else if c.is_ascii_alphanumeric() || matches!(c, '_' | '-') {
            while chars
                .peek()
                .is_some_and(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-'))
            {
                token.push(chars.next().expect("peeked bare key"));
            }
        }
        tokens.push((token, quoted));
    }
    Ok(tokens)
}

fn manifest_candidates(source: &str) -> Result<(Vec<String>, Vec<String>, bool), DomainError> {
    let tokens = manifest_tokens(source)?;
    let mut paths = Vec::new();
    let mut members = Vec::new();
    let mut inherits = false;
    let unsupported = || {
        refusal(
            "EXTERNAL_ROOT_UNSUPPORTED",
            "manifest pre-check requires literal paths and literal non-glob workspace members; explicit workspace pointers are unsupported",
        )
    };
    for (i, (key, _)) in tokens.iter().enumerate() {
        if tokens.get(i + 1) != Some(&("=".into(), false)) {
            continue;
        }
        match key.as_str() {
            "path" => {
                let (path, true) = tokens.get(i + 2).ok_or_else(unsupported)? else {
                    return Err(unsupported());
                };
                paths.push(path.clone());
            }
            "workspace" => {
                // `dep.workspace = true` inherits from a captured ancestor. A
                // package.workspace string can redirect Cargo elsewhere: refuse
                // it before invocation rather than guessing workspace semantics.
                if tokens.get(i + 2) != Some(&("true".into(), false)) {
                    return Err(unsupported());
                }
                inherits = true;
            }
            "members" => {
                if tokens.get(i + 2) != Some(&("[".into(), false)) {
                    return Err(unsupported());
                }
                let mut j = i + 3;
                loop {
                    if tokens.get(j) == Some(&("]".into(), false)) {
                        break;
                    }
                    let (member, true) = tokens.get(j).ok_or_else(unsupported)? else {
                        return Err(unsupported());
                    };
                    if member.is_empty() || member.contains(['*', '?', '[', ']', '{', '}']) {
                        return Err(unsupported());
                    }
                    members.push(member.clone());
                    j += 1;
                    match tokens.get(j) {
                        Some((token, false)) if token == "," => j += 1,
                        Some((token, false)) if token == "]" => break,
                        _ => return Err(unsupported()),
                    }
                }
            }
            _ => {}
        }
    }
    Ok((paths, members, inherits))
}

#[cfg(test)]
fn declared_paths(source: &str) -> Result<Vec<String>, DomainError> {
    Ok(manifest_candidates(source)?.0)
}

// Only possible path-declaration edges are checked for cycles. Membership and
// ancestor revisits are normal. For workspace inheritance, all captured ancestor
// path declarations are possible edges: no Cargo table/alias interpretation.
fn acyclic_paths(
    path: &Path,
    edges: &BTreeMap<PathBuf, Vec<PathBuf>>,
    inherited: &BTreeSet<PathBuf>,
    active: &mut BTreeSet<PathBuf>,
    done: &mut BTreeMap<PathBuf, usize>,
    controls: Controls<'_>,
) -> Result<usize, DomainError> {
    items::check(controls.0, controls.1)?;
    if let Some(height) = done.get(path) {
        return Ok(*height);
    }
    if active.len() >= ANCESTOR_CAP {
        return Err(refusal(
            "EXTERNAL_CORPUS_INCOMPLETE",
            "manifest path hop cap exhausted",
        ));
    }
    if !active.insert(path.to_path_buf()) {
        return Err(refusal(
            "EXTERNAL_ROOT_UNSUPPORTED",
            "manifest path candidate cycle",
        ));
    }
    let mut height = 1;
    let parent = path.parent().expect("manifest parent");
    for manifest in std::iter::once(path.to_path_buf()).chain(
        parent
            .ancestors()
            .skip(1)
            .filter(|_| inherited.contains(path))
            .map(|p| p.join("Cargo.toml")),
    ) {
        if let Some(children) = edges.get(&manifest) {
            for child in children {
                height =
                    height.max(1 + acyclic_paths(child, edges, inherited, active, done, controls)?);
                if height > ANCESTOR_CAP {
                    return Err(refusal(
                        "EXTERNAL_CORPUS_INCOMPLETE",
                        "manifest path hop cap exhausted",
                    ));
                }
            }
        }
    }
    active.remove(path);
    done.insert(path.to_path_buf(), height);
    Ok(height)
}

fn manifest_closure(
    seeds: BTreeSet<PathBuf>,
    tree: &Path,
    limits: &Limits,
    controls: Controls<'_>,
) -> Result<BTreeMap<PathBuf, Vec<u8>>, DomainError> {
    let mut pending: BTreeMap<_, _> = seeds.into_iter().map(|p| (p, 0)).collect();
    let mut observed = BTreeMap::new();
    let mut edges = BTreeMap::new();
    let mut inherited = BTreeSet::new();
    let mut total = 0;
    while let Some((path, depth)) = pending.pop_first() {
        items::check(controls.0, controls.1)?;
        if observed.contains_key(&path) {
            continue;
        }
        if depth >= ANCESTOR_CAP || observed.len() + pending.len() >= limits.max_files {
            return Err(refusal(
                "EXTERNAL_CORPUS_INCOMPLETE",
                "manifest pre-check hop/file cap exhausted",
            ));
        }
        let parent = path.parent().expect("manifest parent");
        disjoint(tree, [parent.to_path_buf()])?;
        let bytes = read(&path, limits.max_file_bytes, controls)?;
        no_links(parent)?;
        let source = std::str::from_utf8(&bytes)
            .map_err(|_| refusal("EXTERNAL_ROOT_UNSUPPORTED", "manifest is not UTF-8"))?;
        let (paths, members, inherits) = manifest_candidates(source)?;
        if inherits {
            inherited.insert(path.clone());
        }
        total += bytes.len();
        if total > limits.max_source_bytes {
            return Err(refusal(
                "EXTERNAL_CORPUS_INCOMPLETE",
                "manifest pre-check byte cap exhausted",
            ));
        }
        observed.insert(path.clone(), bytes);
        let mut children = Vec::new();
        for (candidate, dependency) in paths
            .into_iter()
            .map(|p| (p, true))
            .chain(members.into_iter().map(|p| (p, false)))
        {
            items::check(controls.0, controls.1)?;
            let root = parent.join(candidate);
            disjoint(tree, [root.clone()])?;
            let root = no_links(&root)?;
            // A library/bin target's literal path is also a conservative lexical
            // candidate. Cargo never loads a package manifest from a regular file.
            if root.is_file() {
                continue;
            }
            let child = root.join("Cargo.toml");
            if dependency {
                children.push(child.clone());
            }
            // A dependency can belong to its own enclosing workspace; Cargo
            // reads those manifests and sibling members as well.
            for candidate in ancestor_manifests(&root, controls)?
                .into_iter()
                .chain([child])
            {
                if !observed.contains_key(&candidate) {
                    pending.entry(candidate).or_insert(depth + 1);
                }
            }
            if observed.len() + pending.len() > limits.max_files {
                return Err(refusal(
                    "EXTERNAL_CORPUS_INCOMPLETE",
                    "manifest pre-check file cap exhausted",
                ));
            }
        }
        edges.insert(path.clone(), children);
        let lock = parent.join("Cargo.lock");
        match fs::symlink_metadata(&lock) {
            Ok(_) => {
                let bytes = read(&lock, limits.max_file_bytes, controls)?;
                total += bytes.len();
                if observed.len() >= limits.max_files || total > limits.max_source_bytes {
                    return Err(refusal(
                        "EXTERNAL_CORPUS_INCOMPLETE",
                        "manifest pre-check cap exhausted",
                    ));
                }
                observed.insert(lock, bytes);
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(io_error(e)),
        }
    }
    let mut done = BTreeMap::new();
    for path in edges.keys() {
        items::check(controls.0, controls.1)?;
        acyclic_paths(
            path,
            &edges,
            &inherited,
            &mut BTreeSet::new(),
            &mut done,
            controls,
        )?;
    }
    // Reobserve all closure inputs before the first server-state write, as well
    // as retaining them for the existing final-publication freshness gate.
    for (path, bytes) in &observed {
        if read(path, limits.max_file_bytes, controls)? != *bytes {
            return Err(refusal(
                "EXTERNAL_SOURCE_CHANGED",
                "manifest inputs changed during pre-check",
            ));
        }
    }
    Ok(observed)
}

fn ancestor_manifests(
    directory: &Path,
    controls: Controls<'_>,
) -> Result<BTreeSet<PathBuf>, DomainError> {
    let mut manifests = BTreeSet::new();
    for (hop, ancestor) in directory.ancestors().enumerate() {
        items::check(controls.0, controls.1)?;
        if hop >= ANCESTOR_CAP {
            return Err(refusal(
                "EXTERNAL_CORPUS_INCOMPLETE",
                "manifest ancestor pre-check cap exhausted",
            ));
        }
        let path = ancestor.join("Cargo.toml");
        match fs::symlink_metadata(&path) {
            Ok(_) => {
                manifests.insert(path);
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(io_error(e)),
        }
    }
    Ok(manifests)
}

fn read(path: &Path, cap: usize, controls: Controls<'_>) -> Result<Vec<u8>, DomainError> {
    items::check(controls.0, controls.1)?;
    let before = fs::symlink_metadata(path).map_err(io_error)?;
    if !before.is_file() || before.file_type().is_symlink() || before.len() > cap as u64 {
        return Err(refusal(
            "EXTERNAL_CORPUS_INCOMPLETE",
            "external input is not a bounded regular file",
        ));
    }
    let file = fs::File::open(path).map_err(io_error)?;
    use std::os::unix::fs::MetadataExt;
    let opened = file.metadata().map_err(io_error)?;
    if (before.dev(), before.ino()) != (opened.dev(), opened.ino()) {
        return Err(refusal(
            "EXTERNAL_SOURCE_CHANGED",
            "external file identity changed before capture",
        ));
    }
    let bytes = read_chunks(file, cap, controls)?;
    let after = fs::symlink_metadata(path).map_err(io_error)?;
    if !after.is_file()
        || after.file_type().is_symlink()
        || before.len() != after.len()
        || before.modified().ok() != after.modified().ok()
        || (before.dev(), before.ino()) != (after.dev(), after.ino())
    {
        return Err(refusal(
            "EXTERNAL_SOURCE_CHANGED",
            "external source changed while reading",
        ));
    }
    Ok(bytes)
}
fn read_chunks(
    mut reader: impl Read,
    cap: usize,
    controls: Controls<'_>,
) -> Result<Vec<u8>, DomainError> {
    let mut bytes = Vec::new();
    let mut chunk = [0; 16 * 1024];
    loop {
        items::check(controls.0, controls.1)?;
        let n = reader.read(&mut chunk).map_err(io_error)?;
        if n == 0 {
            break;
        }
        if bytes.len() + n > cap {
            return Err(refusal(
                "EXTERNAL_CORPUS_INCOMPLETE",
                "external file cap exhausted",
            ));
        }
        bytes.extend_from_slice(&chunk[..n]);
    }
    items::check(controls.0, controls.1)?;
    Ok(bytes)
}
fn no_links(path: &Path) -> Result<PathBuf, DomainError> {
    for part in path.ancestors() {
        if fs::symlink_metadata(part)
            .map_err(io_error)?
            .file_type()
            .is_symlink()
        {
            return Err(refusal(
                "EXTERNAL_ROOT_UNSUPPORTED",
                "external path has a symlink component",
            ));
        }
    }
    path.canonicalize().map_err(io_error)
}
fn rust_files(
    root: &Path,
    limits: &Limits,
    controls: Controls<'_>,
) -> Result<Vec<PathBuf>, DomainError> {
    let mut pending = vec![root.to_path_buf()];
    let mut files = Vec::new();
    let mut entries = 0;
    while let Some(directory) = pending.pop() {
        items::check(controls.0, controls.1)?;
        for entry in fs::read_dir(directory).map_err(io_error)? {
            items::check(controls.0, controls.1)?;
            let entry = entry.map_err(io_error)?;
            entries += 1;
            if entries > limits.max_files {
                return Err(refusal(
                    "EXTERNAL_CORPUS_INCOMPLETE",
                    "external directory entry cap exhausted",
                ));
            }
            if matches!(entry.file_name().to_str(), Some("target" | ".git")) {
                continue;
            }
            let kind = entry.file_type().map_err(io_error)?;
            if kind.is_symlink() {
                return Err(refusal(
                    "EXTERNAL_ROOT_UNSUPPORTED",
                    "external package contains a symlink",
                ));
            }
            if kind.is_dir() {
                pending.push(entry.path());
            } else if entry.path().extension().is_some_and(|e| e == "rs") {
                if !kind.is_file() {
                    return Err(refusal(
                        "EXTERNAL_ROOT_UNSUPPORTED",
                        "external source is not regular",
                    ));
                }
                files.push(entry.path());
            }
        }
    }
    files.sort();
    Ok(files)
}
fn validate_modules(
    node: &ra_ap_syntax::SyntaxNode,
    directory: &Path,
    stem: &str,
    files: &[PathBuf],
) -> Result<(), DomainError> {
    use ra_ap_syntax::{
        AstNode,
        ast::{self, HasName},
    };
    let base = if matches!(stem, "" | "lib" | "main" | "mod") {
        directory.to_path_buf()
    } else {
        directory.join(stem)
    };
    fn walk(
        node: &ra_ap_syntax::SyntaxNode,
        base: &Path,
        files: &[PathBuf],
    ) -> Result<(), DomainError> {
        for child in node.children() {
            if let Some(module) = ast::Module::cast(child.clone()) {
                let name = module
                    .name()
                    .ok_or_else(|| {
                        refusal("EXTERNAL_CORPUS_INCOMPLETE", "external module has no name")
                    })?
                    .text()
                    .to_string();
                let name = name.trim_start_matches("r#");
                if let Some(inline) = module.item_list() {
                    walk(inline.syntax(), &base.join(name), files)?;
                } else {
                    let flat = base.join(format!("{name}.rs"));
                    let nested = base.join(name).join("mod.rs");
                    if files.binary_search(&flat).is_ok() == files.binary_search(&nested).is_ok() {
                        return Err(refusal(
                            "EXTERNAL_CORPUS_INCOMPLETE",
                            "external written module is missing or has competing layouts",
                        ));
                    }
                }
            } else {
                walk(&child, base, files)?;
            }
        }
        Ok(())
    }
    walk(node, &base, files)
}
fn capture(
    root: &Path,
    crate_source: &Path,
    namespace: &str,
    limits: &Limits,
    controls: Controls<'_>,
    texts: &mut BTreeMap<String, String>,
    observed: &mut BTreeMap<PathBuf, Vec<u8>>,
) -> Result<(usize, usize), DomainError> {
    let paths = rust_files(root, limits, controls)?;
    let mut bytes = 0;
    for path in &paths {
        let data = read(path, limits.max_file_bytes, controls)?;
        if let Some(prior) = observed.get(path) {
            if prior != &data {
                return Err(refusal(
                    "EXTERNAL_SOURCE_CHANGED",
                    "dependency source changed after discovery capture",
                ));
            }
        } else if observed.len() >= limits.max_files
            || observed.values().map(Vec::len).sum::<usize>() + data.len() > limits.max_source_bytes
        {
            return Err(refusal(
                "EXTERNAL_CORPUS_INCOMPLETE",
                "combined discovery/source capture cap exhausted",
            ));
        }
        bytes += data.len();
        if bytes > limits.max_source_bytes
            || texts.values().map(String::len).sum::<usize>() + data.len() > limits.max_source_bytes
        {
            return Err(refusal(
                "EXTERNAL_CORPUS_INCOMPLETE",
                "external aggregate source cap exhausted",
            ));
        }
        let source = String::from_utf8(data.clone()).map_err(|_| {
            refusal(
                "EXTERNAL_ROOT_UNSUPPORTED",
                "external Rust source is not UTF-8",
            )
        })?;
        let parsed = ra_ap_syntax::SourceFile::parse(&source, ra_ap_syntax::Edition::Edition2024);
        if !parsed.errors().is_empty() {
            return Err(refusal(
                "EXTERNAL_ROOT_UNSUPPORTED",
                "external source has syntax recovery",
            ));
        }
        use ra_ap_syntax::AstNode;
        // Path overrides and filesystem macro inclusion require a different closure
        // authority; refuse rather than letting RA silently see an incomplete library.
        for node in parsed.tree().syntax().descendants() {
            if let Some(attr) = ra_ap_syntax::ast::Attr::cast(node.clone())
                && attr.simple_name().as_deref() == Some("path")
            {
                return Err(refusal(
                    "EXTERNAL_ROOT_UNSUPPORTED",
                    "external #[path] module overrides are unsupported",
                ));
            }
            if let Some(call) = ra_ap_syntax::ast::MacroCall::cast(node)
                && call.path().is_some_and(|p| {
                    matches!(
                        p.to_string().as_str(),
                        "include" | "include_str" | "include_bytes"
                    )
                })
            {
                return Err(refusal(
                    "EXTERNAL_ROOT_UNSUPPORTED",
                    "external filesystem includes are unsupported",
                ));
            }
        }
        validate_modules(
            parsed.tree().syntax(),
            path.parent().expect("source parent"),
            if path == crate_source {
                ""
            } else {
                path.file_stem().and_then(|s| s.to_str()).unwrap_or("")
            },
            &paths,
        )?;
        let relative = path
            .strip_prefix(root)
            .expect("enumerated package file")
            .to_str()
            .ok_or_else(|| {
                refusal(
                    "EXTERNAL_ROOT_UNSUPPORTED",
                    "external source path is not UTF-8",
                )
            })?;
        if texts.len() >= limits.max_files {
            return Err(refusal(
                "EXTERNAL_CORPUS_INCOMPLETE",
                "external aggregate file cap exhausted",
            ));
        }
        texts.insert(format!("{namespace}{relative}"), source);
        observed.insert(path.clone(), data);
    }
    Ok((paths.len(), bytes))
}

pub(crate) fn discover(
    repository: &Path,
    crate_root: &str,
    config: &Configuration,
    limits: &Limits,
    controls: Controls<'_>,
) -> Result<Corpus, DomainError> {
    discover_at(
        repository,
        crate_root,
        config,
        limits,
        controls,
        &state_tree()?,
    )
}
fn discover_at(
    repository: &Path,
    crate_root: &str,
    config: &Configuration,
    limits: &Limits,
    controls: Controls<'_>,
    tree: &Path,
) -> Result<Corpus, DomainError> {
    discover_at_home(
        repository,
        crate_root,
        config,
        limits,
        controls,
        tree,
        &sources::cargo_home()?,
    )
}
fn discover_at_home(
    repository: &Path,
    crate_root: &str,
    config: &Configuration,
    limits: &Limits,
    controls: Controls<'_>,
    tree: &Path,
    cargo_home: &Path,
) -> Result<Corpus, DomainError> {
    items::check(controls.0, controls.1)?;
    disjoint(tree, [repository.to_path_buf()])?;
    let mut directory = repository
        .join(crate_root)
        .parent()
        .ok_or_else(|| refusal("EXTERNAL_DISCOVERY_FAILED", "invalid crate root"))?
        .to_path_buf();
    while !directory.join("Cargo.toml").is_file() {
        if directory == repository || !directory.starts_with(repository) {
            return Err(refusal(
                "EXTERNAL_DISCOVERY_FAILED",
                "no caller manifest inside repository",
            ));
        }
        directory.pop();
    }
    let manifest = directory.join("Cargo.toml");
    // Conservatively include every ancestor manifest, even outside the selected
    // repository: Cargo may inherit paths from an enclosing workspace there.
    let mut manifests = ancestor_manifests(&directory, controls)?;
    for root in config.crates.iter().map(|c| c.root_file.as_str()) {
        let source = repository.join(root);
        if let Some(parent) = source.parent() {
            manifests.extend(ancestor_manifests(parent, controls)?);
        }
    }
    let observed = manifest_closure(manifests, tree, limits, controls)?;
    // Require a captured lock before invocation. Cargo must not synthesize one.
    if !directory
        .ancestors()
        .any(|p| observed.contains_key(&p.join("Cargo.lock")))
    {
        return Err(refusal(
            "EXTERNAL_DISCOVERY_FAILED",
            "an existing locked caller workspace is required",
        ));
    }
    let mut sources =
        sources::Sources::prepare(&directory, cargo_home, tree, observed, limits, controls)?;
    let metadata = metadata_with_sources(&directory, tree, Some(&mut sources), limits, controls)?;
    // Archive comparison shares the budget with later path-library capture,
    // not just the registry inputs already observed before Cargo runs.
    let mut capture_limits = limits.clone();
    capture_limits.max_files = limits.max_files.saturating_sub(sources.comparison.0);
    capture_limits.max_source_bytes = limits.max_source_bytes.saturating_sub(sources.comparison.1);
    sources.recheck(limits, controls)?;
    let mut observed = sources.observed.clone();
    // An external enclosing workspace is allowed only when captured and checked
    // before Cargo. Explicit non-ancestor workspace routes remain unsupported.
    if !observed.contains_key(&metadata.workspace_root.join("Cargo.toml"))
        || !observed.contains_key(&metadata.workspace_root.join("Cargo.lock"))
    {
        return Err(refusal(
            "EXTERNAL_ROOT_UNSUPPORTED",
            "Cargo workspace manifest/lock was not captured before discovery",
        ));
    }
    sources.bind_workspace(&metadata.workspace_root)?;
    disjoint(tree, [metadata.workspace_root.clone()])?;
    let caller = metadata
        .packages
        .iter()
        .find(|p| p.manifest_path == manifest)
        .ok_or_else(|| {
            refusal(
                "EXTERNAL_DISCOVERY_FAILED",
                "caller package absent from metadata",
            )
        })?;
    let root_target = caller
        .targets
        .iter()
        .find(|t| t.src_path == repository.join(crate_root))
        .ok_or_else(|| {
            refusal(
                "EXTERNAL_CONFIGURATION_MISMATCH",
                "selected crate root is not a Cargo target",
            )
        })?;
    if caller.targets.iter().any(|t| {
        t.kind
            .iter()
            .any(|k| k == "custom-build" || k == "proc-macro")
    }) {
        return Err(refusal(
            "EXTERNAL_CONFIGURATION_UNSUPPORTED",
            "caller build-script/proc-macro configuration is not proved",
        ));
    }
    if root_target.kind.iter().any(|k| k != "lib" && k != "bin") {
        return Err(refusal(
            "EXTERNAL_CONFIGURATION_UNSUPPORTED",
            "only ordinary caller library/binary targets are supported",
        ));
    }
    let resolve = metadata.resolve.as_ref().ok_or_else(|| {
        refusal(
            "EXTERNAL_DISCOVERY_FAILED",
            "metadata has no resolved graph",
        )
    })?;
    let caller_node = resolve
        .nodes
        .iter()
        .find(|n| n.id == caller.id)
        .ok_or_else(|| refusal("EXTERNAL_DISCOVERY_FAILED", "caller node absent"))?;
    let mut configuration = config.clone();
    let root_index = configuration
        .crates
        .iter()
        .position(|c| c.root_file == crate_root)
        .ok_or_else(|| {
            refusal(
                "EXTERNAL_CONFIGURATION_MISMATCH",
                "explicit caller configuration required",
            )
        })?;
    let root_config = &configuration.crates[root_index];
    if root_config.name != root_target.name.replace('-', "_")
        || root_config.edition != root_target.edition
        || root_config.features.iter().collect::<BTreeSet<_>>()
            != caller_node.features.iter().collect::<BTreeSet<_>>()
    {
        return Err(refusal(
            "EXTERNAL_CONFIGURATION_MISMATCH",
            "caller target name, edition or resolved features differ from explicit configuration",
        ));
    }
    let mut texts = BTreeMap::new();
    let mut roots = Vec::new();
    let mut identities = BTreeMap::new();
    let mut consulted = Vec::new();
    // Package IDs, not bare Cargo/crate names, key the internal graph. Adapter
    // labels are unique even when two versions share a library name or one
    // package is reachable under multiple renamed edges.
    let mut graph_nodes: BTreeMap<String, String> = BTreeMap::new();
    let mut package_ids = BTreeSet::new();
    let mut package_identities = BTreeSet::new();
    let mut node_ids = BTreeSet::new();
    if metadata.packages.iter().any(|p| {
        !package_ids.insert(&p.id)
            || !package_identities.insert((
                &p.name,
                &p.version,
                &p.source,
                p.source.is_none().then_some(&p.manifest_path),
            ))
    }) || resolve.nodes.iter().any(|n| !node_ids.insert(&n.id))
    {
        return Err(refusal(
            "EXTERNAL_PACKAGE_AMBIGUOUS",
            "duplicate package or resolved node identity",
        ));
    }
    for edge in &caller_node.deps {
        items::check(controls.0, controls.1)?;
        if edge
            .dep_kinds
            .iter()
            .all(|k| k.kind.as_deref() == Some("dev") || k.kind.as_deref() == Some("build"))
        {
            continue;
        }
        if edge.dep_kinds.iter().any(|k| k.target.is_some()) {
            return Err(refusal(
                "EXTERNAL_CONFIGURATION_UNSUPPORTED",
                "target-conditional dependency edges are unsupported",
            ));
        }
        let package = metadata
            .packages
            .iter()
            .find(|p| p.id == edge.pkg)
            .ok_or_else(|| {
                refusal(
                    "EXTERNAL_DISCOVERY_FAILED",
                    "resolved dependency package absent",
                )
            })?;
        let declared = caller
            .dependencies
            .iter()
            .find(|d| {
                d.kind.is_none()
                    && d.rename.as_deref().unwrap_or(&d.name).replace('-', "_") == edge.name
            })
            .ok_or_else(|| {
                refusal(
                    "EXTERNAL_ROOT_UNSUPPORTED",
                    "dependency is not a declared direct edge",
                )
            })?;
        if declared.name != package.name || declared.source != package.source {
            return Err(refusal(
                "EXTERNAL_CONFIGURATION_MISMATCH",
                "declared dependency name/source does not identify resolved package",
            ));
        }
        if package
            .source
            .as_ref()
            .is_some_and(|s| !s.starts_with("registry+") && !s.starts_with("sparse+"))
        {
            return Err(refusal(
                "EXTERNAL_DEPENDENCY_UNSUPPORTED",
                "git and non-registry dependencies are not admitted",
            ));
        }
        let root =
            no_links(package.manifest_path.parent().ok_or_else(|| {
                refusal("EXTERNAL_ROOT_UNSUPPORTED", "invalid package manifest")
            })?)?;
        let provenance = sources.provenance(package, &root, controls)?;
        if package.source.is_none()
            && declared
                .path
                .as_ref()
                .map(|p| p.canonicalize())
                .transpose()
                .map_err(io_error)?
                != Some(root.clone())
        {
            return Err(refusal(
                "EXTERNAL_CONFIGURATION_MISMATCH",
                "declared path does not identify resolved package root",
            ));
        }
        disjoint(tree, [root.clone()])?;
        if provenance == "path" {
            config_guard(&root)?;
        }
        if provenance == "path" && (root.starts_with(repository) || repository.starts_with(&root)) {
            return Err(refusal(
                "EXTERNAL_ROOT_UNSUPPORTED",
                "external path library must be disjoint from caller repository",
            ));
        }
        let node = resolve
            .nodes
            .iter()
            .find(|n| n.id == package.id)
            .ok_or_else(|| refusal("EXTERNAL_DISCOVERY_FAILED", "dependency node absent"))?;
        if !node.deps.is_empty() {
            return Err(refusal(
                "EXTERNAL_DEPENDENCY_UNSUPPORTED",
                "transitive dependency closures are not supported",
            ));
        }
        if package.targets.iter().any(|t| {
            t.kind
                .iter()
                .any(|k| k == "proc-macro" || k == "custom-build")
        }) {
            return Err(refusal(
                "EXTERNAL_CONFIGURATION_UNSUPPORTED",
                "build-script/proc-macro configuration is not proved",
            ));
        }
        let target = package
            .targets
            .iter()
            .find(|t| t.kind.iter().any(|k| k == "lib"))
            .ok_or_else(|| {
                refusal(
                    "EXTERNAL_ROOT_UNSUPPORTED",
                    "dependency has no ordinary library target",
                )
            })?;
        let src = no_links(&target.src_path)?;
        let relative = src
            .strip_prefix(&root)
            .map_err(|_| {
                refusal(
                    "EXTERNAL_ROOT_UNSUPPORTED",
                    "library target escapes package",
                )
            })?
            .to_str()
            .ok_or_else(|| refusal("EXTERNAL_ROOT_UNSUPPORTED", "invalid library path"))?;
        if configuration.crates[root_index]
            .dependencies
            .iter()
            .any(|d| d.name == edge.name)
        {
            return Err(refusal(
                "EXTERNAL_CONFIGURATION_MISMATCH",
                "external dependency alias conflicts with explicit graph",
            ));
        }
        if configuration.crates[root_index].dependencies.len() >= 32 {
            return Err(refusal(
                "EXTERNAL_CORPUS_INCOMPLETE",
                "external dependency edge cap exhausted",
            ));
        }
        if let Some(name) = graph_nodes.get(&package.id) {
            configuration.crates[root_index]
                .dependencies
                .push(DependencyInput {
                    name: edge.name.clone(),
                    crate_name: name.clone(),
                });
            continue;
        }
        let index = roots.len();
        let namespace = format!("{PREFIX}{index}/");
        let name = format!("external_path_{index}");
        if configuration.crates.iter().any(|c| c.name == name) {
            return Err(refusal(
                "EXTERNAL_CONFIGURATION_MISMATCH",
                "external dependency conflicts with explicit graph",
            ));
        }
        if !observed.contains_key(&package.manifest_path) {
            return Err(refusal(
                "EXTERNAL_ROOT_UNSUPPORTED",
                "resolved package manifest was not captured before Cargo",
            ));
        }
        if configuration.crates.len() >= 32
            || configuration.crates[root_index].dependencies.len() >= 32
        {
            return Err(refusal(
                "EXTERNAL_CORPUS_INCOMPLETE",
                "external crate graph cap exhausted",
            ));
        }
        let (files, bytes) = capture(
            &root,
            &src,
            &namespace,
            &capture_limits,
            controls,
            &mut texts,
            &mut observed,
        )?;
        let root_file = format!("{namespace}{relative}");
        if !texts.contains_key(&root_file) {
            return Err(refusal(
                "EXTERNAL_CORPUS_INCOMPLETE",
                "library root missing from captured sources",
            ));
        }
        configuration.crates[root_index]
            .dependencies
            .push(DependencyInput {
                name: edge.name.clone(),
                crate_name: name.clone(),
            });
        graph_nodes.insert(package.id.clone(), name.clone());
        configuration.crates.push(CrateInput {
            name,
            root_file,
            edition: target.edition.clone(),
            features: node.features.clone(),
            cfg: Vec::new(),
            dependencies: Vec::new(),
        });
        consulted.push(ConsultedRoot {
            package_id: package.id.clone(),
            package: package.name.clone(),
            version: package.version.clone(),
            source: package.source.clone().unwrap_or_else(|| "path".into()),
            provenance: provenance.into(),
            root: root
                .to_str()
                .ok_or_else(|| refusal("EXTERNAL_ROOT_UNSUPPORTED", "invalid package root"))?
                .into(),
            namespace,
            features: node.features.clone(),
            edition: target.edition.clone(),
            files,
            bytes,
        });
        use std::os::unix::fs::MetadataExt;
        let meta = fs::metadata(&root).map_err(io_error)?;
        identities.insert(root.clone(), (meta.dev(), meta.ino()));
        roots.push(root);
    }
    let digest = scope::hash_serialized(
        repository,
        &(&consulted, &observed, &configuration),
        controls,
    )?;
    Ok(Corpus { texts, configuration, coverage: Coverage { complete: true, consulted_roots: consulted, corpus_digest: digest, omissions: vec!["direct path/locally cached registry/source-replaced libraries only; no git or transitive closure; no build-script/proc-macro execution, sysroot or compilation".into()] }, observed, roots, identities, limits: limits.clone(), repository: repository.into(), crate_root: crate_root.into(), caller_configuration: config.clone(), cargo_home: cargo_home.into(), discovery_tree: tree.into() })
}
impl Corpus {
    pub(crate) fn recheck(&self, controls: Controls<'_>) -> Result<(), DomainError> {
        for (path, bytes) in &self.observed {
            if read(path, self.limits.max_file_bytes, controls)? != *bytes {
                return Err(refusal(
                    "EXTERNAL_SOURCE_CHANGED",
                    "dependency or discovery inputs changed before publication",
                ));
            }
        }
        for root in &self.roots {
            use std::os::unix::fs::MetadataExt;
            let metadata = fs::metadata(root).map_err(io_error)?;
            if self.identities.get(root) != Some(&(metadata.dev(), metadata.ino())) {
                return Err(refusal(
                    "EXTERNAL_SOURCE_CHANGED",
                    "dependency directory identity changed",
                ));
            }
            if no_links(root)? != *root {
                return Err(refusal(
                    "EXTERNAL_SOURCE_CHANGED",
                    "dependency root identity changed",
                ));
            }
            let fresh = rust_files(root, &self.limits, controls)?;
            let captured: Vec<_> = self
                .observed
                .keys()
                .filter(|p| p.starts_with(root) && p.extension().is_some_and(|e| e == "rs"))
                .cloned()
                .collect();
            if fresh != captured {
                return Err(refusal(
                    "EXTERNAL_SOURCE_CHANGED",
                    "dependency source membership changed before publication",
                ));
            }
        }
        // Re-discovery binds manifests, workspace features and package/edge identity,
        // including configuration appearance between capture and publication.
        let fresh = discover_at_home(
            &self.repository,
            &self.crate_root,
            &self.caller_configuration,
            &self.limits,
            controls,
            &self.discovery_tree,
            &self.cargo_home,
        )?;
        if fresh.coverage.corpus_digest != self.coverage.corpus_digest {
            return Err(refusal(
                "EXTERNAL_SOURCE_CHANGED",
                "dependency graph/configuration changed before publication",
            ));
        }
        Ok(())
    }
}
