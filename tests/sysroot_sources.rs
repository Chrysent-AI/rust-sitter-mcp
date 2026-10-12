#[allow(dead_code)]
#[path = "support/fixture_gen.rs"]
mod fixture_gen;
#[allow(dead_code)]
#[path = "support/move_artifacts.rs"]
mod move_artifacts;
use fixture_gen::{Fixture, observe};
use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Write},
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
};

const SELECTED: &str =
    "pub fn selected() { let mut s = std::string::String::new(); s.push_str(\"x\"); }";
struct Server {
    child: Child,
    input: Option<ChildStdin>,
    output: BufReader<ChildStdout>,
    id: u64,
}
impl Server {
    fn new() -> Self {
        Self::configured(None, None, None)
    }
    fn configured(
        installation: Option<&Fixture>,
        binary: Option<&std::path::Path>,
        cargo_home: Option<&std::path::Path>,
    ) -> Self {
        let mut command = Command::new(
            binary.unwrap_or_else(|| std::path::Path::new(env!("CARGO_BIN_EXE_rust-sitter-mcp"))),
        );
        command
            .env("RUSTUP_TOOLCHAIN", "stable")
            .env("RUSTUP_AUTO_INSTALL", "0");
        if let Some(fixture) = installation {
            let mut paths = vec![fixture.0.join("bin")];
            if let Some(path) = std::env::var_os("PATH") {
                paths.extend(std::env::split_paths(&path));
            }
            command
                .env("RUSTUP_HOME", &fixture.0)
                .env("PATH", std::env::join_paths(paths).unwrap())
                .env("CARGO_HOME", fixture.0.join("cargo-home"));
        }
        if let Some(home) = cargo_home {
            command.env("CARGO_HOME", home);
        }
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let input = child.stdin.take().unwrap();
        let output = BufReader::new(child.stdout.take().unwrap());
        let mut server = Self {
            child,
            input: Some(input),
            output,
            id: 0,
        };
        server.exchange("initialize", json!({"protocolVersion":"2025-06-18", "capabilities":{}, "clientInfo":{"name":"fixture", "version":"1"}}));
        writeln!(
            server.input.as_mut().unwrap(),
            "{}",
            json!({"jsonrpc":"2.0", "method":"notifications/initialized"})
        )
        .unwrap();
        server
    }
    fn exchange(&mut self, method: &str, params: Value) -> Value {
        self.id += 1;
        writeln!(
            self.input.as_mut().unwrap(),
            "{}",
            json!({"jsonrpc":"2.0", "id":self.id, "method":method, "params":params})
        )
        .unwrap();
        self.input.as_mut().unwrap().flush().unwrap();
        let mut line = String::new();
        assert!(self.output.read_line(&mut line).unwrap() > 0);
        let response: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(response["id"], self.id, "{response}");
        response["result"].clone()
    }
    fn run(&mut self, args: Value) -> Value {
        self.exchange("tools/call", json!({"name":"move_item", "arguments":args}))["structuredContent"].clone()
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        drop(self.input.take());
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
fn fixture() -> Fixture {
    let repo = Fixture::generate();
    repo.write("src/lib.rs", "mod source;\n");
    repo.write("src/source.rs", &format!("{SELECTED}\n"));
    repo
}
fn request(repo: &Fixture) -> Value {
    json!({"repo_path":repo.0, "crate_root":"src/lib.rs", "paths":["src"], "resolve_semantic":true, "sysroot_sources":true,
        "semantic_configuration":{"crates":[{"name":"caller", "root_file":"src/lib.rs", "edition":"2024", "features":[], "cfg":[], "dependencies":[]}]},
        "limits":{"time_budget_ms":300000, "diagnostic_count":1000, "response_bytes":16777216},
        "moves":[{"item":move_artifacts::anchor(repo,"src/source.rs",SELECTED),"destination":{"kind":"new_sibling", "path":"src/moved.rs", "parent_path":"src/lib.rs"}}]})
}
#[test]
fn installed_sysroot_move_proves_both_overlays_or_refuses_an_unsupported_installation() {
    let repo = fixture();
    let before = observe(&repo.0);
    let cargo_home = Fixture::empty();
    cargo_home.write("marker", "read-only Cargo home\n");
    let cargo_before = observe(&cargo_home.0);
    let mut server = Server::configured(None, None, Some(&cargo_home.0));
    let args = request(&repo);
    let mut off = args.clone();
    off.as_object_mut().unwrap().remove("sysroot_sources");
    let implicit = server.run(off.clone());
    off["sysroot_sources"] = json!(false);
    assert_eq!(
        serde_json::to_vec(&implicit).unwrap(),
        serde_json::to_vec(&server.run(off)).unwrap()
    );
    assert_eq!(implicit["plan"]["applicable"], false);
    assert!(implicit["coverage"].get("sysroot_sources").is_none());
    let actual_sources = installed_library();
    let sources_before = actual_sources.as_ref().map(|path| observe(path));
    let claims_before: Vec<_> = actual_sources
        .as_ref()
        .into_iter()
        .flat_map(|library| {
            let rustlib = library.ancestors().nth(3).unwrap();
            [
                "components",
                "multirust-channel-manifest.toml",
                "manifest-rust-src",
            ]
            .into_iter()
            .map(move |name| rustlib.join(name))
        })
        .map(|path| {
            let state = observe(&path);
            (path, state)
        })
        .collect();
    let started = std::time::Instant::now();
    let result = server.run(args);
    if result["error"]["code"] == "SYSROOT_IDENTITY_UNSUPPORTED"
        || result["error"]["code"] == "SYSROOT_TOOLCHAIN_UNAVAILABLE"
        || result["error"]["code"] == "SYSROOT_SOURCES_UNAVAILABLE"
    {
        assert!(
            std::env::var_os("REQUIRE_SYSROOT_GATE").is_none(),
            "required installed gate unavailable: {}",
            result["error"]
        );
        eprintln!(
            "installed positive not available on this platform/profile: {}",
            result["error"]
        );
        assert_eq!(result["coverage"]["ra_resolved"].as_u64().unwrap_or(0), 0);
        assert_eq!(result["plan"]["applicable"], false);
        assert_eq!(observe(&repo.0), before);
        assert_eq!(observe(&cargo_home.0), cargo_before);
        return;
    }
    eprintln!(
        "SYSROOT_GATE_EVIDENCE={}",
        json!({"proofs":result["plan"]["binding_proofs"], "coverage":result["coverage"], "resolution":result["plan"]["resolution_coverage"], "elapsed_ms":started.elapsed().as_millis(), "applicable":result["plan"]["applicable"]})
    );
    assert_eq!(result["plan"]["applicable"], true, "{}", result["error"]);
    let proof = result["plan"]["binding_proofs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["declaration"]["anchor"]["expected_text"] == "push_str")
        .expect("real push_str occurrence");
    assert_eq!(proof["class"], "ra_resolved");
    assert_eq!(proof["classification"], "inherent_function");
    assert_eq!(proof["declaration"], proof["final_declaration"]);
    assert_eq!(proof["original_receiver"], proof["final_original_receiver"]);
    assert_eq!(proof["adjusted_receiver"], proof["final_adjusted_receiver"]);
    assert_eq!(proof["source_access"], true);
    assert_eq!(proof["final_access"], true);
    assert_eq!(result["coverage"]["ra_resolved"], 2);
    assert!(
        result["coverage"]
            .get("assumed_declared_identity")
            .is_none()
    );
    assert_eq!(result["coverage"]["sysroot_sources"]["complete"], true);
    assert_eq!(
        result["coverage"]["sysroot_sources"]["capture_usage"]["overlays_materialized"],
        2
    );
    assert_eq!(
        result["coverage"]["sysroot_sources"]["freshness"],
        "observationally_unchanged"
    );
    let copy = move_artifacts::apply(&repo, &result);
    assert!(
        std::fs::read_to_string(copy.0.join("src/moved.rs"))
            .unwrap()
            .contains(SELECTED)
    );
    assert_eq!(observe(&repo.0), before, "engine wrote caller files");
    assert_eq!(
        observe(&cargo_home.0),
        cargo_before,
        "engine wrote Cargo home"
    );
    for (path, state) in claims_before {
        assert_eq!(
            observe(&path),
            state,
            "engine wrote installed component claims"
        );
    }
    if let Some(path) = actual_sources {
        assert_eq!(
            observe(&path),
            sources_before.unwrap(),
            "engine wrote installed library files or membership"
        );
    }
}
#[test]
fn sysroot_requires_resolution_and_explicit_configuration_even_with_no_moves() {
    let repo = fixture();
    let mut server = Server::new();
    for field in ["resolve_semantic", "semantic_configuration"] {
        let mut args = request(&repo);
        args["moves"] = json!([]);
        args.as_object_mut().unwrap().remove(field);
        let result = server.run(args);
        assert_eq!(result["error"]["code"], "INVALID_PARAMS", "{result}");
        assert_eq!(result["plan"]["applicable"], false);
        assert_eq!(result["coverage"]["ra_resolved"].as_u64().unwrap_or(0), 0);
        assert!(result["coverage"].get("sysroot_sources").is_none());
    }
}
fn installed_library() -> Option<std::path::PathBuf> {
    let output = Command::new("rustc")
        .args(["--print", "sysroot"])
        .env("RUSTUP_TOOLCHAIN", "stable")
        .env("RUSTUP_AUTO_INSTALL", "0")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let root = std::path::PathBuf::from(String::from_utf8(output.stdout).ok()?.trim());
    let library = root.join("lib/rustlib/src/rust/library");
    library.is_dir().then_some(library)
}
fn fake_installation(release: &str, source_exists: bool) -> Fixture {
    use std::os::unix::fs::PermissionsExt;
    let mut fixture = Fixture::empty();
    fixture.0 = fixture.0.canonicalize().unwrap();
    let toolchain = fixture.0.join("toolchains/stable");
    std::fs::create_dir_all(&toolchain).unwrap();
    fixture.write("cargo-home/marker", "read-only fixture Cargo home\n");
    if source_exists {
        fixture.write(
            "toolchains/stable/lib/rustlib/src/rust/library/lib.rs",
            "pub struct Thing;\n",
        );
    }
    fixture.write("bin/rustc", &format!("#!/bin/sh\ncase \"$1\" in\n-vV) printf 'release: {release}\\ncommit-hash: 8bab26f4f68e0e26f0bb7960be334d5b520ea452\\nhost: aarch64-apple-darwin\\n';;\n--print) printf '%s\\n' '{}';;\n*) exit 73;;\nesac\n", toolchain.display()));
    std::fs::set_permissions(
        fixture.0.join("bin/rustc"),
        std::fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    fixture
}
#[test]
fn sysroot_absent_sources_and_unexamined_release_refuse_without_any_writes_or_proofs() {
    for (release, source_exists, code) in [
        ("1.97.1", false, "SYSROOT_SOURCES_UNAVAILABLE"),
        ("other", true, "SYSROOT_IDENTITY_UNSUPPORTED"),
    ] {
        let repo = fixture();
        let installation = fake_installation(release, source_exists);
        let caller_before = observe(&repo.0);
        let installation_before = observe(&installation.0);
        let mut server = Server::configured(Some(&installation), None, None);
        let mut args = request(&repo);
        args.as_object_mut().unwrap().remove("sysroot_sources");
        let omitted = server.run(args.clone());
        args["sysroot_sources"] = json!(false);
        assert_eq!(
            serde_json::to_vec(&omitted).unwrap(),
            serde_json::to_vec(&server.run(args.clone())).unwrap()
        );
        assert_eq!(omitted["plan"]["applicable"], false);
        assert!(omitted["coverage"].get("sysroot_sources").is_none());
        args["sysroot_sources"] = json!(true);
        let result = server.run(args);
        assert_eq!(result["error"]["code"], code, "{result}");
        assert_eq!(result["coverage"]["sysroot_sources"]["complete"], false);
        assert!(
            result["coverage"]["sysroot_sources"]
                .get("corpus_digest")
                .is_none()
        );
        assert_eq!(result["coverage"]["ra_resolved"].as_u64().unwrap_or(0), 0);
        assert!(
            result["plan"]["binding_proofs"]
                .as_array()
                .is_none_or(Vec::is_empty)
        );
        assert_eq!(result["plan"]["applicable"], false);
        assert!(result["plan"]["patch"].is_null());
        assert_eq!(result["plan"]["integrity"]["semantic"], "not_performed");
        assert_eq!(observe(&repo.0), caller_before);
        assert_eq!(
            observe(&installation.0),
            installation_before,
            "compiler/toolchain/sources/Cargo home changed"
        );
    }
}
#[test]
fn sysroot_flag_off_matches_pre_extension_binary_when_supplied() {
    let Some(binary) = std::env::var_os("SYSROOT_BASELINE_BINARY") else {
        return;
    };
    let repo = fixture();
    let base_args = request(&repo);
    for source_exists in [false, true] {
        let installation = fake_installation("other", source_exists);
        let mut baseline = Server::configured(
            Some(&installation),
            Some(std::path::Path::new(&binary)),
            None,
        );
        let mut current = Server::configured(Some(&installation), None, None);
        for (source, resolve, invalid) in [
            ("pub fn selected() {}", false, false),
            (SELECTED, true, false),
            (SELECTED, true, true),
        ] {
            repo.write("src/source.rs", &format!("{source}\n"));
            let mut args = base_args.clone();
            args.as_object_mut().unwrap().remove("sysroot_sources");
            args["resolve_semantic"] = json!(resolve);
            args["moves"][0]["item"] = move_artifacts::anchor(&repo, "src/source.rs", source);
            if invalid {
                args["semantic_configuration"]["crates"][0]["name"] = json!("invalid-name");
            }
            let expected = baseline.exchange(
                "tools/call",
                json!({"name":"move_item", "arguments":args.clone()}),
            );
            for enabled in [None, Some(false)] {
                if let Some(value) = enabled {
                    args["sysroot_sources"] = json!(value);
                }
                let actual = current.exchange(
                    "tools/call",
                    json!({"name":"move_item", "arguments":args.clone()}),
                );
                assert_eq!(
                    actual["content"][0]["text"], expected["content"][0]["text"],
                    "baseline byte parity, sources={source_exists}, resolution={resolve}, invalid={invalid}"
                );
                assert_eq!(actual["structuredContent"], expected["structuredContent"]);
            }
        }
    }
}

fn assert_sysroot_failure_withholds_composite_evidence(result: &Value) {
    assert_eq!(result["coverage"]["sysroot_sources"]["complete"], false);
    assert!(
        result["coverage"]["sysroot_sources"]
            .get("corpus_digest")
            .is_none()
    );
    assert_eq!(result["coverage"]["ra_resolved"].as_u64().unwrap_or(0), 0);
    assert_eq!(
        result["coverage"]["assumed_declared_identity"]
            .as_u64()
            .unwrap_or(0),
        0
    );
    assert_eq!(
        result["coverage"]["sysroot_sources"]["capture_usage"]["overlays_materialized"],
        0
    );
    assert!(
        result["plan"]["binding_proofs"]
            .as_array()
            .is_none_or(Vec::is_empty)
    );
    assert!(result["plan"]["resolution_coverage"].is_null());
    assert_eq!(result["plan"]["applicable"], false);
    for artifact in ["patch", "edits", "created_files", "directory_preconditions"] {
        assert!(result["plan"][artifact].is_null(), "{artifact}");
    }
    assert_eq!(result["plan"]["integrity"]["semantic"], "not_performed");
}

#[test]
fn sysroot_identification_output_overflow_refuses_without_partial_publication() {
    use std::os::unix::fs::PermissionsExt;
    for stream in ["stdout", "stderr"] {
        let repo = fixture();
        let installation = fake_installation("1.97.1", true);
        installation.write(
            "bin/rustc",
            &format!(
                "#!/bin/sh\nprintf '%s' '{}'{}\n",
                "x".repeat(65_537),
                if stream == "stderr" { " >&2" } else { "" }
            ),
        );
        std::fs::set_permissions(
            installation.0.join("bin/rustc"),
            std::fs::Permissions::from_mode(0o755),
        )
        .unwrap();
        let before = (observe(&repo.0), observe(&installation.0));
        let mut server = Server::configured(Some(&installation), None, None);
        let result = server.run(request(&repo));
        assert_eq!(
            result["error"]["code"], "SYSROOT_CAPTURE_EXHAUSTED",
            "{result}"
        );
        assert_sysroot_failure_withholds_composite_evidence(&result);
        assert_eq!(
            result["coverage"]["sysroot_sources"]["stage"],
            "identification"
        );
        let usage = &result["coverage"]["sysroot_sources"]["capture_usage"];
        assert_eq!(usage["identification_subprocesses"], 1);
        let charged = usage[format!("identification_{stream}_bytes")]
            .as_u64()
            .unwrap();
        assert!(charged > 0 && charged <= 65_536);
        assert_eq!(usage["files"], 0);
        assert_eq!((observe(&repo.0), observe(&installation.0)), before);
    }
}

#[test]
fn sysroot_discovery_deadline_expires_during_identification_and_withholds_composite_evidence() {
    use std::os::unix::fs::PermissionsExt;
    let repo = fixture();
    let installation = fake_installation("1.97.1", true);
    let executable = installation.0.join("bin/rustc");
    let script = std::fs::read_to_string(&executable).unwrap();
    // exec replaces the shell, so deadline cleanup kills the owned process and
    // closes both pipes without leaving a sleeping grandchild behind.
    installation.write(
        "bin/rustc",
        &script.replace("*) exit 73;;", "-Z) exec /bin/sleep 2;;\n*) exit 73;;"),
    );
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755)).unwrap();
    let before = (observe(&repo.0), observe(&installation.0));
    let mut server = Server::configured(Some(&installation), None, None);
    let mut args = request(&repo);
    args["limits"]["time_budget_ms"] = json!(500);
    let result = server.run(args);
    assert_eq!(result["status"], "partial", "{result}");
    assert!(
        result["truncation_reasons"]
            .as_array()
            .unwrap()
            .iter()
            .any(|reason| reason == "planning_deadline")
    );
    assert!(
        result["plan"]["blockers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|blocker| blocker["code"] == "planning_deadline")
    );
    assert_sysroot_failure_withholds_composite_evidence(&result);
    assert_eq!(
        result["coverage"]["sysroot_sources"]["stage"],
        "identification"
    );
    assert_eq!(
        result["coverage"]["sysroot_sources"]["capture_usage"]["identification_subprocesses"],
        3
    );
    assert_eq!((observe(&repo.0), observe(&installation.0)), before);
    // A subsequent request on the same server succeeds: no orphaned admission
    // slot or identification task remains after the deadline settles.
    let mut off = request(&repo);
    off["sysroot_sources"] = json!(false);
    assert!(server.run(off)["error"].is_null());
}

#[test]
fn sysroot_flag_off_preserves_external_only_baseline_success_and_refusal() {
    let Some(binary) = std::env::var_os("SYSROOT_BASELINE_BINARY") else {
        return;
    };
    let repo = fixture();
    let base_args = request(&repo);
    let mut dependency = Fixture::empty();
    dependency.0 = dependency.0.canonicalize().unwrap();
    dependency.write(
        "Cargo.toml",
        "[package]\nname = \"dependency-library\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    );
    dependency.write("src/lib.rs", "pub struct Thing;\n");
    repo.write("Cargo.toml", &format!("[package]\nname = \"caller\"\nversion = \"0.1.0\"\nedition = \"2024\"\n[dependencies]\ndep = {{ package = \"dependency-library\", path = {:?} }}\n", dependency.0.to_str().unwrap()));
    let output = Command::new("cargo")
        .args(["generate-lockfile", "--offline"])
        .current_dir(&repo.0)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let cargo_home = Fixture::empty();
    cargo_home.write("marker", "read-only Cargo home\n");
    let mut baseline = Server::configured(
        None,
        Some(std::path::Path::new(&binary)),
        Some(&cargo_home.0),
    );
    let mut current = Server::configured(None, None, Some(&cargo_home.0));
    let selected = "pub fn selected() -> u32 { dep::Thing::new().written() }";
    repo.write("src/source.rs", &format!("{selected}\n"));
    for accessible in [true, false] {
        let visibility = if accessible { "pub " } else { "" };
        dependency.write("src/lib.rs", &format!("pub struct Thing; impl Thing {{ pub fn new() -> Self {{ Self }} {visibility}fn written(&self) -> u32 {{ 7 }} }}\n"));
        let before = observe(&repo.0);
        let dependency_before = observe(&dependency.0);
        let cargo_before = observe(&cargo_home.0);
        let mut args = base_args.clone();
        args.as_object_mut().unwrap().remove("sysroot_sources");
        args["external_path_dependencies"] = json!(true);
        args["moves"][0]["item"] = move_artifacts::anchor(&repo, "src/source.rs", selected);
        let expected = baseline.exchange(
            "tools/call",
            json!({"name":"move_item", "arguments":args.clone()}),
        );
        assert_eq!(
            expected["structuredContent"]["plan"]["applicable"], accessible,
            "{}",
            expected["structuredContent"]
        );
        for enabled in [None, Some(false)] {
            if let Some(value) = enabled {
                args["sysroot_sources"] = json!(value);
            }
            let actual = current.exchange(
                "tools/call",
                json!({"name":"move_item", "arguments":args.clone()}),
            );
            assert_eq!(
                actual["content"][0]["text"], expected["content"][0]["text"],
                "external-only byte parity, accessible={accessible}"
            );
            assert_eq!(actual["structuredContent"], expected["structuredContent"]);
            assert!(
                actual["structuredContent"]["coverage"]
                    .get("sysroot_sources")
                    .is_none()
            );
        }
        assert_eq!(observe(&repo.0), before);
        assert_eq!(observe(&dependency.0), dependency_before);
        assert_eq!(observe(&cargo_home.0), cargo_before);
    }
}
