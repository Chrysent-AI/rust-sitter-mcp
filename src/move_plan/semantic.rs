//! Pure, request-owned resolution of admitted bytes, never Cargo/project loading.
use super::{MoveEnvelope, MoveRequest, range};
use crate::{
    items::{self, DecisionReason, Need},
    plan::SourceAnchor,
    result::{ByteRange, DomainError},
    scope::FileSnapshot,
    trivia::MoveOrigin,
};
use ra_ap_base_db::{
    CrateGraphBuilder, CrateName, CrateOrigin, CrateWorkspaceData, DependencyBuilder, FileChange,
    FileId, FileSet, SourceRoot, VfsPath,
    salsa::{Cancelled, Database},
};
use ra_ap_hir::{
    Adt, AsAssocItem, AssocItemContainer, CfgOptions, HasVisibility, Module, ModuleDef,
    PathResolution, Semantics, Symbol, Type,
};
use ra_ap_ide_db::RootDatabase;
use ra_ap_syntax::{
    AstNode, Edition, SyntaxNode,
    ast::{self, HasAttrs, HasGenericParams, HasName},
};
use rmcp::schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{atomic::AtomicBool, mpsc},
    time::{Duration, Instant},
};

#[path = "semantic_context.rs"]
mod context;
#[path = "declarative.rs"]
mod declarative;
#[path = "semantic_identity.rs"]
mod identity;
pub use context::ContextEvaluation;
use context::{FactClass, safe_attr};
use std::cell::{Cell, RefCell};

const ANALYZER: &str = "ra_ap@0.0.357";

#[cfg(test)]
#[path = "declarative_tests.rs"]
mod declarative_tests;
#[cfg(test)]
#[path = "semantic_tests.rs"]
mod tests;

/// One caller-declared configuration. No manifests, environment, or sysroot are discovered.
/// Listed cfg/features are ON evidence at the context gate; unlisted atoms remain unknown.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(deny_unknown_fields)]
pub struct Configuration {
    pub crates: Vec<CrateInput>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(deny_unknown_fields)]
pub struct CrateInput {
    pub name: String,
    pub root_file: String,
    pub edition: String,
    pub features: Vec<String>,
    pub cfg: Vec<CfgInput>,
    pub dependencies: Vec<DependencyInput>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(deny_unknown_fields)]
pub struct CfgInput {
    pub key: String,
    pub value: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(deny_unknown_fields)]
pub struct DependencyInput {
    pub name: String,
    pub crate_name: String,
}
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct ResolutionCoverage {
    pub decisions: usize,
    pub configuration: Configuration,
    pub snapshot_id: String,
    pub semantic_input_digest: String,
    pub final_overlay_digest: String,
    pub analyzer: String,
    pub statement: String,
    pub omissions: Vec<String>,
    pub context_evaluations: Vec<ContextEvaluation>,
}
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(rename_all = "snake_case")]
pub enum ProofClass {
    RaResolved,
    DeclarativeMacroIdentity,
    AssumedDeclaredIdentity,
}
/// Stable written identities, not revision-local RA IDs or pretty-printed types.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct Declaration {
    pub crate_origin: String,
    pub anchor: SourceAnchor,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub declarative_macro: Option<declarative::Evidence>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct Receiver {
    pub references: Vec<String>,
    pub builtin: Option<String>,
    pub declaration: Option<Declaration>,
    pub arguments: Vec<Receiver>,
}
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct Proof {
    pub class: ProofClass,
    pub anchor: SourceAnchor,
    pub item_ids: Vec<String>,
    pub destination_path: String,
    pub final_anchor: SourceAnchor,
    pub original_receiver: Option<Receiver>,
    pub adjusted_receiver: Option<Receiver>,
    pub final_original_receiver: Option<Receiver>,
    pub final_adjusted_receiver: Option<Receiver>,
    pub declaration: Declaration,
    pub final_declaration: Declaration,
    pub classification: String,
    pub basis: String,
    pub source_access: bool,
    pub final_access: bool,
    pub coverage: ResolutionCoverage,
}

pub fn candidate(need: &Need) -> bool {
    matches!(
        need.reason,
        DecisionReason::MemberOrConstructorUnproved | DecisionReason::ExternalOrMissingBinding
    ) || (need.reason == DecisionReason::ConditionalOrInheritedContext
        && need.category == "scope_dependency"
        && need.refusal_basis.iter().any(|basis| {
            matches!(basis.class.as_str(), "derive_veto" | "conditional_context")
                && basis.anchor.path == need.path
                && basis.anchor.range.as_ref() == Some(&need.range)
        }))
}

// One attribute may be admitted for nominal identity and refused for method facts.
type ContextKey = (String, usize, usize, &'static str);

struct Inputs {
    db: RootDatabase,
    ids: BTreeMap<String, FileId>,
    paths: BTreeMap<FileId, String>,
    texts: BTreeMap<String, String>,
    roots: BTreeMap<FileId, String>,
    configuration: Configuration,
    assume_declared_helpers: bool,
    // Per-fact dependency on conditional namespace admission, not a resolver cache.
    assumed_declared_helpers: Cell<bool>,
    context: RefCell<BTreeMap<ContextKey, ContextEvaluation>>,
}
impl Inputs {
    fn new(
        texts: BTreeMap<String, String>,
        config: &Configuration,
        controls: (Instant, &AtomicBool),
    ) -> Option<Self> {
        if config.crates.is_empty() || config.crates.len() > 32 || texts.len() > 100_000 {
            return None;
        }
        let mut set = FileSet::default();
        let mut change = FileChange::default();
        let mut ids = BTreeMap::new();
        let mut paths = BTreeMap::new();
        for (index, (path, text)) in texts.iter().enumerate() {
            items::check(controls.0, controls.1).ok()?;
            if text.len() > u32::MAX as usize {
                return None;
            }
            let id = FileId::from_raw(index as u32);
            set.insert(id, VfsPath::new_virtual_path(format!("/{path}")));
            ids.insert(path.clone(), id);
            paths.insert(id, path.clone());
            change.change_file(id, Some(text.clone()));
        }
        let mut graph = CrateGraphBuilder::default();
        let mut crates = BTreeMap::new();
        let mut roots = BTreeMap::new();
        for input in &config.crates {
            let edition = input.edition.parse::<Edition>().ok()?;
            let file = *ids.get(&input.root_file)?;
            if CrateName::new(&input.name).is_err()
                || input.name.len() > 256
                || crates.contains_key(&input.name)
                || roots.contains_key(&file)
                || input.cfg.len() + input.features.len() > 1024
            {
                return None;
            }
            let mut cfg = CfgOptions::default();
            for atom in &input.cfg {
                if atom.key.is_empty()
                    || atom.key.len() > 256
                    || atom.value.as_ref().is_some_and(|v| v.len() > 256)
                {
                    return None;
                }
                if let Some(value) = &atom.value {
                    cfg.insert_key_value(Symbol::intern(&atom.key), Symbol::intern(value));
                } else {
                    cfg.insert_atom(Symbol::intern(&atom.key));
                }
            }
            for feature in &input.features {
                if feature.len() > 256 {
                    return None;
                }
                cfg.insert_key_value(Symbol::intern("feature"), Symbol::intern(feature));
            }
            let id = graph.add_crate_root(
                file,
                edition,
                None,
                None,
                cfg,
                None,
                Default::default(),
                CrateOrigin::Local {
                    repo: None,
                    name: Some(Symbol::intern(&input.name)),
                },
                Vec::new(),
                false,
                ra_ap_base_db::AbsPathBuf::assert_utf8(std::path::PathBuf::from("/")).into(),
                CrateWorkspaceData {
                    target: Err("target layout not admitted".into()),
                    toolchain: None,
                }
                .into(),
            );
            crates.insert(input.name.clone(), id);
            roots.insert(file, input.name.clone());
        }
        for input in &config.crates {
            let mut names = BTreeSet::new();
            if input.dependencies.len() > 32 {
                return None;
            }
            for dep in &input.dependencies {
                if !names.insert(&dep.name) {
                    return None;
                }
                let name = CrateName::new(&dep.name).ok()?;
                graph
                    .add_dep(
                        crates[&input.name],
                        DependencyBuilder::new(name, *crates.get(&dep.crate_name)?),
                    )
                    .ok()?;
            }
        }
        change.set_roots(vec![SourceRoot::new_local(set)]);
        change.set_crate_graph(graph);
        let mut db = RootDatabase::default();
        change.apply(&mut db);
        Some(Self {
            db,
            ids,
            paths,
            texts,
            roots,
            configuration: config.clone(),
            assume_declared_helpers: false,
            assumed_declared_helpers: Cell::new(false),
            context: RefCell::new(BTreeMap::new()),
        })
    }
    fn declaration(
        &self,
        module: Module,
        node: &SyntaxNode,
        file: ra_ap_hir::HirFileId,
    ) -> Option<Declaration> {
        let file = file.file_id()?.file_id(&self.db);
        let path = self.paths.get(&file)?;
        let start = usize::from(node.text_range().start());
        let end = usize::from(node.text_range().end());
        Some(Declaration {
            declarative_macro: None,
            crate_origin: self
                .roots
                .get(&module.krate(&self.db).root_file(&self.db))?
                .clone(),
            anchor: SourceAnchor {
                path: path.clone(),
                range: range(start, end),
                expected_text: self.texts[path].get(start..end)?.into(),
            },
        })
    }
}

/// Query cancellation observes the same request flag/deadline as the owning engine job.
/// Both databases are dropped and the controller joined before the admission permit releases.
pub fn discharge(
    request: &MoveRequest,
    files: &BTreeMap<String, FileSnapshot>,
    overlay: BTreeMap<String, String>,
    needs: &mut Vec<Need>,
    controls: (Instant, &AtomicBool),
    result: &mut MoveEnvelope,
) -> Result<(), DomainError> {
    let Some(config) = &request.semantic_configuration else {
        configuration_refusal(needs);
        return Ok(());
    };
    let (deadline, cancelled) = controls;
    items::check(deadline, cancelled)?;
    if !config
        .crates
        .iter()
        .any(|c| c.root_file == request.crate_root)
    {
        configuration_refusal(needs);
        return Ok(());
    }
    let original: BTreeMap<_, _> = files
        .iter()
        .map(|(p, f)| (p.clone(), f.source.clone()))
        .collect();
    let snapshot = result.snapshot_id.clone().unwrap_or_default();
    let root = std::path::Path::new(result.root.as_deref().expect("admitted root"));
    let digest = crate::scope::hash_serialized(root, &(&snapshot, config, ANALYZER), controls)?;
    let final_digest = crate::scope::hash_serialized(root, &(&digest, &overlay), controls)?;
    let Some(mut old) = Inputs::new(original, config, controls) else {
        items::check(deadline, cancelled)?;
        configuration_refusal(needs);
        return Ok(());
    };
    let Some(mut new) = Inputs::new(overlay, config, controls) else {
        items::check(deadline, cancelled)?;
        configuration_refusal(needs);
        return Ok(());
    };
    old.assume_declared_helpers = request.assume_declared_helpers;
    new.assume_declared_helpers = request.assume_declared_helpers;
    let coverage = ResolutionCoverage {
        decisions: 0, configuration: config.clone(), snapshot_id: snapshot,
        semantic_input_digest: digest, final_overlay_digest: final_digest,
        analyzer: ANALYZER.into(), statement: String::new(),
        omissions: vec!["one explicit configuration only; no other targets/features/cfg combinations".into(),
            "no discovered sysroot/dependency sources, build-script cfg/environment, proc-macro execution, general macro expansion, generated-member facts, target layout, compilation or equivalence checks; declarative identity admission is capped at one ident fragment, one expansion step, 4096 definition/output tokens and nesting 32".into(),
            "context_evaluations covers only reached context checks; undeclared cfg atoms remain unknown, including unlisted features; inactive cfg_attr payloads are not evaluated".into()],
        context_evaluations: Vec::new(),
    };
    let mut proofs = controlled(&old.db, &new.db, controls, || {
        evaluate(&old, &new, needs, &result.plan.origins, &coverage, controls)
    })?;
    let mut coverage = coverage;
    for (revision, inputs) in [("original", &old), ("final", &new)] {
        coverage
            .context_evaluations
            .extend(inputs.context.borrow().values().cloned().map(|mut e| {
                e.revision = revision.into();
                e
            }));
    }
    coverage.decisions = proofs.len();
    coverage.statement = format!(
        "resolution performed for {} decisions under one explicit configuration; compilation/equivalence not performed",
        proofs.len()
    );
    for proof in &mut proofs {
        proof.coverage = coverage.clone();
    }
    result.coverage.assumed_declared_identity = proofs
        .iter()
        .filter(|proof| matches!(proof.class, ProofClass::AssumedDeclaredIdentity))
        .count();
    result.coverage.ra_resolved = proofs.len() - result.coverage.assumed_declared_identity;
    result.plan.integrity.semantic = "resolution_performed".into();
    result.plan.resolution_coverage = Some(coverage);
    result.plan.binding_proofs.extend(
        proofs
            .into_iter()
            .map(|proof| super::BindingProof::RaResolved(Box::new(proof))),
    );
    Ok(())
}

fn configuration_refusal(needs: &mut [Need]) {
    for need in needs {
        need.refuse_at_occurrence("semantic_configuration_unproved");
    }
}

fn controlled(
    old: &RootDatabase,
    new: &RootDatabase,
    controls: (Instant, &AtomicBool),
    query: impl FnOnce() -> Result<Vec<Proof>, DomainError>,
) -> Result<Vec<Proof>, DomainError> {
    let (deadline, cancelled) = controls;
    let old_token = old.cancellation_token();
    let new_token = new.cancellation_token();
    let (finish, stopped) = mpsc::channel();
    let resolved = std::thread::scope(|threads| {
        let controller = threads.spawn(move || {
            loop {
                if items::check(deadline, cancelled).is_err() {
                    old_token.cancel();
                    new_token.cancel();
                    break;
                }
                match stopped.recv_timeout(Duration::from_millis(5)) {
                    Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                }
            }
        });
        struct FinishOnDrop(mpsc::Sender<()>);
        impl Drop for FinishOnDrop {
            fn drop(&mut self) {
                let _ = self.0.send(());
            }
        }
        let finish = FinishOnDrop(finish);
        let resolved = Cancelled::catch(std::panic::AssertUnwindSafe(query));
        drop(finish);
        controller
            .join()
            .expect("resolution cancellation controller");
        resolved
    });
    items::check(deadline, cancelled)?;
    resolved.map_err(|_| DomainError::new("CANCELLED", "resolution query cancelled"))?
}

fn mapped(anchor: &SourceAnchor, origins: &[MoveOrigin], new: &Inputs) -> Option<SourceAnchor> {
    let mut matches = origins.iter().filter_map(|o| {
        o.mapped(
            &anchor.path,
            &(anchor.range.start_byte..anchor.range.end_byte),
        )
    });
    let (path, span) = matches.next().or_else(|| {
        (!origins.iter().any(|o| o.source_path == anchor.path)).then(|| {
            (
                anchor.path.clone(),
                anchor.range.start_byte..anchor.range.end_byte,
            )
        })
    })?;
    if matches.next().is_some() {
        return None;
    }
    let text = new.texts.get(&path)?.get(span.clone())?;
    if text != anchor.expected_text {
        return None;
    }
    Some(SourceAnchor {
        path,
        range: range(span.start, span.end),
        expected_text: text.into(),
    })
}
fn normalize(decl: &Declaration, origins: &[MoveOrigin], old: &Inputs) -> Option<Declaration> {
    let anchor = &decl.anchor;
    let mut candidates = origins.iter().filter_map(|o| {
        if o.output_path == anchor.path
            && o.output_range.start_byte <= anchor.range.start_byte
            && o.output_range.end_byte >= anchor.range.end_byte
        {
            let start =
                o.source_range.start_byte + anchor.range.start_byte - o.output_range.start_byte;
            Some(SourceAnchor {
                path: o.source_path.clone(),
                range: range(start, start + anchor.expected_text.len()),
                expected_text: anchor.expected_text.clone(),
            })
        } else {
            None
        }
    });
    let anchor = candidates.next().or_else(|| {
        (!origins.iter().any(|o| o.output_path == anchor.path)).then(|| anchor.clone())
    })?;
    if candidates.next().is_some()
        || old
            .texts
            .get(&anchor.path)?
            .get(anchor.range.start_byte..anchor.range.end_byte)?
            != anchor.expected_text
    {
        return None;
    }
    let declarative_macro = if let Some(e) = &decl.declarative_macro {
        let normalize_anchor = |anchor: &SourceAnchor| {
            normalize(
                &Declaration {
                    crate_origin: decl.crate_origin.clone(),
                    anchor: anchor.clone(),
                    declarative_macro: None,
                },
                origins,
                old,
            )
            .map(|d| d.anchor)
        };
        Some(declarative::Evidence {
            invocation: normalize_anchor(&e.invocation)?,
            definition: normalize_anchor(&e.definition)?,
        })
    } else {
        None
    };
    Some(Declaration {
        crate_origin: decl.crate_origin.clone(),
        anchor,
        declarative_macro,
    })
}
fn normalize_receiver(
    receiver: &Receiver,
    origins: &[MoveOrigin],
    old: &Inputs,
) -> Option<Receiver> {
    let mut normalized = receiver.clone();
    if let Some(decl) = &receiver.declaration {
        normalized.declaration = Some(normalize(decl, origins, old)?);
    }
    normalized.arguments = receiver
        .arguments
        .iter()
        .map(|a| normalize_receiver(a, origins, old))
        .collect::<Option<_>>()?;
    Some(normalized)
}

fn evaluate(
    old: &Inputs,
    new: &Inputs,
    needs: &mut Vec<Need>,
    origins: &[MoveOrigin],
    coverage: &ResolutionCoverage,
    controls: (Instant, &AtomicBool),
) -> Result<Vec<Proof>, DomainError> {
    let mut proofs = Vec::new();
    let mut retained = Vec::new();
    for mut need in needs.drain(..) {
        items::check(controls.0, controls.1)?;
        let anchor = SourceAnchor {
            path: need.path.clone(),
            range: need.range.clone(),
            expected_text: old.texts[&need.path][need.range.start_byte..need.range.end_byte].into(),
        };
        let mut refusal_class = "semantic_mapping_unproved";
        let proof = (|| {
            let final_anchor = mapped(&anchor, origins, new)?;
            refusal_class = "semantic_source_fact_unproved";
            let before = fact(old, &anchor)?;
            let before_assumed = old.assumed_declared_helpers.get();
            refusal_class = "semantic_final_fact_unproved";
            let after = fact(new, &final_anchor)?;
            let assumed = before_assumed || new.assumed_declared_helpers.get();
            refusal_class = "semantic_identity_unproved";
            if before.classification != after.classification
                || before.fact_class != after.fact_class
                || before.declaration != normalize(&after.declaration, origins, old)?
            {
                return None;
            }
            for (original, final_type) in [
                (&before.original, &after.original),
                (&before.adjusted, &after.adjusted),
            ] {
                match (original, final_type) {
                    (Some(a), Some(b)) if *a == normalize_receiver(b, origins, old)? => {}
                    (None, None) => {}
                    _ => return None,
                }
            }
            Some(Proof {
                class: if assumed {
                    ProofClass::AssumedDeclaredIdentity
                } else if before.declaration.declarative_macro.is_some() {
                    ProofClass::DeclarativeMacroIdentity
                } else {
                    ProofClass::RaResolved
                },
                anchor,
                item_ids: need.item_ids.clone(),
                destination_path: final_anchor.path.clone(),
                final_anchor,
                original_receiver: before.original,
                adjusted_receiver: before.adjusted,
                final_original_receiver: after.original,
                final_adjusted_receiver: after.adjusted,
                basis: if assumed {
                    declarative::ASSUMED_BASIS
                } else if before.declaration.declarative_macro.is_some() {
                    declarative::BASIS
                } else {
                    before.fact_class.basis()
                }
                .into(),
                declaration: before.declaration,
                final_declaration: after.declaration,
                classification: before.classification.into(),
                source_access: true,
                final_access: true,
                coverage: coverage.clone(),
            })
        })();
        if let Some(proof) = proof {
            proofs.push(proof);
        } else {
            need.refuse_at_occurrence(refusal_class);
            retained.push(need);
        }
    }
    *needs = retained;
    Ok(proofs)
}

struct Fact {
    declaration: Declaration,
    original: Option<Receiver>,
    adjusted: Option<Receiver>,
    classification: &'static str,
    fact_class: FactClass,
}

fn receiver(
    inputs: &Inputs,
    ty: Type<'_>,
    fact_class: FactClass,
    depth: usize,
) -> Option<Receiver> {
    if depth > 16 || ty.contains_unknown() {
        return None;
    }
    let mut ty = ty;
    let mut references = Vec::new();
    while ty.is_reference() {
        references.push(
            if ty.is_mutable_reference() {
                "mutable"
            } else {
                "shared"
            }
            .into(),
        );
        ty = ty.strip_reference();
        if references.len() > 16 {
            return None;
        }
    }
    if let Some(builtin) = ty.as_builtin() {
        return Some(Receiver {
            references,
            builtin: Some(
                builtin
                    .name()
                    .display(&inputs.db, Edition::Edition2024)
                    .to_string(),
            ),
            declaration: None,
            arguments: Vec::new(),
        });
    }
    let (adt, args) = ty.as_adt_with_args()?;
    let declaration = adt_declaration(inputs, adt, fact_class)?;
    let arguments = args
        .into_iter()
        .map(|a| receiver(inputs, a?, fact_class, depth + 1))
        .collect::<Option<_>>()?;
    Some(Receiver {
        references,
        builtin: None,
        declaration: Some(declaration),
        arguments,
    })
}
fn adt_declaration(inputs: &Inputs, adt: Adt, fact_class: FactClass) -> Option<Declaration> {
    let sema = Semantics::new(&inputs.db);
    let expanded = match adt {
        Adt::Struct(s) => sema.source(s)?.file_id.is_macro(),
        Adt::Enum(e) => sema.source(e)?.file_id.is_macro(),
        Adt::Union(_) => return None,
    };
    if expanded {
        return (fact_class == FactClass::NominalIdentity)
            .then(|| declarative::declaration(inputs, &sema, adt))
            .flatten();
    }
    match adt {
        Adt::Struct(s) => {
            let source = sema.source(s)?;
            scoped_context(inputs, &sema, source.value.syntax(), fact_class)?;
            inputs.declaration(
                s.module(&inputs.db),
                source.value.name()?.syntax(),
                source.file_id,
            )
        }
        Adt::Enum(s) => {
            let source = sema.source(s)?;
            scoped_context(inputs, &sema, source.value.syntax(), fact_class)?;
            inputs.declaration(
                s.module(&inputs.db),
                source.value.name()?.syntax(),
                source.file_id,
            )
        }
        Adt::Union(_) => None,
    }
}

/// Generated-item-dependent facts keep the conservative expansion context gate.
fn ordinary_context(
    inputs: &Inputs,
    sema: &Semantics<'_, RootDatabase>,
    node: &SyntaxNode,
) -> Option<Module> {
    scoped_context(inputs, sema, node, FactClass::GeneratedItems)
}

/// Ignore unrelated bodies. Nominal path facts use this admission only after
/// proving stable written bindings; cfg and arbitrary attribute macros still veto.
fn scoped_context(
    inputs: &Inputs,
    sema: &Semantics<'_, RootDatabase>,
    node: &SyntaxNode,
    fact_class: FactClass,
) -> Option<Module> {
    if node
        .ancestors()
        .any(|n| ast::MacroCall::can_cast(n.kind()) || ast::MacroRules::can_cast(n.kind()))
    {
        return None;
    }
    let module = sema.scope(node)?.module();
    for ancestor in node.ancestors() {
        if let Some(item) = ast::Item::cast(ancestor) {
            if item
                .attrs()
                .any(|a| !safe_attr(inputs, sema, module, &a, fact_class))
            {
                return None;
            }
            if let ast::Item::Fn(f) = item
                && f.generic_param_list().is_some()
            {
                return None;
            }
        }
    }
    let file = sema.hir_file_for(node).file_id()?.file_id(&inputs.db);
    if sema.file_to_module_defs(file).count() != 1
        || !ra_ap_syntax::SourceFile::parse(
            inputs.texts.get(inputs.paths.get(&file)?)?,
            module.krate(&inputs.db).edition(&inputs.db),
        )
        .errors()
        .is_empty()
    {
        return None;
    }
    for scope in module.path_to_root(&inputs.db) {
        if let Some(declaration) = scope.declaration_source(&inputs.db) {
            let file = declaration.file_id.file_id()?.file_id(&inputs.db);
            let root = sema.parse_guess_edition(file);
            let module = root
                .syntax()
                .descendants()
                .filter_map(ast::Module::cast)
                .find(|m| m.syntax().text_range() == declaration.value.syntax().text_range())?;
            if module
                .attrs()
                .any(|a| !safe_attr(inputs, sema, scope, &a, fact_class))
            {
                return None;
            }
        }
        let file = scope.krate(&inputs.db).root_file(&inputs.db);
        let root = sema.parse_guess_edition(file);
        if root
            .attrs()
            .any(|a| !safe_attr(inputs, sema, scope, &a, fact_class))
        {
            return None;
        }
        let source = scope.definition_source(&inputs.db);
        let scope_file = source.file_id.file_id()?.file_id(&inputs.db);
        if !ra_ap_syntax::SourceFile::parse(
            inputs.texts.get(inputs.paths.get(&scope_file)?)?,
            scope.krate(&inputs.db).edition(&inputs.db),
        )
        .errors()
        .is_empty()
        {
            return None;
        }
        let parsed = sema.parse_guess_edition(scope_file);
        let syntax = match source.value {
            ra_ap_hir::ModuleSource::SourceFile(_) => {
                if parsed
                    .attrs()
                    .any(|a| !safe_attr(inputs, sema, scope, &a, fact_class))
                {
                    return None;
                }
                parsed.syntax().clone()
            }
            ra_ap_hir::ModuleSource::Module(m) => {
                let module = parsed
                    .syntax()
                    .descendants()
                    .filter_map(ast::Module::cast)
                    .find(|n| n.syntax().text_range() == m.syntax().text_range())?;
                module.item_list()?.syntax().clone()
            }
            ra_ap_hir::ModuleSource::BlockExpr(_) => return None,
        };
        for child in syntax.children() {
            if let Some(call) = ast::MacroCall::cast(child.clone()) {
                if fact_class == FactClass::NominalIdentity
                    && declarative::context_admitted(inputs, sema, scope, &call)
                {
                    continue;
                }
                context::record(
                    inputs,
                    sema,
                    scope,
                    &child,
                    fact_class,
                    ("module_macro", "skipped", None, "module_macro_invocation"),
                );
                return None;
            }
            if let Some(item) = ast::Item::cast(child) {
                // Sibling module conditions do not change this module's namespace.
                // Conditions on a module we actually traverse are checked above.
                if !matches!(item, ast::Item::Module(_))
                    && item
                        .attrs()
                        .any(|a| !safe_attr(inputs, sema, scope, &a, fact_class))
                {
                    return None;
                }
            }
        }
    }
    Some(module)
}
fn exact_path(root: &SyntaxNode, span: &ByteRange) -> Option<ast::Path> {
    root.descendants().filter_map(ast::Path::cast).find(|p| {
        usize::from(p.syntax().text_range().start()) == span.start_byte
            && usize::from(p.syntax().text_range().end()) == span.end_byte
    })
}
fn fact(inputs: &Inputs, anchor: &SourceAnchor) -> Option<Fact> {
    // The next solver requires its own TLS attachment in addition to Salsa's.
    // Return only owned evidence before attaching the other database revision.
    inputs.assumed_declared_helpers.set(false);
    ra_ap_hir::attach_db(&inputs.db, || fact_attached(inputs, anchor))
}
fn fact_attached(inputs: &Inputs, anchor: &SourceAnchor) -> Option<Fact> {
    let sema = Semantics::new(&inputs.db);
    let root = sema.parse_guess_edition(*inputs.ids.get(&anchor.path)?);
    let syntax = root.syntax();
    // A separate scope-dependency need can be anchored on an attached attribute,
    // not a path. Prove only its inert context, never the pattern/body it decorates.
    for attr in syntax.descendants().filter_map(ast::Attr::cast) {
        if usize::from(attr.syntax().text_range().start()) == anchor.range.start_byte
            && usize::from(attr.syntax().text_range().end()) == anchor.range.end_byte
        {
            let fact_class = if attr
                .syntax()
                .ancestors()
                .find_map(ast::Item::cast)
                .is_some_and(|item| {
                    matches!(
                        item,
                        ast::Item::Struct(_) | ast::Item::Enum(_) | ast::Item::Union(_)
                    )
                }) {
                FactClass::NominalIdentity
            } else {
                FactClass::GeneratedItems
            };
            let module = scoped_context(inputs, &sema, attr.syntax(), fact_class)?;
            if !safe_attr(inputs, &sema, module, &attr, fact_class) {
                return None;
            }
            return Some(Fact {
                declaration: inputs.declaration(
                    module,
                    attr.syntax(),
                    sema.hir_file_for(attr.syntax()),
                )?,
                original: None,
                adjusted: None,
                classification: "context_attribute",
                fact_class,
            });
        }
    }
    for call in syntax.descendants().filter_map(ast::MethodCallExpr::cast) {
        let name = call.name_ref()?;
        if usize::from(call.syntax().text_range().start()) != anchor.range.start_byte
            || usize::from(name.syntax().text_range().end()) != anchor.range.end_byte
        {
            continue;
        }
        let module = ordinary_context(inputs, &sema, call.syntax())?;
        let function = sema.resolve_method_call(&call)?;
        let associated = function.as_assoc_item(&inputs.db)?;
        let AssocItemContainer::Impl(implementation) = associated.container(&inputs.db) else {
            return None;
        };
        if implementation.trait_(&inputs.db).is_some()
            || !function.is_visible_from(&inputs.db, module)
        {
            return None;
        }
        let source = sema.source(function)?;
        if source.value.generic_param_list().is_some() {
            return None;
        }
        ordinary_context(inputs, &sema, source.value.syntax())?;
        let declaration = inputs.declaration(
            function.module(&inputs.db),
            source.value.name()?.syntax(),
            source.file_id,
        )?;
        let types = sema.type_of_expr(&call.receiver()?)?;
        let original = receiver(inputs, types.original.clone(), FactClass::GeneratedItems, 0)?;
        let adjusted = receiver(inputs, types.adjusted(), FactClass::GeneratedItems, 0)?;
        return Some(Fact {
            declaration,
            original: Some(original),
            adjusted: Some(adjusted),
            classification: "inherent_function",
            fact_class: FactClass::GeneratedItems,
        });
    }
    for field in syntax.descendants().filter_map(ast::FieldExpr::cast) {
        if usize::from(field.syntax().text_range().start()) != anchor.range.start_byte
            || usize::from(field.syntax().text_range().end()) != anchor.range.end_byte
        {
            continue;
        }
        let module = ordinary_context(inputs, &sema, field.syntax())?;
        let definition = sema.resolve_field(&field)?.left()?;
        if !definition.is_visible_from(&inputs.db, module) {
            return None;
        }
        let source = sema.source(definition)?;
        ordinary_context(inputs, &sema, source.value.syntax())?;
        let ra_ap_hir::FieldSource::Named(named) = &source.value else {
            return None;
        };
        let declaration = inputs.declaration(
            definition.parent_def(&inputs.db).module(&inputs.db),
            named.name()?.syntax(),
            source.file_id,
        )?;
        let types = sema.type_of_expr(&field.expr()?)?;
        let original = receiver(inputs, types.original.clone(), FactClass::GeneratedItems, 0)?;
        let adjusted = receiver(inputs, types.adjusted(), FactClass::GeneratedItems, 0)?;
        return Some(Fact {
            declaration,
            original: Some(original),
            adjusted: Some(adjusted),
            classification: "field",
            fact_class: FactClass::GeneratedItems,
        });
    }
    let path = exact_path(syntax, &anchor.range)?;
    let Some(PathResolution::Def(definition)) = sema.resolve_path(&path) else {
        identity::disclose_missing_path(inputs, &sema, &path, 0);
        return None;
    };
    let fact_class = match definition {
        ModuleDef::Adt(_) | ModuleDef::EnumVariant(_) => {
            let mut path_class = FactClass::NominalIdentity;
            let stable = identity::stable_path(inputs, &sema, &path, 0, &mut path_class).is_some();
            context::record(
                inputs,
                &sema,
                sema.scope(path.syntax())?.module(),
                path.syntax(),
                path_class,
                (
                    "binding",
                    if stable { "admitted" } else { "skipped" },
                    None,
                    if stable && path_class == FactClass::GeneratedItems {
                        "configured_dependency_root_with_conservative_context"
                    } else if stable {
                        "stable_written_identity"
                    } else {
                        "stable_written_identity_unproved"
                    },
                ),
            );
            if !stable {
                // An unexpanded identity cannot establish that a glob or missing
                // written route survives generated imports. Retain the need;
                // also disclose any reached conservative attribute veto.
                let _ = scoped_context(inputs, &sema, path.syntax(), FactClass::GeneratedItems);
                return None;
            }
            path_class
        }
        _ => FactClass::GeneratedItems,
    };
    let module = scoped_context(inputs, &sema, path.syntax(), fact_class)?;
    if !definition.is_visible_from(&inputs.db, module) {
        return None;
    }
    match definition {
        ModuleDef::EnumVariant(variant) => {
            let source = sema.source(variant)?;
            let declaration_module =
                scoped_context(inputs, &sema, source.value.syntax(), fact_class)?;
            // Variants are not ast::Items: audit their own attributes explicitly,
            // including written siblings that RA may omit under incomplete cfg.
            let enum_source = sema.source(variant.parent_enum(&inputs.db))?;
            for written in enum_source.value.variant_list()?.variants() {
                if written
                    .syntax()
                    .descendants()
                    .filter_map(ast::Attr::cast)
                    .any(|a| !safe_attr(inputs, &sema, declaration_module, &a, fact_class))
                {
                    return None;
                }
            }
            let declaration = inputs.declaration(
                variant.module(&inputs.db),
                source.value.name()?.syntax(),
                source.file_id,
            )?;
            // The parent enum is part of the anchored identity, not merely the
            // terminal spelling. Generic/unknown enum arguments still refuse.
            let original = receiver(
                inputs,
                variant.parent_enum(&inputs.db).ty(&inputs.db),
                fact_class,
                0,
            )?;
            Some(Fact {
                declaration,
                original: Some(original.clone()),
                adjusted: Some(original),
                classification: "variant_path",
                fact_class,
            })
        }
        ModuleDef::Adt(adt) => {
            let declaration = adt_declaration(inputs, adt, fact_class)?;
            if declaration.declarative_macro.is_some() {
                // Identity-only evidence must not infer fields, layout, constructors
                // or members, even when RA happens to know the generated output.
                if !path
                    .syntax()
                    .parent()
                    .is_some_and(|n| ast::PathType::can_cast(n.kind()))
                    || path
                        .syntax()
                        .descendants()
                        .any(|n| ast::GenericArgList::can_cast(n.kind()))
                    || declaration.crate_origin
                        != *inputs
                            .roots
                            .get(&module.krate(&inputs.db).root_file(&inputs.db))?
                {
                    return None;
                }
                return Some(Fact {
                    declaration,
                    original: None,
                    adjusted: None,
                    classification: "declaration_identity",
                    fact_class,
                });
            }
            // Construction needs field access too, not just access to the type name.
            if path
                .syntax()
                .ancestors()
                .any(|n| ast::RecordExpr::can_cast(n.kind()) || ast::CallExpr::can_cast(n.kind()))
            {
                match adt {
                    Adt::Struct(s)
                        if s.fields(&inputs.db)
                            .iter()
                            .all(|f| f.is_visible_from(&inputs.db, module)) => {}
                    _ => return None,
                }
            }
            let original = receiver(inputs, adt.ty(&inputs.db), fact_class, 0)?;
            Some(Fact {
                declaration,
                original: Some(original.clone()),
                adjusted: Some(original),
                classification: "type_or_constructor",
                fact_class,
            })
        }
        ModuleDef::Function(function) => {
            let source = sema.source(function)?;
            ordinary_context(inputs, &sema, source.value.syntax())?;
            if source.value.generic_param_list().is_some() {
                return None;
            }
            let declaration = inputs.declaration(
                function.module(&inputs.db),
                source.value.name()?.syntax(),
                source.file_id,
            )?;
            if let Some(associated) = function.as_assoc_item(&inputs.db) {
                let AssocItemContainer::Impl(implementation) = associated.container(&inputs.db)
                else {
                    return None;
                };
                if implementation.trait_(&inputs.db).is_some() {
                    return None;
                }
                let original = receiver(inputs, implementation.self_ty(&inputs.db), fact_class, 0)?;
                Some(Fact {
                    declaration,
                    original: Some(original.clone()),
                    adjusted: Some(original),
                    classification: "inherent_function",
                    fact_class,
                })
            } else {
                Some(Fact {
                    declaration,
                    original: None,
                    adjusted: None,
                    classification: "written_function",
                    fact_class,
                })
            }
        }
        _ => None,
    }
}
