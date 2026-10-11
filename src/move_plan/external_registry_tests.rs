use super::super::tests::fixture_gen::{Fixture, observe};
use super::*;
use serde_json::{Value, json};

const REGISTRY: &str = "index.crates.io-1949cf8c6b5b557f";
const SOURCE: &str = "registry+https://github.com/rust-lang/crates.io-index";
const LIBRARY: &str = "pub struct Thing; impl Thing { pub fn new() -> Self { Self } pub fn configure(&self) -> u32 { 7 } }\n";
const SELECTED: &str = "pub fn selected() -> u32 { dep::Thing::new().configure() }";

fn sha256(bytes: &[u8]) -> String {
    sources::checksum(
        bytes,
        (
            Instant::now() + Duration::from_secs(30),
            &AtomicBool::new(false),
        ),
    )
    .unwrap()
}
fn manifest(version: &str) -> String {
    format!(
        "[package]\nname='cached-library'\nversion='{version}'\nedition='2024'\n[lib]\npath='src/lib.rs'\n"
    )
}
fn package_archive(version: &str) -> Vec<u8> {
    // Reuse identical compressed bytes for the same package in all synthetic
    // source replacements; tar timestamps otherwise make package hashes differ.
    static ARCHIVES: std::sync::OnceLock<std::sync::Mutex<BTreeMap<String, Vec<u8>>>> =
        std::sync::OnceLock::new();
    let mut archives = ARCHIVES.get_or_init(Default::default).lock().unwrap();
    archives
        .entry(version.into())
        .or_insert_with(|| {
            let source = Fixture::empty();
            let name = format!("cached-library-{version}");
            source.write(&format!("{name}/Cargo.toml"), &manifest(version));
            source.write(&format!("{name}/src/lib.rs"), LIBRARY);
            let output = Command::new("tar")
                .env("COPYFILE_DISABLE", "1")
                .args(["-czf", "-"])
                .arg("-C")
                .arg(&source.0)
                .arg(&name)
                .output()
                .unwrap();
            assert!(output.status.success());
            output.stdout
        })
        .clone()
}
fn vendor_checksums(root: &Path, package: &str) {
    let mut files = BTreeMap::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(path) = pending.pop() {
        for entry in fs::read_dir(path).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                pending.push(path);
            } else if path.file_name().unwrap() != ".cargo-checksum.json" {
                files.insert(
                    path.strip_prefix(root)
                        .unwrap()
                        .to_str()
                        .unwrap()
                        .to_string(),
                    sha256(&fs::read(&path).unwrap()),
                );
            }
        }
    }
    fs::write(
        root.join(".cargo-checksum.json"),
        json!({"files":files,"package":package}).to_string(),
    )
    .unwrap();
}
fn package(home: &Fixture, version: &str, vendor: bool) -> PathBuf {
    let prefix = if vendor {
        format!("vendor/library-{version}")
    } else {
        format!("registry/src/{REGISTRY}/cached-library-{version}")
    };
    home.write(&format!("{prefix}/Cargo.toml"), &manifest(version));
    home.write(&format!("{prefix}/src/lib.rs"), LIBRARY);
    let root = home.0.join(&prefix);
    let bytes = package_archive(version);
    if vendor {
        vendor_checksums(&root, &sha256(&bytes));
    } else {
        home.write(&format!("{prefix}/.cargo-ok"), "{\"v\":1}");
        let archive = home.0.join(format!(
            "registry/cache/{REGISTRY}/cached-library-{version}.crate"
        ));
        fs::create_dir_all(archive.parent().unwrap()).unwrap();
        fs::write(archive, bytes).unwrap();
    }
    root
}
fn registry_index(home: &Fixture, versions: &[&str]) {
    home.write(
        &format!("registry/index/{REGISTRY}/config.json"),
        "{\"dl\":\"https://invalid.invalid/crates\",\"api\":\"https://invalid.invalid\"}",
    );
    // The fixture models Cargo's existing sparse cache, not a downloaded crate.
    // Cargo selects graph identity; runtime checks only its selected checksum row.
    let mut bytes = vec![3, 2, 0, 0, 0];
    bytes.extend_from_slice(b"etag: \"fixture\"\0");
    for version in versions {
        bytes.extend_from_slice(version.as_bytes());
        bytes.push(0);
        bytes.extend_from_slice(json!({"name":"cached-library","vers":version,"deps":[],"cksum":sha256(&package_archive(version)),"features":{},"yanked":false}).to_string().as_bytes());
        bytes.push(0);
    }
    let path = home.0.join(format!(
        "registry/index/{REGISTRY}/.cache/ca/ch/cached-library"
    ));
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, bytes).unwrap();
}
fn fixture(vendor: bool, duplicate_version: bool) -> (Fixture, Fixture, Fixture, Configuration) {
    let repo = Fixture::generate();
    let home = Fixture::empty();
    let state = Fixture::empty();
    package(if vendor { &repo } else { &home }, "1.0.0", vendor);
    if duplicate_version {
        package(if vendor { &repo } else { &home }, "2.0.0", vendor);
    }
    if vendor {
        repo.write(".cargo/config.toml", "[source.crates-io]\nreplace-with='local-copy'\n[source.local-copy]\ndirectory='vendor'\n");
    } else {
        registry_index(
            &home,
            if duplicate_version {
                &["1.0.0", "2.0.0"]
            } else {
                &["1.0.0"]
            },
        );
    }
    repo.write("Cargo.toml", &format!("[package]\nname='caller'\nversion='0.1.0'\nedition='2024'\n[dependencies]\ndep={{package='cached-library',version='=1.0.0'}}\n{}", if duplicate_version { "second={package='cached-library',version='=2.0.0'}\n" } else { "" }));
    repo.write("src/lib.rs", "mod source;\n");
    repo.write("src/source.rs", &format!("{SELECTED}\n"));
    // Setup is allowed to write only the synthetic home, before measurement.
    let output = Command::new("cargo")
        .args(["generate-lockfile", "--offline"])
        .env("CARGO_HOME", &home.0)
        .env("RUSTUP_AUTO_INSTALL", "0")
        .current_dir(&repo.0)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let config = serde_json::from_value(json!({"crates":[{"name":"caller","root_file":"src/lib.rs","edition":"2024","features":[],"cfg":[],"dependencies":[]}]})).unwrap();
    (repo, home, state, config)
}
fn discover_fixture(
    repo: &Fixture,
    home: &Fixture,
    state: &Fixture,
    config: &Configuration,
) -> Result<Corpus, DomainError> {
    discover_at_home(
        &repo.0.canonicalize().unwrap(),
        "src/lib.rs",
        config,
        &Limits::default(),
        (
            Instant::now() + Duration::from_secs(30),
            &AtomicBool::new(false),
        ),
        &state.0,
        &home.0,
    )
}

#[test]
fn cached_registry_and_vendor_sources_have_exact_graph_identity_and_no_writes() {
    for vendor in [false, true] {
        let (repo, home, state, config) = fixture(vendor, true);
        let before = observe(&repo.0);
        let cache = observe(&home.0);
        let corpus = discover_fixture(&repo, &home, &state, &config).unwrap();
        let roots = &corpus.coverage.consulted_roots;
        assert_eq!(roots.len(), 2);
        assert_eq!(roots[0].source, SOURCE);
        assert_ne!(roots[0].package_id, roots[1].package_id);
        assert_eq!(roots[0].package, roots[1].package);
        assert_ne!(roots[0].version, roots[1].version);
        assert_ne!(roots[0].namespace, roots[1].namespace);
        assert_eq!(
            roots[0].provenance,
            if vendor {
                "source_replacement"
            } else {
                "registry_cache"
            }
        );
        assert_eq!(corpus.configuration.crates.len(), 3);
        assert_eq!(corpus.configuration.crates[0].dependencies.len(), 2);
        corpus
            .recheck((
                Instant::now() + Duration::from_secs(30),
                &AtomicBool::new(false),
            ))
            .unwrap();
        assert_eq!(observe(&repo.0), before);
        assert_eq!(observe(&home.0), cache);
        assert!(fs::read_dir(&state.0).unwrap().next().is_none());
    }
}

#[test]
fn registry_vendor_refusals_never_download_or_mutate_caller_storage() {
    for case in [
        "missing",
        "archive_missing",
        "locked_version_missing",
        "cache_marker",
        "wrong_version",
        "wrong_name",
        "stale_lock",
        "rename",
        "alias_conflict",
        "features",
        "duplicates",
        "git",
        "wrapper",
        "transitive",
        "index_missing",
        "symlink",
        "checksum_missing",
        "build_script",
        "proc_macro",
        "lock_duplicate",
        "global_config",
        "target",
    ] {
        let vendor = matches!(
            case,
            "duplicates" | "transitive" | "checksum_missing" | "build_script" | "proc_macro"
        );
        let (repo, home, state, mut config) = fixture(vendor, case == "locked_version_missing");
        let root = if vendor {
            repo.0.join("vendor/library-1.0.0")
        } else {
            home.0
                .join(format!("registry/src/{REGISTRY}/cached-library-1.0.0"))
        };
        match case {
            "missing" | "locked_version_missing" => {
                fs::rename(&root, root.with_extension("absent")).unwrap();
            }
            "cache_marker" => fs::write(root.join(".cargo-ok"), "invalid").unwrap(),
            "archive_missing" => {
                let archive = home.0.join(format!("registry/cache/{REGISTRY}/cached-library-1.0.0.crate"));
                fs::rename(&archive, archive.with_extension("absent")).unwrap();
            }
            "wrong_version" => {
                let text = fs::read_to_string(root.join("Cargo.toml")).unwrap();
                fs::write(
                    root.join("Cargo.toml"),
                    text.replace("version='1.0.0'", "version='9.0.0'"),
                )
                .unwrap();
            }
            "wrong_name" => {
                let text = fs::read_to_string(root.join("Cargo.toml")).unwrap();
                fs::write(
                    root.join("Cargo.toml"),
                    text.replace("name='cached-library'", "name='other-library'"),
                )
                .unwrap();
            }
            "stale_lock" => {
                let text = fs::read_to_string(repo.0.join("Cargo.lock")).unwrap();
                repo.write(
                    "Cargo.lock",
                    &text.replace(
                        "name = \"caller\"\nversion = \"0.1.0\"",
                        "name = \"caller\"\nversion = \"9.0.0\"",
                    ),
                );
            }
            "rename" => {
                let text = fs::read_to_string(repo.0.join("Cargo.toml")).unwrap();
                repo.write(
                    "Cargo.toml",
                    &text.replace("package='cached-library'", "package='missing-library'"),
                );
            }
            "alias_conflict" => config.crates[0].dependencies.push(DependencyInput {
                name: "dep".into(),
                crate_name: "caller".into(),
            }),
            "features" => config.crates[0].features.push("invented".into()),
            "duplicates" => {
                package(&repo, "1.0.0-copy", true);
                repo.write(
                    "vendor/library-1.0.0-copy/Cargo.toml",
                    "[package]\nname='cached-library'\nversion='1.0.0'\nedition='2024'\n",
                );
            }
            "git" => {
                let text = fs::read_to_string(repo.0.join("Cargo.lock")).unwrap();
                repo.write(
                    "Cargo.lock",
                    &text.replace(
                        SOURCE,
                        "git+https://invalid.invalid/repo#0000000000000000000000000000000000000000",
                    ),
                );
            }
            "wrapper" => repo.write(
                ".cargo/config.toml",
                "[env]\nCARGO_HOME={value='/forbidden',force=true}\n",
            ),
            "transitive" => {
                repo.write(
                    "vendor/leaf/Cargo.toml",
                    "[package]\nname='leaf'\nversion='0.1.0'\nedition='2024'\n",
                );
                repo.write("vendor/leaf/src/lib.rs", "pub struct Leaf;\n");
                vendor_checksums(&repo.0.join("vendor/leaf"), &sha256(b"synthetic leaf archive"));
                let text = fs::read_to_string(root.join("Cargo.toml")).unwrap();
                fs::write(
                    root.join("Cargo.toml"),
                    format!("{text}\n[dependencies]\nleaf='=0.1.0'\n"),
                )
                .unwrap();
                vendor_checksums(&root, &sha256(&package_archive("1.0.0")));
                let output = Command::new("cargo")
                    .args(["generate-lockfile", "--offline"])
                    .env("CARGO_HOME", &home.0)
                    .current_dir(&repo.0)
                    .output()
                    .unwrap();
                assert!(
                    output.status.success(),
                    "{}",
                    String::from_utf8_lossy(&output.stderr)
                );
            }
            "index_missing" => {
                fs::rename(
                    home.0.join("registry/index"),
                    home.0.join("registry/unused-index"),
                )
                .unwrap();
            }
            "symlink" => {
                std::os::unix::fs::symlink(root.join("src/lib.rs"), root.join("src/link.rs"))
                    .unwrap();
            }
            "checksum_missing" => {
                fs::rename(root.join(".cargo-checksum.json"), root.join("unused-checksum.json")).unwrap();
            }
            "build_script" => fs::write(root.join("build.rs"), "fn main() { std::fs::write(\"executed\", \"forbidden\").unwrap(); }").unwrap(),
            "proc_macro" => {
                let text = fs::read_to_string(root.join("Cargo.toml")).unwrap();
                fs::write(root.join("Cargo.toml"), format!("{text}\nproc-macro=true\n")).unwrap();
            }
            "lock_duplicate" => {
                let text = fs::read_to_string(repo.0.join("Cargo.lock")).unwrap();
                repo.write("Cargo.lock", &format!("{text}\n[[package]]\nname='cached-library'\nversion='1.0.0'\nsource='{SOURCE}'\nchecksum='{}'\n", sha256(&package_archive("1.0.0"))));
            }
            "global_config" => home.write("config.toml", "[source.crates-io]\nregistry='sparse+https://invalid.invalid/' replace-with='missing'\n"),
            "target" => {
                let text = fs::read_to_string(repo.0.join("Cargo.toml")).unwrap();
                repo.write("Cargo.toml", &text.replace("[dependencies]", "[target.'cfg(unix)'.dependencies]"));
            }
            _ => unreachable!(),
        }
        if matches!(case, "build_script" | "proc_macro") {
            vendor_checksums(&root, &sha256(&package_archive("1.0.0")));
        }
        let before = observe(&repo.0);
        let cache = observe(&home.0);
        if case == "transitive" {
            let corpus = discover_fixture(&repo, &home, &state, &config).unwrap();
            assert_eq!(corpus.coverage.consulted_roots.len(), 2);
            assert_eq!(corpus.coverage.consulted_roots[1].package, "leaf");
            corpus
                .recheck((
                    Instant::now() + Duration::from_secs(30),
                    &AtomicBool::new(false),
                ))
                .unwrap();
            assert_eq!(observe(&repo.0), before);
            assert_eq!(observe(&home.0), cache);
            assert!(fs::read_dir(&state.0).unwrap().next().is_none());
            continue;
        }
        let error = discover_fixture(&repo, &home, &state, &config)
            .err()
            .unwrap();
        assert_eq!(
            error.code,
            match case {
                "missing" | "archive_missing" | "cache_marker" | "wrong_version" | "wrong_name" =>
                    "EXTERNAL_PACKAGE_UNAVAILABLE",
                "features" | "alias_conflict" => "EXTERNAL_CONFIGURATION_MISMATCH",
                "duplicates" | "lock_duplicate" => "EXTERNAL_PACKAGE_AMBIGUOUS",
                "git" => "EXTERNAL_DEPENDENCY_UNSUPPORTED",
                "wrapper" | "build_script" | "proc_macro" | "global_config" | "target" =>
                    "EXTERNAL_CONFIGURATION_UNSUPPORTED",
                "symlink" => "EXTERNAL_ROOT_UNSUPPORTED",
                "checksum_missing" => "EXTERNAL_DEPENDENCY_UNREADABLE",
                _ => "EXTERNAL_DISCOVERY_FAILED",
            },
            "{case}: {error:?}"
        );
        eprintln!("{case}: {}", error.code);
        assert!(!root.join("executed").exists(), "{case}");
        assert_eq!(observe(&repo.0), before, "{case}");
        assert_eq!(observe(&home.0), cache, "{case}");
        assert!(fs::read_dir(&state.0).unwrap().next().is_none(), "{case}");
    }
}

#[test]
fn registry_vendor_preflight_overlap_and_nested_path_candidates_refuse_before_state() {
    for vendor in [false, true] {
        for nested in [false, true] {
            let (repo, home, _, config) = fixture(vendor, false);
            let root = if vendor {
                repo.0.join("vendor/library-1.0.0")
            } else {
                home.0
                    .join(format!("registry/src/{REGISTRY}/cached-library-1.0.0"))
            };
            let leaf = Fixture::empty();
            leaf.write("Cargo.toml", "[package]\nname='leaf'\nversion='0.1.0'\n");
            leaf.write("src/lib.rs", "pub struct Leaf;\n");
            if nested {
                let text = fs::read_to_string(root.join("Cargo.toml")).unwrap();
                fs::write(
                    root.join("Cargo.toml"),
                    format!(
                        "{text}\n[dependencies]\nleaf={{path={:?}}}\n",
                        leaf.0.canonicalize().unwrap().to_str().unwrap()
                    ),
                )
                .unwrap();
            }
            let tree = if nested { &leaf.0 } else { &root }.join("server-state/discovery");
            let before = observe(&repo.0);
            let cache = observe(&home.0);
            let dependency = observe(&leaf.0);
            let error = discover_at_home(
                &repo.0.canonicalize().unwrap(),
                "src/lib.rs",
                &config,
                &Limits::default(),
                (
                    Instant::now() + Duration::from_secs(30),
                    &AtomicBool::new(false),
                ),
                &tree,
                &home.0,
            )
            .err()
            .unwrap();
            assert_eq!(error.code, "EXTERNAL_STATE_OVERLAP");
            assert!(!tree.parent().unwrap().exists());
            assert_eq!(observe(&repo.0), before);
            assert_eq!(observe(&home.0), cache);
            assert_eq!(observe(&leaf.0), dependency);
        }
    }
}

#[test]
fn cargo_home_overlap_refuses_even_without_a_registry_cache() {
    for vendor in [false, true] {
        let (repo, home, _, config) = fixture(vendor, false);
        let tree = home.0.join("server-state/discovery");
        let before = observe(&repo.0);
        let cache = observe(&home.0);
        let error = discover_at_home(
            &repo.0.canonicalize().unwrap(),
            "src/lib.rs",
            &config,
            &Limits::default(),
            (
                Instant::now() + Duration::from_secs(30),
                &AtomicBool::new(false),
            ),
            &tree,
            &home.0,
        )
        .err()
        .unwrap();
        assert_eq!(error.code, "EXTERNAL_STATE_OVERLAP");
        assert!(!tree.parent().unwrap().exists());
        assert_eq!(observe(&repo.0), before);
        assert_eq!(observe(&home.0), cache);
    }
}

#[test]
fn registry_vendor_mutation_withholds_publication() {
    for vendor in [false, true] {
        for changed in [
            "source",
            "manifest",
            "config",
            "home_config",
            "index",
            "archive",
            "checksum",
            "membership",
        ] {
            if (vendor && matches!(changed, "index" | "archive"))
                || (!vendor && changed == "checksum")
            {
                continue;
            }
            let (repo, home, state, config) = fixture(vendor, false);
            let corpus = discover_fixture(&repo, &home, &state, &config).unwrap();
            let root = PathBuf::from(&corpus.coverage.consulted_roots[0].root);
            match changed {
                "source" => fs::write(root.join("src/lib.rs"), "pub struct Changed;\n").unwrap(),
                "manifest" => fs::write(root.join("Cargo.toml"), "invalid").unwrap(),
                "config" => {
                    repo.write(".cargo/config.toml", "[build]\nrustc-wrapper='forbidden'\n")
                }
                "home_config" => home.write("config.toml", "[build]\nrustc-wrapper='forbidden'\n"),
                "index" => home.write(&format!("registry/index/{REGISTRY}/config.json"), "{}"),
                "archive" => home.write(
                    &format!("registry/cache/{REGISTRY}/cached-library-1.0.0.crate"),
                    "changed archive",
                ),
                "checksum" => fs::write(root.join(".cargo-checksum.json"), "{}").unwrap(),
                "membership" => fs::write(root.join("src/new.rs"), "pub struct Added;\n").unwrap(),
                _ => unreachable!(),
            }
            assert!(
                corpus
                    .recheck((
                        Instant::now() + Duration::from_secs(30),
                        &AtomicBool::new(false)
                    ))
                    .is_err(),
                "{vendor}: {changed}"
            );
        }
    }
}

#[test]
fn offline_discovery_never_contacts_the_configured_download_endpoint() {
    let (repo, home, state, config) = fixture(false, false);
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    home.write(
        &format!("registry/index/{REGISTRY}/config.json"),
        &json!({"dl":format!("http://{}/crates", listener.local_addr().unwrap())}).to_string(),
    );
    let archive = home.0.join(format!(
        "registry/cache/{REGISTRY}/cached-library-1.0.0.crate"
    ));
    fs::rename(&archive, archive.with_extension("absent")).unwrap();
    let before = observe(&repo.0);
    let cache = observe(&home.0);
    assert_eq!(
        discover_fixture(&repo, &home, &state, &config)
            .err()
            .unwrap()
            .code,
        "EXTERNAL_PACKAGE_UNAVAILABLE"
    );
    assert_eq!(
        listener.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
    assert_eq!(observe(&repo.0), before);
    assert_eq!(observe(&home.0), cache);
    assert!(fs::read_dir(&state.0).unwrap().next().is_none());
}

#[test]
fn source_configuration_uses_cargo_home_paths_precedence_and_original_source_ids() {
    for mode in ["global", "ancestor", "named_registry"] {
        let (repo, home, state, config) = fixture(false, false);
        package(&home, "1.0.0", true);
        let relative = format!("{}/vendor", home.0.file_name().unwrap().to_str().unwrap());
        home.write("config", &format!("[source.crates-io]\nreplace-with='global-copy'\n[source.global-copy]\ndirectory='{relative}'\n"));
        // Both are safe; extensionless wins, even though the shadowed config
        // points at a missing replacement name.
        home.write(
            "config.toml",
            "[source.crates-io]\nreplace-with='shadowed-copy'\n",
        );
        if mode != "global" {
            package(&repo, "1.0.0", true);
            repo.write(".cargo/config.toml", "[source.crates-io]\nreplace-with='caller-copy'\n[source.caller-copy]\ndirectory='vendor'\n");
        }
        let alternate = "sparse+https://alternate.invalid/index/";
        if mode == "named_registry" {
            repo.write(".cargo/config.toml", &format!("[registries.alternate]\nindex='{alternate}'\n[source.crates-io]\nreplace-with='caller-copy'\n[source.alternate]\nregistry='{alternate}'\nreplace-with='caller-copy'\n[source.caller-copy]\ndirectory='vendor'\n"));
            let text = fs::read_to_string(repo.0.join("Cargo.toml")).unwrap();
            repo.write("Cargo.toml", &format!("{text}second={{package='cached-library',version='=1.0.0',registry='alternate'}}\n"));
            let output = Command::new("cargo")
                .args(["generate-lockfile", "--offline"])
                .env("CARGO_HOME", &home.0)
                .current_dir(&repo.0)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        let before = observe(&repo.0);
        let cache = observe(&home.0);
        let corpus = discover_fixture(&repo, &home, &state, &config).unwrap();
        let roots = &corpus.coverage.consulted_roots;
        assert_eq!(roots.len(), if mode == "named_registry" { 2 } else { 1 });
        let expected = if mode == "global" { &home } else { &repo }
            .0
            .join("vendor/library-1.0.0")
            .canonicalize()
            .unwrap();
        for root in roots {
            assert_eq!(root.root, expected.to_str().unwrap());
            assert_eq!(root.provenance, "source_replacement");
            assert_eq!(root.version, "1.0.0");
        }
        if mode == "named_registry" {
            assert_ne!(roots[0].package_id, roots[1].package_id);
            assert_ne!(roots[0].source, roots[1].source);
            assert_ne!(roots[0].namespace, roots[1].namespace);
            assert!(roots.iter().any(|r| r.source == alternate));
            assert!(roots.iter().any(|r| r.source == SOURCE));
        }
        corpus
            .recheck((
                Instant::now() + Duration::from_secs(30),
                &AtomicBool::new(false),
            ))
            .unwrap();
        assert_eq!(observe(&repo.0), before);
        assert_eq!(observe(&home.0), cache);
    }
}

#[test]
fn source_preflight_limits_cancellation_and_unsafe_configuration_never_create_state() {
    for case in [
        "files",
        "bytes",
        "file_bytes",
        "cancel",
        "dotted",
        "inline",
        "env",
        "git",
        "local",
        "multiline",
    ] {
        let (repo, home, state, config) = fixture(false, false);
        let mut limits = Limits::default();
        let cancel = AtomicBool::new(case == "cancel");
        match case {
            "files" => limits.max_files = 4,
            "bytes" => limits.max_source_bytes = 512,
            "file_bytes" => limits.max_file_bytes = 32,
            "dotted" => home.write("config.toml", "source.crates-io.replace-with='vendor'\n"),
            "inline" => home.write(
                "config.toml",
                "[source]\ncrates-io={replace-with='vendor'}\n",
            ),
            "env" => home.write("config.toml", "[env]\nMARKER='forbidden'\n"),
            "git" => home.write(
                "config.toml",
                "[source.crates-io]\ngit='https://invalid.invalid/'\n",
            ),
            "local" => home.write(
                "config.toml",
                "[source.crates-io]\nlocal-registry='registry'\n",
            ),
            "multiline" => home.write(
                "config.toml",
                "[source.crates-io]\nregistry='''sparse+https://invalid.invalid/\n'''\n",
            ),
            _ => {}
        }
        let tree = state.0.join("server-state/discovery");
        let before = observe(&repo.0);
        let cache = observe(&home.0);
        let error = discover_at_home(
            &repo.0.canonicalize().unwrap(),
            "src/lib.rs",
            &config,
            &limits,
            (Instant::now() + Duration::from_secs(30), &cancel),
            &tree,
            &home.0,
        )
        .err()
        .unwrap();
        assert_eq!(
            error.code,
            match case {
                "cancel" => "CANCELLED",
                "files" | "bytes" | "file_bytes" => "EXTERNAL_CORPUS_INCOMPLETE",
                _ => "EXTERNAL_CONFIGURATION_UNSUPPORTED",
            },
            "{case}"
        );
        assert!(!tree.parent().unwrap().exists(), "{case}");
        assert_eq!(observe(&repo.0), before);
        assert_eq!(observe(&home.0), cache);
    }
}

#[test]
fn scratch_package_mutation_cannot_substitute_discovery_inputs() {
    let (repo, home, state, _) = fixture(false, false);
    let controls = (
        Instant::now() + Duration::from_secs(30),
        &AtomicBool::new(false),
    );
    let directory = repo.0.canonicalize().unwrap();
    let observed = manifest_closure(
        ancestor_manifests(&directory, controls).unwrap(),
        &state.0,
        &Limits::default(),
        controls,
    )
    .unwrap();
    let mut sources = sources::Sources::prepare(
        &directory,
        &home.0,
        &state.0,
        observed,
        &Limits::default(),
        controls,
    )
    .unwrap();
    sources.bind_workspace(&directory).unwrap();
    let root = sources.registry_roots.iter().next().unwrap();
    let fake = Package {
        id: "untrusted".into(),
        name: "cached-library".into(),
        version: "9.0.0".into(),
        source: Some(SOURCE.into()),
        manifest_path: root.join("Cargo.toml"),
        targets: vec![],
        dependencies: vec![],
    };
    assert_eq!(
        sources.provenance(&fake, root, controls).unwrap_err().code,
        "EXTERNAL_CONFIGURATION_MISMATCH"
    );
    for case in ["bytes", "membership"] {
        let scratch = Scratch::new(&state.0).unwrap();
        sources.stage(&scratch.0, controls).unwrap();
        assert!(!scratch.0.join("registry/src").exists());
        // Deliberately materialize a copy only for this self-consistency test;
        // production stage never writes registry/src before Cargo unpacks it.
        for (path, bytes) in sources.observed.iter().filter(|(p, _)| p.starts_with(root)) {
            let target = scratch.0.join(path.strip_prefix(&sources.home).unwrap());
            fs::create_dir_all(target.parent().unwrap()).unwrap();
            fs::write(target, bytes).unwrap();
        }
        let staged = scratch.0.join(root.strip_prefix(&sources.home).unwrap());
        let relative = if case == "bytes" {
            "src/lib.rs"
        } else {
            "src/added.rs"
        };
        fs::write(staged.join(relative), "pub struct Other;\n").unwrap();
        assert_eq!(
            sources
                .verify_staged(&scratch.0, &Limits::default(), controls)
                .unwrap_err()
                .code,
            "EXTERNAL_PACKAGE_UNAVAILABLE"
        );
    }
}

#[test]
fn raw_sha256_checksums_and_cancellation() {
    assert_eq!(
        sha256(b""),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
    assert_eq!(
        sha256(b"abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    assert_eq!(
        sources::checksum(
            b"abc",
            (
                Instant::now() + Duration::from_secs(30),
                &AtomicBool::new(true)
            )
        )
        .unwrap_err()
        .code,
        "CANCELLED"
    );
}

#[test]
fn registry_archive_comparison_shares_capture_budgets() {
    for files in [false, true] {
        let (repo, home, state, _) = fixture(false, false);
        let limits = Limits::default();
        let controls = (
            Instant::now() + Duration::from_secs(30),
            &AtomicBool::new(false),
        );
        let directory = repo.0.canonicalize().unwrap();
        let observed = manifest_closure(
            ancestor_manifests(&directory, controls).unwrap(),
            &state.0,
            &limits,
            controls,
        )
        .unwrap();
        let mut sources =
            sources::Sources::prepare(&directory, &home.0, &state.0, observed, &limits, controls)
                .unwrap();
        let mut limited = limits.clone();
        if files {
            limited.max_files = sources.observed.len();
        } else {
            limited.max_source_bytes = sources.observed.values().map(Vec::len).sum();
        }
        let repo_before = observe(&repo.0);
        let home_before = observe(&home.0);
        let error =
            metadata_with_sources(&directory, &state.0, Some(&mut sources), &limited, controls)
                .err()
                .unwrap();
        assert_eq!(error.code, "EXTERNAL_CORPUS_INCOMPLETE");
        assert_eq!(observe(&repo.0), repo_before);
        assert_eq!(observe(&home.0), home_before);
        assert!(fs::read_dir(&state.0).unwrap().next().is_none());
    }
}

#[test]
fn mixed_registry_path_sources_share_archive_comparison_budgets() {
    let (repo, home, state, config) = fixture(false, false);
    let path_library = Fixture::empty();
    path_library.write(
        "Cargo.toml",
        "[package]\nname='path-library'\nversion='0.1.0'\nedition='2024'\n",
    );
    for file in ["lib", "a", "b", "c", "d", "e"] {
        path_library.write(
            &format!("src/{file}.rs"),
            &format!("pub struct Local;\n// {}\n", "padding".repeat(128)),
        );
    }
    let manifest = fs::read_to_string(repo.0.join("Cargo.toml")).unwrap();
    repo.write(
        "Cargo.toml",
        &format!(
            "{manifest}local={{package='path-library',path={:?}}}\n",
            path_library.0.canonicalize().unwrap().to_str().unwrap()
        ),
    );
    let output = Command::new("cargo")
        .args(["generate-lockfile", "--offline"])
        .env("CARGO_HOME", &home.0)
        .current_dir(&repo.0)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        discover_fixture(&repo, &home, &state, &config)
            .unwrap()
            .coverage
            .consulted_roots
            .len(),
        2
    );
    let controls = (
        Instant::now() + Duration::from_secs(30),
        &AtomicBool::new(false),
    );
    let limits = Limits::default();
    let directory = repo.0.canonicalize().unwrap();
    let observed = manifest_closure(
        ancestor_manifests(&directory, controls).unwrap(),
        &state.0,
        &limits,
        controls,
    )
    .unwrap();
    let sources =
        sources::Sources::prepare(&directory, &home.0, &state.0, observed, &limits, controls)
            .unwrap();
    let path_bytes: usize = ["lib", "a", "b", "c", "d", "e"]
        .iter()
        .map(|file| {
            fs::read(path_library.0.join(format!("src/{file}.rs")))
                .unwrap()
                .len()
        })
        .sum();
    for files in [false, true] {
        let mut limited = limits.clone();
        // Initial capture plus either the path sources or archive comparison fits;
        // their combined charge must refuse, not reset the shared budget.
        if files {
            limited.max_files = sources.observed.len() + 6;
        } else {
            limited.max_source_bytes =
                sources.observed.values().map(Vec::len).sum::<usize>() + path_bytes;
        }
        let before = observe(&repo.0);
        let cache = observe(&home.0);
        let dependency = observe(&path_library.0);
        let error = discover_at_home(
            &directory,
            "src/lib.rs",
            &config,
            &limited,
            controls,
            &state.0,
            &home.0,
        )
        .err()
        .expect("archive comparison and path capture must share one budget");
        assert_eq!(error.code, "EXTERNAL_CORPUS_INCOMPLETE");
        assert_eq!(observe(&repo.0), before);
        assert_eq!(observe(&home.0), cache);
        assert_eq!(observe(&path_library.0), dependency);
        assert!(fs::read_dir(&state.0).unwrap().next().is_none());
    }
}

#[test]
fn checksum_mismatches_withhold_proofs_and_all_move_artifacts() {
    for (vendor, case) in [
        (false, "archive_bytes"),
        (false, "checkout_bytes"),
        (false, "checkout_extra_file"),
        (false, "checkout_missing_file"),
        (false, "lock_checksum_missing"),
        (false, "lock_checksum_malformed"),
        (false, "lock_checksum_wrong"),
        (false, "index_checksum_wrong"),
        (true, "vendor_bytes"),
        (true, "vendor_entry_wrong"),
        (true, "vendor_entry_missing"),
        (true, "vendor_entry_malformed"),
        (true, "vendor_entry_duplicate"),
        (true, "vendor_entry_extra"),
        (true, "vendor_map_malformed"),
        (true, "vendor_package_wrong"),
        (true, "vendor_package_missing"),
        (true, "vendor_extra_file"),
        (true, "vendor_manifest_identity"),
    ] {
        let (repo, home, state, _) = fixture(vendor, false);
        let root = if vendor {
            repo.0.join("vendor/library-1.0.0")
        } else {
            home.0
                .join(format!("registry/src/{REGISTRY}/cached-library-1.0.0"))
        };
        let archive = home.0.join(format!(
            "registry/cache/{REGISTRY}/cached-library-1.0.0.crate"
        ));
        let map_path = root.join(".cargo-checksum.json");
        let wrong = sha256(b"not the selected package");
        match case {
            "archive_bytes" => fs::write(&archive, b"altered compressed bytes").unwrap(),
            "checkout_bytes" | "vendor_bytes" => fs::write(
                root.join("src/lib.rs"),
                format!("{LIBRARY}// altered locally\n"),
            )
            .unwrap(),
            "checkout_extra_file" | "vendor_extra_file" => {
                fs::write(root.join("unexpected.txt"), b"not in the package").unwrap()
            }
            "checkout_missing_file" => {
                fs::rename(root.join("src/lib.rs"), root.join("src/other.rs")).unwrap()
            }
            "lock_checksum_missing" | "lock_checksum_malformed" | "lock_checksum_wrong" => {
                let text = fs::read_to_string(repo.0.join("Cargo.lock")).unwrap();
                let prior = sha256(&package_archive("1.0.0"));
                let replacement = match case {
                    "lock_checksum_missing" => text
                        .lines()
                        .filter(|line| !line.starts_with("checksum = "))
                        .collect::<Vec<_>>()
                        .join("\n"),
                    "lock_checksum_malformed" => text.replace(&prior, "not-a-sha256"),
                    _ => text.replace(&prior, &wrong),
                };
                repo.write("Cargo.lock", &replacement);
            }
            "index_checksum_wrong" => {
                let index = home.0.join(format!(
                    "registry/index/{REGISTRY}/.cache/ca/ch/cached-library"
                ));
                let text = String::from_utf8(fs::read(&index).unwrap()).unwrap();
                fs::write(
                    index,
                    text.replace(&sha256(&package_archive("1.0.0")), &wrong),
                )
                .unwrap();
            }
            "vendor_map_malformed" => fs::write(&map_path, b"invalid JSON").unwrap(),
            "vendor_manifest_identity" => {
                fs::write(root.join("Cargo.toml"), manifest("9.0.0")).unwrap();
                vendor_checksums(&root, &sha256(&package_archive("1.0.0")));
            }
            _ => {
                let mut map: Value = serde_json::from_slice(&fs::read(&map_path).unwrap()).unwrap();
                match case {
                    "vendor_entry_wrong" => map["files"]["src/lib.rs"] = json!(wrong),
                    "vendor_entry_missing" => {
                        map["files"].as_object_mut().unwrap().remove("src/lib.rs");
                    }
                    "vendor_entry_malformed" => map["files"]["src/lib.rs"] = json!("invalid hash"),
                    "vendor_entry_extra" => map["files"]["absent.txt"] = json!(wrong),
                    "vendor_package_wrong" => map["package"] = json!(wrong),
                    "vendor_package_missing" => {
                        map.as_object_mut().unwrap().remove("package");
                    }
                    "vendor_entry_duplicate" => {
                        let digest = map["files"]["src/lib.rs"].as_str().unwrap();
                        let text = map.to_string().replace(
                            "\"src/lib.rs\":",
                            &format!("\"src/lib.rs\":\"{digest}\",\"src/lib.rs\":"),
                        );
                        fs::write(&map_path, text).unwrap();
                        // Keep the two identical entries: ordinary JSON Value
                        // would erase the duplicate and conceal this regression.
                        let output = refusal_worker(&repo, &home, &state, case);
                        eprintln!("{output}");
                        continue;
                    }
                    _ => unreachable!(),
                }
                fs::write(&map_path, map.to_string()).unwrap();
            }
        }
        eprintln!("{}", refusal_worker(&repo, &home, &state, case));
    }
}
fn refusal_worker(repo: &Fixture, home: &Fixture, state: &Fixture, case: &str) -> String {
    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "move_plan::external::registry_tests::registry_vendor_worker",
            "--nocapture",
        ])
        .env("READ_ONLY_SOURCE_FIXTURE", &repo.0)
        .env("CHECKSUM_REFUSAL_CASE", case)
        .env("CARGO_HOME", &home.0)
        .env("XDG_DATA_HOME", &state.0)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{case}\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

// Run production Engine entry points in isolated test processes rather than
// mutating this process's environment (other tests run concurrently).
#[test]
fn registry_vendor_worker() {
    let Some(root) = std::env::var_os("READ_ONLY_SOURCE_FIXTURE") else {
        return;
    };
    let repo = PathBuf::from(root);
    let home = sources::cargo_home().unwrap();
    let before = observe(&repo);
    let cache = observe(&home);
    let source = fs::read_to_string(repo.join("src/source.rs")).unwrap();
    let selected = source.trim();
    let args = json!({"repo_path":repo,"crate_root":"src/lib.rs","paths":["src"],"resolve_semantic":true,"external_path_dependencies":true,"semantic_configuration":{"crates":[{"name":"caller","root_file":"src/lib.rs","edition":"2024","features":[],"cfg":[],"dependencies":[]}]},"limits":{"time_budget_ms":30000},"moves":[{"item":{"path":"src/source.rs","range":{"start_byte":0,"end_byte":selected.len()},"expected_text":selected},"destination":{"kind":"new_sibling","path":"src/moved.rs","parent_path":"src/lib.rs"}}]});
    let mut off = args.clone();
    off.as_object_mut()
        .unwrap()
        .remove("external_path_dependencies");
    let run = |args: Value| {
        serde_json::to_value(super::super::run(
            &repo,
            serde_json::from_value(args).unwrap(),
            &AtomicBool::new(false),
        ))
        .unwrap()
    };
    let implicit = run(off.clone());
    off["external_path_dependencies"] = json!(false);
    assert_eq!(implicit, run(off));
    let result = run(args.clone());
    if let Some(case) = std::env::var_os("CHECKSUM_REFUSAL_CASE") {
        assert!(!result["error"].is_null(), "{result}");
        assert!(
            result["error"]["code"]
                .as_str()
                .unwrap()
                .starts_with("EXTERNAL_"),
            "{result}"
        );
        assert_eq!(result["plan"]["applicable"], false, "{result}");
        assert!(
            result["plan"]["binding_proofs"]
                .as_array()
                .is_none_or(Vec::is_empty),
            "{result}"
        );
        assert!(
            result["coverage"]["ra_resolved"].as_u64().unwrap_or(0) == 0,
            "{result}"
        );
        for field in ["patch", "edits", "created_files"] {
            assert!(result["plan"][field].is_null(), "{result}");
        }
        assert_eq!(observe(&repo), before);
        assert_eq!(observe(&home), cache);
        println!(
            "{}: refused {}; zero proofs; all artifacts withheld; caller/cache unchanged",
            case.to_string_lossy(),
            result["error"]["code"]
        );
        return;
    }
    assert_eq!(result["plan"]["applicable"], true, "{result}");
    let proofs = result["plan"]["binding_proofs"].as_array().unwrap();
    assert!(
        proofs
            .iter()
            .any(|p| p["declaration"]["anchor"]["expected_text"] == "configure")
    );
    assert_eq!(result["coverage"]["ra_resolved"], proofs.len());
    let roots = result["coverage"]["external_dependencies"]["consulted_roots"]
        .as_array()
        .unwrap();
    let proof_roots: BTreeSet<_> = proofs
        .iter()
        .map(|p| {
            p["declaration"]["anchor"]["path"]
                .as_str()
                .unwrap()
                .split("/src/")
                .next()
                .unwrap()
        })
        .collect();
    assert_eq!(proof_roots.len(), roots.len());
    for root in roots {
        assert!(proof_roots.contains(root["namespace"].as_str().unwrap().trim_end_matches('/')));
        assert_eq!(root["package"], "cached-library");
        assert_eq!(root["source"], SOURCE);
        assert!(matches!(
            root["provenance"].as_str(),
            Some("registry_cache" | "source_replacement")
        ));
    }
    for proof in proofs {
        assert_eq!(proof["class"], "ra_resolved");
        assert_eq!(proof["classification"], "inherent_function");
        assert_eq!(proof["declaration"], proof["final_declaration"]);
        assert!(
            proof["declaration"]["anchor"]["path"]
                .as_str()
                .unwrap()
                .starts_with(PREFIX)
        );
        assert_eq!(proof["source_access"], true);
        assert_eq!(proof["final_access"], true);
    }
    let advice = crate::engine::Engine::new(repo.clone()).unwrap().suggest_split( serde_json::from_value(json!({"repo_path":repo,"crate_root":"src/lib.rs","source_path":"src/source.rs","paths":["src"],"external_path_dependencies":true,"semantic_configuration":args["semantic_configuration"],"limits":{"time_budget_ms":30000}})).unwrap(), &AtomicBool::new(false));
    let advice = serde_json::to_value(advice).unwrap();
    assert!(advice["error"].is_null(), "{advice}");
    assert_eq!(advice["integrity"]["semantic"], "not_performed");
    assert!(
        advice["coverage"]["external_dependencies"]["consulted_roots"]
            .as_array()
            .unwrap()
            .len()
            == roots.len()
    );
    assert!(
        advice["inventory"]
            .as_array()
            .unwrap()
            .iter()
            .all(|i| i["path"] == "src/source.rs")
    );
    for field in ["selection", "destination"] {
        let mut abuse = args.clone();
        if field == "selection" {
            abuse["moves"][0]["item"]["path"] = json!("@external/0/src/lib.rs");
        } else {
            abuse["moves"][0]["destination"]["path"] = json!("@external/0/src/new.rs");
        }
        let refused = run(abuse);
        assert_eq!(refused["plan"]["applicable"], false, "{refused}");
        assert!(refused["plan"]["patch"].is_null());
    }
    assert_eq!(observe(&repo), before);
    assert_eq!(observe(&home), cache);
    println!(
        "applicable both-overlay proof; advice semantic not_performed; caller and complete synthetic Cargo home hashes unchanged"
    );
    let changed = PathBuf::from(roots[0]["root"].as_str().unwrap()).join("src/lib.rs");
    let stale = super::super::run_with_recheck(
        &repo,
        serde_json::from_value(args).unwrap(),
        &AtomicBool::new(false),
        || {
            fs::write(changed, "pub struct Changed;\n").unwrap();
        },
    );
    assert_eq!(stale.error.unwrap().code, "EXTERNAL_SOURCE_CHANGED");
    assert!(!stale.plan.applicable);
    assert!(stale.plan.patch.is_none());
    assert!(stale.plan.edits.is_none());
    assert!(stale.plan.created_files.is_none());
    assert_eq!(
        observe(&repo).get(Path::new("src/source.rs")),
        before.get(Path::new("src/source.rs"))
    );
}
#[test]
fn registry_and_vendor_vertical_slices_use_production_entry_points() {
    for (vendor, two_versions) in [(false, false), (true, false), (false, true), (true, true)] {
        let (repo, home, state, _) = fixture(vendor, two_versions);
        if two_versions {
            repo.write("src/source.rs", "pub fn selected() -> u32 { dep::Thing::new().configure() + second::Thing::new().configure() }\n");
        }
        let output = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "move_plan::external::registry_tests::registry_vendor_worker",
                "--nocapture",
            ])
            .env("READ_ONLY_SOURCE_FIXTURE", &repo.0)
            .env("CARGO_HOME", &home.0)
            .env("XDG_DATA_HOME", &state.0)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "vendor={vendor}\n{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        eprintln!("{}", String::from_utf8_lossy(&output.stdout));
    }
}
