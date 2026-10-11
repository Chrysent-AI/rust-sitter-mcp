use super::super::tests::fixture_gen::{Fixture, observe};
use super::*;

#[test]
fn preflight_probe_transitive_path_overlap() {
    let (repo, dep, _, config) = fixture();
    let transitive = Fixture::empty();
    transitive.write(
        "Cargo.toml",
        "[package]\nname='transitive-library'\nversion='0.1.0'\nedition='2024'\n",
    );
    transitive.write("src/lib.rs", "pub struct Transitive;\n");
    let text = fs::read_to_string(dep.0.join("Cargo.toml")).unwrap();
    dep.write(
        "Cargo.toml",
        &format!(
            "{text}\n[dependencies]\ntransitive={{package='transitive-library',path={:?}}}\n",
            transitive.0.canonicalize().unwrap().to_str().unwrap()
        ),
    );
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
    let tree = transitive.0.join("server-state/discovery");
    let before = observe(&repo.0);
    let dependency = observe(&dep.0);
    let nested = observe(&transitive.0);
    let error = discover_at(
        &repo.0.canonicalize().unwrap(),
        "src/lib.rs",
        &config,
        &Limits::default(),
        (
            Instant::now() + Duration::from_secs(30),
            &AtomicBool::new(false),
        ),
        &tree,
    )
    .err()
    .unwrap();
    eprintln!(
        "transitive refusal: {}; state parent exists: {}",
        error.code,
        tree.parent().unwrap().exists()
    );
    assert!(
        !tree.parent().unwrap().exists(),
        "scratch state created in transitive path root before refusal"
    );
    assert_eq!(error.code, "EXTERNAL_STATE_OVERLAP");
    assert_eq!(observe(&repo.0), before);
    assert_eq!(observe(&dep.0), dependency);
    assert_eq!(observe(&transitive.0), nested);
}

#[test]
fn preflight_probe_unconfigured_workspace_member_overlap() {
    let (workspace, repo, dep, config) = enclosing_workspace_fixture();
    let other_dep = Fixture::empty();
    other_dep.write(
        "Cargo.toml",
        "[package]\nname='other-library'\nversion='0.1.0'\nedition='2024'\n",
    );
    other_dep.write("src/lib.rs", "pub struct Other;\n");
    workspace.write("other/Cargo.toml", &format!("[package]\nname='other-member'\nversion='0.1.0'\nedition='2024'\n[dependencies]\nother={{package='other-library',path={:?}}}\n", other_dep.0.canonicalize().unwrap().to_str().unwrap()));
    workspace.write("other/src/lib.rs", "pub struct Member;\n");
    let text = fs::read_to_string(workspace.0.join("Cargo.toml")).unwrap();
    workspace.write(
        "Cargo.toml",
        &text.replace("members=['member']", "members=['member','other']"),
    );
    let workspace_paths =
        declared_paths(&fs::read_to_string(workspace.0.join("Cargo.toml")).unwrap()).unwrap();
    let dependency_paths =
        declared_paths(&fs::read_to_string(dep.0.join("Cargo.toml")).unwrap()).unwrap();
    assert_eq!(workspace_paths.len(), 1);
    assert!(dependency_paths.is_empty());
    assert_eq!(config.crates.len(), 1);
    let output = Command::new("cargo")
        .args(["generate-lockfile", "--offline"])
        .current_dir(&workspace.0)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let before = observe(&workspace.0);
    let dependency = observe(&dep.0);
    let nested = observe(&other_dep.0);
    let tree = other_dep.0.join("server-state/discovery");
    let result = discover_at(
        &repo.0.canonicalize().unwrap(),
        "src/lib.rs",
        &config,
        &Limits::default(),
        (
            Instant::now() + Duration::from_secs(30),
            &AtomicBool::new(false),
        ),
        &tree,
    );
    eprintln!(
        "unconfigured member discovery result: {}; state parent exists: {}",
        match &result {
            Ok(_) => "success",
            Err(e) => e.code.as_str(),
        },
        tree.parent().unwrap().exists()
    );
    assert!(
        !tree.parent().unwrap().exists(),
        "scratch state created in an unconfigured workspace member's path root"
    );
    assert_eq!(result.err().unwrap().code, "EXTERNAL_STATE_OVERLAP");
    assert_eq!(observe(&workspace.0), before);
    assert_eq!(observe(&dep.0), dependency);
    assert_eq!(observe(&other_dep.0), nested);
}

#[test]
fn path_candidate_cycles_refuse_before_scratch_and_terminate() {
    let (repo, dep, state, config) = fixture();
    let text = fs::read_to_string(dep.0.join("Cargo.toml")).unwrap();
    dep.write(
        "Cargo.toml",
        &format!(
            "{text}\n[dependencies]\ncaller={{path={:?}}}\n",
            repo.0.canonicalize().unwrap().to_str().unwrap()
        ),
    );
    let before = observe(&repo.0);
    let dependency = observe(&dep.0);
    let tree = state.0.join("server-state/discovery");
    let error = discover_at(
        &repo.0.canonicalize().unwrap(),
        "src/lib.rs",
        &config,
        &Limits::default(),
        (
            Instant::now() + Duration::from_secs(30),
            &AtomicBool::new(false),
        ),
        &tree,
    )
    .err()
    .unwrap();
    assert_eq!(error.code, "EXTERNAL_ROOT_UNSUPPORTED");
    assert!(error.message.contains("cycle"));
    assert!(!tree.parent().unwrap().exists());
    assert_eq!(observe(&repo.0), before);
    assert_eq!(observe(&dep.0), dependency);
}

#[test]
fn ambiguous_membership_and_workspace_pointers_refuse_before_scratch() {
    for membership in [
        "members=['member','other/*']",
        "members=['other/?']",
        "members=['other/[a-z]']",
        "members=['other/{a,b}']",
        "members='member'",
        "members=[1]",
        "members=[['member']]",
        "members=['member' 'other']",
        "members=['member',,]",
        "members=['member'",
        "members=[]\n[package]\nworkspace='../other'",
    ] {
        let (workspace, repo, _, config) = enclosing_workspace_fixture();
        workspace.write("Cargo.toml", &format!("[workspace]\n{membership}\n"));
        let state = Fixture::empty();
        let tree = state.0.join("server-state/discovery");
        let before = observe(&workspace.0);
        let error = discover_at(
            &repo.0.canonicalize().unwrap(),
            "src/lib.rs",
            &config,
            &Limits::default(),
            (
                Instant::now() + Duration::from_secs(30),
                &AtomicBool::new(false),
            ),
            &tree,
        )
        .err()
        .unwrap();
        assert_eq!(error.code, "EXTERNAL_ROOT_UNSUPPORTED", "{membership}");
        assert!(!tree.parent().unwrap().exists(), "{membership}");
        assert_eq!(observe(&workspace.0), before, "{membership}");
    }
    let (_, members, _) = manifest_candidates(
        r#"
# members = ['ignored/*']
description = "members = ['ignored/*']"
[workspace]
"mem\u0062ers" = [
    'member', # ordinary member
    "other",
]
"#,
    )
    .unwrap();
    assert_eq!(members, ["member", "other"]);
}

#[test]
fn transitive_manifest_failures_and_shared_budgets_refuse_before_scratch() {
    for case in [
        "missing",
        "directory",
        "symlink",
        "utf8",
        "literal",
        "files",
        "bytes",
        "depth",
    ] {
        let (repo, dep, state, config) = fixture();
        let path = dep.0.join("Cargo.toml");
        let mut limits = Limits::default();
        match case {
            "missing" => fs::rename(&path, dep.0.join("saved.toml")).unwrap(),
            "directory" => {
                fs::rename(&path, dep.0.join("saved.toml")).unwrap();
                fs::create_dir(&path).unwrap();
            }
            "symlink" => {
                fs::rename(&path, dep.0.join("saved.toml")).unwrap();
                std::os::unix::fs::symlink("saved.toml", &path).unwrap();
            }
            "utf8" => fs::write(&path, [0xff]).unwrap(),
            "literal" => dep.write("Cargo.toml", "[dependencies]\npath=42"),
            "files" => limits.max_files = 2,
            "bytes" => limits.max_source_bytes = fs::read(repo.0.join("Cargo.toml")).unwrap().len(),
            "depth" => {
                // All roots are siblings: only the candidate graph, not ancestor
                // directory depth, can hit the hop limit in this fixture.
                for i in 0..ANCESTOR_CAP {
                    let name = format!("chain-{i}");
                    let next = format!("../chain-{}", i + 1);
                    state.write(&format!("{name}/Cargo.toml"), &format!("[package]\nname='chain-{i}'\nversion='0.1.0'\n[dependencies]\nnext={{path='{next}'}}\n"));
                }
                state.write(
                    &format!("chain-{ANCESTOR_CAP}/Cargo.toml"),
                    "[package]\nname='last'\nversion='0.1.0'\n",
                );
                dep.write(
                    "Cargo.toml",
                    &format!(
                        "[dependencies]\nnext={{path={:?}}}\n",
                        state
                            .0
                            .join("chain-0")
                            .canonicalize()
                            .unwrap()
                            .to_str()
                            .unwrap()
                    ),
                );
            }
            _ => unreachable!(),
        }
        let tree = state.0.join("server-state/discovery");
        let before = observe(&repo.0);
        let dependency = observe(&dep.0);
        let error = discover_at(
            &repo.0.canonicalize().unwrap(),
            "src/lib.rs",
            &config,
            &limits,
            (
                Instant::now() + Duration::from_secs(30),
                &AtomicBool::new(false),
            ),
            &tree,
        )
        .err()
        .unwrap();
        let expected = match case {
            "missing" => "EXTERNAL_DEPENDENCY_UNREADABLE",
            "utf8" | "literal" => "EXTERNAL_ROOT_UNSUPPORTED",
            _ => "EXTERNAL_CORPUS_INCOMPLETE",
        };
        assert_eq!(error.code, expected, "{case}: {}", error.message);
        assert!(!tree.parent().unwrap().exists(), "{case}");
        assert_eq!(observe(&repo.0), before, "{case}");
        assert_eq!(observe(&dep.0), dependency, "{case}");
    }
}

#[test]
fn benign_transitive_candidates_join_the_resolved_source_closure() {
    let (repo, dep, state, config) = fixture();
    let leaf = Fixture::empty();
    leaf.write(
        "Cargo.toml",
        "[package]\nname='leaf'\nversion='0.1.0'\nedition='2024'\n",
    );
    leaf.write("src/lib.rs", "pub struct Leaf;\n");
    let text = fs::read_to_string(dep.0.join("Cargo.toml")).unwrap();
    dep.write(
        "Cargo.toml",
        &format!(
            "{text}\n[dependencies]\nleaf={{path={:?}}}\n",
            leaf.0.canonicalize().unwrap().to_str().unwrap()
        ),
    );
    let controls = (
        Instant::now() + Duration::from_secs(30),
        &AtomicBool::new(false),
    );
    let tree = state.0.join("server-state/discovery");
    let observed = manifest_closure(
        ancestor_manifests(&repo.0.canonicalize().unwrap(), controls).unwrap(),
        &tree,
        &Limits::default(),
        controls,
    )
    .unwrap();
    assert!(observed.contains_key(&leaf.0.canonicalize().unwrap().join("Cargo.toml")));
    assert!(
        observed
            .keys()
            .all(|p| p.file_name().unwrap() == "Cargo.toml"
                || p.file_name().unwrap() == "Cargo.lock")
    );
    assert!(!tree.parent().unwrap().exists());
    let output = Command::new("cargo")
        .args(["generate-lockfile", "--offline"])
        .current_dir(&repo.0)
        .output()
        .unwrap();
    assert!(output.status.success());
    let before = observe(&repo.0);
    let dependency = observe(&dep.0);
    let nested = observe(&leaf.0);
    let corpus = discover_at(
        &repo.0.canonicalize().unwrap(),
        "src/lib.rs",
        &config,
        &Limits::default(),
        controls,
        &tree,
    )
    .unwrap();
    assert_eq!(corpus.coverage.consulted_roots.len(), 2);
    assert_eq!(corpus.coverage.consulted_roots[1].package, "leaf");
    assert!(corpus.texts.values().any(|s| s == "pub struct Leaf;\n"));
    corpus.recheck(controls).unwrap();
    assert_eq!(observe(&repo.0), before);
    assert_eq!(observe(&dep.0), dependency);
    assert_eq!(observe(&leaf.0), nested);
}

#[test]
fn unconfigured_disjoint_workspace_inputs_are_bound_without_source_admission() {
    let (workspace, repo, dep, config) = enclosing_workspace_fixture();
    let other = Fixture::empty();
    other.write(
        "Cargo.toml",
        "[package]\nname='other-library'\nversion='0.1.0'\n",
    );
    other.write("src/lib.rs", "pub struct Other;\n");
    workspace.write("other/Cargo.toml", &format!("[package]\nname='other-member'\nversion='0.1.0'\n[dependencies]\nother={{package='other-library',path={:?}}}\n", other.0.canonicalize().unwrap().to_str().unwrap()));
    workspace.write("other/src/lib.rs", "pub struct Member;\n");
    let text = fs::read_to_string(workspace.0.join("Cargo.toml")).unwrap();
    workspace.write(
        "Cargo.toml",
        &text.replace("members=['member']", "members=['member','other']"),
    );
    let output = Command::new("cargo")
        .args(["generate-lockfile", "--offline"])
        .current_dir(&workspace.0)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let state = Fixture::empty();
    let controls = (
        Instant::now() + Duration::from_secs(30),
        &AtomicBool::new(false),
    );
    let corpus = discover_at(
        &repo.0.canonicalize().unwrap(),
        "src/lib.rs",
        &config,
        &Limits::default(),
        controls,
        &state.0,
    )
    .unwrap();
    assert!(
        corpus
            .observed
            .contains_key(&other.0.canonicalize().unwrap().join("Cargo.toml"))
    );
    assert_eq!(corpus.coverage.consulted_roots.len(), 1);
    assert_eq!(
        corpus.coverage.consulted_roots[0].root,
        dep.0.canonicalize().unwrap().to_str().unwrap()
    );
    assert_eq!(corpus.configuration.crates.len(), config.crates.len() + 1);
    corpus.recheck(controls).unwrap();
    let text = fs::read_to_string(other.0.join("Cargo.toml")).unwrap();
    other.write(
        "Cargo.toml",
        &format!("{text}\n# changed indirect manifest\n"),
    );
    assert_eq!(
        corpus.recheck(controls).unwrap_err().code,
        "EXTERNAL_SOURCE_CHANGED"
    );
}

#[test]
fn inherited_path_candidate_cycles_refuse_before_scratch() {
    let (workspace, repo, mut dep, config) = enclosing_workspace_fixture();
    fs::rename(&dep.0, workspace.0.join("dep-member")).unwrap();
    dep.0 = workspace.0.join("dep-member");
    workspace.write("Cargo.toml", "[workspace]\nmembers=['member','dep-member']\nresolver='3'\n[workspace.dependencies]\ndep={package='dependency-library',path='dep-member'}\ncaller={path='member'}\n");
    let text = fs::read_to_string(dep.0.join("Cargo.toml")).unwrap();
    dep.write(
        "Cargo.toml",
        &format!("{text}\n[dependencies]\ncaller.workspace=true\n"),
    );
    let state = Fixture::empty();
    let tree = state.0.join("server-state/discovery");
    let before = observe(&workspace.0);
    let error = discover_at(
        &repo.0.canonicalize().unwrap(),
        "src/lib.rs",
        &config,
        &Limits::default(),
        (
            Instant::now() + Duration::from_secs(30),
            &AtomicBool::new(false),
        ),
        &tree,
    )
    .err()
    .unwrap();
    assert_eq!(error.code, "EXTERNAL_ROOT_UNSUPPORTED");
    assert!(error.message.contains("cycle"));
    assert!(!tree.parent().unwrap().exists());
    assert_eq!(observe(&workspace.0), before);
}

#[test]
fn candidate_hop_budget_includes_cached_dag_tails() {
    let mut edges = BTreeMap::new();
    for i in 1..=ANCESTOR_CAP {
        edges.insert(
            PathBuf::from(format!("/candidate/{i}")),
            vec![PathBuf::from(format!("/candidate/{}", i - 1))],
        );
    }
    let controls = (
        Instant::now() + Duration::from_secs(30),
        &AtomicBool::new(false),
    );
    let mut done = BTreeMap::new();
    let inherited = BTreeSet::new();
    // Cache short tails first: memoization must not hide a too-long chain.
    for i in 0..ANCESTOR_CAP {
        assert_eq!(
            acyclic_paths(
                &PathBuf::from(format!("/candidate/{i}")),
                &edges,
                &inherited,
                &mut BTreeSet::new(),
                &mut done,
                controls
            )
            .unwrap(),
            i + 1
        );
    }
    let error = acyclic_paths(
        &PathBuf::from(format!("/candidate/{ANCESTOR_CAP}")),
        &edges,
        &inherited,
        &mut BTreeSet::new(),
        &mut done,
        controls,
    )
    .unwrap_err();
    assert_eq!(error.code, "EXTERNAL_CORPUS_INCOMPLETE");
}

#[test]
fn dependency_enclosing_workspace_siblings_join_the_preflight() {
    let workspace = Fixture::empty();
    let (repo, mut dep, _, config) = fixture();
    fs::rename(&dep.0, workspace.0.join("dep-member")).unwrap();
    dep.0 = workspace.0.join("dep-member");
    repo.write("Cargo.toml", &format!("[package]\nname='caller'\nversion='0.1.0'\nedition='2024'\n[dependencies]\ndep={{package='dependency-library',path={:?}}}\n", dep.0.canonicalize().unwrap().to_str().unwrap()));
    let nested = Fixture::empty();
    nested.write("Cargo.toml", "[package]\nname='nested'\nversion='0.1.0'\n");
    nested.write("src/lib.rs", "pub struct Nested;\n");
    workspace.write(
        "Cargo.toml",
        "[workspace]\nmembers=['dep-member','other']\nresolver='3'\n",
    );
    workspace.write(
        "other/Cargo.toml",
        &format!(
            "[package]\nname='other'\nversion='0.1.0'\n[dependencies]\nnested={{path={:?}}}\n",
            nested.0.canonicalize().unwrap().to_str().unwrap()
        ),
    );
    workspace.write("other/src/lib.rs", "pub struct Other;\n");
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
    let tree = nested.0.join("server-state/discovery");
    let before = observe(&workspace.0);
    let error = discover_at(
        &repo.0.canonicalize().unwrap(),
        "src/lib.rs",
        &config,
        &Limits::default(),
        (
            Instant::now() + Duration::from_secs(30),
            &AtomicBool::new(false),
        ),
        &tree,
    )
    .err()
    .unwrap();
    assert_eq!(error.code, "EXTERNAL_STATE_OVERLAP");
    assert!(!tree.parent().unwrap().exists());
    assert_eq!(observe(&workspace.0), before);
}

fn fixture() -> (Fixture, Fixture, Fixture, Configuration) {
    let repo = Fixture::generate();
    let dep = Fixture::empty();
    let state = Fixture::empty();
    dep.write(
        "Cargo.toml",
        "[package]\nname='dependency-library'\nversion='0.1.0'\nedition='2024'\n",
    );
    dep.write("src/lib.rs", "pub struct Thing; impl Thing { pub fn new() -> Self { Self } pub fn configure(&self) -> u32 { 7 } }\n");
    repo.write("Cargo.toml", &format!("[package]\nname='caller'\nversion='0.1.0'\nedition='2024'\n[dependencies]\ndep={{package='dependency-library',path={:?}}}\n", dep.0.canonicalize().unwrap().to_str().unwrap()));
    repo.write("src/lib.rs", "mod source;\n");
    repo.write(
        "src/source.rs",
        "pub fn selected() -> u32 { dep::Thing::new().configure() }\n",
    );
    let output = Command::new("cargo")
        .args(["generate-lockfile", "--offline"])
        .current_dir(&repo.0)
        .output()
        .unwrap();
    assert!(output.status.success());
    let config = serde_json::from_value(serde_json::json!({"crates":[{"name":"caller","root_file":"src/lib.rs","edition":"2024","features":[],"cfg":[],"dependencies":[]}]})).unwrap();
    (repo, dep, state, config)
}
fn enclosing_workspace_fixture() -> (Fixture, Fixture, Fixture, Configuration) {
    let workspace = Fixture::empty();
    let (mut repo, dep, _, config) = fixture();
    fs::rename(&repo.0, workspace.0.join("member")).unwrap();
    repo.0 = workspace.0.join("member");
    repo.write("Cargo.toml", "[package]\nname='caller'\nversion='0.1.0'\nedition='2024'\n[dependencies]\ndep.workspace=true\n");
    workspace.write("Cargo.toml", &format!("[workspace]\nmembers=['member']\nresolver='3'\n[workspace.dependencies]\ndep={{package='dependency-library',path='../{}'}}\n", dep.0.file_name().unwrap().to_str().unwrap()));
    let output = Command::new("cargo")
        .args(["generate-lockfile", "--offline"])
        .current_dir(&workspace.0)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    // Keep the pre-existing member lock so the old discovery path reaches Cargo.
    (workspace, repo, dep, config)
}
#[test]
fn enclosing_workspace_overlap_refuses_before_creating_discovery_state() {
    let (workspace, repo, dep, config) = enclosing_workspace_fixture();
    let tree = dep.0.join("server-state/discovery");
    let before = observe(&workspace.0);
    let dependency = observe(&dep.0);
    let error = discover_at(
        &repo.0.canonicalize().unwrap(),
        "src/lib.rs",
        &config,
        &Limits::default(),
        (
            Instant::now() + Duration::from_secs(30),
            &AtomicBool::new(false),
        ),
        &tree,
    )
    .err()
    .unwrap();
    assert!(
        !dep.0.join("server-state").exists(),
        "scratch state was created before refusal"
    );
    assert_eq!(error.code, "EXTERNAL_STATE_OVERLAP");
    assert_eq!(observe(&workspace.0), before);
    assert_eq!(observe(&dep.0), dependency);
}
#[test]
fn enclosing_workspace_root_and_unsafe_manifests_refuse_before_scratch() {
    for case in [
        "workspace_overlap",
        "directory",
        "symlink",
        "utf8",
        "literal",
        "cap",
    ] {
        let (workspace, repo, dep, config) = enclosing_workspace_fixture();
        let state = Fixture::empty();
        let tree = if case == "workspace_overlap" {
            workspace.0.join("server-state/discovery")
        } else {
            state.0.join("server-state/discovery")
        };
        let mut limits = Limits::default();
        match case {
            "directory" => {
                fs::rename(
                    workspace.0.join("Cargo.toml"),
                    workspace.0.join("saved.toml"),
                )
                .unwrap();
                fs::create_dir(workspace.0.join("Cargo.toml")).unwrap();
            }
            "symlink" => {
                fs::rename(
                    workspace.0.join("Cargo.toml"),
                    workspace.0.join("saved.toml"),
                )
                .unwrap();
                std::os::unix::fs::symlink("saved.toml", workspace.0.join("Cargo.toml")).unwrap();
            }
            "utf8" => fs::write(workspace.0.join("Cargo.toml"), [0xff]).unwrap(),
            "literal" => workspace.write("Cargo.toml", "[workspace]\npath = \"unterminated"),
            "cap" => limits.max_file_bytes = 1,
            _ => {}
        }
        let before = observe(&workspace.0);
        let dependency = observe(&dep.0);
        let error = discover_at(
            &repo.0.canonicalize().unwrap(),
            "src/lib.rs",
            &config,
            &limits,
            (
                Instant::now() + Duration::from_secs(30),
                &AtomicBool::new(false),
            ),
            &tree,
        )
        .err()
        .unwrap();
        let code = match case {
            "workspace_overlap" => "EXTERNAL_STATE_OVERLAP",
            "directory" | "symlink" | "cap" => "EXTERNAL_CORPUS_INCOMPLETE",
            _ => "EXTERNAL_ROOT_UNSUPPORTED",
        };
        assert_eq!(error.code, code, "{case}");
        assert!(!tree.parent().unwrap().exists(), "{case}");
        assert_eq!(observe(&workspace.0), before, "{case}");
        assert_eq!(observe(&dep.0), dependency, "{case}");
    }
}
#[test]
fn ancestor_walk_refuses_instead_of_truncating_at_the_hop_cap() {
    let directory = PathBuf::from("/").join("nested/".repeat(ANCESTOR_CAP));
    let error = ancestor_manifests(
        &directory,
        (
            Instant::now() + Duration::from_secs(30),
            &AtomicBool::new(false),
        ),
    )
    .unwrap_err();
    assert_eq!(error.code, "EXTERNAL_CORPUS_INCOMPLETE");
}
#[test]
fn enclosing_workspace_lock_and_manifest_are_bound_at_publication() {
    for case in ["positive", "manifest", "lock"] {
        let (workspace, repo, dep, config) = enclosing_workspace_fixture();
        let state = Fixture::empty();
        // Cargo's authoritative workspace lock alone is sufficient for discovery.
        fs::rename(repo.0.join("Cargo.lock"), repo.0.join("unused.lock")).unwrap();
        let before = observe(&workspace.0);
        let dependency = observe(&dep.0);
        let controls = (
            Instant::now() + Duration::from_secs(30),
            &AtomicBool::new(false),
        );
        let corpus = discover_at(
            &repo.0.canonicalize().unwrap(),
            "src/lib.rs",
            &config,
            &Limits::default(),
            controls,
            &state.0,
        )
        .unwrap();
        assert!(
            corpus
                .observed
                .contains_key(&workspace.0.canonicalize().unwrap().join("Cargo.toml"))
        );
        assert!(
            corpus
                .observed
                .contains_key(&workspace.0.canonicalize().unwrap().join("Cargo.lock"))
        );
        if case == "positive" {
            corpus.recheck(controls).unwrap();
            assert_eq!(observe(&workspace.0), before);
            assert_eq!(observe(&dep.0), dependency);
        } else {
            let path = workspace.0.join(if case == "manifest" {
                "Cargo.toml"
            } else {
                "Cargo.lock"
            });
            let mut source = fs::read_to_string(&path).unwrap();
            source.push_str("\n# changed before publication\n");
            fs::write(path, source).unwrap();
            assert_eq!(
                corpus.recheck(controls).unwrap_err().code,
                "EXTERNAL_SOURCE_CHANGED"
            );
        }
    }
}
#[test]
fn publication_reobserves_dependency_bytes_membership_and_directory_identity() {
    for case in ["bytes", "membership", "root_identity", "manifest"] {
        let (repo, dep, state, config) = fixture();
        let flag = AtomicBool::new(false);
        let controls = (Instant::now() + Duration::from_secs(30), &flag);
        let corpus = discover_at(
            &repo.0.canonicalize().unwrap(),
            "src/lib.rs",
            &config,
            &Limits::default(),
            controls,
            &state.0,
        )
        .unwrap();
        assert!(
            fs::read_dir(&state.0).unwrap().next().is_none(),
            "Cargo home must be request-ephemeral"
        );
        match case {
            "bytes" => dep.write("src/lib.rs", "pub struct Changed;"),
            "membership" => dep.write("src/new.rs", "pub struct Added;"),
            "manifest" => dep.write(
                "Cargo.toml",
                "[package]\nname='dependency-library'\nversion='0.2.0'\nedition='2024'\n",
            ),
            "root_identity" => {
                let retained = dep.0.join("retained");
                // Preserve all bytes but replace the directory inode.
                let sibling = dep.0.with_extension("replaced");
                fs::rename(&dep.0, &sibling).unwrap();
                fs::create_dir(&dep.0).unwrap();
                fs::rename(sibling.join("Cargo.toml"), dep.0.join("Cargo.toml")).unwrap();
                fs::rename(sibling.join("src"), dep.0.join("src")).unwrap();
                fs::rename(&sibling, retained).unwrap();
            }
            _ => unreachable!(),
        }
        assert_eq!(
            corpus.recheck(controls).unwrap_err().code,
            "EXTERNAL_SOURCE_CHANGED",
            "{case}"
        );
    }
}
#[test]
fn declared_dependency_overlap_refuses_before_creating_discovery_state() {
    let (repo, dep, _, config) = fixture();
    let tree = dep.0.join("server-state/discovery");
    let before = observe(&repo.0);
    let dependency = observe(&dep.0);
    let flag = AtomicBool::new(false);
    let error = discover_at(
        &repo.0.canonicalize().unwrap(),
        "src/lib.rs",
        &config,
        &Limits::default(),
        (Instant::now() + Duration::from_secs(30), &flag),
        &tree,
    )
    .err()
    .unwrap();
    assert_eq!(error.code, "EXTERNAL_STATE_OVERLAP");
    // Scratch::new creates this parent permanently, even when Drop removes the
    // request Cargo home. Its absence proves we refused before that write.
    assert!(!dep.0.join("server-state").exists());
    assert_eq!(observe(&repo.0), before);
    assert_eq!(observe(&dep.0), dependency);
}
#[test]
fn lexical_path_candidates_cover_toml_strings_keys_tables_and_comments() {
    let source = r#"
# path = '/comment'
description = 'path = "/not-a-declaration"'
[dependencies.one]
path = '../one'
[dependencies]
two = { "pa\u0074h" = "../t\u0077o" }
[target.'cfg(unix)'.dependencies.three]
'path' = '''
../three'''
[workspace.dependencies]
four.path = """
../fo\
    ur"""
[package.metadata]
path = '/also-a-conservative-candidate'
"#;
    assert_eq!(
        declared_paths(source).unwrap(),
        [
            "../one",
            "../two",
            "../three",
            "../four",
            "/also-a-conservative-candidate"
        ]
    );
    assert_eq!(
        declared_paths(r#"path = "\U0000002fdep""#).unwrap(),
        ["/dep"]
    );
    assert_eq!(
        declared_paths("path = \"\"\"\r\n/one\r\n\"\"\"\npath = '''\r\n/two\r\n''' ").unwrap(),
        ["/one\n", "/two\n"]
    );
    assert_eq!(
        declared_paths("path = \"/unterminated").unwrap_err().code,
        "EXTERNAL_ROOT_UNSUPPORTED"
    );
}

#[test]
fn workspace_and_admitted_member_paths_refuse_before_scratch_creation() {
    for case in ["workspace", "admitted_member"] {
        let (repo, dep, _, mut config) = fixture();
        let declared = format!(
            "dep = {{ path = {:?} }}\n",
            dep.0.canonicalize().unwrap().to_str().unwrap()
        );
        repo.write("member/src/lib.rs", "pub struct Member;\n");
        let crate_root = if case == "workspace" {
            repo.write(
                "Cargo.toml",
                &format!("[workspace]\nmembers=['member']\n[workspace.dependencies]\n{declared}"),
            );
            repo.write("member/Cargo.toml", "[package]\nname='caller'\nversion='0.1.0'\nedition='2024'\n[dependencies]\ndep.workspace=true\n");
            config.crates[0].root_file = "member/src/lib.rs".into();
            "member/src/lib.rs"
        } else {
            repo.write("member/Cargo.toml", &format!("[package]\nname='member'\nversion='0.1.0'\nedition='2024'\n[dependencies]\n{declared}"));
            let mut member = config.crates[0].clone();
            member.name = "member".into();
            member.root_file = "member/src/lib.rs".into();
            config.crates.push(member);
            "src/lib.rs"
        };
        let tree = dep.0.join("server-state/discovery");
        let before = observe(&repo.0);
        let dependency = observe(&dep.0);
        let error = discover_at(
            &repo.0.canonicalize().unwrap(),
            crate_root,
            &config,
            &Limits::default(),
            (
                Instant::now() + Duration::from_secs(30),
                &AtomicBool::new(false),
            ),
            &tree,
        )
        .err()
        .unwrap();
        assert_eq!(error.code, "EXTERNAL_STATE_OVERLAP", "{case}");
        assert!(!dep.0.join("server-state").exists(), "{case}");
        assert_eq!(observe(&repo.0), before);
        assert_eq!(observe(&dep.0), dependency);
    }
}

#[test]
fn discovery_state_failure_overlap_and_cancel_leave_sources_untouched() {
    let (repo, dep, state, config) = fixture();
    let before = observe(&repo.0);
    let dependency = observe(&dep.0);
    let flag = AtomicBool::new(false);
    let controls = (Instant::now() + Duration::from_secs(30), &flag);
    state.write("occupied", "not a directory");
    let error = discover_at(
        &repo.0.canonicalize().unwrap(),
        "src/lib.rs",
        &config,
        &Limits::default(),
        controls,
        &state.0.join("occupied"),
    );
    assert_eq!(error.err().unwrap().code, "EXTERNAL_STATE_UNAVAILABLE");
    assert_eq!(
        disjoint(&repo.0.join("state"), [repo.0.clone()])
            .unwrap_err()
            .code,
        "EXTERNAL_STATE_OVERLAP"
    );
    assert_eq!(
        disjoint(&dep.0.join("state"), [dep.0.clone()])
            .unwrap_err()
            .code,
        "EXTERNAL_STATE_OVERLAP"
    );
    flag.store(true, Ordering::Relaxed);
    assert_eq!(
        discover_at(
            &repo.0.canonicalize().unwrap(),
            "src/lib.rs",
            &config,
            &Limits::default(),
            controls,
            &state.0
        )
        .err()
        .unwrap()
        .code,
        "CANCELLED"
    );
    assert_eq!(observe(&repo.0), before);
    assert_eq!(observe(&dep.0), dependency);
}
#[test]
fn cancellation_during_metadata_reaps_the_process_and_removes_cargo_home() {
    let (repo, dep, state, _) = fixture();
    let before = observe(&repo.0);
    let dependency = observe(&dep.0);
    let flag = AtomicBool::new(false);
    let error = std::thread::scope(|threads| {
        threads.spawn(|| {
            // Wait for the scratch directory, not a timer: cancellation happens
            // inside discovery after its initial admission check.
            let until = Instant::now() + Duration::from_secs(10);
            while Instant::now() < until {
                if fs::read_dir(&state.0).unwrap().next().is_some() {
                    flag.store(true, Ordering::Relaxed);
                    return;
                }
                std::thread::yield_now();
            }
            panic!("scratch Cargo home was not observed");
        });
        metadata(
            &repo.0,
            &state.0,
            (Instant::now() + Duration::from_secs(30), &flag),
        )
        .err()
        .unwrap()
    });
    assert_eq!(error.code, "CANCELLED");
    assert!(fs::read_dir(&state.0).unwrap().next().is_none());
    assert_eq!(observe(&repo.0), before);
    assert_eq!(observe(&dep.0), dependency);
}
#[test]
fn output_and_capture_caps_never_return_partial_success() {
    let flag = AtomicBool::new(false);
    let exceeded = AtomicBool::new(false);
    let bytes = bounded_output(std::io::repeat(0).take(OUTPUT_CAP as u64 + 1), &exceeded).unwrap();
    assert!(exceeded.load(Ordering::Relaxed));
    assert!(bytes.len() <= OUTPUT_CAP);
    let (_, dep, _, _) = fixture();
    let limits = Limits {
        max_source_bytes: 1,
        ..Limits::default()
    };
    let error = capture(
        &dep.0,
        &dep.0.join("src/lib.rs"),
        "@external/0/",
        &limits,
        (Instant::now() + Duration::from_secs(30), &flag),
        &mut BTreeMap::new(),
        &mut BTreeMap::new(),
    )
    .unwrap_err();
    assert_eq!(error.code, "EXTERNAL_CORPUS_INCOMPLETE");
    dep.write("src/lib.rs", "mod missing; pub struct Thing;");
    let error = capture(
        &dep.0,
        &dep.0.join("src/lib.rs"),
        "@external/0/",
        &Limits::default(),
        (Instant::now() + Duration::from_secs(30), &flag),
        &mut BTreeMap::new(),
        &mut BTreeMap::new(),
    )
    .unwrap_err();
    assert_eq!(error.code, "EXTERNAL_CORPUS_INCOMPLETE");
}
#[test]
fn cancellation_between_capture_chunks_refuses_instead_of_publishing_a_prefix() {
    struct CancellingReader<'a>(&'a AtomicBool);
    impl Read for CancellingReader<'_> {
        fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
            bytes[0] = b'x';
            self.0.store(true, Ordering::Relaxed);
            Ok(1)
        }
    }
    let flag = AtomicBool::new(false);
    let error = read_chunks(
        CancellingReader(&flag),
        1024,
        (Instant::now() + Duration::from_secs(30), &flag),
    )
    .unwrap_err();
    assert_eq!(error.code, "CANCELLED");
}
#[test]
fn stale_dependency_at_move_publication_withholds_all_artifacts() {
    let (repo, dep, _, config) = fixture();
    let text = "pub fn selected() -> u32 { dep::Thing::new().configure() }";
    let source = fs::read_to_string(repo.0.join("src/source.rs")).unwrap();
    let start = source.find(text).unwrap();
    let request = serde_json::from_value(serde_json::json!({"repo_path":repo.0,"crate_root":"src/lib.rs","paths":["src"],"resolve_semantic":true,"external_path_dependencies":true,"semantic_configuration":config,"limits":{"time_budget_ms":30000},"moves":[{"item":{"path":"src/source.rs","range":{"start_byte":start,"end_byte":start+text.len()},"expected_text":text},"destination":{"kind":"new_sibling","path":"src/moved.rs","parent_path":"src/lib.rs"}}]})).unwrap();
    let result = super::super::run_with_recheck(&repo.0, request, &AtomicBool::new(false), || {
        dep.write("src/lib.rs", "pub struct Changed;")
    });
    assert_eq!(result.error.unwrap().code, "EXTERNAL_SOURCE_CHANGED");
    assert!(!result.plan.applicable);
    assert!(result.plan.patch.is_none());
    assert!(result.plan.edits.is_none());
    assert!(result.plan.created_files.is_none());
}
