//! The finite written-library closure of Cargo's resolved graph, never a resolver.
use super::*;
use std::collections::VecDeque;

pub(super) const CRATE_CAP: usize = 32;
pub(super) struct Graph<'a> {
    packages: BTreeMap<&'a str, &'a Package>,
    nodes: BTreeMap<&'a str, &'a Node>,
}
impl<'a> Graph<'a> {
    pub(super) fn new(metadata: &'a Metadata, controls: Controls<'_>) -> Result<Self, DomainError> {
        let resolve = metadata.resolve.as_ref().ok_or_else(|| {
            refusal(
                "EXTERNAL_DISCOVERY_FAILED",
                "metadata has no resolved graph",
            )
        })?;
        let mut packages = BTreeMap::new();
        let mut identities = BTreeSet::new();
        let mut nodes = BTreeMap::new();
        for package in &metadata.packages {
            items::check(controls.0, controls.1)?;
            if packages.insert(package.id.as_str(), package).is_some()
                || !identities.insert((
                    &package.name,
                    &package.version,
                    &package.source,
                    package.source.is_none().then_some(&package.manifest_path),
                ))
            {
                return Err(refusal(
                    "EXTERNAL_PACKAGE_AMBIGUOUS",
                    "duplicate package identity",
                ));
            }
        }
        for node in &resolve.nodes {
            items::check(controls.0, controls.1)?;
            if nodes.insert(node.id.as_str(), node).is_some() {
                return Err(refusal(
                    "EXTERNAL_PACKAGE_AMBIGUOUS",
                    "duplicate resolved node identity",
                ));
            }
        }
        Ok(Self { packages, nodes })
    }

    pub(super) fn package(&self, id: &str) -> Result<&'a Package, DomainError> {
        self.packages.get(id).copied().ok_or_else(|| {
            refusal(
                "EXTERNAL_DISCOVERY_FAILED",
                "resolved dependency package absent",
            )
        })
    }
    pub(super) fn node(&self, id: &str) -> Result<&'a Node, DomainError> {
        self.nodes.get(id).copied().ok_or_else(|| {
            refusal(
                "EXTERNAL_DISCOVERY_FAILED",
                "resolved dependency node absent",
            )
        })
    }

    // Preserve direct-edge ordering and assign one namespace per package ID.
    // This walk and the cycle check are bounded before any source capture/RA work.
    pub(super) fn closure(
        &self,
        caller: &str,
        available: usize,
        controls: Controls<'_>,
    ) -> Result<Vec<&'a Package>, DomainError> {
        let mut pending = VecDeque::from([caller]);
        let mut seen = BTreeSet::from([caller]);
        let mut libraries = Vec::new();
        let mut edges = BTreeMap::new();
        while let Some(id) = pending.pop_front() {
            items::check(controls.0, controls.1)?;
            self.package(id)?;
            let mut next = Vec::new();
            let mut aliases = BTreeSet::new();
            for edge in normal_edges(self.node(id)?) {
                items::check(controls.0, controls.1)?;
                if edge.dep_kinds.iter().any(|k| k.target.is_some()) {
                    return Err(refusal(
                        "EXTERNAL_CONFIGURATION_UNSUPPORTED",
                        "target-conditional dependency edges are unsupported",
                    ));
                }
                if !aliases.insert(&edge.name) {
                    return Err(refusal(
                        "EXTERNAL_PACKAGE_AMBIGUOUS",
                        "duplicate dependency alias in resolved node",
                    ));
                }
                if next.len() >= CRATE_CAP {
                    return Err(refusal(
                        "EXTERNAL_CORPUS_INCOMPLETE",
                        "external dependency edge cap exhausted (32 per crate)",
                    ));
                }
                let package = self.package(&edge.pkg)?;
                self.node(&edge.pkg)?;
                next.push(edge.pkg.as_str());
                if seen.insert(edge.pkg.as_str()) {
                    if libraries.len() >= available {
                        return Err(refusal(
                            "EXTERNAL_CORPUS_INCOMPLETE",
                            "external crate graph cap exhausted (32 including explicit crates)",
                        ));
                    }
                    libraries.push(package);
                    pending.push_back(&edge.pkg);
                }
            }
            edges.insert(id, next);
        }
        fn acyclic<'a>(
            id: &'a str,
            edges: &BTreeMap<&'a str, Vec<&'a str>>,
            active: &mut BTreeSet<&'a str>,
            done: &mut BTreeSet<&'a str>,
            controls: Controls<'_>,
        ) -> Result<(), DomainError> {
            items::check(controls.0, controls.1)?;
            if done.contains(id) {
                return Ok(());
            }
            if !active.insert(id) {
                return Err(refusal(
                    "EXTERNAL_ROOT_UNSUPPORTED",
                    "resolved library dependency cycle",
                ));
            }
            for next in &edges[id] {
                acyclic(next, edges, active, done, controls)?;
            }
            active.remove(id);
            done.insert(id);
            Ok(())
        }
        acyclic(
            caller,
            &edges,
            &mut BTreeSet::new(),
            &mut BTreeSet::new(),
            controls,
        )?;
        Ok(libraries)
    }

    pub(super) fn check_edge(&self, owner: &Package, edge: &Edge) -> Result<(), DomainError> {
        let package = self.package(&edge.pkg)?;
        let declared = owner
            .dependencies
            .iter()
            .find(|d| {
                d.kind.is_none()
                    && d.rename.as_deref().unwrap_or(&d.name).replace('-', "_") == edge.name
            })
            .ok_or_else(|| {
                refusal(
                    "EXTERNAL_ROOT_UNSUPPORTED",
                    "resolved dependency is not a declared normal edge",
                )
            })?;
        if declared.name != package.name || declared.source != package.source {
            return Err(refusal(
                "EXTERNAL_CONFIGURATION_MISMATCH",
                "declared dependency name/source does not identify resolved package",
            ));
        }
        if package.source.is_none() {
            let root = no_links(package.manifest_path.parent().ok_or_else(|| {
                refusal("EXTERNAL_ROOT_UNSUPPORTED", "invalid package manifest")
            })?)?;
            if declared
                .path
                .as_ref()
                .map(|p| p.canonicalize())
                .transpose()
                .map_err(io_error)?
                != Some(root)
            {
                return Err(refusal(
                    "EXTERNAL_CONFIGURATION_MISMATCH",
                    "declared path does not identify resolved package root",
                ));
            }
        }
        Ok(())
    }
}
pub(super) fn normal_edges(node: &Node) -> impl Iterator<Item = &Edge> {
    node.deps
        .iter()
        .filter(|edge| edge.dep_kinds.iter().any(|k| k.kind.is_none()))
}
