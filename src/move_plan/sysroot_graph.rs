//! Finite source-analysis graph, not Cargo resolution or a shipped binary recipe.
use super::{CfgFact, Coverage, Identity, PREFIX, refuse};
use crate::{
    move_plan::semantic::{CfgInput, Configuration, CrateInput, DependencyInput},
    result::DomainError,
};
use rmcp::schemars::JsonSchema;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct Root {
    pub name: String,
    pub root_file: String,
    pub manifest: String,
    pub edition: String,
    pub features: Vec<String>,
    pub dependencies: Vec<DependencyInput>,
    pub language_origin: String,
    pub source_root: String,
}
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct Edge {
    pub from: String,
    pub alias: String,
    pub to: String,
    pub in_extern_prelude: bool,
    pub sysroot_injected: bool,
}
#[derive(Clone)]
pub(crate) struct Profile {
    pub configuration: Configuration,
    pub origins: BTreeMap<String, String>,
    pub cfg: BTreeMap<String, BTreeMap<(String, Option<String>), bool>>,
}
struct Node {
    name: &'static str,
    directory: &'static str,
    edition: &'static str,
    manifest_hash: &'static str,
    features: &'static [&'static str],
    feature_domain: &'static [&'static str],
    dependencies: &'static [(&'static str, &'static str)],
}
// These hashes identify the manifests reviewed for this exact analysis recipe;
// they are neither signatures nor independent compiler-release authentication.
const NODES: &[Node] = &[
    Node {
        name: "core",
        directory: "core",
        edition: "2024",
        manifest_hash: "234c3bd2559267eefe9daf6f2b96c63ce4c3e8cb79c11a9d871ed2a493c9413a",
        features: &[],
        feature_domain: &[
            "panic_immediate_abort",
            "optimize_for_size",
            "debug_refcell",
            "llvm_enzyme",
        ],
        dependencies: &[],
    },
    Node {
        name: "compiler_builtins",
        directory: "compiler-builtins/compiler-builtins",
        edition: "2024",
        manifest_hash: "d89e31fed70f2cad9c6e91b787bf4c0fb680c631196b2a7215c81745aaf7fb7e",
        features: &["arch", "compiler-builtins", "unmangled-names"],
        feature_domain: &[
            "default",
            "arch",
            "c",
            "compiler-builtins",
            "mem",
            "unmangled-names",
            "unstable-public-internals",
        ],
        dependencies: &[("core", "core")],
    },
    Node {
        name: "alloc",
        directory: "alloc",
        edition: "2024",
        manifest_hash: "3a0273fc73bd77700c12cacb61a751cffd3ab24d26899af33d5d7109b252f6f4",
        features: &[],
        feature_domain: &[
            "compiler-builtins-mem",
            "compiler-builtins-c",
            "optimize_for_size",
        ],
        dependencies: &[("core", "core"), ("compiler_builtins", "compiler_builtins")],
    },
    Node {
        name: "workspace_core",
        directory: "rustc-std-workspace-core",
        edition: "2024",
        manifest_hash: "f5e111f26351f69199f2a251b8f5b6c686920b8b234df084a1a616690357b5aa",
        features: &[],
        feature_domain: &[],
        dependencies: &[("core", "core"), ("compiler_builtins", "compiler_builtins")],
    },
    Node {
        name: "workspace_alloc",
        directory: "rustc-std-workspace-alloc",
        edition: "2024",
        manifest_hash: "40f761fd329b7ac42e0f163784227059780b99310dd7ea4f618641dab07c7514",
        features: &[],
        feature_domain: &[],
        dependencies: &[("alloc", "alloc")],
    },
    Node {
        name: "cfg_if",
        directory: "vendor/cfg-if-1.0.4",
        edition: "2018",
        manifest_hash: "281d508beb1fe3927cf03d3f2f8c9a5117b1e4fe97ae21b9026cf318e8c35273",
        features: &["rustc-dep-of-std"],
        feature_domain: &["core", "rustc-dep-of-std"],
        dependencies: &[("core", "workspace_core")],
    },
    Node {
        name: "libc",
        directory: "vendor/libc-0.2.185",
        edition: "2021",
        manifest_hash: "89d682c2bf38c3051425fd0fc4319e31d55d4d20082fc6f3445f4681e95a4b8d",
        features: &["align", "rustc-dep-of-std", "rustc-std-workspace-core"],
        feature_domain: &[
            "align",
            "const-extern-fn",
            "default",
            "extra_traits",
            "rustc-dep-of-std",
            "rustc-std-workspace-core",
            "std",
            "use_std",
        ],
        dependencies: &[("rustc_std_workspace_core", "workspace_core")],
    },
    Node {
        name: "panic_abort",
        directory: "panic_abort",
        edition: "2024",
        manifest_hash: "ace456c256fad9b7344fba2b806b65e1321245c81d8fc8e7a0bdc24b391e6a55",
        features: &[],
        feature_domain: &[],
        dependencies: &[("core", "workspace_core")],
    },
    Node {
        name: "unwind",
        directory: "unwind",
        edition: "2024",
        manifest_hash: "cde3322f8dcdabd1bba87e0412073ded92a28d2e5b49b4a3eb62d79e137d2013",
        features: &[],
        feature_domain: &["llvm-libunwind", "system-llvm-libunwind"],
        dependencies: &[("core", "workspace_core"), ("libc", "libc")],
    },
    Node {
        name: "std_detect",
        directory: "std_detect",
        edition: "2024",
        manifest_hash: "7e42961094ca6980d4852bb5b38350ac505542df04d71a66d86b4f3ff72dd72f",
        features: &[],
        feature_domain: &["std_detect_env_override"],
        dependencies: &[
            ("core", "workspace_core"),
            ("alloc", "workspace_alloc"),
            ("libc", "libc"),
        ],
    },
    Node {
        name: "hashbrown",
        directory: "vendor/hashbrown-0.17.1",
        edition: "2024",
        manifest_hash: "afcb0e62d498f756bd4ddf69785c8282d01ee4fa40016158c7eac12ceb302f8c",
        features: &[
            "nightly",
            "core",
            "alloc",
            "rustc-internal-api",
            "rustc-dep-of-std",
        ],
        feature_domain: &[
            "default",
            "default-hasher",
            "inline-more",
            "allocator-api2",
            "equivalent",
            "raw-entry",
            "nightly",
            "core",
            "alloc",
            "rustc-internal-api",
            "rustc-dep-of-std",
            "serde",
            "serde_core",
            "foldhash",
            "rayon",
        ],
        dependencies: &[("core", "workspace_core"), ("alloc", "workspace_alloc")],
    },
    Node {
        name: "rustc_demangle",
        directory: "vendor/rustc-demangle-0.1.27",
        edition: "2015",
        manifest_hash: "ec462e775f134cf7646d9dda346c43bbc2c0cd44def02e3af5b678bb6a9d879c",
        features: &["core", "rustc-dep-of-std"],
        feature_domain: &["core", "compiler_builtins", "rustc-dep-of-std", "std"],
        dependencies: &[("core", "workspace_core")],
    },
    Node {
        name: "std",
        directory: "std",
        edition: "2024",
        manifest_hash: "ff5272b18cb16a9e67256a1045dc35d9d08ae38916a218f684a2ae2f4360f078",
        features: &[],
        feature_domain: &[
            "backtrace",
            "backtrace-trace-only",
            "panic-unwind",
            "compiler-builtins-c",
            "compiler-builtins-mem",
            "llvm-libunwind",
            "system-llvm-libunwind",
            "optimize_for_size",
            "debug_refcell",
            "llvm_enzyme",
            "windows_raw_dylib",
        ],
        dependencies: &[
            ("alloc", "alloc"),
            ("cfg_if", "cfg_if"),
            ("panic_abort", "panic_abort"),
            ("core", "core"),
            ("unwind", "unwind"),
            ("hashbrown", "hashbrown"),
            ("std_detect", "std_detect"),
            ("rustc_demangle", "rustc_demangle"),
            ("libc", "libc"),
        ],
    },
];
// No wildcard: additions require reviewing the exact installed predicate and
// disclosing the individual source-analysis value and its consequence.
const INTERNAL_OFF: &[&str] = &[
    "no_global_oom_handling",
    "no_fp_fmt_parse",
    "no_rc",
    "no_sync",
    "rustc_use_supported_cpu_features",
    "test",
    "doc",
    "doctest",
    "docsrs",
    "rustfmt",
    "miri",
    "bootstrap",
];
const IDENTIFICATION_DOMAINS: &[&str] = &[
    "target_arch",
    "target_os",
    "target_env",
    "target_vendor",
    "target_family",
    "target_endian",
    "target_pointer_width",
    "target_feature",
    "target_has_atomic",
    "target_has_atomic_load_store",
    "target_has_atomic_primitive_alignment",
    "target_thread_local",
    "target_has_reliable_f16",
    "target_has_reliable_f128",
    "target_has_reliable_f16_math",
    "target_object_format",
    "target_abi",
    "fmt_debug",
    "relocation_model",
    "panic",
    "unix",
    "windows",
    "debug_assertions",
    "ub_checks",
    "overflow_checks",
    "contract_checks",
];
fn name(node: &str) -> String {
    format!("sysroot_{node}")
}
fn add_fact(
    profile: &mut Profile,
    coverage: &mut Coverage,
    crate_name: &str,
    key: &str,
    value: Option<String>,
    enabled: bool,
    evidence: &str,
) -> Result<(), DomainError> {
    let facts = profile.cfg.entry(crate_name.into()).or_default();
    let atom = (key.into(), value.clone());
    if let Some(existing) = facts.get(&atom) {
        if *existing != enabled {
            return Err(refuse(
                "SYSROOT_CONFIGURATION_UNSUPPORTED",
                &format!("conflicting sysroot cfg recipe: {crate_name}: {key} = {value:?}"),
            ));
        }
    } else {
        facts.insert(atom, enabled);
        coverage.cfg_recipe.push(CfgFact { crate_name: crate_name.into(), key: key.into(), atom_value: value, enabled, evidence: evidence.into(), consequence: format!("sysroot-only predicate is {}; unknown atoms outside this enumerated recipe still refuse", if enabled { "ON" } else { "OFF" }) });
    }
    Ok(())
}
pub(super) fn build(
    library: &Path,
    observed: &BTreeMap<PathBuf, (Vec<u8>, Identity)>,
    texts: &BTreeMap<String, String>,
    caller: &Configuration,
    compiler_cfg: &[CfgInput],
    coverage: &mut Coverage,
) -> Result<Profile, DomainError> {
    if caller.crates.len() + NODES.len() > 32 {
        return Err(refuse(
            "SYSROOT_GRAPH_EXHAUSTED",
            "composite graph exceeds 32 crates",
        ));
    }
    let mut profile = Profile {
        configuration: caller.clone(),
        origins: BTreeMap::new(),
        cfg: BTreeMap::new(),
    };
    for node in NODES {
        let manifest = library.join(node.directory).join("Cargo.toml");
        let data = observed.get(&manifest).ok_or_else(|| {
            refuse(
                "SYSROOT_LAYOUT_UNSUPPORTED",
                "required graph manifest absent",
            )
        })?;
        if super::hex(&Sha256::digest(&data.0)) != node.manifest_hash {
            return Err(refuse(
                "SYSROOT_LAYOUT_UNSUPPORTED",
                &format!("unexamined required manifest for {}", node.name),
            ));
        }
        let crate_name = name(node.name);
        if caller.crates.iter().any(|input| input.name == crate_name) {
            return Err(refuse(
                "SYSROOT_NAMESPACE_CONFLICT",
                "configured crate collides with sysroot graph",
            ));
        }
        let source = if matches!(node.name, "workspace_core" | "workspace_alloc") {
            "lib.rs"
        } else {
            "src/lib.rs"
        };
        let root_file = format!("{PREFIX}{}/{source}", node.directory);
        if !texts.contains_key(&root_file) {
            return Err(refuse(
                "SYSROOT_LAYOUT_UNSUPPORTED",
                "required graph library root absent",
            ));
        }
        let dependencies: Vec<_> = node
            .dependencies
            .iter()
            .map(|(alias, target)| DependencyInput {
                name: (*alias).into(),
                crate_name: name(target),
            })
            .collect();
        coverage.capture_usage.graph_edges += dependencies.len();
        let mut input = CrateInput {
            name: crate_name.clone(),
            root_file: root_file.clone(),
            edition: node.edition.into(),
            features: node.features.iter().map(|s| (*s).into()).collect(),
            cfg: compiler_cfg.to_vec(),
            dependencies: dependencies.clone(),
        };
        coverage.consulted_roots.push(Root {
            name: crate_name.clone(),
            root_file,
            manifest: format!("{PREFIX}{}/Cargo.toml", node.directory),
            edition: node.edition.into(),
            features: input.features.clone(),
            dependencies,
            language_origin: node.name.into(),
            source_root: PREFIX.into(),
        });
        profile.origins.insert(crate_name.clone(), node.name.into());
        for atom in compiler_cfg {
            add_fact(
                &mut profile,
                coverage,
                &crate_name,
                &atom.key,
                atom.value.clone(),
                true,
                "complete selected compiler --print cfg identification; source analysis only",
            )?;
        }
        for feature in node.feature_domain {
            add_fact(
                &mut profile,
                coverage,
                &crate_name,
                "feature",
                Some((*feature).into()),
                node.features.contains(feature),
                "finite audited manifest feature recipe; required normal-edge features ON, optional analysis features OFF; not a claim of binary build equivalence",
            )?;
        }
        for atom in INTERNAL_OFF {
            add_fact(
                &mut profile,
                coverage,
                &crate_name,
                atom,
                None,
                false,
                "explicit audited compiler-internal source-analysis recipe; installed sources, no testing/documentation/custom std build",
            )?;
        }
        if matches!(node.name, "core" | "std") {
            add_fact(
                &mut profile,
                coverage,
                &crate_name,
                "feature",
                Some("as_crate".into()),
                false,
                "captured portable-simd sources are literal included modules, not standalone core_simd/std_float crates; as_crate OFF",
            )?;
        }
        if node.name == "std" {
            for feature in ["std", "cpp_demangle", "serialize-serde", "ruzstd"] {
                add_fact(
                    &mut profile,
                    coverage,
                    &crate_name,
                    "feature",
                    Some(feature.into()),
                    false,
                    "captured backtrace sources included in libstd, not built as a standalone crate; optional standalone features OFF",
                )?;
            }
            for (key, enabled) in [
                ("restricted_std", false),
                ("netbsd10", false),
                ("vxworks_lt_25_09", false),
                ("backtrace_in_libstd", true),
            ] {
                add_fact(
                    &mut profile,
                    coverage,
                    &crate_name,
                    key,
                    None,
                    enabled,
                    "captured std/build.rs audited for fixed macos host; no build script run",
                )?;
                if enabled {
                    input.cfg.push(CfgInput {
                        key: key.into(),
                        value: None,
                    });
                }
            }
        }
        if node.name == "libc" {
            add_fact(
                &mut profile,
                coverage,
                &crate_name,
                "libc_deny_warnings",
                None,
                false,
                "captured libc build.rs: LIBC_CI-only warnings control OFF for official non-CI source analysis; build script not run",
            )?;
        }
        if node.name == "compiler_builtins" {
            // Audited captured configure.rs/build.rs formulas for this exact host;
            // no build-script execution and no caller environment propagation.
            for (key, enabled) in [
                ("f16_enabled", true),
                ("f128_enabled", true),
                ("intrinsics_enabled", true),
                ("mem_unaligned", true),
                ("optimizations_enabled", true),
                ("thumb", false),
                ("thumb_1", false),
                ("x86_no_sse2", false),
                ("kernel_user_helpers", false),
                ("assert_no_panic", false),
            ] {
                add_fact(
                    &mut profile,
                    coverage,
                    &crate_name,
                    key,
                    None,
                    enabled,
                    "captured compiler-builtins/libm/configure.rs and compiler-builtins/build.rs audited for the fixed aarch64 release source-analysis recipe; no build script run",
                )?;
                if enabled {
                    input.cfg.push(CfgInput {
                        key: key.into(),
                        value: None,
                    });
                }
            }
        }
        profile.configuration.crates.push(input);
    }
    for input in profile
        .configuration
        .crates
        .iter_mut()
        .take(caller.crates.len())
    {
        for target in ["core", "alloc", "std"] {
            if input.dependencies.iter().any(|dep| dep.name == target) {
                return Err(refuse(
                    "SYSROOT_CONFIGURATION_UNSUPPORTED",
                    "caller alias core/alloc/std conflicts with injected sysroot graph",
                ));
            }
            input.dependencies.push(DependencyInput {
                name: target.into(),
                crate_name: name(target),
            });
            coverage.capture_usage.graph_edges += 1;
        }
    }
    for input in &profile.configuration.crates {
        if input.dependencies.len() > 32 {
            return Err(refuse(
                "SYSROOT_GRAPH_EXHAUSTED",
                "composite graph exceeds 32 edges per crate",
            ));
        }
        for dependency in &input.dependencies {
            let (in_extern_prelude, sysroot_injected) =
                profile.edge_flags(&input.name, &dependency.crate_name);
            coverage.graph.push(Edge {
                from: input.name.clone(),
                alias: dependency.name.clone(),
                to: dependency.crate_name.clone(),
                in_extern_prelude,
                sysroot_injected,
            });
        }
    }
    if coverage.graph.len() > coverage.capture_usage.max_graph_edges {
        return Err(refuse(
            "SYSROOT_GRAPH_EXHAUSTED",
            "composite graph edge cap exhausted",
        ));
    }
    coverage.capture_usage.graph_edges = coverage.graph.len();
    Ok(profile)
}
impl Profile {
    pub(crate) fn edge_flags(&self, from: &str, target: &str) -> (bool, bool) {
        let injected = !self.origins.contains_key(from) && self.origins.contains_key(target);
        (
            !(injected && self.origins.get(target).is_some_and(|name| name == "alloc")),
            injected,
        )
    }
    pub(crate) fn value(
        &self,
        crate_name: &str,
        key: &str,
        value: &Option<String>,
    ) -> Option<bool> {
        self.cfg
            .get(crate_name)?
            .get(&(key.into(), value.clone()))
            .copied()
    }
    pub(super) fn observe_identification_atom(
        &mut self,
        coverage: &mut Coverage,
        crate_name: &str,
        key: &str,
        value: Option<String>,
    ) -> Result<(), DomainError> {
        if IDENTIFICATION_DOMAINS.contains(&key) && self.value(crate_name, key, &value).is_none() {
            add_fact(
                self,
                coverage,
                crate_name,
                key,
                value,
                false,
                "absent value in the complete selected compiler identification domain; explicit sysroot-only OFF atom",
            )?;
        }
        Ok(())
    }
    pub(super) fn validate_recipe(&self, coverage: &Coverage) -> Result<(), DomainError> {
        let mut disclosed = BTreeMap::new();
        for fact in &coverage.cfg_recipe {
            if disclosed
                .entry(fact.crate_name.clone())
                .or_insert_with(BTreeMap::new)
                .insert((fact.key.clone(), fact.atom_value.clone()), fact.enabled)
                .is_some()
            {
                return Err(refuse(
                    "SYSROOT_CONFIGURATION_UNSUPPORTED",
                    "duplicate cfg disclosure",
                ));
            }
        }
        if disclosed != self.cfg {
            return Err(refuse(
                "SYSROOT_CONFIGURATION_UNSUPPORTED",
                "cfg disclosure differs from admission recipe",
            ));
        }
        for input in self
            .configuration
            .crates
            .iter()
            .filter(|c| self.origins.contains_key(&c.name))
        {
            let configured: BTreeMap<_, _> = input
                .cfg
                .iter()
                .map(|a| ((a.key.clone(), a.value.clone()), true))
                .chain(
                    input
                        .features
                        .iter()
                        .map(|f| (("feature".into(), Some(f.clone())), true)),
                )
                .collect();
            let admitted: BTreeMap<_, _> = self
                .cfg
                .get(&input.name)
                .into_iter()
                .flat_map(|atoms| atoms.iter())
                .filter(|(_, enabled)| **enabled)
                .map(|(atom, enabled)| (atom.clone(), *enabled))
                .collect();
            if configured != admitted {
                return Err(refuse(
                    "SYSROOT_CONFIGURATION_UNSUPPORTED",
                    "analyzer cfg differs from admission recipe",
                ));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile() -> (Profile, Coverage) {
        let mut coverage = super::super::test_coverage();
        let mut profile = Profile {
            configuration: Configuration {
                crates: vec![CrateInput {
                    name: "sysroot_fixture".into(),
                    root_file: format!("{PREFIX}fixture/lib.rs"),
                    edition: "2024".into(),
                    features: vec![],
                    cfg: vec![],
                    dependencies: vec![],
                }],
            },
            origins: BTreeMap::from([("sysroot_fixture".into(), "alloc".into())]),
            cfg: BTreeMap::new(),
        };
        for key in ["ub_checks", "overflow_checks"] {
            profile.configuration.crates[0].cfg.push(CfgInput {
                key: key.into(),
                value: None,
            });
            add_fact(
                &mut profile,
                &mut coverage,
                "sysroot_fixture",
                key,
                None,
                true,
                "compiler identification",
            )
            .unwrap();
        }
        (profile, coverage)
    }

    #[test]
    fn sysroot_identification_on_atoms_cannot_be_overwritten_by_an_off_recipe() {
        let (mut profile, mut coverage) = profile();
        for key in ["ub_checks", "overflow_checks"] {
            assert!(!INTERNAL_OFF.contains(&key));
            profile
                .observe_identification_atom(&mut coverage, "sysroot_fixture", key, None)
                .unwrap();
            assert_eq!(profile.value("sysroot_fixture", key, &None), Some(true));
            assert_eq!(
                add_fact(
                    &mut profile,
                    &mut coverage,
                    "sysroot_fixture",
                    key,
                    None,
                    false,
                    "conflicting recipe"
                )
                .unwrap_err()
                .code,
                "SYSROOT_CONFIGURATION_UNSUPPORTED"
            );
            assert_eq!(profile.value("sysroot_fixture", key, &None), Some(true));
        }
        profile
            .observe_identification_atom(&mut coverage, "sysroot_fixture", "contract_checks", None)
            .unwrap();
        assert_eq!(
            profile.value("sysroot_fixture", "contract_checks", &None),
            Some(false)
        );
        profile
            .observe_identification_atom(&mut coverage, "sysroot_fixture", "unexamined", None)
            .unwrap();
        assert_eq!(profile.value("sysroot_fixture", "unexamined", &None), None);
        profile.validate_recipe(&coverage).unwrap();
    }

    #[test]
    fn sysroot_cfg_disclosure_admission_and_analyzer_must_agree() {
        for mutation in ["disclosure", "duplicate", "admission", "analyzer"] {
            let (mut profile, mut coverage) = profile();
            profile.validate_recipe(&coverage).unwrap();
            match mutation {
                "disclosure" => coverage.cfg_recipe[0].enabled = false,
                "duplicate" => coverage.cfg_recipe.push(coverage.cfg_recipe[0].clone()),
                "admission" => {
                    profile
                        .cfg
                        .get_mut("sysroot_fixture")
                        .unwrap()
                        .insert(("ub_checks".into(), None), false);
                }
                "analyzer" => profile.configuration.crates[0].cfg.clear(),
                _ => unreachable!(),
            }
            assert_eq!(
                profile.validate_recipe(&coverage).unwrap_err().code,
                "SYSROOT_CONFIGURATION_UNSUPPORTED",
                "{mutation}"
            );
        }
    }
}
