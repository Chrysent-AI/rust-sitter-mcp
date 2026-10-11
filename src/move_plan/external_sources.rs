//! Offline Cargo inputs copied into request state, never a Cargo resolver.
//! Cargo selects sources/versions/edges; lock records only bound the finite copy.
use super::*;
use sha2::{Digest, Sha256};

fn integrity() -> DomainError {
    refusal(
        "EXTERNAL_PACKAGE_UNAVAILABLE",
        "locked package checksum or file correspondence is missing, malformed or mismatched",
    )
}

fn valid_checksum(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

pub(super) fn checksum(bytes: &[u8], controls: Controls<'_>) -> Result<String, DomainError> {
    let mut hash = Sha256::new();
    for chunk in bytes.chunks(16 * 1024) {
        items::check(controls.0, controls.1)?;
        hash.update(chunk);
    }
    items::check(controls.0, controls.1)?;
    Ok(hash.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

fn check_checksum(bytes: &[u8], expected: &str, controls: Controls<'_>) -> Result<(), DomainError> {
    if !valid_checksum(expected) || checksum(bytes, controls)? != expected {
        return Err(integrity());
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct LockedPackage {
    name: String,
    version: String,
    source: String,
    checksum: String,
}

pub(super) struct Sources {
    pub home: PathBuf,
    pub observed: BTreeMap<PathBuf, Vec<u8>>,
    mirrored: BTreeSet<PathBuf>,
    pub registry_roots: BTreeSet<PathBuf>,
    pub vendor_roots: BTreeSet<PathBuf>,
    pub comparison: (usize, usize),
    home_config: String,
    locked: BTreeSet<LockedPackage>,
}

pub(super) fn cargo_home() -> Result<PathBuf, DomainError> {
    let home = std::env::var_os("CARGO_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".cargo")))
        .ok_or_else(|| {
            refusal(
                "EXTERNAL_CONFIGURATION_UNSUPPORTED",
                "Cargo home unavailable",
            )
        })?;
    if !home.is_absolute() {
        return Err(refusal(
            "EXTERNAL_CONFIGURATION_UNSUPPORTED",
            "relative Cargo home is unsupported",
        ));
    }
    Ok(home)
}

fn unsupported() -> DomainError {
    refusal(
        "EXTERNAL_CONFIGURATION_UNSUPPORTED",
        "only literal source/registry configuration is admitted; wrappers, environment, git/local registries, inline or dotted tables are unsupported",
    )
}

// Deliberately a whitelist, not a TOML implementation. Require one simple
// [source.name]/[registries.name] header or string assignment per line. Keeping
// line boundaries prevents normalization from making invalid global config valid.
fn source_config(source: &str, base: &Path) -> Result<(String, Vec<PathBuf>), DomainError> {
    let mut section = Vec::new();
    let mut output = String::new();
    let mut directories = Vec::new();
    let mut keys = BTreeSet::new();
    for line in source.lines() {
        let tokens = manifest_tokens(line).map_err(|_| unsupported())?;
        if tokens.is_empty() {
            continue;
        }
        if tokens[0] == ("[".into(), false) {
            section = match tokens.as_slice() {
                [(open, false), (name, _), (close, false)]
                    if open == "[" && name == "source" && close == "]" =>
                {
                    vec![name.clone()]
                }
                [
                    (open, false),
                    (kind, _),
                    (dot, false),
                    (name, _),
                    (close, false),
                ] if open == "["
                    && matches!(kind.as_str(), "source" | "registries")
                    && dot == "."
                    && close == "]"
                    && !name.is_empty() =>
                {
                    vec![kind.clone(), name.clone()]
                }
                _ => return Err(unsupported()),
            };
            output.push('[');
            output.push_str(
                &section
                    .iter()
                    .map(|s| serde_json::to_string(s).expect("string"))
                    .collect::<Vec<_>>()
                    .join("."),
            );
            output.push_str("]\n");
        } else {
            let [(key, _), (equals, false), (value, true)] = tokens.as_slice() else {
                return Err(unsupported());
            };
            if section.len() != 2
                || equals != "="
                || !keys.insert((section.clone(), key.clone()))
                || !(section[0] == "source"
                    && matches!(key.as_str(), "directory" | "registry" | "replace-with")
                    || section[0] == "registries" && key == "index")
            {
                return Err(unsupported());
            }
            let value = if key == "directory" {
                let root = no_links(&base.join(value))?;
                directories.push(root.clone());
                root.to_str().ok_or_else(unsupported)?.to_string()
            } else {
                value.clone()
            };
            output.push_str(&format!(
                "{} = {}\n",
                serde_json::to_string(key).expect("string"),
                serde_json::to_string(&value).expect("string")
            ));
        }
    }
    Ok((output, directories))
}

fn lock_packages(bytes: &[u8]) -> Result<BTreeSet<LockedPackage>, DomainError> {
    let source = std::str::from_utf8(bytes).map_err(|_| unsupported())?;
    let tokens = manifest_tokens(source)?;
    let mut packages = BTreeSet::new();
    for (i, window) in tokens.windows(5).enumerate() {
        if window
            != [
                ("[".into(), false),
                ("[".into(), false),
                ("package".into(), false),
                ("]".into(), false),
                ("]".into(), false),
            ]
        {
            continue;
        }
        let end = (i + 5..tokens.len())
            .find(|&j| {
                tokens[j] == ("[".into(), false) && tokens.get(j + 1) == Some(&("[".into(), false))
            })
            .unwrap_or(tokens.len());
        let mut fields = BTreeMap::new();
        for entry in tokens[i + 5..end].windows(3) {
            if matches!(
                entry[0].0.as_str(),
                "name" | "version" | "source" | "checksum"
            ) && entry[1] == ("=".into(), false)
                && (!entry[2].1
                    || fields
                        .insert(entry[0].0.clone(), entry[2].0.clone())
                        .is_some())
            {
                return Err(unsupported());
            }
        }
        if let Some(source) = fields.remove("source") {
            if !source.starts_with("registry+") && !source.starts_with("sparse+") {
                return Err(refusal(
                    "EXTERNAL_DEPENDENCY_UNSUPPORTED",
                    "git and non-registry locked sources are unsupported, even when cached",
                ));
            }
            let checksum = fields.remove("checksum").ok_or_else(integrity)?;
            if !valid_checksum(&checksum) {
                return Err(integrity());
            }
            let package = LockedPackage {
                name: fields.remove("name").ok_or_else(unsupported)?,
                version: fields.remove("version").ok_or_else(unsupported)?,
                source,
                checksum,
            };
            // These strings select directory names, never paths or versions to resolve.
            if [&package.name, &package.version].iter().any(|s| {
                s.is_empty() || s.len() > 256 || s.contains(['/', '\\']) || s.as_str() == ".."
            }) || !package
                .name
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_'))
            {
                return Err(unsupported());
            }
            if packages.iter().any(|p: &LockedPackage| {
                p.name == package.name && p.version == package.version && p.source == package.source
            }) {
                return Err(refusal(
                    "EXTERNAL_PACKAGE_AMBIGUOUS",
                    "duplicate locked package identity",
                ));
            }
            packages.insert(package);
        }
    }
    Ok(packages)
}

fn entries(
    path: &Path,
    limits: &Limits,
    controls: Controls<'_>,
) -> Result<Vec<PathBuf>, DomainError> {
    let mut paths = Vec::new();
    for entry in fs::read_dir(path).map_err(io_error)? {
        items::check(controls.0, controls.1)?;
        let entry = entry.map_err(io_error)?;
        if paths.len() >= limits.max_files {
            return Err(refusal(
                "EXTERNAL_CORPUS_INCOMPLETE",
                "source directory entry cap exhausted",
            ));
        }
        if entry.file_type().map_err(io_error)?.is_symlink() {
            return Err(refusal(
                "EXTERNAL_ROOT_UNSUPPORTED",
                "source directory contains a symlink",
            ));
        }
        paths.push(entry.path());
    }
    paths.sort();
    Ok(paths)
}

impl Sources {
    fn remember(
        &mut self,
        path: &Path,
        limits: &Limits,
        controls: Controls<'_>,
    ) -> Result<(), DomainError> {
        if self.observed.contains_key(path) {
            return Ok(());
        }
        let bytes = read(path, limits.max_file_bytes, controls)?;
        if self.observed.len() >= limits.max_files
            || self.observed.values().map(Vec::len).sum::<usize>() + bytes.len()
                > limits.max_source_bytes
        {
            return Err(refusal(
                "EXTERNAL_CORPUS_INCOMPLETE",
                "Cargo input capture budget exhausted",
            ));
        }
        self.observed.insert(path.into(), bytes);
        Ok(())
    }

    fn mirror_tree(
        &mut self,
        root: &Path,
        mirror: bool,
        limits: &Limits,
        controls: Controls<'_>,
    ) -> Result<(), DomainError> {
        let mut pending = vec![root.to_path_buf()];
        let mut count = 0;
        while let Some(directory) = pending.pop() {
            for path in entries(&directory, limits, controls)? {
                count += 1;
                if count > limits.max_files {
                    return Err(refusal(
                        "EXTERNAL_CORPUS_INCOMPLETE",
                        "Cargo copy entry cap exhausted",
                    ));
                }
                if path.file_name().is_some_and(|n| n == ".git") {
                    return Err(refusal(
                        "EXTERNAL_CONFIGURATION_UNSUPPORTED",
                        "only locally cached sparse registry indices are admitted",
                    ));
                }
                if fs::symlink_metadata(&path).map_err(io_error)?.is_dir() {
                    pending.push(path);
                } else {
                    self.remember(&path, limits, controls)?;
                    if mirror {
                        self.mirrored.insert(path);
                    }
                }
            }
        }
        Ok(())
    }

    fn mirror_index(
        &mut self,
        index: &Path,
        limits: &Limits,
        controls: Controls<'_>,
    ) -> Result<(), DomainError> {
        let mut probes = 0;
        for registry in entries(index, limits, controls)? {
            if !registry.is_dir() {
                return Err(unsupported());
            }
            // Preserve Cargo's opaque cache bytes and registry-directory keys.
            // The documented index name layout only locates finite lock-selected
            // input files; it does not pick a registry, version, feature or edge.
            let mut files = vec![registry.join("config.json")];
            for package in &self.locked {
                let name = package.name.to_ascii_lowercase();
                let slot = match name.len() {
                    1 => format!("1/{name}"),
                    2 => format!("2/{name}"),
                    3 => format!("3/{}/{name}", &name[..1]),
                    _ => format!("{}/{}/{name}", &name[..2], &name[2..4]),
                };
                files.push(registry.join(".cache").join(slot));
            }
            for path in files {
                items::check(controls.0, controls.1)?;
                probes += 1;
                if probes > limits.max_files {
                    return Err(refusal(
                        "EXTERNAL_CORPUS_INCOMPLETE",
                        "sparse cache lookup cap exhausted",
                    ));
                }
                match fs::symlink_metadata(&path) {
                    Ok(_) => {
                        no_links(&path)?;
                        self.remember(&path, limits, controls)?;
                        self.mirrored.insert(path);
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => return Err(io_error(e)),
                }
            }
        }
        Ok(())
    }

    pub(super) fn prepare(
        directory: &Path,
        home: &Path,
        tree: &Path,
        observed: BTreeMap<PathBuf, Vec<u8>>,
        limits: &Limits,
        controls: Controls<'_>,
    ) -> Result<Self, DomainError> {
        let home = resolved(home)?;
        let state = resolved(tree)?;
        if state.starts_with(&home) || home.starts_with(&state) {
            return Err(refusal(
                "EXTERNAL_STATE_OVERLAP",
                "server state overlaps the caller Cargo home",
            ));
        }
        let mut result = Self {
            home,
            observed,
            mirrored: BTreeSet::new(),
            registry_roots: BTreeSet::new(),
            vendor_roots: BTreeSet::new(),
            comparison: (0, 0),
            home_config: String::new(),
            locked: BTreeSet::new(),
        };
        // Ancestor locks select a conservative finite copy, not the authoritative
        // workspace. Cargo identifies the actual workspace; bind_workspace then
        // requires its exact captured lock identity for evidence admission.
        let mut found_lock = false;
        for lock in directory.ancestors().map(|p| p.join("Cargo.lock")) {
            if let Some(bytes) = result.observed.get(&lock) {
                found_lock = true;
                result.locked.extend(lock_packages(bytes)?);
            }
        }
        if !found_lock {
            return Err(refusal(
                "EXTERNAL_DISCOVERY_FAILED",
                "an existing locked caller workspace is required",
            ));
        }
        let mut configs: Vec<_> = directory
            .ancestors()
            .map(|p| (p.join(".cargo"), p.to_path_buf(), false))
            .collect();
        configs.push((
            result.home.clone(),
            result.home.parent().ok_or_else(unsupported)?.to_path_buf(),
            true,
        ));
        let mut vendor_bases = BTreeSet::new();
        let mut home_selected = false;
        for (config_dir, base, is_home) in configs {
            for name in ["config", "config.toml"] {
                let path = config_dir.join(name);
                match fs::symlink_metadata(&path) {
                    Ok(_) => {
                        no_links(&config_dir)?;
                        disjoint(tree, [config_dir.clone()])?;
                        result.remember(&path, limits, controls)?;
                        let text = std::str::from_utf8(&result.observed[&path])
                            .map_err(|_| unsupported())?;
                        let (normalized, directories) = source_config(text, &base)?;
                        // Cargo prefers the extensionless file; checking both
                        // conservatively refuses unsafe shadowed configuration.
                        if is_home && !home_selected {
                            home_selected = true;
                            result.home_config = normalized;
                        }
                        for root in directories {
                            disjoint(tree, [root.clone()])?;
                            vendor_bases.insert(root);
                        }
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => return Err(io_error(e)),
                }
            }
        }
        let mut seeds = BTreeSet::new();
        for base in vendor_bases {
            let mut identities = BTreeSet::new();
            for root in entries(&base, limits, controls)? {
                if !root.is_dir() {
                    continue;
                }
                disjoint(tree, [root.clone()])?;
                let manifest = root.join("Cargo.toml");
                result.remember(&manifest, limits, controls)?;
                result.remember(&root.join(".cargo-checksum.json"), limits, controls)?;
                let (name, version) = package_identity(&result.observed[&manifest])?;
                if !identities.insert((name, version)) {
                    return Err(refusal(
                        "EXTERNAL_PACKAGE_AMBIGUOUS",
                        "directory source contains duplicate package name/version identities",
                    ));
                }
                seeds.insert(manifest);
                result.vendor_roots.insert(root);
            }
        }
        if !result.locked.is_empty() {
            // Registry layout is preserved, including Cargo's opaque registry
            // hashes. We never derive a hash from a URL or select a source here.
            let registry = result.home.join("registry");
            if registry.exists() {
                no_links(&registry)?;
                disjoint(tree, [result.home.clone()])?;
                let index = registry.join("index");
                if index.exists() {
                    result.mirror_index(&index, limits, controls)?;
                }
                let sources = registry.join("src");
                if sources.exists() {
                    let mut probes = 0;
                    for registry in entries(&sources, limits, controls)? {
                        if !registry.is_dir() {
                            return Err(unsupported());
                        }
                        for package in &result.locked.clone() {
                            items::check(controls.0, controls.1)?;
                            probes += 1;
                            if probes > limits.max_files {
                                return Err(refusal(
                                    "EXTERNAL_CORPUS_INCOMPLETE",
                                    "registry checkout lookup cap exhausted",
                                ));
                            }
                            let root =
                                registry.join(format!("{}-{}", package.name, package.version));
                            match fs::symlink_metadata(&root) {
                                Ok(_) => {
                                    disjoint(tree, [root.clone()])?;
                                    no_links(&root)?;
                                    seeds.insert(root.join("Cargo.toml"));
                                    result.registry_roots.insert(root);
                                }
                                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                                Err(e) => return Err(io_error(e)),
                            }
                        }
                    }
                }
            }
            if result.registry_roots.is_empty() && result.vendor_roots.is_empty() {
                return Err(refusal(
                    "EXTERNAL_PACKAGE_UNAVAILABLE",
                    "locked registry packages are not already checked out locally; downloads or reconstructing absent caller checkouts are not permitted",
                ));
            }
        }
        // Extend the existing path/workspace closure to every candidate, including
        // possible path edges inside registry/vendor manifests, BEFORE state writes.
        let closure = manifest_closure(seeds, tree, limits, controls)?;
        for (path, bytes) in closure {
            if let Some(prior) = result.observed.insert(path, bytes.clone())
                && prior != bytes
            {
                return Err(refusal(
                    "EXTERNAL_SOURCE_CHANGED",
                    "Cargo inputs changed during preflight",
                ));
            }
        }
        for root in result.vendor_roots.clone() {
            result.mirror_tree(&root, false, limits, controls)?;
        }
        for root in result.registry_roots.clone() {
            result.mirror_tree(&root, true, limits, controls)?;
            let hash = root
                .parent()
                .and_then(Path::file_name)
                .ok_or_else(unsupported)?;
            let archive = result.home.join("registry/cache").join(hash).join(format!(
                "{}.crate",
                root.file_name().ok_or_else(unsupported)?.to_string_lossy()
            ));
            if !archive.exists() {
                return Err(integrity());
            }
            no_links(&archive)?;
            result.remember(&archive, limits, controls)?;
            let actual = checksum(&result.observed[&archive], controls)?;
            if !result.locked.iter().any(|p| {
                root.file_name()
                    .is_some_and(|n| n == format!("{}-{}", p.name, p.version).as_str())
                    && p.checksum == actual
            }) {
                return Err(integrity());
            }
            result.mirrored.insert(archive);
        }
        if result.observed.len() > limits.max_files
            || result.observed.values().map(Vec::len).sum::<usize>() > limits.max_source_bytes
        {
            return Err(refusal(
                "EXTERNAL_CORPUS_INCOMPLETE",
                "combined discovery input cap exhausted",
            ));
        }
        result.recheck(limits, controls)?;
        Ok(result)
    }

    pub(super) fn recheck(
        &self,
        limits: &Limits,
        controls: Controls<'_>,
    ) -> Result<(), DomainError> {
        for (path, bytes) in &self.observed {
            if read(path, limits.max_file_bytes, controls)? != *bytes {
                return Err(refusal(
                    "EXTERNAL_SOURCE_CHANGED",
                    "captured Cargo inputs changed",
                ));
            }
        }
        Ok(())
    }

    pub(super) fn stage(&self, scratch: &Path, controls: Controls<'_>) -> Result<(), DomainError> {
        for path in &self.mirrored {
            items::check(controls.0, controls.1)?;
            let relative = path.strip_prefix(&self.home).map_err(|_| unsupported())?;
            // Do not seed registry/src: Cargo must reconstruct the selected
            // package from the validated compressed archive in private state.
            // A copied .cargo-ok/checkout would make comparison self-referential.
            if relative.starts_with("registry/src") {
                continue;
            }
            let target = scratch.join(relative);
            fs::create_dir_all(target.parent().expect("copy parent")).map_err(io_error)?;
            // Fresh independent files, never symlinks/hardlinks to caller storage.
            fs::write(target, &self.observed[path]).map_err(io_error)?;
        }
        if !self.home_config.is_empty() {
            fs::write(scratch.join("config.toml"), &self.home_config).map_err(io_error)?;
        }
        Ok(())
    }

    pub(super) fn verify_staged(
        &self,
        scratch: &Path,
        limits: &Limits,
        controls: Controls<'_>,
    ) -> Result<(usize, usize), DomainError> {
        // Freshly unpacked selected packages must match every captured regular
        // file, including Cargo's marker; checksum binding is checked separately.
        let mut count = self.observed.len();
        let mut total = self.observed.values().map(Vec::len).sum::<usize>();
        for root in &self.registry_roots {
            let staged_root =
                scratch.join(root.strip_prefix(&self.home).map_err(|_| unsupported())?);
            // Cargo may select a vendor replacement instead of this candidate.
            if !staged_root.exists() {
                continue;
            }
            let expected: BTreeSet<_> = self
                .mirrored
                .iter()
                .filter(|p| p.starts_with(root))
                .map(|p| {
                    p.strip_prefix(&self.home)
                        .expect("mirrored home path")
                        .to_path_buf()
                })
                .collect();
            let mut actual = BTreeSet::new();
            let mut pending = vec![staged_root];
            while let Some(directory) = pending.pop() {
                for path in entries(&directory, limits, controls)? {
                    count += 1;
                    if count > limits.max_files {
                        return Err(refusal(
                            "EXTERNAL_CORPUS_INCOMPLETE",
                            "staged package membership cap exhausted",
                        ));
                    }
                    if path.is_dir() {
                        pending.push(path);
                    } else {
                        let relative = path
                            .strip_prefix(scratch)
                            .map_err(|_| unsupported())?
                            .to_path_buf();
                        let original = self.home.join(&relative);
                        let data = read(&path, limits.max_file_bytes, controls)?;
                        total += data.len();
                        if total > limits.max_source_bytes {
                            return Err(refusal(
                                "EXTERNAL_CORPUS_INCOMPLETE",
                                "archive comparison byte cap exhausted",
                            ));
                        }
                        if self.observed.get(&original) != Some(&data) {
                            return Err(integrity());
                        }
                        actual.insert(relative);
                    }
                }
            }
            if expected != actual {
                return Err(integrity());
            }
        }
        Ok((
            count - self.observed.len(),
            total - self.observed.values().map(Vec::len).sum::<usize>(),
        ))
    }

    pub(super) fn restore_paths(
        &self,
        metadata: &mut Metadata,
        scratch: &Path,
    ) -> Result<(), DomainError> {
        for package in &mut metadata.packages {
            if let Ok(relative) = package.manifest_path.strip_prefix(scratch) {
                let original = self.home.join(relative);
                if !original
                    .parent()
                    .is_some_and(|p| self.registry_roots.contains(p))
                {
                    return Err(refusal(
                        "EXTERNAL_PACKAGE_UNAVAILABLE",
                        "Cargo selected a package not present in the captured local checkouts",
                    ));
                }
                package.manifest_path = original;
                for dependency in &mut package.dependencies {
                    if let Some(path) = &mut dependency.path
                        && let Ok(relative) = path.strip_prefix(scratch)
                    {
                        *path = self.home.join(relative);
                    }
                }
                for target in &mut package.targets {
                    let relative = target
                        .src_path
                        .strip_prefix(scratch)
                        .map_err(|_| unsupported())?;
                    target.src_path = self.home.join(relative);
                }
            }
        }
        Ok(())
    }

    pub(super) fn bind_graph_inputs(
        &self,
        libraries: &[&Package],
        tree: &Path,
        limits: &Limits,
        controls: Controls<'_>,
    ) -> Result<(), DomainError> {
        let mut seeds = BTreeSet::new();
        for package in libraries {
            items::check(controls.0, controls.1)?;
            if !self.observed.contains_key(&package.manifest_path) {
                return Err(refusal(
                    "EXTERNAL_ROOT_UNSUPPORTED",
                    "resolved transitive manifest was not captured before Cargo",
                ));
            }
            seeds.insert(package.manifest_path.clone());
        }
        // Cargo's resolved edges choose the seeds; lexical candidates still
        // supply the preflight authority for every path/workspace it can read.
        for (path, bytes) in manifest_closure(seeds, tree, limits, controls)? {
            if self.observed.get(&path) != Some(&bytes) {
                return Err(refusal(
                    "EXTERNAL_SOURCE_CHANGED",
                    "resolved manifest closure differs from preflight inputs",
                ));
            }
        }
        Ok(())
    }

    pub(super) fn bind_workspace(&mut self, workspace: &Path) -> Result<(), DomainError> {
        let bytes = self
            .observed
            .get(&workspace.join("Cargo.lock"))
            .ok_or_else(unsupported)?;
        self.locked = lock_packages(bytes)?;
        Ok(())
    }

    pub(super) fn provenance(
        &self,
        package: &Package,
        root: &Path,
        controls: Controls<'_>,
    ) -> Result<&'static str, DomainError> {
        let Some(source) = &package.source else {
            return Ok("path");
        };
        let locked = self
            .locked
            .iter()
            .find(|p| p.name == package.name && p.version == package.version && &p.source == source)
            .ok_or_else(|| {
                refusal(
                    "EXTERNAL_CONFIGURATION_MISMATCH",
                    "resolved registry package identity differs from captured lock",
                )
            })?;
        let manifest = self
            .observed
            .get(&root.join("Cargo.toml"))
            .ok_or_else(integrity)?;
        let (name, version) = package_identity(manifest)?;
        if name != locked.name || version != locked.version {
            return Err(integrity());
        }
        if self.registry_roots.contains(root) {
            let registry = root
                .parent()
                .and_then(Path::file_name)
                .ok_or_else(unsupported)?;
            let archive = self
                .home
                .join("registry/cache")
                .join(registry)
                .join(format!("{}-{}.crate", locked.name, locked.version));
            check_checksum(
                self.observed.get(&archive).ok_or_else(integrity)?,
                &locked.checksum,
                controls,
            )?;
            let name = locked.name.to_ascii_lowercase();
            let slot = match name.len() {
                1 => format!("1/{name}"),
                2 => format!("2/{name}"),
                3 => format!("3/{}/{name}", &name[..1]),
                _ => format!("{}/{}/{name}", &name[..2], &name[2..4]),
            };
            let index = self
                .home
                .join("registry/index")
                .join(registry)
                .join(".cache")
                .join(slot);
            let bytes = self.observed.get(&index).ok_or_else(integrity)?;
            // Only Cargo's existing version-3 sparse cache encoding is admitted.
            // Its selected source/key comes from Cargo's unpacked manifest path,
            // not a URL-to-directory hash invented by this engine.
            if !bytes.starts_with(&[3, 2, 0, 0, 0]) {
                return Err(integrity());
            }
            let mut parts = bytes[5..].split(|b| *b == 0);
            let _etag = parts.next().ok_or_else(integrity)?;
            let mut found = false;
            while let Some(version) = parts.next() {
                if version.is_empty() && parts.next().is_none() {
                    break;
                }
                let entry: serde_json::Value =
                    serde_json::from_slice(parts.next().ok_or_else(integrity)?)
                        .map_err(|_| integrity())?;
                items::check(controls.0, controls.1)?;
                if version == locked.version.as_bytes() {
                    if found
                        || entry["name"] != locked.name
                        || entry["vers"] != locked.version
                        || entry["cksum"] != locked.checksum
                    {
                        return Err(integrity());
                    }
                    found = true;
                }
            }
            if !found {
                return Err(integrity());
            }
            Ok("registry_cache")
        } else if self.vendor_roots.contains(root) {
            let checksum_path = root.join(".cargo-checksum.json");
            let map: VendorChecksums =
                serde_json::from_slice(self.observed.get(&checksum_path).ok_or_else(integrity)?)
                    .map_err(|_| integrity())?;
            if map.package != locked.checksum {
                return Err(integrity());
            }
            let mut files = BTreeSet::new();
            for (path, bytes) in self
                .observed
                .iter()
                .filter(|(p, _)| p.starts_with(root) && **p != checksum_path)
            {
                items::check(controls.0, controls.1)?;
                let relative = path
                    .strip_prefix(root)
                    .map_err(|_| integrity())?
                    .to_str()
                    .ok_or_else(integrity)?;
                check_checksum(
                    bytes,
                    map.files.0.get(relative).ok_or_else(integrity)?,
                    controls,
                )?;
                files.insert(relative);
            }
            if files != map.files.0.keys().map(String::as_str).collect() {
                return Err(integrity());
            }
            Ok("source_replacement")
        } else {
            Err(refusal(
                "EXTERNAL_PACKAGE_UNAVAILABLE",
                "resolved package root was not captured before Cargo",
            ))
        }
    }
}

fn package_identity(bytes: &[u8]) -> Result<(String, String), DomainError> {
    let tokens = manifest_tokens(std::str::from_utf8(bytes).map_err(|_| integrity())?)?;
    let start = tokens
        .windows(3)
        .position(|w| {
            w == [
                ("[".into(), false),
                ("package".into(), false),
                ("]".into(), false),
            ]
        })
        .ok_or_else(integrity)?
        + 3;
    let end = tokens[start..]
        .iter()
        .position(|t| t == &("[".into(), false))
        .map_or(tokens.len(), |i| start + i);
    let mut fields = BTreeMap::new();
    for w in tokens[start..end]
        .windows(3)
        .filter(|w| matches!(w[0].0.as_str(), "name" | "version") && w[1] == ("=".into(), false))
    {
        if !w[2].1 || fields.insert(w[0].0.clone(), w[2].0.clone()).is_some() {
            return Err(integrity());
        }
    }
    Ok((
        fields.remove("name").ok_or_else(integrity)?,
        fields.remove("version").ok_or_else(integrity)?,
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct VendorChecksums {
    package: String,
    files: ChecksumFiles,
}
struct ChecksumFiles(BTreeMap<String, String>);
impl<'de> Deserialize<'de> for ChecksumFiles {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Visitor;
        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = ChecksumFiles;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("a unique file-to-SHA-256 map")
            }
            fn visit_map<M: serde::de::MapAccess<'de>>(
                self,
                mut map: M,
            ) -> Result<Self::Value, M::Error> {
                let mut files = BTreeMap::new();
                while let Some((path, checksum)) = map.next_entry::<String, String>()? {
                    if !valid_checksum(&checksum) || files.insert(path, checksum).is_some() {
                        return Err(serde::de::Error::custom(
                            "malformed or duplicate checksum entry",
                        ));
                    }
                }
                Ok(ChecksumFiles(files))
            }
        }
        deserializer.deserialize_map(Visitor)
    }
}
