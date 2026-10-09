use crate::result::{DomainError, SearchEnvelope, SearchRequest, skip};
use ignore::{
    WalkBuilder,
    overrides::{Override, OverrideBuilder},
};
use std::os::unix::fs::MetadataExt;
use std::{
    fs::{self, File, Metadata},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    process::{Command, Stdio},
    sync::{Arc, Mutex},
    time::Instant,
};

pub struct FileSnapshot {
    pub path: String,
    pub source: String,
    pub mode: u32,
}
pub struct Scope {
    pub root: PathBuf,
    pub paths: Option<Vec<String>>,
    pub globs: Option<Vec<String>>,
    includes: Option<Override>,
}

pub fn git(root: &Path) -> Command {
    let mut command = Command::new("git");
    command.arg("-C").arg(root).env("GIT_OPTIONAL_LOCKS", "0");
    for key in [
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_INDEX_FILE",
        "GIT_COMMON_DIR",
        "GIT_OBJECT_DIRECTORY",
        "GIT_ALTERNATE_OBJECT_DIRECTORIES",
        "GIT_PREFIX",
        "GIT_IMPLICIT_WORK_TREE",
        "GIT_NAMESPACE",
        "GIT_SHALLOW_FILE",
    ] {
        command.env_remove(key);
    }
    command
}
fn git_output(root: &Path, args: &[&str]) -> Result<String, DomainError> {
    let mut child = git(root)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| DomainError::new("GIT_READ_FAILED", "Git 2.39+ must be available"))?;
    let mut output = Vec::new();
    let read = child
        .stdout
        .take()
        .expect("piped stdout")
        .take(64 * 1024 + 1)
        .read_to_end(&mut output);
    if output.len() > 64 * 1024 {
        let _ = child.kill();
    }
    let status = child
        .wait()
        .map_err(|_| DomainError::new("GIT_READ_FAILED", "could not reap Git"))?;
    if !status.success() && args == ["rev-parse", "--show-toplevel"] {
        return Err(DomainError::new(
            "NOT_GIT_WORKTREE",
            "supplied path has no Git working-tree root",
        ));
    }
    if read.is_err() || !status.success() || output.len() > 64 * 1024 {
        return Err(DomainError::new(
            "GIT_READ_FAILED",
            "Git could not resolve the supplied worktree",
        ));
    }
    if output.last() == Some(&b'\n') {
        output.pop();
    }
    String::from_utf8(output)
        .map_err(|_| DomainError::new("INVALID_PARAMS", "Git root is not a UTF-8 path"))
}
pub fn resolve(path: &str, launch: &Path) -> Result<PathBuf, DomainError> {
    if path.is_empty() || path.contains('\0') {
        return Err(DomainError::new(
            "INVALID_PARAMS",
            "repo_path must name an existing file or directory",
        ));
    }
    let supplied = launch.join(path);
    let canonical = fs::canonicalize(supplied).map_err(|_| {
        DomainError::new(
            "PATH_NOT_FOUND",
            "repo_path does not exist or cannot be accessed",
        )
    })?;
    let metadata = fs::metadata(&canonical)
        .map_err(|_| DomainError::new("PATH_NOT_FOUND", "cannot inspect repo_path"))?;
    let directory = if metadata.is_file() {
        canonical.parent().expect("canonical file parent")
    } else if metadata.is_dir() {
        canonical.as_path()
    } else {
        return Err(DomainError::new(
            "INVALID_PARAMS",
            "repo_path is not a file or directory",
        ));
    };
    let root = git_output(directory, &["rev-parse", "--show-toplevel"])?;
    if git_output(directory, &["rev-parse", "--is-bare-repository"])? != "false" {
        return Err(DomainError::new(
            "NOT_GIT_WORKTREE",
            "bare repositories are not searchable",
        ));
    }
    let root = fs::canonicalize(root)
        .map_err(|_| DomainError::new("GIT_READ_FAILED", "Git returned an inaccessible root"))?;
    if root.to_str().is_none() {
        return Err(DomainError::new("INVALID_PARAMS", "root must be UTF-8"));
    }
    Ok(root)
}
impl Scope {
    pub fn new(root: PathBuf, request: &SearchRequest) -> Result<Self, DomainError> {
        let mut paths = request.paths.clone();
        if let Some(paths) = &mut paths {
            if paths.is_empty() {
                return Err(DomainError::new("INVALID_PARAMS", "paths cannot be empty"));
            }
            for path in paths.iter_mut() {
                if path.is_empty()
                    || path.contains('\0')
                    || Path::new(path).is_absolute()
                    || path.split('/').any(|part| part == "..")
                {
                    return Err(DomainError::new(
                        "OUTSIDE_ROOT",
                        "paths must be root-relative without '..'",
                    ));
                }
                let normalized: PathBuf = Path::new(path)
                    .components()
                    .filter(|c| *c != Component::CurDir)
                    .collect();
                let mut current = root.clone();
                for component in normalized.components() {
                    current.push(component);
                    let meta = fs::symlink_metadata(&current).map_err(|_| {
                        DomainError::new("PATH_NOT_FOUND", "a paths restriction does not exist")
                    })?;
                    if meta.file_type().is_symlink() {
                        return Err(DomainError::new(
                            "OUTSIDE_ROOT",
                            "paths cannot contain symlink components",
                        ));
                    }
                    if component.as_os_str() == ".git" || component.as_os_str() == "target" {
                        return Err(DomainError::new(
                            "INVALID_PARAMS",
                            "paths cannot override hard exclusions",
                        ));
                    }
                }
                let canonical = fs::canonicalize(&current).map_err(|_| {
                    DomainError::new("PATH_NOT_FOUND", "cannot resolve restriction")
                })?;
                if !canonical.starts_with(&root) {
                    return Err(DomainError::new("OUTSIDE_ROOT", "restriction escapes root"));
                }
                let meta = fs::metadata(&current).map_err(|_| {
                    DomainError::new("PATH_NOT_FOUND", "cannot inspect restriction")
                })?;
                if !meta.is_file() && !meta.is_dir() {
                    return Err(DomainError::new(
                        "INVALID_PARAMS",
                        "paths must name regular files or directories",
                    ));
                }
                *path = normalized.to_str().unwrap_or(".").to_owned();
                if path.is_empty() {
                    *path = ".".into();
                }
            }
            paths.sort();
            paths.dedup();
        }
        let mut globs = request.globs.clone();
        let includes = if let Some(globs) = &mut globs {
            if globs.is_empty() {
                return Err(DomainError::new("INVALID_GLOB", "globs cannot be empty"));
            }
            let mut builder = OverrideBuilder::new(&root);
            for glob in globs.iter() {
                if glob.is_empty()
                    || glob.starts_with(['/', '!', '#', '~'])
                    || glob.ends_with('/')
                    || glob.contains(['\0', '{', '}'])
                    || glob.split('/').any(|s| s == "..")
                {
                    return Err(DomainError::new(
                        "INVALID_GLOB",
                        "use positive root-relative gitignore inclusion patterns",
                    ));
                }
                builder
                    .add(&format!("/{glob}"))
                    .map_err(|e| DomainError::new("INVALID_GLOB", e.to_string()))?;
            }
            globs.sort();
            globs.dedup();
            Some(
                builder
                    .build()
                    .map_err(|e| DomainError::new("INVALID_GLOB", e.to_string()))?,
            )
        } else {
            None
        };
        Ok(Self {
            root,
            paths,
            globs,
            includes,
        })
    }
    /// Evaluate an absent regular-file candidate with the same in-root ignore policy.
    /// Ancestors are evaluated before descendants: a leaf negation cannot reopen a pruned dir.
    pub fn admit_new_path(
        &self,
        path: &str,
        deadline: Instant,
        cancelled: &std::sync::atomic::AtomicBool,
    ) -> Result<(), DomainError> {
        normalized_path(path)?;
        let relative = Path::new(path);
        if !self.admits(relative) {
            return Err(DomainError::new(
                "INVALID_DESTINATION",
                "new path excluded by paths/globs",
            ));
        }
        let parts: Vec<_> = relative.components().collect();
        let mut current = self.root.clone();
        let mut ignores = Vec::new();
        for (index, component) in parts.iter().enumerate() {
            if cancelled.load(std::sync::atomic::Ordering::Relaxed) {
                return Err(DomainError::new("CANCELLED", "request cancelled"));
            }
            if Instant::now() >= deadline {
                return Err(DomainError::new(
                    "planning_deadline",
                    "new-path observation deadline",
                ));
            }
            let ignore_path = current.join(".gitignore");
            match fs::symlink_metadata(&ignore_path) {
                Ok(meta) => {
                    if !meta.is_file() || meta.file_type().is_symlink() {
                        return Err(DomainError::new(
                            "scan_incomplete",
                            "ignore file cannot be safely read",
                        ));
                    }
                    let mut builder = ignore::gitignore::GitignoreBuilder::new(&current);
                    if builder.add(&ignore_path).is_some() {
                        return Err(DomainError::new(
                            "scan_incomplete",
                            "ignore read/parse failed",
                        ));
                    }
                    ignores.push(builder.build().map_err(|_| {
                        DomainError::new("scan_incomplete", "ignore matcher failed")
                    })?);
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => {
                    return Err(DomainError::new(
                        "scan_incomplete",
                        "cannot inspect ignore file",
                    ));
                }
            }
            if component.as_os_str() == ".git" || component.as_os_str() == "target" {
                return Err(DomainError::new(
                    "INVALID_DESTINATION",
                    "hard-excluded component",
                ));
            }
            // Conservatively reject differently cased spellings, even on case-sensitive hosts.
            let entries = fs::read_dir(&current)
                .map_err(|_| DomainError::new("scan_incomplete", "cannot inspect path aliases"))?;
            for entry in entries {
                crate::items::check(deadline, cancelled)?;
                let entry = entry.map_err(|_| {
                    DomainError::new("scan_incomplete", "directory observation failed")
                })?;
                if let (Some(found), Some(wanted)) =
                    (entry.file_name().to_str(), component.as_os_str().to_str())
                    && found != wanted
                    && found.to_lowercase() == wanted.to_lowercase()
                {
                    return Err(DomainError::new(
                        "DESTINATION_ALREADY_EXISTS",
                        "case-folded path alias",
                    ));
                }
            }
            current.push(component);
            let is_dir = index + 1 < parts.len();
            let mut ignored = false;
            for matcher in &ignores {
                crate::items::check(deadline, cancelled)?;
                match matcher.matched(&current, is_dir) {
                    ignore::Match::Ignore(_) => ignored = true,
                    ignore::Match::Whitelist(_) => ignored = false,
                    ignore::Match::None => {}
                }
            }
            if ignored {
                return Err(DomainError::new(
                    "INVALID_DESTINATION",
                    "new path or ancestor is ignored",
                ));
            }
            match fs::symlink_metadata(&current) {
                Ok(meta) if !is_dir => {
                    let _ = meta;
                    return Err(DomainError::new(
                        "DESTINATION_ALREADY_EXISTS",
                        "destination is already present",
                    ));
                }
                Ok(meta) => {
                    if !meta.is_dir()
                        || meta.file_type().is_symlink()
                        || !fs::canonicalize(&current)
                            .map_err(|_| {
                                DomainError::new("scan_incomplete", "cannot resolve ancestor")
                            })?
                            .starts_with(&self.root)
                    {
                        return Err(DomainError::new(
                            "INVALID_DESTINATION",
                            "unsafe/non-directory ancestor",
                        ));
                    }
                    match fs::symlink_metadata(current.join(".git")) {
                        Ok(_) => {
                            return Err(DomainError::new(
                                "INVALID_DESTINATION",
                                "nested repository ancestor",
                            ));
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                        Err(_) => {
                            return Err(DomainError::new(
                                "scan_incomplete",
                                "cannot inspect repository boundary",
                            ));
                        }
                    }
                }
                Err(e) if !is_dir && e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    return Err(DomainError::new(
                        "INVALID_DESTINATION",
                        "no directory creation is supported",
                    ));
                }
                Err(_) => {
                    return Err(DomainError::new(
                        "scan_incomplete",
                        "cannot inspect destination",
                    ));
                }
            }
        }
        Ok(())
    }
    fn admits(&self, path: &Path) -> bool {
        self.paths
            .as_ref()
            .is_none_or(|paths| paths.iter().any(|p| p == "." || path.starts_with(p)))
            && self
                .includes
                .as_ref()
                .is_none_or(|globs| globs.matched(path, false).is_whitelist())
    }
}
pub fn normalized_path(path: &str) -> Result<(), DomainError> {
    if path.is_empty()
        || Path::new(path).is_absolute()
        || path.contains('\0')
        || path.split('/').any(|part| matches!(part, "" | "." | ".."))
        || Path::new(path).extension().is_none_or(|ext| ext != "rs")
    {
        return Err(DomainError::new(
            "INVALID_DESTINATION",
            "require normalized root-relative .rs path",
        ));
    }
    Ok(())
}
fn same(a: &Metadata, b: &Metadata) -> bool {
    a.dev() == b.dev()
        && a.ino() == b.ino()
        && a.len() == b.len()
        && a.mode() == b.mode()
        && a.mtime() == b.mtime()
        && a.mtime_nsec() == b.mtime_nsec()
}

pub fn discover(
    scope: &Scope,
    result: &mut SearchEnvelope,
    deadline: Instant,
    cancelled: &std::sync::atomic::AtomicBool,
) -> Result<(Vec<FileSnapshot>, Option<String>), DomainError> {
    discover_observed(scope, result, deadline, cancelled, None)
}

/// Exact effective in-root policy inputs. Absence is evidence too; corpus hashes stay unchanged.
#[derive(Debug, Default, serde::Serialize)]
pub struct ScopeInputManifest {
    pub ignores: Vec<IgnoreInput>,
    /// Observed directory/source identities and permissions, not source-content hashes.
    pub identities: Vec<(String, u64, u64, u32)>,
    #[serde(skip)]
    accounted: usize,
}
#[derive(Debug, serde::Serialize)]
pub struct IgnoreInput {
    pub path: String,
    pub bytes: Option<Vec<u8>>,
}
impl ScopeInputManifest {
    fn identity(
        &mut self,
        path: &Path,
        root: &Path,
        metadata: &Metadata,
    ) -> Result<(), DomainError> {
        let name = path
            .strip_prefix(root)
            .ok()
            .and_then(Path::to_str)
            .ok_or_else(|| {
                DomainError::new(
                    "scan_incomplete",
                    "input identity is outside the observed root",
                )
            })?
            .to_owned();
        self.accounted = self.accounted.saturating_add(name.capacity() + 128);
        if self.identities.len() >= 200_000 || self.accounted > 16 * 1024 * 1024 {
            return Err(DomainError::new(
                "scan_incomplete",
                "input identity manifest exceeds bounded policy limits",
            ));
        }
        self.identities
            .push((name, metadata.dev(), metadata.ino(), metadata.mode()));
        Ok(())
    }
    fn observe(&mut self, directory: &Path, root: &Path) -> Result<(), DomainError> {
        let path = directory.join(".gitignore");
        let fail = || {
            DomainError::new(
                "scan_incomplete",
                "effective ignore input cannot be safely read within bounded policy limits",
            )
        };
        if self.ignores.len() >= 100_000 {
            return Err(fail());
        }
        let bytes = match fs::symlink_metadata(&path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(_) => return Err(fail()),
            Ok(before) => {
                if !before.is_file()
                    || before.file_type().is_symlink()
                    || before.len() > 1024 * 1024
                {
                    return Err(fail());
                }
                let mut file = File::open(&path).map_err(|_| fail())?;
                let opened = file.metadata().map_err(|_| fail())?;
                if !same(&before, &opened) {
                    return Err(fail());
                }
                let mut bytes = Vec::new();
                Read::by_ref(&mut file)
                    .take(1024 * 1024 + 1)
                    .read_to_end(&mut bytes)
                    .map_err(|_| fail())?;
                let after = fs::symlink_metadata(&path).map_err(|_| fail())?;
                if !same(&opened, &after) || bytes.len() as u64 != after.len() {
                    return Err(fail());
                }
                self.identity(&path, root, &opened)?;
                Some(bytes)
            }
        };
        let name = path
            .strip_prefix(root)
            .map_err(|_| fail())?
            .to_str()
            .ok_or_else(fail)?
            .to_owned();
        self.accounted = self
            .accounted
            .saturating_add(name.capacity())
            .saturating_add(bytes.as_ref().map_or(0, Vec::capacity))
            .saturating_add(2 * std::mem::size_of::<IgnoreInput>());
        if self.accounted > 16 * 1024 * 1024 {
            return Err(fail());
        }
        self.ignores.push(IgnoreInput { path: name, bytes });
        Ok(())
    }
    pub fn accounted_allocation(&self) -> usize {
        self.accounted
    }
    pub fn digest(
        &self,
        scope: &Scope,
        snapshot: &str,
        controls: (Instant, &std::sync::atomic::AtomicBool),
    ) -> Result<String, DomainError> {
        hash_serialized(
            &scope.root,
            &(snapshot, &scope.paths, &scope.globs, self),
            controls,
        )
    }
}
pub fn discover_observed(
    scope: &Scope,
    result: &mut SearchEnvelope,
    deadline: Instant,
    cancelled: &std::sync::atomic::AtomicBool,
    mut inputs: Option<&mut ScopeInputManifest>,
) -> Result<(Vec<FileSnapshot>, Option<String>), DomainError> {
    let boundaries = Arc::new(Mutex::new((
        crate::result::skipped_map(),
        result.limits.diagnostic_count,
    )));
    let observed = boundaries.clone();
    let boundary_root = scope.root.clone();
    let mut walker = WalkBuilder::new(&scope.root);
    walker
        .hidden(false)
        .follow_links(false)
        .parents(false)
        .ignore(false)
        .git_ignore(true)
        .git_global(false)
        .git_exclude(false)
        .sort_by_file_path(Path::cmp)
        .filter_entry(move |entry| {
            if entry.depth() == 0 {
                return true;
            }
            if entry.file_name() == ".git" || entry.file_name() == "target" {
                return false;
            }
            let reason = if entry.file_type().is_some_and(|kind| kind.is_symlink()) {
                Some("symlink")
            } else if entry.file_type().is_some_and(|kind| kind.is_dir())
                && fs::symlink_metadata(entry.path().join(".git")).is_ok()
            {
                Some("nested_repository")
            } else {
                None
            };
            if let Some(reason) = reason {
                let mut state = observed.lock().expect("boundary lock");
                let (skipped, budget) = &mut *state;
                skip(
                    skipped,
                    reason,
                    entry
                        .path()
                        .strip_prefix(&boundary_root)
                        .expect("boundary within root"),
                    "POLICY_BOUNDARY",
                    budget,
                );
                false
            } else {
                true
            }
        });
    let mut paths = Vec::new();
    let mut exhausted = true;
    let mut example_budget = result.limits.diagnostic_count;
    for entry in walker.build() {
        if cancelled.load(std::sync::atomic::Ordering::Relaxed) {
            return Err(DomainError::new("CANCELLED", "request cancelled"));
        }
        if Instant::now() >= deadline {
            exhausted = false;
            result
                .truncation_reasons
                .push("discovery_deadline: narrow paths/globs".into());
            break;
        }
        match entry {
            Err(_) => {
                skip(
                    &mut result.skipped,
                    "traversal_error",
                    Path::new("."),
                    "WALK_FAILED",
                    &mut example_budget,
                );
            }
            Ok(entry) => {
                if let Some(inputs) = &mut inputs
                    && entry.file_type().is_some_and(|kind| kind.is_dir())
                {
                    let metadata = fs::symlink_metadata(entry.path()).map_err(|_| {
                        DomainError::new("scan_incomplete", "cannot observe directory identity")
                    })?;
                    if !metadata.is_dir() || metadata.file_type().is_symlink() {
                        return Err(DomainError::new(
                            "scan_incomplete",
                            "directory identity changed during discovery",
                        ));
                    }
                    inputs.identity(entry.path(), &scope.root, &metadata)?;
                    inputs.observe(entry.path(), &scope.root)?;
                }
                if entry.error().is_some() {
                    skip(
                        &mut result.skipped,
                        "traversal_error",
                        entry
                            .path()
                            .strip_prefix(&scope.root)
                            .unwrap_or(Path::new(".")),
                        "IGNORE_READ_FAILED",
                        &mut example_budget,
                    );
                }
                if entry.file_type().is_none_or(|kind| !kind.is_file())
                    || entry.path().extension().is_none_or(|ext| ext != "rs")
                {
                    continue;
                }
                let relative = entry
                    .path()
                    .strip_prefix(&scope.root)
                    .expect("walker within root");
                if relative.to_str().is_none() {
                    skip(
                        &mut result.skipped,
                        "non_utf8_path",
                        relative,
                        "PATH_ENCODING",
                        &mut example_budget,
                    );
                    continue;
                }
                if !scope.admits(relative) {
                    continue;
                }
                if paths.len() == result.limits.max_files {
                    exhausted = false;
                    result
                        .truncation_reasons
                        .push("max_files: narrow paths/globs".into());
                    break;
                }
                paths.push(relative.to_owned());
            }
        }
    }
    for (name, boundary) in &boundaries.lock().expect("boundary lock").0 {
        if boundary.count == 0 {
            continue;
        }
        let record = result.skipped.get_mut(name).expect("fixed reason");
        record.count = boundary.count;
        record.count_saturated = boundary.count_saturated;
        let keep = boundary.examples.len().min(example_budget);
        record
            .examples
            .extend_from_slice(&boundary.examples[..keep]);
        record.examples_omitted = boundary.count.saturating_sub(keep as u64);
        example_budget -= keep;
    }
    paths.sort();
    result.counts.discovered_files = paths.len();
    let mut files = Vec::new();
    let mut bytes = 0usize;
    let mut outcomes = Vec::new();
    for relative in paths {
        if cancelled.load(std::sync::atomic::Ordering::Relaxed) {
            return Err(DomainError::new("CANCELLED", "request cancelled"));
        }
        if Instant::now() >= deadline {
            exhausted = false;
            result
                .truncation_reasons
                .push("snapshot_deadline: narrow paths/globs".into());
            break;
        }
        result.counts.scanned_files += 1;
        let path = scope.root.join(&relative);
        let mut mode = 0;
        let read = (|| -> Result<(Vec<u8>, Metadata), &'static str> {
            let before = fs::symlink_metadata(&path).map_err(|_| "unreadable")?;
            mode = if before.mode() & 0o111 != 0 {
                0o100755
            } else {
                0o100644
            };
            if before.file_type().is_symlink() {
                return Err("symlink");
            }
            if !before.is_file()
                || !fs::canonicalize(&path)
                    .map_err(|_| "unreadable")?
                    .starts_with(&scope.root)
            {
                return Err("unreadable");
            }
            if before.len() > result.limits.max_file_bytes as u64 {
                return Err("oversized");
            }
            let mut handle = File::open(&path).map_err(|_| "unreadable")?;
            let opened = handle.metadata().map_err(|_| "unreadable")?;
            if !same(&before, &opened) {
                return Err("unreadable");
            }
            let mut buffer = Vec::new();
            Read::by_ref(&mut handle)
                .take(result.limits.max_file_bytes as u64 + 1)
                .read_to_end(&mut buffer)
                .map_err(|_| "unreadable")?;
            if buffer.len() > result.limits.max_file_bytes {
                return Err("oversized");
            }
            let after = fs::symlink_metadata(&path).map_err(|_| "unreadable")?;
            if !same(&opened, &after)
                || after.file_type().is_symlink()
                || buffer.len() as u64 != after.len()
            {
                return Err("unreadable");
            }
            Ok((buffer, opened))
        })();
        let name = relative.to_str().expect("UTF-8 admitted").to_owned();
        let eligible = match read {
            Ok((buffer, metadata)) => {
                if let Some(inputs) = &mut inputs {
                    inputs.identity(&path, &scope.root, &metadata)?;
                }
                if buffer.iter().take(8192).any(|b| *b == 0) {
                    Err("binary")
                } else {
                    String::from_utf8(buffer).map_err(|_| "non_utf8")
                }
            }
            Err(reason) => Err(reason),
        };
        match eligible {
            Err(reason) => {
                skip(
                    &mut result.skipped,
                    reason,
                    &relative,
                    "FILE_INELIGIBLE",
                    &mut example_budget,
                );
                outcomes.push((name, mode, reason));
            }
            Ok(source) => {
                if source.len() > result.limits.max_source_bytes - bytes {
                    exhausted = false;
                    result
                        .truncation_reasons
                        .push("max_source_bytes: narrow paths/globs".into());
                    break;
                }
                bytes += source.len();
                files.push(FileSnapshot {
                    path: name,
                    source,
                    mode,
                });
            }
        }
    }
    result.counts.eligible_files = files.len();
    let failures = [
        "oversized",
        "binary",
        "non_utf8",
        "unreadable",
        "non_utf8_path",
        "traversal_error",
    ]
    .iter()
    .any(|reason| result.skipped[*reason].count > 0);
    result.coverage.scan_exhausted = exhausted;
    result.coverage.eligible_scan_complete = exhausted
        && result.skipped["traversal_error"].count == 0
        && result.skipped["unreadable"].count == 0;
    result.coverage.scope_exhaustive = exhausted && !failures;
    let snapshot = if result.coverage.eligible_scan_complete {
        fingerprint(
            scope,
            &files,
            &outcomes,
            &result.skipped,
            deadline,
            cancelled,
        )?
    } else {
        None
    };
    if result.coverage.eligible_scan_complete && snapshot.is_none() {
        result.coverage.eligible_scan_complete = false;
        result.coverage.scope_exhaustive = false;
        result
            .truncation_reasons
            .push("fingerprint_deadline: narrow paths/globs".into());
    }
    Ok((files, snapshot))
}
/// Hash immutable semantic inputs with the same read-only, filter-free Git boundary.
pub(crate) fn hash_serialized(
    root: &Path,
    value: &impl serde::Serialize,
    controls: (Instant, &std::sync::atomic::AtomicBool),
) -> Result<String, DomainError> {
    let (deadline, cancelled) = controls;
    let format = git_output(root, &["rev-parse", "--show-object-format"])?;
    let mut child = git(root)
        .args(["hash-object", "--stdin", "--no-filters"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| {
            DomainError::new("GIT_READ_FAILED", "could not start semantic input hasher")
        })?;
    struct ControlledWriter<'a> {
        input: std::process::ChildStdin,
        deadline: Instant,
        cancelled: &'a std::sync::atomic::AtomicBool,
    }
    impl Write for ControlledWriter<'_> {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            for chunk in bytes.chunks(64 * 1024) {
                if crate::items::check(self.deadline, self.cancelled).is_err() {
                    return Err(std::io::ErrorKind::Interrupted.into());
                }
                self.input.write_all(chunk)?;
            }
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            self.input.flush()
        }
    }
    let streamed = serde_json::to_writer(
        ControlledWriter {
            input: child.stdin.take().expect("piped hasher input"),
            deadline,
            cancelled,
        },
        value,
    );
    if streamed.is_err() {
        let _ = child.kill();
    }
    let mut output = Vec::new();
    let read = child
        .stdout
        .take()
        .expect("piped hasher output")
        .take(129)
        .read_to_end(&mut output);
    if output.len() > 128 {
        let _ = child.kill();
    }
    let status = child
        .wait()
        .map_err(|_| DomainError::new("GIT_READ_FAILED", "could not reap semantic input hasher"))?;
    crate::items::check(deadline, cancelled)?;
    streamed.map_err(|_| DomainError::new("GIT_READ_FAILED", "semantic input hashing failed"))?;
    let oid = String::from_utf8(output)
        .map_err(|_| DomainError::new("GIT_READ_FAILED", "invalid hash output"))?;
    let oid = oid.trim_end_matches('\n');
    if read.is_err()
        || !status.success()
        || !matches!(format.as_str(), "sha1" | "sha256")
        || oid.len() != if format == "sha1" { 40 } else { 64 }
        || !oid.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err(DomainError::new(
            "GIT_READ_FAILED",
            "invalid semantic input hash",
        ));
    }
    Ok(format!("{format}:{oid}"))
}
fn fingerprint(
    scope: &Scope,
    files: &[FileSnapshot],
    outcomes: &[(String, u32, &str)],
    skipped: &crate::result::Skipped,
    deadline: Instant,
    cancelled: &std::sync::atomic::AtomicBool,
) -> Result<Option<String>, DomainError> {
    let format = git_output(&scope.root, &["rev-parse", "--show-object-format"])?;
    let mut child = git(&scope.root)
        .args(["hash-object", "--stdin", "--no-filters"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| {
            DomainError::new("GIT_READ_FAILED", "could not start read-only corpus hasher")
        })?;
    let streamed = (|| -> std::io::Result<()> {
        let mut input = child.stdin.take().expect("piped stdin");
        let mut field = |bytes: &[u8]| -> std::io::Result<()> {
            input.write_all(&(bytes.len() as u64).to_be_bytes())?;
            for chunk in bytes.chunks(64 * 1024) {
                if cancelled.load(std::sync::atomic::Ordering::Relaxed)
                    || Instant::now() >= deadline
                {
                    return Err(std::io::Error::from(std::io::ErrorKind::Interrupted));
                }
                input.write_all(chunk)?;
            }
            Ok(())
        };
        field(scope.root.as_os_str().as_encoded_bytes())?;
        field(&serde_json::to_vec(&(&scope.paths, &scope.globs)).expect("scope JSON"))?;
        field(&serde_json::to_vec(skipped).expect("skip outcome JSON"))?;
        for (path, mode, outcome) in outcomes {
            field(b"skipped")?;
            field(path.as_bytes())?;
            field(&mode.to_be_bytes())?;
            field(outcome.as_bytes())?;
        }
        for file in files {
            field(b"eligible")?;
            field(file.path.as_bytes())?;
            field(&file.mode.to_be_bytes())?;
            field(file.source.as_bytes())?;
        }
        Ok(())
    })();
    let stopped = streamed
        .as_ref()
        .is_err_and(|e| e.kind() == std::io::ErrorKind::Interrupted);
    if stopped {
        let _ = child.kill();
    }
    let mut output = Vec::new();
    let read = child
        .stdout
        .take()
        .expect("piped stdout")
        .take(129)
        .read_to_end(&mut output);
    if output.len() > 128 {
        let _ = child.kill();
    }
    let status = child
        .wait()
        .map_err(|_| DomainError::new("GIT_READ_FAILED", "could not reap hasher"))?;
    if cancelled.load(std::sync::atomic::Ordering::Relaxed) {
        return Err(DomainError::new(
            "CANCELLED",
            "request cancelled during fingerprinting",
        ));
    }
    if stopped || Instant::now() >= deadline {
        return Ok(None);
    }
    if streamed.is_err() || read.is_err() || !status.success() {
        return Err(DomainError::new("GIT_READ_FAILED", "corpus hashing failed"));
    }
    if output.last() == Some(&b'\n') {
        output.pop();
    }
    let oid = String::from_utf8(output)
        .map_err(|_| DomainError::new("GIT_READ_FAILED", "invalid hash output"))?;
    if !matches!(format.as_str(), "sha1" | "sha256")
        || oid.len() != if format == "sha1" { 40 } else { 64 }
        || !oid.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err(DomainError::new(
            "GIT_READ_FAILED",
            "invalid corpus object ID",
        ));
    }
    Ok(Some(format!("{format}:{oid}")))
}
