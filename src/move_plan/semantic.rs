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

const ANALYZER: &str = "ra_ap@0.0.357";

#[cfg(test)]
#[path = "semantic_tests.rs"]
mod tests;

/// One caller-declared configuration. No manifests, environment, or sysroot are discovered.
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
}
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(rename_all = "snake_case")]
pub enum ProofClass {
    RaResolved,
}
/// Stable written identities, not revision-local RA IDs or pretty-printed types.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct Declaration {
    pub crate_origin: String,
    pub anchor: SourceAnchor,
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
    pub source_access: bool,
    pub final_access: bool,
    pub coverage: ResolutionCoverage,
}

pub fn candidate(need: &Need) -> bool {
    matches!(
        need.reason,
        DecisionReason::MemberOrConstructorUnproved | DecisionReason::ExternalOrMissingBinding
    )
}

struct Inputs {
    db: RootDatabase,
    ids: BTreeMap<String, FileId>,
    paths: BTreeMap<FileId, String>,
    texts: BTreeMap<String, String>,
    roots: BTreeMap<FileId, String>,
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
    let Some(old) = Inputs::new(original, config, controls) else {
        items::check(deadline, cancelled)?;
        configuration_refusal(needs);
        return Ok(());
    };
    let Some(new) = Inputs::new(overlay, config, controls) else {
        items::check(deadline, cancelled)?;
        configuration_refusal(needs);
        return Ok(());
    };
    let coverage = ResolutionCoverage {
        decisions: 0, configuration: config.clone(), snapshot_id: snapshot,
        semantic_input_digest: digest, final_overlay_digest: final_digest,
        analyzer: ANALYZER.into(), statement: String::new(),
        omissions: vec!["one explicit configuration only; no other targets/features/cfg combinations".into(),
            "no discovered sysroot/dependency sources, build-script cfg/environment, macro expansion, target layout, compilation or equivalence checks".into()],
    };
    let mut proofs = controlled(&old.db, &new.db, controls, || {
        evaluate(&old, &new, needs, &result.plan.origins, &coverage, controls)
    })?;
    let mut coverage = coverage;
    coverage.decisions = proofs.len();
    coverage.statement = format!(
        "resolution performed for {} decisions under one explicit configuration; compilation/equivalence not performed",
        proofs.len()
    );
    for proof in &mut proofs {
        proof.coverage = coverage.clone();
    }
    result.coverage.ra_resolved = proofs.len();
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
    Some(Declaration {
        crate_origin: decl.crate_origin.clone(),
        anchor,
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
            refusal_class = "semantic_final_fact_unproved";
            let after = fact(new, &final_anchor)?;
            refusal_class = "semantic_identity_unproved";
            if before.classification != after.classification
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
                class: ProofClass::RaResolved,
                anchor,
                item_ids: need.item_ids.clone(),
                destination_path: final_anchor.path.clone(),
                final_anchor,
                original_receiver: before.original,
                adjusted_receiver: before.adjusted,
                final_original_receiver: after.original,
                final_adjusted_receiver: after.adjusted,
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
}

fn receiver(inputs: &Inputs, ty: Type<'_>, depth: usize) -> Option<Receiver> {
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
    let declaration = adt_declaration(inputs, adt)?;
    let arguments = args
        .into_iter()
        .map(|a| receiver(inputs, a?, depth + 1))
        .collect::<Option<_>>()?;
    Some(Receiver {
        references,
        builtin: None,
        declaration: Some(declaration),
        arguments,
    })
}
fn adt_declaration(inputs: &Inputs, adt: Adt) -> Option<Declaration> {
    let sema = Semantics::new(&inputs.db);
    match adt {
        Adt::Struct(s) => {
            let source = sema.source(s)?;
            ordinary_context(inputs, &sema, source.value.syntax())?;
            inputs.declaration(
                s.module(&inputs.db),
                source.value.name()?.syntax(),
                source.file_id,
            )
        }
        Adt::Enum(s) => {
            let source = sema.source(s)?;
            ordinary_context(inputs, &sema, source.value.syntax())?;
            inputs.declaration(
                s.module(&inputs.db),
                source.value.name()?.syntax(),
                source.file_id,
            )
        }
        Adt::Union(_) => None,
    }
}

/// Ignore bodies of unrelated items, but refuse unknown namespace/impl effects in
/// the visible module chain and at the selected written occurrence/declaration.
fn ordinary_context(
    inputs: &Inputs,
    sema: &Semantics<'_, RootDatabase>,
    node: &SyntaxNode,
) -> Option<Module> {
    if node
        .ancestors()
        .any(|n| ast::MacroCall::can_cast(n.kind()) || ast::MacroRules::can_cast(n.kind()))
    {
        return None;
    }
    for ancestor in node.ancestors() {
        if let Some(item) = ast::Item::cast(ancestor) {
            if item.attrs().any(|a| !safe_attr(&a)) {
                return None;
            }
            if let ast::Item::Fn(f) = item
                && f.generic_param_list().is_some()
            {
                return None;
            }
        }
    }
    let module = sema.scope(node)?.module();
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
        let file = scope.krate(&inputs.db).root_file(&inputs.db);
        let root = sema.parse_guess_edition(file);
        if root.attrs().any(|a| !safe_attr(&a)) {
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
        let syntax = match source.value {
            ra_ap_hir::ModuleSource::SourceFile(f) => f.syntax().clone(),
            ra_ap_hir::ModuleSource::Module(m) => m.item_list()?.syntax().clone(),
            ra_ap_hir::ModuleSource::BlockExpr(_) => return None,
        };
        for child in syntax.children() {
            if ast::MacroCall::can_cast(child.kind()) {
                return None;
            }
            if let Some(item) = ast::Item::cast(child) {
                // Sibling module conditions do not change this module's namespace.
                // Conditions on a module we actually traverse are checked above.
                if !matches!(item, ast::Item::Module(_)) && item.attrs().any(|a| !safe_attr(&a)) {
                    return None;
                }
            }
        }
    }
    Some(module)
}
fn safe_attr(attr: &ast::Attr) -> bool {
    matches!(
        attr.simple_name().as_deref(),
        Some(
            "allow"
                | "warn"
                | "deny"
                | "forbid"
                | "doc"
                | "inline"
                | "cold"
                | "must_use"
                | "proc_macro"
                | "proc_macro_attribute"
                | "proc_macro_derive"
                | "no_std"
                | "no_core"
        )
    )
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
    ra_ap_hir::attach_db(&inputs.db, || fact_attached(inputs, anchor))
}
fn fact_attached(inputs: &Inputs, anchor: &SourceAnchor) -> Option<Fact> {
    let sema = Semantics::new(&inputs.db);
    let root = sema.parse_guess_edition(*inputs.ids.get(&anchor.path)?);
    let syntax = root.syntax();
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
        let original = receiver(inputs, types.original.clone(), 0)?;
        let adjusted = receiver(inputs, types.adjusted(), 0)?;
        return Some(Fact {
            declaration,
            original: Some(original),
            adjusted: Some(adjusted),
            classification: "inherent_function",
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
        let original = receiver(inputs, types.original.clone(), 0)?;
        let adjusted = receiver(inputs, types.adjusted(), 0)?;
        return Some(Fact {
            declaration,
            original: Some(original),
            adjusted: Some(adjusted),
            classification: "field",
        });
    }
    let path = exact_path(syntax, &anchor.range)?;
    let module = ordinary_context(inputs, &sema, path.syntax())?;
    let PathResolution::Def(definition) = sema.resolve_path(&path)? else {
        return None;
    };
    if !definition.is_visible_from(&inputs.db, module) {
        return None;
    }
    match definition {
        ModuleDef::Adt(adt) => {
            let declaration = adt_declaration(inputs, adt)?;
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
            let original = receiver(inputs, adt.ty(&inputs.db), 0)?;
            Some(Fact {
                declaration,
                original: Some(original.clone()),
                adjusted: Some(original),
                classification: "type_or_constructor",
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
                let original = receiver(inputs, implementation.self_ty(&inputs.db), 0)?;
                Some(Fact {
                    declaration,
                    original: Some(original.clone()),
                    adjusted: Some(original),
                    classification: "inherent_function",
                })
            } else {
                Some(Fact {
                    declaration,
                    original: None,
                    adjusted: None,
                    classification: "written_function",
                })
            }
        }
        _ => None,
    }
}
