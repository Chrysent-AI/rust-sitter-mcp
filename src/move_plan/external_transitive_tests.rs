use super::super::tests::{
    fixture_gen::{Fixture, observe},
    move_artifacts,
};
use super::*;
use serde_json::{Value, json};

const REGISTRY: &str = "index.crates.io-1949cf8c6b5b557f";
const SOURCE: &str = "registry+https://github.com/rust-lang/crates.io-index";
const SELECTED: &str = "pub fn selected() -> u32 { dep::Thing::new().configure() }";
const LEAF: &str = "pub struct Thing; impl Thing { pub fn new() -> Self { Self } pub fn configure(&self) -> u32 { 7 } }\n";
fn configuration() -> Configuration {
    serde_json::from_value(json!({"crates":[{"name":"caller","root_file":"src/lib.rs","edition":"2024","features":[],"cfg":[],"dependencies":[]}]})).unwrap()
}
fn lock(repo: &Fixture, home: &Fixture) {
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
fn sha(bytes: &[u8]) -> String {
    sources::checksum(
        bytes,
        (
            Instant::now() + Duration::from_secs(30),
            &AtomicBool::new(false),
        ),
    )
    .unwrap()
}
fn vendor_map(root: &Path, checksum: &str) {
    let files = BTreeMap::from([
        (
            "Cargo.toml",
            sha(&fs::read(root.join("Cargo.toml")).unwrap()),
        ),
        (
            "src/lib.rs",
            sha(&fs::read(root.join("src/lib.rs")).unwrap()),
        ),
    ]);
    fs::write(
        root.join(".cargo-checksum.json"),
        json!({"files":files,"package":checksum}).to_string(),
    )
    .unwrap();
}
fn cached_package(
    home: &Fixture,
    name: &str,
    source: &str,
    dependencies: &str,
    deps: Value,
    vendor: bool,
) -> PathBuf {
    let manifest =
        format!("[package]\nname='{name}'\nversion='1.0.0'\nedition='2024'\n{dependencies}");
    let archive = Fixture::empty();
    let prefix = format!("{name}-1.0.0");
    archive.write(&format!("{prefix}/Cargo.toml"), &manifest);
    archive.write(&format!("{prefix}/src/lib.rs"), source);
    let output = Command::new("tar")
        .env("COPYFILE_DISABLE", "1")
        .args(["-czf", "-"])
        .arg("-C")
        .arg(&archive.0)
        .arg(&prefix)
        .output()
        .unwrap();
    assert!(output.status.success());
    let checksum = sha(&output.stdout);
    let prefix = if vendor {
        format!("vendor/{prefix}")
    } else {
        format!("registry/src/{REGISTRY}/{prefix}")
    };
    home.write(&format!("{prefix}/Cargo.toml"), &manifest);
    home.write(&format!("{prefix}/src/lib.rs"), source);
    let root = home.0.join(&prefix);
    if vendor {
        vendor_map(&root, &checksum);
    } else {
        home.write(&format!("{prefix}/.cargo-ok"), "{\"v\":1}");
        let archive_path = home
            .0
            .join(format!("registry/cache/{REGISTRY}/{name}-1.0.0.crate"));
        fs::create_dir_all(archive_path.parent().unwrap()).unwrap();
        fs::write(archive_path, output.stdout).unwrap();
        let entry = json!({"name":name,"vers":"1.0.0","deps":deps,"cksum":checksum,"features":{},"yanked":false});
        let mut bytes = vec![3, 2, 0, 0, 0];
        bytes.extend_from_slice(b"etag: \"fixture\"\0");
        bytes.extend_from_slice(b"1.0.0\0");
        bytes.extend_from_slice(entry.to_string().as_bytes());
        bytes.push(0);
        let path = home.0.join(format!(
            "registry/index/{REGISTRY}/.cache/{}/{}/{name}",
            &name[..2],
            &name[2..4]
        ));
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }
    root
}
// Each origin exercises the same production metadata/capture/overlay boundary.
fn fixture(origin: &str) -> (Fixture, Fixture, Fixture, Fixture, PathBuf, PathBuf) {
    let repo = Fixture::generate();
    let home = Fixture::empty();
    let state = Fixture::empty();
    let paths = Fixture::empty();
    let (bridge, leaf) = if origin == "path" {
        paths.write(
            "leaf/Cargo.toml",
            "[package]\nname='leaf-library'\nversion='1.0.0'\nedition='2024'\n",
        );
        paths.write("leaf/src/lib.rs", LEAF);
        paths.write("bridge/Cargo.toml", "[package]\nname='bridge-library'\nversion='1.0.0'\nedition='2024'\n[dependencies]\nleaf={package='leaf-library',path='../leaf'}\n");
        paths.write("bridge/src/lib.rs", "pub use leaf::Thing;\n");
        let bridge = paths.0.join("bridge");
        repo.write("Cargo.toml", &format!("[package]\nname='caller'\nversion='0.1.0'\nedition='2024'\n[dependencies]\ndep={{package='bridge-library',path={:?}}}\n", bridge.canonicalize().unwrap().to_str().unwrap()));
        (bridge, paths.0.join("leaf"))
    } else {
        let vendor = origin == "vendor";
        let holder = if vendor { &repo } else { &home };
        let leaf = cached_package(holder, "leaf-library", LEAF, "", json!([]), vendor);
        let deps = json!([{"name":"leaf","req":"=1.0.0","features":[],"optional":false,"default_features":true,"target":null,"kind":"normal","registry":null,"package":"leaf-library"}]);
        let bridge = cached_package(
            holder,
            "bridge-library",
            "pub use leaf::Thing;\n",
            "[dependencies]\nleaf={package='leaf-library',version='=1.0.0'}\n",
            deps,
            vendor,
        );
        if vendor {
            repo.write(".cargo/config.toml", "[source.crates-io]\nreplace-with='local-copy'\n[source.local-copy]\ndirectory='vendor'\n");
        } else {
            home.write(
                &format!("registry/index/{REGISTRY}/config.json"),
                "{\"dl\":\"https://invalid.invalid/crates\"}",
            );
        }
        if origin == "mixed" {
            paths.write("bridge/Cargo.toml", "[package]\nname='bridge-library'\nversion='1.0.0'\nedition='2024'\n[dependencies]\nleaf={package='leaf-library',version='=1.0.0'}\n");
            paths.write("bridge/src/lib.rs", "pub use leaf::Thing;\n");
            let bridge = paths.0.join("bridge");
            repo.write("Cargo.toml", &format!("[package]\nname='caller'\nversion='0.1.0'\nedition='2024'\n[dependencies]\ndep={{package='bridge-library',path={:?}}}\n", bridge.canonicalize().unwrap().to_str().unwrap()));
            (bridge, leaf)
        } else {
            repo.write("Cargo.toml", "[package]\nname='caller'\nversion='0.1.0'\nedition='2024'\n[dependencies]\ndep={package='bridge-library',version='=1.0.0'}\n");
            (bridge, leaf)
        }
    };
    repo.write("src/lib.rs", "mod source;\n");
    repo.write("src/source.rs", &format!("{SELECTED}\n"));
    lock(&repo, &home);
    (repo, home, state, paths, bridge, leaf)
}
fn discover_fixture(
    repo: &Fixture,
    home: &Fixture,
    state: &Fixture,
    limits: &Limits,
) -> Result<Corpus, DomainError> {
    discover_at_home(
        &repo.0.canonicalize().unwrap(),
        "src/lib.rs",
        &configuration(),
        limits,
        (
            Instant::now() + Duration::from_secs(30),
            &AtomicBool::new(false),
        ),
        &state.0,
        &home.0,
    )
}
fn request(repo: &Path) -> Value {
    let text = fs::read_to_string(repo.join("src/source.rs")).unwrap();
    let selected = text.trim();
    json!({"repo_path":repo,"crate_root":"src/lib.rs","paths":["src"],"resolve_semantic":true,"external_path_dependencies":true,"semantic_configuration":configuration(),"limits":{"time_budget_ms":30000},"moves":[{"item":{"path":"src/source.rs","range":{"start_byte":0,"end_byte":selected.len()},"expected_text":selected},"destination":{"kind":"new_sibling","path":"src/moved.rs","parent_path":"src/lib.rs"}}]})
}
#[test]
fn transitive_method_vertical_slices_are_applicable_and_compile_without_source_writes() {
    for origin in ["path", "registry", "vendor", "mixed"] {
        let (repo, home, state, paths, _, _) = fixture(origin);
        let output = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "move_plan::external::transitive_tests::transitive_worker",
                "--nocapture",
            ])
            .env("TRANSITIVE_SOURCE_FIXTURE", &repo.0)
            .env("TRANSITIVE_ORIGIN", origin)
            .env("CARGO_HOME", &home.0)
            .env("XDG_DATA_HOME", &state.0)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{origin}\n{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        eprintln!("{}", String::from_utf8_lossy(&output.stdout));
        assert!(!paths.0.join("target").exists());
    }
}
#[test]
fn transitive_worker() {
    let Some(repo) = std::env::var_os("TRANSITIVE_SOURCE_FIXTURE") else {
        return;
    };
    let repo = PathBuf::from(repo);
    let home = sources::cargo_home().unwrap();
    let before = observe(&repo);
    let cache = observe(&home);
    let corpus = discover(
        &repo.canonicalize().unwrap(),
        "src/lib.rs",
        &configuration(),
        &Limits::default(),
        (
            Instant::now() + Duration::from_secs(30),
            &AtomicBool::new(false),
        ),
    )
    .unwrap();
    let captured: Vec<_> = corpus
        .coverage
        .consulted_roots
        .iter()
        .map(|r| (PathBuf::from(&r.root), observe(Path::new(&r.root))))
        .collect();
    let run = |args: Value| {
        serde_json::to_value(super::super::run(
            &repo,
            serde_json::from_value(args).unwrap(),
            &AtomicBool::new(false),
        ))
        .unwrap()
    };
    let args = request(&repo);
    let mut off = args.clone();
    off.as_object_mut()
        .unwrap()
        .remove("external_path_dependencies");
    let implicit = serde_json::to_vec(&run(off.clone())).unwrap();
    off["external_path_dependencies"] = json!(false);
    assert_eq!(
        implicit,
        serde_json::to_vec(&run(off)).unwrap(),
        "default-off bytes"
    );
    let value = run(args.clone());
    assert_eq!(value["plan"]["applicable"], true, "{value}");
    let coverage = &value["coverage"]["external_dependencies"];
    let roots = coverage["consulted_roots"].as_array().unwrap();
    assert_eq!(roots.len(), 2);
    assert_eq!(roots[0]["package"], "bridge-library");
    assert_eq!(roots[1]["package"], "leaf-library");
    assert_eq!(roots[1]["version"], "1.0.0");
    assert_ne!(roots[0]["package_id"], roots[1]["package_id"]);
    assert_ne!(roots[0]["namespace"], roots[1]["namespace"]);
    let origin = std::env::var("TRANSITIVE_ORIGIN").unwrap();
    assert_eq!(
        roots[1]["source"],
        if origin == "path" { "path" } else { SOURCE }
    );
    assert_eq!(
        roots[1]["provenance"],
        match origin.as_str() {
            "path" => "path",
            "registry" | "mixed" => "registry_cache",
            _ => "source_replacement",
        }
    );
    let usage = &coverage["capture_usage"];
    assert_eq!(usage["graph_edges"], 2);
    assert_eq!(usage["source_files_per_overlay"], 2);
    let source_bytes: u64 = roots.iter().map(|r| r["bytes"].as_u64().unwrap()).sum();
    assert_eq!(usage["source_bytes_per_overlay"], source_bytes);
    assert_eq!(coverage["corpus_digest"], corpus.coverage.corpus_digest);
    let proofs = value["plan"]["binding_proofs"].as_array().unwrap();
    assert!(!proofs.is_empty());
    assert_eq!(value["coverage"]["ra_resolved"], proofs.len());
    for proof in proofs {
        assert_eq!(proof["class"], "ra_resolved");
        assert_eq!(proof["classification"], "inherent_function");
        assert_eq!(proof["declaration"], proof["final_declaration"]);
        assert!(
            proof["declaration"]["anchor"]["path"]
                .as_str()
                .unwrap()
                .starts_with(roots[1]["namespace"].as_str().unwrap())
        );
        assert_eq!(proof["source_access"], true);
        assert_eq!(proof["final_access"], true);
        assert_eq!(proof["original_receiver"], proof["final_original_receiver"]);
    }
    let advice = crate::engine::Engine::new(repo.clone()).unwrap().suggest_split(serde_json::from_value(json!({"repo_path":repo,"crate_root":"src/lib.rs","source_path":"src/source.rs","paths":["src"],"external_path_dependencies":true,"semantic_configuration":configuration(),"limits":{"time_budget_ms":30000}})).unwrap(), &AtomicBool::new(false));
    let advice = serde_json::to_value(advice).unwrap();
    assert!(advice["error"].is_null());
    assert_eq!(advice["integrity"]["semantic"], "not_performed");
    assert_eq!(
        advice["coverage"]["external_dependencies"]["consulted_roots"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(observe(&repo), before);
    assert_eq!(observe(&home), cache);
    for (root, captured) in &captured {
        assert_eq!(&observe(root), captured);
    }
    // The server doesn't compile. Independently apply/check in a disposable copy
    // with a separate writable test Cargo home and target, never the real cache.
    let borrowed = std::mem::ManuallyDrop::new(Fixture(repo.clone()));
    let copy = move_artifacts::apply(&borrowed, &value);
    let borrowed_home = std::mem::ManuallyDrop::new(Fixture(home.clone()));
    let build_home = borrowed_home.copy();
    let output = Command::new("cargo")
        .args(["check", "--offline", "--locked"])
        .env("CARGO_HOME", &build_home.0)
        .env("CARGO_TARGET_DIR", copy.0.join("target"))
        .env_remove("XDG_DATA_HOME")
        .current_dir(&copy.0)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(observe(&repo), before);
    assert_eq!(observe(&home), cache);
    for (root, captured) in &captured {
        assert_eq!(&observe(root), captured);
    }
    println!(
        "{origin}: transitive reexport both-overlay proof, roots=2 files/overlay=2 bytes/overlay={source_bytes}; default-off byte equivalence; patch/JSON match; independent cargo check passes; all caller/source/cache trees unchanged"
    );
    let flag = AtomicBool::new(false);
    let stale =
        super::super::run_with_recheck(&repo, serde_json::from_value(args).unwrap(), &flag, || {
            flag.store(true, Ordering::Relaxed)
        });
    assert_eq!(stale.error.unwrap().code, "CANCELLED");
    assert!(!stale.plan.applicable);
    assert!(stale.plan.binding_proofs.is_empty());
    assert_eq!(stale.coverage.ra_resolved, 0);
    assert!(stale.plan.patch.is_none());
}

#[test]
fn required_transitive_inputs_and_mid_closure_budgets_refuse_without_writes() {
    for origin in ["path", "registry", "vendor"] {
        for case in ["missing_root", "missing_source", "files", "bytes"] {
            let (repo, home, state, paths, _, leaf) = fixture(origin);
            repo.write("src/lib.rs", "mod source;\nmod destination;\n");
            repo.write("src/destination.rs", "");
            let corpus = discover_fixture(&repo, &home, &state, &Limits::default()).unwrap();
            let mut limits = Limits::default();
            match case {
                "missing_root" => fs::rename(&leaf, repo.0.join("absent-leaf")).unwrap(),
                "missing_source" => {
                    fs::rename(leaf.join("src/lib.rs"), leaf.join("src/absent.rs")).unwrap()
                }
                "files" => {
                    limits.max_files = corpus.coverage.capture_usage.input_files
                        + corpus.coverage.capture_usage.archive_comparison_entries
                        + usize::from(origin == "path");
                    if origin != "path" {
                        limits.max_files -= 1;
                    }
                }
                "bytes" => {
                    limits.max_source_bytes = corpus.coverage.capture_usage.input_bytes
                        + corpus.coverage.capture_usage.archive_comparison_bytes;
                    if origin == "path" {
                        limits.max_source_bytes += "pub use leaf::Thing;\n".len();
                    } else {
                        limits.max_source_bytes -= 1;
                    }
                }
                _ => unreachable!(),
            }
            let before = observe(&repo.0);
            let cache = observe(&home.0);
            let external = observe(&paths.0);
            let error = discover_fixture(&repo, &home, &state, &limits)
                .err()
                .unwrap_or_else(|| panic!("{origin}/{case}: expected refusal"));
            assert!(
                error.code.starts_with("EXTERNAL_"),
                "{origin}/{case}: {error}"
            );
            if matches!(case, "files" | "bytes") {
                assert_eq!(
                    error.code, "EXTERNAL_CORPUS_INCOMPLETE",
                    "{origin}/{case}: {error}"
                );
                assert!(error.message.contains("cap") || error.message.contains("budget"));
                if origin == "path" {
                    assert!(
                        error.message.contains("closure captured 1 roots"),
                        "{error}"
                    );
                    assert!(error.message.contains("limits"), "{error}");
                }
            }
            eprintln!("{origin}/{case}: {error}");
            assert_eq!(observe(&repo.0), before);
            assert_eq!(observe(&home.0), cache);
            assert_eq!(observe(&paths.0), external);
            assert!(fs::read_dir(&state.0).unwrap().next().is_none());
            let output = Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "move_plan::external::transitive_tests::transitive_refusal_worker",
                    "--nocapture",
                ])
                .env("TRANSITIVE_REFUSAL_FIXTURE", &repo.0)
                .env("TRANSITIVE_MAX_FILES", limits.max_files.to_string())
                .env("TRANSITIVE_MAX_BYTES", limits.max_source_bytes.to_string())
                .env("CARGO_HOME", &home.0)
                .env("XDG_DATA_HOME", &state.0)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{origin}/{case}\n{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            eprintln!("{}", String::from_utf8_lossy(&output.stdout));
            assert_eq!(observe(&repo.0), before);
            assert_eq!(observe(&home.0), cache);
            assert_eq!(observe(&paths.0), external);
        }
    }
}
#[test]
fn transitive_freshness_gates_withhold_proofs_and_every_artifact() {
    for case in [
        "source",
        "membership",
        "manifest",
        "lock",
        "config",
        "cancel",
    ] {
        let (repo, _home, _state, paths, _bridge, leaf) = fixture("path");
        let before = observe(&repo.0);
        let flag = AtomicBool::new(false);
        let result = super::super::run_with_recheck(
            &repo.0,
            serde_json::from_value(request(&repo.0)).unwrap(),
            &flag,
            || match case {
                "source" => fs::write(leaf.join("src/lib.rs"), "pub struct Changed;\n").unwrap(),
                "membership" => {
                    fs::write(leaf.join("src/added.rs"), "pub struct Added;\n").unwrap()
                }
                "manifest" => {
                    let path = leaf.join("Cargo.toml");
                    let text = fs::read_to_string(&path).unwrap();
                    fs::write(path, format!("{text}# changed graph input\n")).unwrap();
                }
                "lock" => {
                    let path = repo.0.join("Cargo.lock");
                    let text = fs::read_to_string(&path).unwrap();
                    fs::write(path, format!("{text}# changed lock\n")).unwrap();
                }
                "config" => paths.write(
                    "leaf/.cargo/config.toml",
                    "[build]\nrustc-wrapper='forbidden'\n",
                ),
                "cancel" => flag.store(true, Ordering::Relaxed),
                _ => unreachable!(),
            },
        );
        assert!(result.error.is_some(), "{case}: no refusal");
        assert!(!result.plan.applicable);
        assert!(
            result.plan.binding_proofs.is_empty(),
            "{case}: stale proof published"
        );
        assert_eq!(result.coverage.ra_resolved, 0);
        assert!(result.plan.resolution_coverage.is_none());
        assert!(!result.coverage.scan.external_dependencies.unwrap().complete);
        assert!(result.plan.patch.is_none());
        assert!(result.plan.edits.is_none());
        assert!(result.plan.created_files.is_none());
        if case != "lock" {
            assert_eq!(observe(&repo.0), before);
        }
    }
}

#[test]
fn resolved_graph_cycles_missing_nodes_and_caps_are_typed_and_bounded() {
    let make = |edges: Value| {
        serde_json::from_value::<Metadata>(json!({"workspace_root":"/fixture","packages":[{"id":"caller","name":"caller","version":"1","source":null,"manifest_path":"/fixture/Cargo.toml","targets":[],"dependencies":[]},{"id":"leaf","name":"leaf","version":"1","source":null,"manifest_path":"/leaf/Cargo.toml","targets":[],"dependencies":[]}],"resolve":{"nodes":edges}})).unwrap()
    };
    for (edges, available, expected) in [
        (
            json!([{"id":"caller","features":[],"deps":[{"name":"leaf","pkg":"leaf","dep_kinds":[{"kind":null,"target":null}]}]},{"id":"leaf","features":[],"deps":[{"name":"caller","pkg":"caller","dep_kinds":[{"kind":null,"target":null}]}]}]),
            31,
            "EXTERNAL_ROOT_UNSUPPORTED",
        ),
        (
            json!([{"id":"caller","features":[],"deps":[{"name":"leaf","pkg":"leaf","dep_kinds":[{"kind":null,"target":null}]}]}]),
            31,
            "EXTERNAL_DISCOVERY_FAILED",
        ),
        (
            json!([{"id":"caller","features":[],"deps":[{"name":"leaf","pkg":"leaf","dep_kinds":[{"kind":null,"target":null}]}]},{"id":"leaf","features":[],"deps":[]}]),
            0,
            "EXTERNAL_CORPUS_INCOMPLETE",
        ),
    ] {
        let metadata = make(edges);
        let graph = graph::Graph::new(
            &metadata,
            (
                Instant::now() + Duration::from_secs(1),
                &AtomicBool::new(false),
            ),
        )
        .unwrap();
        let error = graph
            .closure(
                "caller",
                available,
                (
                    Instant::now() + Duration::from_secs(1),
                    &AtomicBool::new(false),
                ),
            )
            .err()
            .unwrap();
        assert_eq!(error.code, expected);
    }
}

#[test]
fn diamond_closures_deduplicate_ids_but_keep_transitive_versions_distinct() {
    for two_versions in [false, true] {
        let (repo, home, state, paths, _, _) = fixture("path");
        let leaf_path = if two_versions {
            "../other-leaf"
        } else {
            "../leaf"
        };
        if two_versions {
            paths.write(
                "other-leaf/Cargo.toml",
                "[package]\nname='leaf-library'\nversion='2.0.0'\nedition='2024'\n",
            );
            paths.write("other-leaf/src/lib.rs", LEAF);
        }
        paths.write("other/Cargo.toml", &format!("[package]\nname='other-bridge'\nversion='1.0.0'\nedition='2024'\n[dependencies]\nrenamed={{package='leaf-library',path='{leaf_path}'}}\n"));
        paths.write("other/src/lib.rs", "pub use renamed::Thing;\n");
        let text = fs::read_to_string(repo.0.join("Cargo.toml")).unwrap();
        repo.write(
            "Cargo.toml",
            &format!(
                "{text}other={{package='other-bridge',path={:?}}}\n",
                paths
                    .0
                    .join("other")
                    .canonicalize()
                    .unwrap()
                    .to_str()
                    .unwrap()
            ),
        );
        repo.write("src/source.rs", "pub fn selected() -> u32 { dep::Thing::new().configure() + other::Thing::new().configure() }\n");
        lock(&repo, &home);
        let before = observe(&repo.0);
        let external = observe(&paths.0);
        let corpus = discover_fixture(&repo, &home, &state, &Limits::default()).unwrap();
        assert_eq!(
            corpus.coverage.consulted_roots.len(),
            if two_versions { 4 } else { 3 }
        );
        assert_eq!(corpus.coverage.capture_usage.graph_edges, 4);
        let leaves: Vec<_> = corpus
            .coverage
            .consulted_roots
            .iter()
            .filter(|r| r.package == "leaf-library")
            .collect();
        assert_eq!(leaves.len(), if two_versions { 2 } else { 1 });
        if two_versions {
            assert_ne!(leaves[0].version, leaves[1].version);
            assert_ne!(leaves[0].namespace, leaves[1].namespace);
            assert_ne!(leaves[0].package_id, leaves[1].package_id);
        }
        let value = serde_json::to_value(super::super::run(
            &repo.0,
            serde_json::from_value(request(&repo.0)).unwrap(),
            &AtomicBool::new(false),
        ))
        .unwrap();
        assert_eq!(value["plan"]["applicable"], true, "{value}");
        let proofs = value["plan"]["binding_proofs"].as_array().unwrap();
        assert_eq!(value["coverage"]["ra_resolved"], proofs.len());
        let proof_namespaces: BTreeSet<_> = proofs
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
        assert_eq!(proof_namespaces.len(), leaves.len());
        for leaf in leaves {
            assert!(proof_namespaces.contains(leaf.namespace.trim_end_matches('/')));
        }
        let copy = move_artifacts::apply(&repo, &value);
        let output = Command::new("cargo")
            .args(["check", "--offline", "--locked"])
            .env("CARGO_HOME", &home.0)
            .env("CARGO_TARGET_DIR", copy.0.join("target"))
            .current_dir(&copy.0)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(observe(&repo.0), before);
        assert_eq!(observe(&paths.0), external);
    }
}

#[test]
fn blocked_external_plans_still_recheck_before_publishing_informational_proofs() {
    let (repo, home, _state, _paths, _, leaf) = fixture("path");
    fs::write(leaf.join("src/lib.rs"), "pub struct Thing { pub number: u32 } impl Thing { pub fn new() -> Self { Self { number: 7 } } }\n").unwrap();
    repo.write(
        "src/source.rs",
        "pub fn selected() -> u32 { dep::Thing::new().number }\n",
    );
    lock(&repo, &home);
    let args = request(&repo.0);
    let stable = super::super::run(
        &repo.0,
        serde_json::from_value(args.clone()).unwrap(),
        &AtomicBool::new(false),
    );
    assert!(!stable.plan.applicable);
    assert!(stable.error.is_none());
    assert!(
        !stable.plan.binding_proofs.is_empty(),
        "fixture must have informational inherent-method proof"
    );
    let result = super::super::run_with_recheck(
        &repo.0,
        serde_json::from_value(args).unwrap(),
        &AtomicBool::new(false),
        || fs::write(leaf.join("src/lib.rs"), "pub struct Changed;\n").unwrap(),
    );
    assert_eq!(result.error.unwrap().code, "EXTERNAL_SOURCE_CHANGED");
    assert!(result.plan.binding_proofs.is_empty());
    assert_eq!(result.coverage.ra_resolved, 0);
    assert!(result.plan.resolution_coverage.is_none());
    assert!(result.plan.patch.is_none());
    assert!(result.plan.edits.is_none());
    assert!(result.plan.created_files.is_none());
}

#[test]
fn transitive_graph_redirection_withholds_old_proofs_but_fresh_analysis_succeeds() {
    let (repo, home, _state, paths, bridge, _) = fixture("path");
    paths.write(
        "alternate/Cargo.toml",
        "[package]\nname='leaf-library'\nversion='2.0.0'\nedition='2024'\n",
    );
    paths.write("alternate/src/lib.rs", LEAF);
    let mut expected = None;
    let stale = super::super::run_with_recheck(
        &repo.0,
        serde_json::from_value(request(&repo.0)).unwrap(),
        &AtomicBool::new(false),
        || {
            let manifest = bridge.join("Cargo.toml");
            let source = fs::read_to_string(&manifest).unwrap();
            fs::write(
                &manifest,
                source.replace("path='../leaf'", "path='../alternate'"),
            )
            .unwrap();
            lock(&repo, &home);
            expected = Some((observe(&repo.0), observe(&home.0), observe(&paths.0)));
        },
    );
    assert_eq!(stale.error.unwrap().code, "EXTERNAL_SOURCE_CHANGED");
    assert!(!stale.plan.applicable);
    assert!(stale.plan.binding_proofs.is_empty());
    assert_eq!(stale.coverage.ra_resolved, 0);
    assert!(stale.plan.resolution_coverage.is_none());
    assert!(!stale.coverage.scan.external_dependencies.unwrap().complete);
    assert!(stale.plan.patch.is_none());
    assert!(stale.plan.edits.is_none());
    assert!(stale.plan.created_files.is_none());
    let expected = expected.unwrap();
    assert_eq!(
        (observe(&repo.0), observe(&home.0), observe(&paths.0)),
        expected
    );
    let fresh = super::super::run(
        &repo.0,
        serde_json::from_value(request(&repo.0)).unwrap(),
        &AtomicBool::new(false),
    );
    assert!(fresh.plan.applicable, "{:?}", fresh.error);
    let roots = &fresh
        .coverage
        .scan
        .external_dependencies
        .unwrap()
        .consulted_roots;
    assert_eq!(roots.len(), 2);
    assert_eq!(roots[1].version, "2.0.0");
    assert!(roots[1].root.ends_with("/alternate"));
    assert_eq!(
        (observe(&repo.0), observe(&home.0), observe(&paths.0)),
        expected
    );
}

#[test]
fn transitive_capture_cancellation_discards_the_unpublished_prefix() {
    struct CancellingFile<'a> {
        file: fs::File,
        cancelled: &'a AtomicBool,
        chunks: usize,
    }
    impl Read for CancellingFile<'_> {
        fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
            let n = self.file.read(bytes)?;
            self.chunks += 1;
            if self.chunks == 2 {
                self.cancelled.store(true, Ordering::Relaxed);
            }
            Ok(n)
        }
    }
    let (repo, home, state, paths, bridge, leaf) = fixture("path");
    fs::write(
        leaf.join("src/lib.rs"),
        format!("{LEAF}//{}\n", "x".repeat(48 * 1024)),
    )
    .unwrap();
    let before = (observe(&repo.0), observe(&home.0), observe(&paths.0));
    let flag = AtomicBool::new(false);
    let controls = (Instant::now() + Duration::from_secs(30), &flag);
    let limits = Limits::default();
    let mut texts = BTreeMap::new();
    let mut observed = BTreeMap::new();
    capture(
        &bridge,
        &bridge.join("src/lib.rs"),
        "@external/0/",
        &limits,
        controls,
        &mut texts,
        &mut observed,
    )
    .unwrap();
    let prefix = (texts.clone(), observed.clone());
    // Use the production chunk reader with real transitive source bytes and a
    // deterministic cancellation after its second chunk, not a timing race.
    let error = read_chunks(
        CancellingFile {
            file: fs::File::open(leaf.join("src/lib.rs")).unwrap(),
            cancelled: &flag,
            chunks: 0,
        },
        limits.max_file_bytes,
        controls,
    )
    .unwrap_err();
    assert_eq!(error.code, "CANCELLED");
    let error = capture(
        &leaf,
        &leaf.join("src/lib.rs"),
        "@external/1/",
        &limits,
        controls,
        &mut texts,
        &mut observed,
    )
    .unwrap_err();
    assert_eq!(error.code, "CANCELLED");
    assert_eq!((texts, observed), prefix);
    assert_eq!(
        (observe(&repo.0), observe(&home.0), observe(&paths.0)),
        before
    );
    assert!(fs::read_dir(&state.0).unwrap().next().is_none());
}

#[test]
fn unsafe_transitive_libraries_refuse_without_executing_or_writing_sources() {
    for case in [
        "build_script",
        "proc_macro",
        "target_edge",
        "path_override",
        "include",
        "syntax",
        "symlink",
    ] {
        let (repo, home, state, paths, _, leaf) = fixture("path");
        let manifest = fs::read_to_string(leaf.join("Cargo.toml")).unwrap();
        match case {
            "build_script" => paths.write(
                "leaf/build.rs",
                "fn main() { std::fs::write(\"executed-marker\", \"unexpected\").unwrap(); }\n",
            ),
            "proc_macro" => paths.write(
                "leaf/Cargo.toml",
                &format!("{manifest}[lib]\nproc-macro=true\n"),
            ),
            "target_edge" => {
                paths.write(
                    "spare/Cargo.toml",
                    "[package]\nname='spare-library'\nversion='1.0.0'\nedition='2024'\n",
                );
                paths.write("spare/src/lib.rs", "pub struct Spare;\n");
                paths.write("leaf/Cargo.toml", &format!("{manifest}[target.'cfg(unix)'.dependencies]\nspare={{package='spare-library',path='../spare'}}\n"));
            }
            "path_override" => paths.write("leaf/src/lib.rs", "#[path=\"other.rs\"] mod other;\n"),
            "include" => paths.write("leaf/src/lib.rs", "include!(\"other.rs\");\n"),
            "syntax" => paths.write("leaf/src/lib.rs", "pub struct {\n"),
            "symlink" => std::os::unix::fs::symlink("lib.rs", leaf.join("src/linked.rs")).unwrap(),
            _ => unreachable!(),
        }
        lock(&repo, &home);
        let before = (observe(&repo.0), observe(&home.0), observe(&paths.0));
        let error = discover_fixture(&repo, &home, &state, &Limits::default())
            .err()
            .unwrap();
        assert_eq!(
            error.code,
            if matches!(case, "build_script" | "proc_macro" | "target_edge") {
                "EXTERNAL_CONFIGURATION_UNSUPPORTED"
            } else {
                "EXTERNAL_ROOT_UNSUPPORTED"
            },
            "{case}: {error}"
        );
        assert_eq!(
            (observe(&repo.0), observe(&home.0), observe(&paths.0)),
            before,
            "{case}"
        );
        assert!(!leaf.join("executed-marker").exists());
        assert!(fs::read_dir(&state.0).unwrap().next().is_none());
    }
}

#[test]
fn transitive_refusal_worker() {
    let Some(repo) = std::env::var_os("TRANSITIVE_REFUSAL_FIXTURE") else {
        return;
    };
    let repo = PathBuf::from(repo);
    let home = sources::cargo_home().unwrap();
    let before = observe(&repo);
    let cache = observe(&home);
    let mut args = request(&repo);
    args["paths"] = json!(["src/lib.rs", "src/source.rs", "src/destination.rs"]);
    args["moves"][0]["destination"] = json!({"kind":"existing","path":"src/destination.rs"});
    args["limits"]["max_files"] = json!(
        std::env::var("TRANSITIVE_MAX_FILES")
            .unwrap()
            .parse::<usize>()
            .unwrap()
    );
    args["limits"]["max_source_bytes"] = json!(
        std::env::var("TRANSITIVE_MAX_BYTES")
            .unwrap()
            .parse::<usize>()
            .unwrap()
    );
    let result = super::super::run(
        &repo,
        serde_json::from_value(args).unwrap(),
        &AtomicBool::new(false),
    );
    let error = result.error.unwrap();
    assert!(error.code.starts_with("EXTERNAL_"), "{error}");
    assert!(!result.plan.applicable);
    assert!(result.plan.binding_proofs.is_empty());
    assert_eq!(result.coverage.ra_resolved, 0);
    assert!(result.plan.resolution_coverage.is_none());
    assert!(result.plan.patch.is_none());
    assert!(result.plan.edits.is_none());
    assert!(result.plan.created_files.is_none());
    assert_eq!(observe(&repo), before);
    assert_eq!(observe(&home), cache);
    println!(
        "production refusal: {error}; zero proofs; all artifacts withheld; caller/cache unchanged"
    );
}
