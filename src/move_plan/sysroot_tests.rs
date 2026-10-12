//! Small synthetic mechanism tests; installed-source feasibility is tested over stdio.
use super::super::tests::fixture_gen::{Fixture, observe};
use super::*;
use std::os::unix::fs::{PermissionsExt, symlink};

fn controls(cancelled: &AtomicBool) -> Controls<'_> {
    (Instant::now() + Duration::from_secs(60), cancelled)
}
fn empty() -> Fixture {
    let mut fixture = Fixture::empty();
    fixture.0 = fixture.0.canonicalize().unwrap();
    fixture
}
fn installed() -> (Fixture, PathBuf, Compiler) {
    let fixture = empty();
    let toolchain = fixture.0.join("toolchains/audited");
    fixture.write(
        "toolchains/audited/lib/rustlib/src/rust/library/lib.rs",
        "pub struct Thing;\n",
    );
    fixture.write(
        "toolchains/audited/lib/rustlib/components",
        "rust-src\nrustc-aarch64-apple-darwin\nrust-std-aarch64-apple-darwin\n",
    );
    fixture.write("toolchains/audited/lib/rustlib/multirust-channel-manifest.toml", "[pkg.rustc]\nversion = \"1.97.1 (8bab26f4f 2026-07-14)\"\n[pkg.rust-src]\nversion = \"1.97.1 (8bab26f4f 2026-07-14)\"\n");
    fixture.write(
        "toolchains/audited/lib/rustlib/manifest-rust-src",
        "file:lib/rustlib/src/rust/library/lib.rs\n",
    );
    let script = format!(
        "#!/bin/sh\ncase \"$1\" in\n-vV) printf 'release: 1.97.1\\ncommit-hash: 8bab26f4f68e0e26f0bb7960be334d5b520ea452\\nhost: aarch64-apple-darwin\\n';;\n--print) printf '%s\\n' '{}';;\n-Z) printf 'unix\\n';;\n*) exit 1;;\nesac\n",
        toolchain.display()
    );
    fixture.write("bin/rustc", &script);
    let executable = fixture.0.join("bin/rustc");
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o755)).unwrap();
    let compiler = Compiler {
        executable,
        rustup_home: fixture.0.clone(),
        selection: "audited".into(),
    };
    (fixture, toolchain, compiler)
}
fn corpus(toolchain: PathBuf, compiler: Compiler, cancelled: &AtomicBool) -> Corpus {
    let control = controls(cancelled);
    let mut coverage = Coverage::pending(&Limits::default());
    coverage.compiler = compiler
        .identify(&["-vV"], control, &mut coverage.capture_usage, false)
        .unwrap();
    coverage.source_root = toolchain
        .join("lib/rustlib/src/rust/library")
        .to_str()
        .unwrap()
        .into();
    let claims = claims::capture(&toolchain, &mut coverage, control, false).unwrap();
    let observed = capture(
        Path::new(&coverage.source_root),
        &mut coverage.capture_usage,
        control,
        false,
    )
    .unwrap();
    claims::membership(&toolchain, &claims, &observed).unwrap();
    coverage.corpus_digest = "synthetic-test-corpus".into();
    coverage.complete = true;
    let configuration = Configuration { crates: Vec::new() };
    Corpus {
        texts: BTreeMap::new(),
        configuration: configuration.clone(),
        coverage,
        profile: Profile {
            configuration,
            origins: BTreeMap::new(),
            cfg: BTreeMap::new(),
        },
        observed,
        toolchain_identity: identity(&fs::metadata(&toolchain).unwrap()),
        compiler_cfg: "unix\n".into(),
        toolchain,
        compiler,
        claims,
    }
}
#[test]
fn sysroot_capture_is_bounded_unsafe_forms_refuse_and_partial_usage_survives() {
    let fixture = empty();
    fixture.write("library/a.rs", "12345");
    fixture.write("library/b.rs", "67890");
    let cancelled = AtomicBool::new(false);
    for (entries, file_bytes, total_bytes) in [(1, 8, 100), (10, 4, 100), (10, 8, 8)] {
        let mut usage = Usage {
            max_entries: entries,
            max_file_bytes: file_bytes,
            max_source_bytes: total_bytes,
            ..Usage::default()
        };
        let result = capture(
            &fixture.0.join("library"),
            &mut usage,
            controls(&cancelled),
            false,
        );
        assert_eq!(result.err().unwrap().code, "SYSROOT_CAPTURE_EXHAUSTED");
        assert!(usage.entries > 0);
    }
    symlink("a.rs", fixture.0.join("library/link.rs")).unwrap();
    let mut usage = Coverage::pending(&Limits::default()).capture_usage;
    assert_eq!(
        capture(
            &fixture.0.join("library"),
            &mut usage,
            controls(&cancelled),
            false
        )
        .err()
        .unwrap()
        .code,
        "SYSROOT_INPUT_UNSAFE"
    );
    cancelled.store(true, Ordering::Relaxed);
    assert_eq!(
        capture(
            &fixture.0.join("library"),
            &mut usage,
            controls(&cancelled),
            false
        )
        .err()
        .unwrap()
        .code,
        "CANCELLED"
    );
}
#[test]
fn sysroot_component_claims_bind_release_and_exact_source_membership() {
    let (fixture, toolchain, _) = installed();
    let cancelled = AtomicBool::new(false);
    let mut coverage = Coverage::pending(&Limits::default());
    let claims = claims::capture(&toolchain, &mut coverage, controls(&cancelled), false).unwrap();
    let observed = capture(
        &toolchain.join("lib/rustlib/src/rust/library"),
        &mut coverage.capture_usage,
        controls(&cancelled),
        false,
    )
    .unwrap();
    claims::membership(&toolchain, &claims, &observed).unwrap();
    fixture.write(
        "toolchains/audited/lib/rustlib/src/rust/library/extra.rs",
        "pub fn extra() {}\n",
    );
    let extra = capture(
        &toolchain.join("lib/rustlib/src/rust/library"),
        &mut coverage.capture_usage,
        controls(&cancelled),
        false,
    )
    .unwrap();
    assert_eq!(
        claims::membership(&toolchain, &claims, &extra)
            .unwrap_err()
            .code,
        "SYSROOT_IDENTITY_UNSUPPORTED"
    );
    fixture.write("toolchains/audited/lib/rustlib/multirust-channel-manifest.toml", "[pkg.rustc]\nversion = \"1.97.1 (8bab26f4f 2026-07-14)\"\n[pkg.rust-src]\nversion = \"other\"\n");
    assert_eq!(
        claims::capture(&toolchain, &mut coverage, controls(&cancelled), false)
            .err()
            .unwrap()
            .code,
        "SYSROOT_IDENTITY_UNSUPPORTED"
    );
}
#[test]
fn sysroot_publication_rechecks_bytes_membership_claims_identity_and_cancellation() {
    for mutation in [
        "bytes",
        "membership",
        "claims",
        "identity",
        "cancel",
        "unchanged",
    ] {
        let (fixture, toolchain, compiler) = installed();
        let cancelled = AtomicBool::new(false);
        let corpus = corpus(toolchain, compiler, &cancelled);
        let before = observe(&fixture.0);
        match mutation {
            "bytes" => fixture.write(
                "toolchains/audited/lib/rustlib/src/rust/library/lib.rs",
                "pub struct Changed;\n",
            ),
            "membership" => fixture.write(
                "toolchains/audited/lib/rustlib/src/rust/library/extra.rs",
                "pub struct Extra;\n",
            ),
            "claims" => fixture.write(
                "toolchains/audited/lib/rustlib/components",
                "rust-src\nrustc-aarch64-apple-darwin\nrust-std-aarch64-apple-darwin\n\n",
            ),
            "identity" => {
                fixture.write("bin/rustc", "#!/bin/sh\nprintf 'changed\\n'\n");
                fs::set_permissions(
                    fixture.0.join("bin/rustc"),
                    fs::Permissions::from_mode(0o755),
                )
                .unwrap();
            }
            "cancel" => cancelled.store(true, Ordering::Relaxed),
            _ => {}
        }
        let mut coverage = corpus.coverage.clone();
        let result = corpus.recheck(&mut coverage, controls(&cancelled));
        if mutation == "unchanged" {
            result.unwrap();
            assert_eq!(coverage.freshness, "observationally_unchanged");
            assert_eq!(
                observe(&fixture.0),
                before,
                "read-only identification and freshness"
            );
        } else {
            assert_eq!(
                result.unwrap_err().code,
                if mutation == "cancel" {
                    "CANCELLED"
                } else {
                    "SYSROOT_SOURCE_CHANGED"
                },
                "{mutation}"
            );
            assert_eq!(coverage.freshness, "failed_or_interrupted");
        }
    }
}
#[test]
fn sysroot_literal_routes_refuse_unknown_cfg_missing_dynamic_and_escaping_includes() {
    let cancelled = AtomicBool::new(false);
    for (source, expected) in [
        ("include!(\"missing.rs\");", "SYSROOT_LAYOUT_UNSUPPORTED"),
        (
            "include!(concat!(\"a\", \".rs\"));",
            "SYSROOT_LAYOUT_UNSUPPORTED",
        ),
        (
            "include!(\"../../../../escape.rs\");",
            "SYSROOT_LAYOUT_UNSUPPORTED",
        ),
        (
            "#[cfg(unknown)] mod missing;",
            "SYSROOT_CONFIGURATION_UNSUPPORTED",
        ),
    ] {
        let path = format!("{PREFIX}fixture/src/lib.rs");
        let configuration = Configuration {
            crates: vec![super::super::semantic::CrateInput {
                name: "sysroot_fixture".into(),
                root_file: path.clone(),
                edition: "2024".into(),
                features: vec![],
                cfg: vec![],
                dependencies: vec![],
            }],
        };
        let mut profile = Profile {
            configuration,
            origins: BTreeMap::from([("sysroot_fixture".into(), "alloc".into())]),
            cfg: BTreeMap::new(),
        };
        let mut coverage = Coverage::pending(&Limits::default());
        coverage.consulted_roots.push(graph::Root {
            name: "sysroot_fixture".into(),
            root_file: path.clone(),
            manifest: format!("{PREFIX}fixture/Cargo.toml"),
            edition: "2024".into(),
            features: vec![],
            dependencies: vec![],
            language_origin: "alloc".into(),
            source_root: PREFIX.into(),
        });
        assert_eq!(
            routes::audit(
                &BTreeMap::from([(path, source.into())]),
                &mut profile,
                &mut coverage,
                controls(&cancelled)
            )
            .unwrap_err()
            .code,
            expected
        );
    }
}

fn assert_no_corpus(coverage: &Coverage) {
    assert!(!coverage.complete);
    assert!(coverage.corpus_digest.is_empty());
    assert_eq!(coverage.capture_usage.overlays_materialized, 0);
}

#[test]
fn sysroot_identification_stdout_and_stderr_caps_are_inclusive_and_refuse_overflow() {
    let (fixture, _, compiler) = installed();
    let cancelled = AtomicBool::new(false);
    for comparison in [false, true] {
        for stderr in [false, true] {
            for bytes in [OUTPUT_CAP, OUTPUT_CAP + 1] {
                fixture.write(
                    "bin/rustc",
                    &format!(
                        "#!/bin/sh\nprintf '%s' '{}'{}\n",
                        "x".repeat(bytes),
                        if stderr { " >&2" } else { "" }
                    ),
                );
                fs::set_permissions(&compiler.executable, fs::Permissions::from_mode(0o755))
                    .unwrap();
                let before = observe(&fixture.0);
                let mut coverage = Coverage::pending(&Limits::default());
                let result = compiler.identify(
                    &["-vV"],
                    controls(&cancelled),
                    &mut coverage.capture_usage,
                    comparison,
                );
                let usage = &coverage.capture_usage;
                let (processes, stdout_bytes, stderr_bytes) = if comparison {
                    (
                        usage.comparison_subprocesses,
                        usage.comparison_stdout_bytes,
                        usage.comparison_stderr_bytes,
                    )
                } else {
                    (
                        usage.identification_subprocesses,
                        usage.identification_stdout_bytes,
                        usage.identification_stderr_bytes,
                    )
                };
                assert_eq!(processes, 1);
                assert_eq!(if stderr { stdout_bytes } else { stderr_bytes }, 0);
                let retained = if stderr { stderr_bytes } else { stdout_bytes };
                assert!(retained > 0 && retained <= OUTPUT_CAP);
                if bytes == OUTPUT_CAP {
                    assert_eq!(result.unwrap().len(), if stderr { 0 } else { bytes });
                    assert_eq!(retained, OUTPUT_CAP);
                } else {
                    let error = result.unwrap_err();
                    assert_eq!(error.code, "SYSROOT_CAPTURE_EXHAUSTED");
                    assert_eq!(error.message, "compiler output cap exhausted");
                }
                assert_no_corpus(&coverage);
                assert_eq!(observe(&fixture.0), before);
            }
        }
    }
}

#[test]
fn sysroot_enumeration_depth_is_inclusive_and_refuses_without_a_prefix_corpus() {
    let fixture = empty();
    let cancelled = AtomicBool::new(false);
    let mut directory = String::from("library");
    for _ in 0..32 {
        directory.push_str("/nested");
    }
    fixture.write(&format!("{directory}/leaf.rs"), "pub struct Leaf;\n");
    let mut coverage = Coverage::pending(&Limits::default());
    let observed = capture(
        &fixture.0.join("library"),
        &mut coverage.capture_usage,
        controls(&cancelled),
        false,
    )
    .unwrap();
    assert_eq!(observed.directories.len(), 33);
    assert_eq!(observed.files.len(), 1);
    fixture.write(
        &format!("{directory}/nested/extra.rs"),
        "pub struct Extra;\n",
    );
    let before = observe(&fixture.0);
    let error = capture(
        &fixture.0.join("library"),
        &mut coverage.capture_usage,
        controls(&cancelled),
        false,
    )
    .err()
    .unwrap();
    assert_eq!(error.code, "SYSROOT_CAPTURE_EXHAUSTED");
    assert_eq!(error.message, "library enumeration depth cap exhausted");
    assert!(coverage.capture_usage.entries > 32);
    assert_eq!(coverage.capture_usage.files, 1);
    assert_no_corpus(&coverage);
    assert_eq!(observe(&fixture.0), before);
}

fn caller_configuration(count: usize, dependencies: usize) -> Configuration {
    Configuration {
        crates: (0..count)
            .map(|index| super::super::semantic::CrateInput {
                name: format!("caller_{index}"),
                root_file: format!("caller_{index}.rs"),
                edition: "2024".into(),
                features: vec![],
                cfg: vec![],
                dependencies: (0..dependencies)
                    .map(|edge| super::super::semantic::DependencyInput {
                        name: format!("dependency_{edge}"),
                        crate_name: "sysroot_core".into(),
                    })
                    .collect(),
            })
            .collect(),
    }
}

#[test]
fn sysroot_composite_node_limit_refuses_before_publishing_a_graph_or_digest() {
    let mut coverage = Coverage::pending(&Limits::default());
    let error = graph::build(
        Path::new("/synthetic/library"),
        &BTreeMap::new(),
        &BTreeMap::new(),
        &caller_configuration(20, 0),
        &[],
        &mut coverage,
    )
    .err()
    .unwrap();
    assert_eq!(error.code, "SYSROOT_GRAPH_EXHAUSTED");
    assert_eq!(error.message, "composite graph exceeds 32 crates");
    assert!(coverage.consulted_roots.is_empty());
    assert!(coverage.graph.is_empty());
    assert_no_corpus(&coverage);
}

#[test]
fn sysroot_composite_graph_node_and_edge_boundaries_with_audited_manifests() {
    // Only the hash-bound manifests come from the installation. Root texts and
    // all graph limits are synthetic; no full-library capture or overlays run.
    let version = Command::new("rustc")
        .arg("-vV")
        .env("RUSTUP_TOOLCHAIN", "stable")
        .env("RUSTUP_AUTO_INSTALL", "0")
        .output()
        .unwrap();
    let version = String::from_utf8(version.stdout).unwrap();
    if !version.lines().any(|line| line == "release: 1.97.1")
        || !version
            .lines()
            .any(|line| line == "host: aarch64-apple-darwin")
    {
        assert!(std::env::var_os("REQUIRE_SYSROOT_GATE").is_none());
        eprintln!("audited manifest graph fixture unavailable on this profile");
        return;
    }
    let output = Command::new("rustc")
        .args(["--print", "sysroot"])
        .env("RUSTUP_TOOLCHAIN", "stable")
        .env("RUSTUP_AUTO_INSTALL", "0")
        .output()
        .unwrap();
    assert!(output.status.success());
    let library = PathBuf::from(String::from_utf8(output.stdout).unwrap().trim())
        .join("lib/rustlib/src/rust/library");
    let mut observed = BTreeMap::new();
    let mut texts = BTreeMap::new();
    for directory in [
        "core",
        "compiler-builtins/compiler-builtins",
        "alloc",
        "rustc-std-workspace-core",
        "rustc-std-workspace-alloc",
        "vendor/cfg-if-1.0.4",
        "vendor/libc-0.2.185",
        "panic_abort",
        "unwind",
        "std_detect",
        "vendor/hashbrown-0.17.1",
        "vendor/rustc-demangle-0.1.27",
        "std",
    ] {
        let manifest = library.join(directory).join("Cargo.toml");
        observed.insert(manifest.clone(), (fs::read(manifest).unwrap(), (0, 0)));
        let source = if directory.starts_with("rustc-std-workspace-") {
            "lib.rs"
        } else {
            "src/lib.rs"
        };
        texts.insert(format!("{PREFIX}{directory}/{source}"), String::new());
    }
    for (callers, dependencies, edge_cap, succeeds, message) in [
        (19, 0, 1024, true, ""),
        (1, 29, 1024, true, ""),
        (
            1,
            30,
            1024,
            false,
            "composite graph exceeds 32 edges per crate",
        ),
        (1, 0, 29, true, ""),
        (1, 0, 28, false, "composite graph edge cap exhausted"),
    ] {
        let mut coverage = Coverage::pending(&Limits::default());
        coverage.capture_usage.max_graph_edges = edge_cap;
        let result = graph::build(
            &library,
            &observed,
            &texts,
            &caller_configuration(callers, dependencies),
            &[],
            &mut coverage,
        );
        if succeeds {
            let profile = result.unwrap();
            assert_eq!(profile.configuration.crates.len(), callers + 13);
            assert_eq!(coverage.graph.len(), 26 + callers * (dependencies + 3));
            assert!(
                profile
                    .configuration
                    .crates
                    .iter()
                    .all(|c| c.dependencies.len() <= 32)
            );
        } else {
            let error = result.err().unwrap();
            assert_eq!(error.code, "SYSROOT_GRAPH_EXHAUSTED");
            assert_eq!(error.message, message);
            assert!(!coverage.consulted_roots.is_empty());
            assert!(coverage.capture_usage.graph_edges > 0);
        }
        assert_no_corpus(&coverage);
    }
}

#[test]
fn sysroot_literal_route_depth_and_edge_caps_refuse_without_a_partial_corpus() {
    let cancelled = AtomicBool::new(false);
    let root = format!("{PREFIX}fixture/src/lib.rs");
    for (depth, edge_cap, succeeds, expected) in [
        (32, 32, true, ""),
        (33, 100, false, "literal route depth cap exhausted"),
        (2, 2, true, ""),
        (2, 1, false, "literal route-edge cap exhausted"),
    ] {
        let mut texts = BTreeMap::from([(root.clone(), "include!(\"level_1.rs\");".into())]);
        for level in 1..=depth {
            texts.insert(
                format!("{PREFIX}fixture/src/level_{level}.rs"),
                if level == depth {
                    "pub struct Leaf;".into()
                } else {
                    format!("include!(\"level_{}.rs\");", level + 1)
                },
            );
        }
        let mut configuration = caller_configuration(1, 0);
        configuration.crates[0].root_file = root.clone();
        let mut profile = Profile {
            configuration,
            origins: BTreeMap::from([("caller_0".into(), "alloc".into())]),
            cfg: BTreeMap::new(),
        };
        let mut coverage = Coverage::pending(&Limits::default());
        coverage.capture_usage.max_route_edges = edge_cap;
        coverage.consulted_roots.push(graph::Root {
            name: "caller_0".into(),
            root_file: root.clone(),
            manifest: format!("{PREFIX}fixture/Cargo.toml"),
            edition: "2024".into(),
            features: vec![],
            dependencies: vec![],
            language_origin: "alloc".into(),
            source_root: PREFIX.into(),
        });
        let result = routes::audit(&texts, &mut profile, &mut coverage, controls(&cancelled));
        if succeeds {
            result.unwrap();
            assert_eq!(coverage.capture_usage.route_edges, depth);
        } else {
            let error = result.unwrap_err();
            assert_eq!(error.code, "SYSROOT_CAPTURE_EXHAUSTED");
            assert_eq!(error.message, expected);
            assert_eq!(coverage.capture_usage.route_edges, depth);
        }
        assert_no_corpus(&coverage);
    }
}

#[test]
fn sysroot_expired_capture_and_publication_deadlines_refuse() {
    let (fixture, toolchain, compiler) = installed();
    let cancelled = AtomicBool::new(false);
    let corpus = corpus(toolchain, compiler, &cancelled);
    let before = observe(&fixture.0);
    let expired = (Instant::now() - Duration::from_millis(1), &cancelled);
    let mut coverage = Coverage::pending(&Limits::default());
    let error = capture(
        Path::new(&corpus.coverage.source_root),
        &mut coverage.capture_usage,
        expired,
        false,
    )
    .err()
    .unwrap();
    assert_eq!(error.code, "planning_deadline");
    assert_no_corpus(&coverage);
    let mut publication = corpus.coverage.clone();
    assert_eq!(
        corpus.recheck(&mut publication, expired).unwrap_err().code,
        "planning_deadline"
    );
    assert_eq!(publication.stage, "publication_recheck");
    assert_eq!(publication.freshness, "failed_or_interrupted");
    let mut envelope = super::super::MoveEnvelope::empty(Limits::default());
    envelope.coverage.scan.sysroot_sources = Some(publication);
    envelope.withhold();
    envelope.withhold_external_resolution();
    assert_no_corpus(envelope.coverage.scan.sysroot_sources.as_ref().unwrap());
    assert!(!envelope.plan.applicable);
    assert!(envelope.plan.patch.is_none());
    assert!(envelope.plan.binding_proofs.is_empty());
    assert!(envelope.plan.resolution_coverage.is_none());
    assert_eq!(envelope.coverage.ra_resolved, 0);
    assert_eq!(observe(&fixture.0), before);
}
