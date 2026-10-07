//! Admission of written attributes under one explicit, positive cfg declaration.
use super::{CrateInput, Inputs};
use crate::{items, plan::SourceAnchor};
use ra_ap_hir::{Module, Semantics};
use ra_ap_ide_db::RootDatabase;
use ra_ap_syntax::{AstNode, AstToken, SyntaxKind, SyntaxNode, ast};
use rmcp::schemars::JsonSchema;
use serde::Serialize;

/// Derives append items; they cannot replace a written declaration. Only facts
/// about that declaration may ignore their unknown generated output.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum FactClass {
    NominalIdentity,
    GeneratedItems,
}
impl FactClass {
    pub(super) fn name(self) -> &'static str {
        match self {
            Self::NominalIdentity => "nominal_identity",
            Self::GeneratedItems => "generated_items",
        }
    }
    pub(super) fn basis(self) -> &'static str {
        match self {
            Self::NominalIdentity => {
                "nominal identity via written declaration; derive emits additional items only; generated-item facts retain their veto; no expansion, compilation or equivalence checking"
            }
            Self::GeneratedItems => {
                "written resolution with generated-item context audited; unknown/non-builtin derives retain their veto; no expansion, compilation or equivalence checking"
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct ContextEvaluation {
    pub revision: String,
    pub crate_name: String,
    pub anchor: SourceAnchor,
    pub kind: String,
    pub fact_class: String,
    pub basis: String,
    pub status: String,
    pub value: Option<bool>,
    pub reason: String,
}

pub(super) fn record(
    inputs: &Inputs,
    sema: &Semantics<'_, RootDatabase>,
    module: Module,
    node: &SyntaxNode,
    fact_class: FactClass,
    outcome: (&str, &str, Option<bool>, &str),
) {
    let Some(declaration) = inputs.declaration(module, node, sema.hir_file_for(node)) else {
        return;
    };
    let anchor = declaration.anchor;
    inputs.context.borrow_mut().insert(
        (
            anchor.path.clone(),
            anchor.range.start_byte,
            anchor.range.end_byte,
            fact_class.name(),
        ),
        ContextEvaluation {
            revision: String::new(),
            crate_name: declaration.crate_origin,
            anchor,
            kind: outcome.0.into(),
            fact_class: fact_class.name().into(),
            basis: fact_class.basis().into(),
            status: outcome.1.into(),
            value: outcome.2,
            reason: outcome.3.into(),
        },
    );
}

pub(super) fn safe_attr(
    inputs: &Inputs,
    sema: &Semantics<'_, RootDatabase>,
    module: Module,
    attr: &ast::Attr,
    fact_class: FactClass,
) -> bool {
    let root = module.krate(&inputs.db).root_file(&inputs.db);
    let Some(config) = inputs
        .configuration
        .crates
        .iter()
        .find(|c| inputs.ids.get(&c.root_file) == Some(&root))
    else {
        return false;
    };
    // Parser recovery is never evidence, including malformed token-tree derives.
    let Some(file) = sema.hir_file_for(attr.syntax()).file_id() else {
        return false;
    };
    let Some(text) = inputs
        .paths
        .get(&file.file_id(&inputs.db))
        .and_then(|p| inputs.texts.get(p))
    else {
        return false;
    };
    let parsed =
        ra_ap_syntax::SourceFile::parse(text, module.krate(&inputs.db).edition(&inputs.db));
    let malformed = parsed
        .errors()
        .iter()
        .any(|e| attr.syntax().text_range().contains_range(e.range()));
    let outcome = if malformed {
        Err("unparseable_attribute")
    } else if let Some(meta) = attr.meta() {
        safe_meta(inputs, sema, module, config, &meta, fact_class, 0)
    } else {
        Err("unparseable_attribute")
    };
    record(
        inputs,
        sema,
        module,
        attr.syntax(),
        fact_class,
        (
            "attribute",
            if outcome.is_ok() {
                "admitted"
            } else {
                "skipped"
            },
            None,
            outcome.err().unwrap_or(match fact_class {
                FactClass::NominalIdentity => "written_identity_under_declared_configuration",
                FactClass::GeneratedItems => "inert_under_declared_configuration",
            }),
        ),
    );
    outcome.is_ok()
}

fn safe_meta(
    inputs: &Inputs,
    sema: &Semantics<'_, RootDatabase>,
    module: Module,
    config: &CrateInput,
    meta: &ast::Meta,
    fact_class: FactClass,
    depth: usize,
) -> Result<(), &'static str> {
    if depth > 32 {
        return Err("attribute_depth_limit");
    }
    match meta {
        ast::Meta::CfgMeta(cfg) => {
            predicate(
                inputs,
                sema,
                module,
                config,
                &cfg.cfg_predicate().ok_or("unparseable_cfg")?,
                fact_class,
                0,
            )?;
            Ok(())
        }
        ast::Meta::CfgAttrMeta(cfg) => {
            let enabled = predicate(
                inputs,
                sema,
                module,
                config,
                &cfg.cfg_predicate().ok_or("unparseable_cfg")?,
                fact_class,
                0,
            )?;
            let metas: Vec<_> = cfg.metas().collect();
            if metas.is_empty() {
                return Err("unparseable_cfg_attr");
            }
            for nested in metas {
                if enabled {
                    let result =
                        safe_meta(inputs, sema, module, config, &nested, fact_class, depth + 1);
                    record(
                        inputs,
                        sema,
                        module,
                        nested.syntax(),
                        fact_class,
                        (
                            "attribute",
                            if result.is_ok() {
                                "admitted"
                            } else {
                                "skipped"
                            },
                            None,
                            result.err().unwrap_or(match fact_class {
                                FactClass::NominalIdentity => {
                                    "written_identity_under_declared_configuration"
                                }
                                FactClass::GeneratedItems => "inert_under_declared_configuration",
                            }),
                        ),
                    );
                    result?;
                } else {
                    record(
                        inputs,
                        sema,
                        module,
                        nested.syntax(),
                        fact_class,
                        ("attribute", "inactive", None, "cfg_attr_condition_false"),
                    );
                }
            }
            Ok(())
        }
        _ if meta.simple_name().as_deref() == Some("derive") => {
            let (_, tree) = meta.as_simple_call().ok_or("unparseable_derive")?;
            let tokens: Vec<_> = tree
                .syntax()
                .descendants_with_tokens()
                .filter_map(|n| n.into_token())
                .filter(|t| !t.kind().is_trivia())
                .collect();
            if tokens.first().map(|t| t.text()) != Some("(")
                || tokens.last().map(|t| t.text()) != Some(")")
            {
                return Err("unparseable_derive");
            }
            if matches!(fact_class, FactClass::NominalIdentity)
                && meta
                    .syntax()
                    .ancestors()
                    .find_map(ast::Item::cast)
                    .is_some_and(|item| {
                        matches!(
                            item,
                            ast::Item::Struct(_) | ast::Item::Enum(_) | ast::Item::Union(_)
                        )
                    })
            {
                // https://doc.rust-lang.org/reference/procedural-macros.html#macro.proc.derive.output
                // A custom derive's output is additive, including on this item.
                // This admits only declaration identity, never its generated impls.
                return nominal_derive_paths(&tokens[1..tokens.len() - 1]);
            }
            let mut name = true;
            let mut count = 0;
            let mut names = Vec::new();
            for token in &tokens[1..tokens.len() - 1] {
                if name {
                    if token.kind() != SyntaxKind::IDENT
                        || !items::BUILTIN_DERIVES
                            .iter()
                            .any(|(n, _)| *n == token.text().trim_start_matches("r#"))
                    {
                        return Err("non_builtin_derive");
                    }
                    names.push(token.text().trim_start_matches("r#"));
                    count += 1;
                } else if token.text() != "," {
                    return Err("non_builtin_derive");
                }
                name = !name;
            }
            if count == 0 {
                return Err("unparseable_derive");
            }
            let scope = sema.scope(meta.syntax()).ok_or("derive_scope_unproved")?;
            let mut shadowed = false;
            scope.process_all_names(&mut |name, definition| {
                if names.contains(&name.as_str())
                    && matches!(definition, ra_ap_hir::ScopeDef::ModuleDef(ra_ap_hir::ModuleDef::Macro(m)) if m.builtin_derive_kind(&inputs.db).is_none())
                {
                    shadowed = true;
                }
            });
            if shadowed {
                Err("non_builtin_derive")
            } else {
                Ok(())
            }
        }
        _ if matches!(
            meta.simple_name().as_deref(),
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
        ) =>
        {
            Ok(())
        }
        _ => Err("unsupported_attribute"),
    }
}

/// Validate the written derive path list without resolving or executing macros.
fn nominal_derive_paths(tokens: &[ra_ap_syntax::SyntaxToken]) -> Result<(), &'static str> {
    // Token trees retain separate punctuation tokens in the pinned RA parser.
    // A path separator must be a joint pair, not two whitespace-separated colons.
    let separator = |at: usize| match (tokens.get(at), tokens.get(at + 1)) {
        (Some(a), _) if a.text() == "::" => 1,
        (Some(a), Some(b))
            if a.text() == ":"
                && b.text() == ":"
                && a.text_range().end() == b.text_range().start() =>
        {
            2
        }
        _ => 0,
    };
    let mut at = 0;
    while at < tokens.len() {
        at += separator(at);
        loop {
            let token = tokens.get(at).ok_or("unparseable_derive")?;
            if !matches!(
                token.kind(),
                SyntaxKind::IDENT
                    | SyntaxKind::CRATE_KW
                    | SyntaxKind::SELF_KW
                    | SyntaxKind::SUPER_KW
            ) {
                return Err("unparseable_derive");
            }
            at += 1;
            let width = separator(at);
            if width != 0 {
                at += width;
            } else {
                break;
            }
        }
        if at == tokens.len() {
            return Ok(());
        }
        if tokens[at].text() != "," {
            return Err("unparseable_derive");
        }
        at += 1;
        if at == tokens.len() {
            return Ok(());
        }
    }
    Err("unparseable_derive")
}

fn predicate(
    inputs: &Inputs,
    sema: &Semantics<'_, RootDatabase>,
    module: Module,
    config: &CrateInput,
    pred: &ast::CfgPredicate,
    fact_class: FactClass,
    depth: usize,
) -> Result<bool, &'static str> {
    let outcome = (|| {
        if depth > 32 {
            return Err("cfg_depth_limit");
        }
        match pred {
            ast::CfgPredicate::CfgAtom(atom) => {
                if atom.true_token().is_some() {
                    return Ok(true);
                }
                if atom.false_token().is_some() {
                    return Ok(false);
                }
                let key = atom.ident_token().ok_or("unparseable_cfg")?;
                let key = key.text().trim_start_matches("r#");
                let value = if atom.eq_token().is_some() {
                    Some(
                        ast::String::cast(atom.string_token().ok_or("unparseable_cfg")?)
                            .ok_or("unparseable_cfg")?
                            .value()
                            .map_err(|_| "unparseable_cfg")?
                            .into_owned(),
                    )
                } else {
                    None
                };
                if config.cfg.iter().any(|c| c.key == key && c.value == value)
                    || (key == "feature"
                        && value.as_ref().is_some_and(|v| config.features.contains(v)))
                {
                    Ok(true)
                } else {
                    Err("undeclared_cfg_atom")
                }
            }
            ast::CfgPredicate::CfgComposite(composite) => {
                let keyword = composite.keyword().ok_or("unparseable_cfg")?;
                if !matches!(keyword.text(), "all" | "any" | "not") {
                    return Err("unsupported_cfg_predicate");
                }
                // Deliberately evaluate every operand: short-circuiting cannot hide unknown atoms.
                let values: Vec<_> = composite
                    .cfg_predicates()
                    .map(|p| predicate(inputs, sema, module, config, &p, fact_class, depth + 1))
                    .collect();
                let values = values.into_iter().collect::<Result<Vec<_>, _>>()?;
                match keyword.text() {
                    "all" => Ok(values.iter().all(|v| *v)),
                    "any" => Ok(values.iter().any(|v| *v)),
                    "not" if values.len() == 1 => Ok(!values[0]),
                    _ => Err("unsupported_cfg_predicate"),
                }
            }
        }
    })();
    record(
        inputs,
        sema,
        module,
        pred.syntax(),
        fact_class,
        (
            "cfg_predicate",
            if outcome.is_ok() {
                "evaluated"
            } else {
                "skipped"
            },
            outcome.ok(),
            outcome.err().unwrap_or("declared_configuration"),
        ),
    );
    outcome
}
