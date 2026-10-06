//! Evidence for failed ordinary module hops, not an alternative module resolver.
use super::*;
#[cfg(test)]
mod tests;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(rename_all = "snake_case")]
pub enum ChainReason {
    SourceNotInRootChain,
    ChainFileMissing,
    ChainFileUnadmitted,
    ConditionalDeclaration,
    PathAttribute,
    CompetingDeclarations,
    CompetingFileLayout,
    InlineModuleLayout,
    UnexaminedDeclarationAttributes,
    InheritedUncertainty,
    AmbiguousParent,
    NoOrdinarySiblingParent,
    OrdinaryLayoutMismatch,
    MacroGeneratedModuleTree,
    RootAttributeChainUncertainty,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(rename_all = "snake_case")]
pub enum ChainOrigin {
    SyntaxRecovery,
    ScopeAttributes,
    MultipleInclusionContexts,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(rename_all = "snake_case")]
pub enum ChainRole {
    Source,
    Destination,
    DeclarationParent,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(rename_all = "snake_case")]
pub enum ChainRelation {
    Direct,
    PossibleAncestor,
    RootSearchExhausted,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct ChainLocation {
    pub path: String,
    pub range: ByteRange,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct ChainDiagnostic {
    pub id: String,
    pub reason: ChainReason,
    pub crate_root: String,
    pub requested_path: String,
    pub role: ChainRole,
    pub evidenced_prefix_paths: Vec<String>,
    pub at_file_path: String,
    pub declaration: Option<ChainLocation>,
    pub candidate_paths: Vec<String>,
    pub origin_reasons: Vec<ChainOrigin>,
    pub parent_candidates: Vec<String>,
    pub relation: ChainRelation,
}
impl ChainDiagnostic {
    pub(crate) fn message(&self) -> String {
        let cause = match self.reason {
            ChainReason::MacroGeneratedModuleTree => Some(
                "macro_generated_module_tree: module declarations occur in a macro token tree; syntactic analysis cannot prove the expanded module tree",
            ),
            ChainReason::RootAttributeChainUncertainty => Some(
                "root_attribute_chain_uncertainty: a non-allowlisted inner attribute in the chain requires context the syntactic stage cannot prove",
            ),
            _ => None,
        };
        match cause {
            Some(cause) => format!(
                "{cause} at {}; this corpus is advice-only for the syntactic stage; choose a source/root with an ordinary, unambiguous chain; acknowledgment cannot remove this refusal",
                self.at_file_path
            ),
            None => format!(
                "ordinary chain from supplied root {} cannot prove {}: {:?}",
                self.crate_root, self.requested_path, self.reason
            ),
        }
    }
    pub(crate) fn boundary(root: &str, path: &str, role: ChainRole, reason: ChainReason) -> Self {
        Self {
            id: String::new(),
            reason,
            crate_root: root.into(),
            requested_path: path.into(),
            role,
            evidenced_prefix_paths: Vec::new(),
            at_file_path: root.into(),
            declaration: None,
            candidate_paths: Vec::new(),
            origin_reasons: Vec::new(),
            parent_candidates: Vec::new(),
            relation: ChainRelation::Direct,
        }
    }
}
/// The stored failure is a declaration-local diagnostic before request projection.
pub(crate) type ChainFailure = ChainDiagnostic;
#[derive(Default, Serialize)]
pub struct ModuleAnalysis {
    pub contexts: BTreeMap<String, ModuleEvidence>,
    pub(crate) failures: Vec<ChainFailure>,
}
impl ModuleAnalysis {
    /// Observe literal declaration-shaped tokens, never expand or resolve macros.
    /// Only top-level invocation inputs and rule output bodies can obstruct this file's tree.
    pub(crate) fn record_macro_declarations(
        &mut self,
        file: &FileSnapshot,
        data: &ParsedFile,
        evidence: &ModuleEvidence,
        controls: (Instant, &AtomicBool),
        bytes: &mut usize,
    ) -> Result<(), DomainError> {
        let mut stack = vec![data.tree.root_node()];
        while let Some(node) = stack.pop() {
            check(controls.0, controls.1)?;
            match node.kind() {
                "source_file"
                | "macro_definition"
                | "expression_statement"
                | "macro_invocation" => {
                    for i in (0..node.named_child_count()).rev() {
                        stack.push(node.named_child(i as u32).expect("child"));
                    }
                }
                "macro_rule" => {
                    if let Some(body) = node.child_by_field_name("right") {
                        stack.push(body);
                    }
                }
                "token_tree" => {
                    let mut previous: [Option<Node<'_>>; 2] = [None, None];
                    for i in 0..node.child_count() {
                        check(controls.0, controls.1)?;
                        let token = node.child(i).expect("child");
                        if matches!(token.kind(), "line_comment" | "block_comment") {
                            continue;
                        }
                        if let [Some(keyword), Some(name)] = previous
                            && keyword.kind() == "mod"
                            && name.kind() == "identifier"
                            && (token.kind() == ";"
                                || (token.kind() == "token_tree"
                                    && token.child(0).is_some_and(|n| n.kind() == "{")))
                        {
                            let flat = child_path(
                                &file.path,
                                &evidence.crate_root,
                                file.source[name.byte_range()].trim_start_matches("r#"),
                            );
                            let mut failure = ChainDiagnostic::boundary(
                                &evidence.crate_root,
                                "",
                                ChainRole::Source,
                                ChainReason::MacroGeneratedModuleTree,
                            );
                            failure.at_file_path = file.path.clone();
                            failure.evidenced_prefix_paths = evidence.filesystem_paths.clone();
                            failure.declaration = Some(ChainLocation {
                                path: file.path.clone(),
                                range: ByteRange {
                                    start_byte: keyword.start_byte(),
                                    end_byte: token.end_byte(),
                                },
                            });
                            failure.candidate_paths = vec![
                                flat.clone(),
                                format!("{}/mod.rs", flat.trim_end_matches(".rs")),
                            ];
                            self.record(failure, bytes)?;
                        }
                        previous = [previous[1], Some(token)];
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }
    pub(crate) fn project(
        &self,
        root: &str,
        requested: &str,
        role: ChainRole,
        controls: (Instant, &AtomicBool),
    ) -> Result<Vec<ChainDiagnostic>, DomainError> {
        let context = self.contexts.get(requested);
        if context.is_some_and(|e| e.unresolved.is_empty()) {
            return Ok(Vec::new());
        }
        let mut relevant = Vec::new();
        let mut projected_bytes = 0;
        let mut longest = 0;
        for failure in &self.failures {
            check(controls.0, controls.1)?;
            let direct = failure.candidate_paths.iter().any(|p| p == requested);
            let inherited = matches!(
                failure.reason,
                ChainReason::InheritedUncertainty | ChainReason::RootAttributeChainUncertainty
            ) && context.is_some_and(|e| {
                failure
                    .candidate_paths
                    .iter()
                    .any(|p| e.filesystem_paths.contains(p))
            });
            let possible = failure.candidate_paths.iter().any(|p| {
                let directory = p
                    .strip_suffix("/mod.rs")
                    .or_else(|| p.strip_suffix(".rs"))
                    .unwrap_or(p);
                requested.starts_with(&format!("{directory}/"))
            });
            if !direct && !inherited && !possible {
                continue;
            }
            let length = failure.evidenced_prefix_paths.len();
            if context.is_none() && length < longest {
                continue;
            }
            if context.is_none() && length > longest {
                relevant.clear();
                longest = length;
            }
            let mut diagnostic = failure.clone();
            diagnostic.requested_path = requested.into();
            diagnostic.role = role;
            diagnostic.relation = if direct || inherited {
                ChainRelation::Direct
            } else {
                ChainRelation::PossibleAncestor
            };
            account_chain(&diagnostic, &mut projected_bytes)?;
            relevant.push(diagnostic);
        }
        if relevant.is_empty() {
            let mut diagnostic =
                ChainDiagnostic::boundary(root, requested, role, ChainReason::SourceNotInRootChain);
            if self.contexts.contains_key(root) {
                diagnostic.evidenced_prefix_paths.push(root.into());
            }
            diagnostic.relation = ChainRelation::RootSearchExhausted;
            account_chain(&diagnostic, &mut projected_bytes)?;
            relevant.push(diagnostic);
        }
        Ok(relevant)
    }
    pub(crate) fn record(
        &mut self,
        failure: ChainFailure,
        bytes: &mut usize,
    ) -> Result<(), DomainError> {
        account_chain(&failure, bytes)?;
        self.failures.push(failure);
        Ok(())
    }
}
/// Account before growing evidence arrays; use a streaming counter rather than a JSON copy.
pub(crate) fn account_chain(value: &impl Serialize, bytes: &mut usize) -> Result<(), DomainError> {
    struct Counter<'a>(&'a mut usize);
    impl std::io::Write for Counter<'_> {
        fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
            *self.0 = self.0.saturating_add(data.len());
            if *self.0 > 128 * 1024 * 1024 {
                return Err(std::io::ErrorKind::OutOfMemory.into());
            }
            Ok(data.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    serde_json::to_writer(Counter(bytes), value).map_err(|_| {
        DomainError::new(
            "analysis_descriptor_bytes",
            "chain evidence descriptor guard reached",
        )
    })
}
pub(crate) fn finalize_chain(diagnostics: &mut [ChainDiagnostic]) -> BTreeMap<String, String> {
    diagnostics.sort_by(|a, b| {
        (
            &a.role,
            &a.requested_path,
            &a.at_file_path,
            a.declaration.as_ref().map(|d| &d.range),
            a.reason,
            &a.candidate_paths,
            &a.origin_reasons,
            &a.parent_candidates,
        )
            .cmp(&(
                &b.role,
                &b.requested_path,
                &b.at_file_path,
                b.declaration.as_ref().map(|d| &d.range),
                b.reason,
                &b.candidate_paths,
                &b.origin_reasons,
                &b.parent_candidates,
            ))
    });
    diagnostics
        .iter_mut()
        .enumerate()
        .map(|(i, d)| {
            let old = std::mem::replace(&mut d.id, format!("chain/{i}"));
            (old, d.id.clone())
        })
        .collect()
}
/// Attribute names are grammar identifiers, not substring guesses over attribute payloads.
pub(crate) fn declaration_reasons(
    data: &ParsedFile,
    file: &FileSnapshot,
    item: &Item,
) -> Vec<ChainReason> {
    let node = data
        .tree
        .root_node()
        .named_descendant_for_byte_range(item.span.range.start_byte, item.span.range.end_byte)
        .expect("item");
    let mut reasons = Vec::new();
    if node.child_by_field_name("body").is_some() {
        reasons.push(ChainReason::InlineModuleLayout);
    }
    for attribute in &item.attributes {
        let node = data
            .tree
            .root_node()
            .named_descendant_for_byte_range(attribute.range.start_byte, attribute.range.end_byte);
        let name = node
            .and_then(|n| n.named_child(0))
            .and_then(|n| n.named_child(0))
            .map(|n| &file.source[n.byte_range()]);
        let reason = match name {
            Some("cfg" | "cfg_attr") => ChainReason::ConditionalDeclaration,
            Some("path") => ChainReason::PathAttribute,
            _ => ChainReason::UnexaminedDeclarationAttributes,
        };
        if !reasons.contains(&reason) {
            reasons.push(reason);
        }
    }
    reasons
}
