//! Declaration identity from written tokens, never generated-member semantics.
//! Declarative substitution executes no code; proc macros do and never qualify.
use super::{Declaration, Inputs, context, context::FactClass};
use crate::plan::SourceAnchor;
use ra_ap_hir::{Adt, MacroKind, Module, Semantics};
use ra_ap_ide_db::RootDatabase;
use ra_ap_syntax::{
    AstNode, SyntaxKind, SyntaxNode, SyntaxToken,
    ast::{self, HasAttrs, HasGenericParams, HasName},
};
use rmcp::schemars::JsonSchema;
use serde::Serialize;

pub(super) const BASIS: &str = "declaration identity established through bounded declarative-macro expansion of written tokens; resolved ADT and written argument provenance checked at both overlays; original/final invocation and in-crate macro_rules definition anchors match; provider-uncertain attributes refused, helper registration not inferred from derive spelling; no generated impl, method, field, constructor or variant facts; no proc-macro execution, compilation or equivalence checking";
pub(super) const ASSUMED_BASIS: &str = "caller enabled assume_declared_helpers and asserts each provider-uncertain written or generated type attribute belongs to a registered derive helper; declaration identity assumed with the type, not engine-classified; written declaration anchors and resolved identities at both overlays still required; bounded written-token and resolved ADT/argument provenance checks additionally required for generated declarations, with matching original/final invocation and definition anchors for generated declarations; companion derive paths checked syntactically only; no procedural expansion, macro hygiene, generated impl/member/constructor/variant, compilation or equivalence claims";
const CONTEXT_BASIS: &str = "bounded declarative-macro written-token admission only; declaration identity additionally requires matching original/final invocation and definition anchors; provider-uncertain attributes refused, helper registration not inferred from derive spelling; generated members and proc macros are not admitted";
const TOKEN_CAP: usize = 4096;
const NESTING_CAP: usize = 32;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct Evidence {
    pub invocation: SourceAnchor,
    pub definition: SourceAnchor,
}

struct Expansion {
    invocation: Declaration,
    definition: Declaration,
    argument: String,
    syntax: SyntaxNode,
    assumed_helpers: bool,
}

fn tokens(node: &SyntaxNode) -> Result<Vec<SyntaxToken>, &'static str> {
    let tokens: Vec<_> = node
        .descendants_with_tokens()
        .filter_map(|n| n.into_token())
        .filter(|t| !t.kind().is_trivia())
        .take(TOKEN_CAP + 1)
        .collect();
    if tokens.len() > TOKEN_CAP {
        return Err("declarative_token_limit");
    }
    let mut depth = 0usize;
    for token in &tokens {
        match token.text() {
            "(" | "{" | "[" => {
                depth += 1;
                if depth > NESTING_CAP {
                    return Err("declarative_nesting_limit");
                }
            }
            ")" | "}" | "]" => {
                depth = depth.checked_sub(1).ok_or("declarative_unparseable")?;
            }
            _ => {}
        }
    }
    if depth != 0 {
        return Err("declarative_unparseable");
    }
    Ok(tokens)
}

fn unconditional(
    inputs: &Inputs,
    sema: &Semantics<'_, RootDatabase>,
    module: Module,
    node: &SyntaxNode,
) -> Result<(), &'static str> {
    for ancestor in node.ancestors() {
        for attr in ancestor.children().filter_map(ast::Attr::cast) {
            if attr.syntax().descendants().any(|n| {
                matches!(
                    ast::Meta::cast(n),
                    Some(ast::Meta::CfgMeta(_) | ast::Meta::CfgAttrMeta(_))
                )
            }) {
                return Err("declarative_conditional_context");
            }
            if !crate::items::identity_inert_metadata(attr.simple_name().as_deref())
                && attr.simple_name().as_deref() != Some("macro_export")
            {
                return Err(provider_uncertain(inputs, sema, module, &attr, None));
            }
        }
    }
    Ok(())
}

fn written_module(
    inputs: &Inputs,
    sema: &Semantics<'_, RootDatabase>,
    module: Module,
) -> Result<(), &'static str> {
    for scope in module.path_to_root(&inputs.db) {
        if let Some(declaration) = scope.declaration_source(&inputs.db) {
            unconditional(inputs, sema, scope, declaration.value.syntax())?;
        }
        let source = scope.definition_source(&inputs.db);
        let file = source
            .file_id
            .file_id()
            .ok_or("declarative_definition_unproved")?
            .file_id(&inputs.db);
        let path = inputs
            .paths
            .get(&file)
            .ok_or("declarative_definition_unproved")?;
        let text = inputs
            .texts
            .get(path)
            .ok_or("declarative_definition_unproved")?;
        let parsed =
            ra_ap_syntax::SourceFile::parse(text, scope.krate(&inputs.db).edition(&inputs.db));
        if !parsed.errors().is_empty() {
            return Err("declarative_unparseable");
        }
        unconditional(inputs, sema, scope, sema.parse_guess_edition(file).syntax())?;
    }
    Ok(())
}

fn record_definition(inputs: &Inputs, definition: &Declaration, outcome: Result<(), &'static str>) {
    let anchor = definition.anchor.clone();
    let key = (
        anchor.path.clone(),
        anchor.range.start_byte,
        anchor.range.end_byte,
        FactClass::NominalIdentity.name(),
    );
    inputs.context.borrow_mut().insert(
        key,
        context::ContextEvaluation {
            revision: String::new(),
            crate_name: definition.crate_origin.clone(),
            anchor,
            kind: "declarative_macro_definition".into(),
            fact_class: FactClass::NominalIdentity.name().into(),
            basis: CONTEXT_BASIS.into(),
            status: if outcome.is_ok() {
                "admitted"
            } else {
                "skipped"
            }
            .into(),
            value: None,
            reason: outcome
                .err()
                .unwrap_or("bounded_declarative_definition")
                .into(),
        },
    );
}

/// This is an admission budget, not another macro interpreter. RA owns expansion.
fn matcher(definition: &ast::MacroRules) -> Result<(), &'static str> {
    let tree = definition.token_tree().ok_or("declarative_unparseable")?;
    let ts = tokens(tree.syntax())?;
    // One arm: { ($name:ident) => { ... }; }. Punctuation is separate RA tokens.
    if ts.len() < 13
        || ts[..3].iter().map(|t| t.text()).collect::<Vec<_>>() != ["{", "(", "$"]
        || ts[3].kind() != SyntaxKind::IDENT
        || ts[4..9].iter().map(|t| t.text()).collect::<Vec<_>>() != [":", "ident", ")", "=", ">"]
        || ts[9].text() != "{"
    {
        return Err("declarative_fragment_limit");
    }
    let body = tree
        .syntax()
        .children()
        .filter_map(ast::TokenTree::cast)
        .nth(1)
        .ok_or("declarative_fragment_limit")?;
    let end = body.syntax().text_range().end();
    let tail: Vec<_> = ts
        .iter()
        .filter(|t| t.text_range().start() >= end)
        .map(|t| t.text())
        .collect();
    if tail != [";", "}"] && tail != ["}"] {
        return Err("declarative_fragment_limit");
    }
    let body_tokens = tokens(body.syntax())?;
    for (at, token) in body_tokens.iter().enumerate() {
        if token.text() == "$"
            && body_tokens
                .get(at + 1)
                .is_none_or(|t| t.kind() != SyntaxKind::IDENT || t.text() != ts[3].text())
        {
            return Err("declarative_fragment_limit");
        }
        if token.text() == "!"
            && body_tokens
                .get(at + 1)
                .is_some_and(|t| matches!(t.text(), "(" | "{" | "["))
        {
            return Err("declarative_recursion_limit");
        }
    }
    Ok(())
}

/// A helper's spelling does not prove its registration or prevent replacement.
/// Expansion attributes lack real-file anchors; disclose their written definition.
fn provider_uncertain(
    inputs: &Inputs,
    sema: &Semantics<'_, RootDatabase>,
    module: Module,
    attr: &ast::Attr,
    definition: Option<&Declaration>,
) -> &'static str {
    let reason = "declarative_attribute_provider_uncertain";
    let written = inputs.declaration(module, attr.syntax(), sema.hir_file_for(attr.syntax()));
    if let Some(declaration) = written.as_ref().or(definition) {
        let anchor = declaration.anchor.clone();
        inputs.context.borrow_mut().insert(
            (
                anchor.path.clone(),
                anchor.range.start_byte,
                anchor.range.end_byte,
                FactClass::NominalIdentity.name(),
            ),
            context::ContextEvaluation {
                revision: String::new(),
                crate_name: declaration.crate_origin.clone(),
                anchor,
                kind: if written.is_some() {
                    "attribute"
                } else {
                    "declarative_macro_definition"
                }
                .into(),
                fact_class: FactClass::NominalIdentity.name().into(),
                basis: format!(
                    "{CONTEXT_BASIS}; provider-uncertain attribute {}; helper registration and attribute inertness not proved",
                    attr.syntax().text()
                ),
                status: "skipped".into(),
                value: None,
                reason: reason.into(),
            },
        );
    }
    reason
}

fn builtin_derives(inputs: &Inputs, sema: &Semantics<'_, RootDatabase>, attr: &ast::Attr) -> bool {
    let Some((_, tree)) = attr.meta().and_then(|m| m.as_simple_call()) else {
        return false;
    };
    let Ok(ts) = tokens(tree.syntax()) else {
        return false;
    };
    if ts.len() < 3 || ts[0].text() != "(" || ts[ts.len() - 1].text() != ")" {
        return false;
    }
    let mut names = Vec::new();
    for (at, token) in ts[1..ts.len() - 1].iter().enumerate() {
        if at % 2 == 0 {
            let name = token.text().trim_start_matches("r#");
            if token.kind() != SyntaxKind::IDENT
                || !crate::items::BUILTIN_DERIVES
                    .iter()
                    .any(|(n, _)| *n == name)
            {
                return false;
            }
            names.push(name);
        } else if token.text() != "," {
            return false;
        }
    }
    builtin_names_unshadowed(inputs, sema, attr, &names)
}

fn builtin_names_unshadowed(
    inputs: &Inputs,
    sema: &Semantics<'_, RootDatabase>,
    attr: &ast::Attr,
    names: &[&str],
) -> bool {
    let Some(scope) = sema.scope(attr.syntax()) else {
        return false;
    };
    let mut shadowed = false;
    scope.process_all_names(&mut |name, definition| {
        if names.contains(&name.as_str())
            && matches!(definition, ra_ap_hir::ScopeDef::ModuleDef(ra_ap_hir::ModuleDef::Macro(m)) if m.builtin_derive_kind(&inputs.db).is_none())
        {
            shadowed = true;
        }
    });
    !shadowed
}

/// Default admission exempts only inert metadata and unshadowed built-in derives.
/// The opt-in asserts helper registration on generated types, never written owners.
fn type_attributes(
    inputs: &Inputs,
    sema: &Semantics<'_, RootDatabase>,
    module: Module,
    definition: &Declaration,
    item: &ast::Item,
) -> Result<bool, &'static str> {
    let attrs: Vec<_> = item.attrs().collect();
    let mut assumed = false;
    // Check helpers before derives so a familiar derive spelling cannot hide the
    // specific replacement-capable attribute in the anchored disclosure.
    for attr in attrs
        .iter()
        .filter(|a| a.simple_name().as_deref() != Some("derive"))
    {
        if !crate::items::identity_inert_metadata(attr.simple_name().as_deref()) {
            // Control attributes cannot become helpers by caller assertion.
            if !inputs.assume_declared_helpers
                || attr.meta().is_none()
                || matches!(
                    attr.simple_name().as_deref(),
                    Some("cfg" | "cfg_attr" | "no_implicit_prelude" | "no_std" | "no_core")
                )
            {
                return Err(provider_uncertain(
                    inputs,
                    sema,
                    module,
                    attr,
                    Some(definition),
                ));
            }
            assumed = true;
            record_definition(inputs, definition, Ok(()));
            let key = (
                definition.anchor.path.clone(),
                definition.anchor.range.start_byte,
                definition.anchor.range.end_byte,
                FactClass::NominalIdentity.name(),
            );
            if let Some(evaluation) = inputs.context.borrow_mut().get_mut(&key) {
                evaluation.reason = "assumed_declared_helpers".into();
                evaluation.basis = format!(
                    "{ASSUMED_BASIS}; caller-assumed helper {}",
                    attr.syntax().text()
                );
            }
        }
    }
    for attr in attrs
        .iter()
        .filter(|a| a.simple_name().as_deref() == Some("derive"))
    {
        if !builtin_derives(inputs, sema, attr) {
            // A helper-bearing type may carry custom derives, but only the
            // written path list is admitted: no derive provider is executed.
            let paths_valid = assumed
                && attr
                    .meta()
                    .and_then(|m| m.as_simple_call())
                    .is_some_and(|(_, tree)| {
                        tokens(tree.syntax()).is_ok_and(|ts| {
                            let builtins: Vec<_> = ts
                                .iter()
                                .filter_map(|t| {
                                    let name = t.text().trim_start_matches("r#");
                                    crate::items::BUILTIN_DERIVES
                                        .iter()
                                        .any(|(n, _)| *n == name)
                                        .then_some(name)
                                })
                                .collect();
                            ts.len() >= 3
                                && ts[0].text() == "("
                                && ts[ts.len() - 1].text() == ")"
                                && context::nominal_derive_paths(&ts[1..ts.len() - 1]).is_ok()
                                && builtin_names_unshadowed(inputs, sema, attr, &builtins)
                        })
                    });
            if paths_valid {
                continue;
            }
            return Err(provider_uncertain(
                inputs,
                sema,
                module,
                attr,
                Some(definition),
            ));
        }
    }
    Ok(assumed)
}

fn inspect(
    inputs: &Inputs,
    sema: &Semantics<'_, RootDatabase>,
    module: Module,
    call: &ast::MacroCall,
) -> Result<Expansion, &'static str> {
    unconditional(inputs, sema, module, call.syntax())?;
    let mac = sema
        .resolve_macro_call(call)
        .ok_or("declarative_definition_unproved")?;
    if mac.is_proc_macro() || mac.kind(&inputs.db) != MacroKind::Declarative {
        return Err("declarative_definition_unproved");
    }
    if mac.module(&inputs.db).krate(&inputs.db) != module.krate(&inputs.db) {
        return Err("declarative_external_definition");
    }
    let source = sema.source(mac).ok_or("declarative_definition_unproved")?;
    let Some(ast::Macro::MacroRules(def)) = source.value.left() else {
        return Err("declarative_definition_unproved");
    };
    let definition = inputs
        .declaration(mac.module(&inputs.db), def.syntax(), source.file_id)
        .ok_or("declarative_definition_unproved")?;
    let invocation = inputs
        .declaration(module, call.syntax(), sema.hir_file_for(call.syntax()))
        .ok_or("declarative_invocation_unproved")?;
    let definition_admission = unconditional(inputs, sema, mac.module(&inputs.db), def.syntax())
        .and_then(|()| written_module(inputs, sema, mac.module(&inputs.db)))
        .and_then(|()| written_module(inputs, sema, module))
        .and_then(|()| matcher(&def));
    record_definition(inputs, &definition, definition_admission);
    definition_admission?;
    let args = call.token_tree().ok_or("declarative_fragment_limit")?;
    let ts = tokens(args.syntax())?;
    if ts.len() != 3 || ts[1].kind() != SyntaxKind::IDENT {
        return Err("declarative_fragment_limit");
    }
    let argument = ts[1].text().to_string();
    let id = sema.to_def(call).ok_or("declarative_expansion_unproved")?;
    let expanded = sema.expand(id);
    if expanded.err.is_some() {
        return Err("declarative_expansion_unproved");
    }
    let syntax = expanded.value;
    tokens(&syntax)?;
    // Expansion ASTs intentionally lack whitespace. Re-lexing their text would
    // merge tokens (pub + struct); inspect RA's token-aware parse errors instead.
    if id.parse_macro_expansion_error(&inputs.db).is_some() {
        return Err("declarative_unparseable");
    }
    if syntax
        .descendants()
        .any(|n| ast::MacroCall::can_cast(n.kind()) || ast::MacroRules::can_cast(n.kind()))
    {
        return Err("declarative_recursion_limit");
    }
    let mut assumed_helpers = false;
    for item in syntax.children().filter_map(ast::Item::cast) {
        match &item {
            ast::Item::Struct(s) if s.generic_param_list().is_none() => {
                assumed_helpers |= type_attributes(inputs, sema, module, &definition, &item)?;
            }
            ast::Item::Enum(e) if e.generic_param_list().is_none() => {
                assumed_helpers |= type_attributes(inputs, sema, module, &definition, &item)?;
            }
            ast::Item::Impl(_) => {}
            _ => return Err("declarative_output_unproved"),
        }
    }
    Ok(Expansion {
        invocation,
        definition,
        argument,
        syntax,
        assumed_helpers,
    })
}

pub(super) fn context_admitted(
    inputs: &Inputs,
    sema: &Semantics<'_, RootDatabase>,
    module: Module,
    call: &ast::MacroCall,
) -> bool {
    let outcome = inspect(inputs, sema, module, call);
    let assumed = outcome.as_ref().is_ok_and(|e| e.assumed_helpers);
    if assumed {
        inputs.assumed_declared_helpers.set(true);
    }
    context::record(
        inputs,
        sema,
        module,
        call.syntax(),
        FactClass::NominalIdentity,
        (
            "declarative_macro",
            if outcome.is_ok() {
                "admitted"
            } else {
                "skipped"
            },
            None,
            outcome
                .as_ref()
                .err()
                .copied()
                .unwrap_or("bounded_declarative_namespace"),
        ),
    );
    if let Some(declaration) =
        inputs.declaration(module, call.syntax(), sema.hir_file_for(call.syntax()))
    {
        let anchor = declaration.anchor;
        let key = (
            anchor.path,
            anchor.range.start_byte,
            anchor.range.end_byte,
            FactClass::NominalIdentity.name(),
        );
        if let Some(evaluation) = inputs.context.borrow_mut().get_mut(&key) {
            evaluation.basis = if assumed {
                ASSUMED_BASIS
            } else {
                CONTEXT_BASIS
            }
            .into();
            if assumed {
                evaluation.reason = "assumed_declared_helpers".into();
            }
        }
    }
    outcome.is_ok()
}

pub(super) fn declaration(
    inputs: &Inputs,
    sema: &Semantics<'_, RootDatabase>,
    adt: Adt,
) -> Option<Declaration> {
    let source = match adt {
        Adt::Struct(s) => sema.source(s)?.map(|s| s.syntax().clone()),
        Adt::Enum(e) => sema.source(e)?.map(|e| e.syntax().clone()),
        Adt::Union(_) => return None,
    };
    let depth = source.file_id.macro_expansion_depth(&inputs.db);
    if depth == 0 {
        return None;
    }
    // Audit the outer written call even when nested expansion cannot prove identity.
    let site = source.file_id.original_call_node(&inputs.db)?;
    let file = site.file_id.file_id(&inputs.db);
    let root = sema.parse_guess_edition(file);
    let call = root
        .syntax()
        .descendants()
        .filter_map(ast::MacroCall::cast)
        .find(|c| c.syntax().text_range() == site.value.text_range())?;
    let module = sema.scope(call.syntax())?.module();
    if !context_admitted(inputs, sema, module, &call) || depth != 1 {
        return None;
    }
    let expansion = inspect(inputs, sema, module, &call).ok()?;
    let mut names = expansion
        .syntax
        .children()
        .filter_map(ast::Item::cast)
        .filter_map(|item| match item {
            ast::Item::Struct(s) if sema.to_def(&s).map(Adt::Struct) == Some(adt) => s.name(),
            ast::Item::Enum(e) if sema.to_def(&e).map(Adt::Enum) == Some(adt) => e.name(),
            _ => None,
        });
    let name = names.next()?;
    if names.next().is_some() || name.text() != expansion.argument {
        return None;
    }
    let tree = call.token_tree()?;
    let ts = tokens(tree.syntax()).ok()?;
    let anchor = inputs.declaration(module, tree.syntax(), sema.hir_file_for(call.syntax()))?;
    let start = usize::from(ts[1].text_range().start());
    let end = usize::from(ts[1].text_range().end());
    // A coincidentally equal literal name in the definition is not substitution
    // evidence. Require RA's exact, non-fallback token provenance to the argument.
    let provenance = sema.original_range_opt(name.syntax())?;
    if provenance.file_id.file_id(&inputs.db) != file || provenance.range != ts[1].text_range() {
        return None;
    }
    Some(Declaration {
        crate_origin: expansion.invocation.crate_origin,
        anchor: SourceAnchor {
            path: anchor.anchor.path,
            range: super::range(start, end),
            expected_text: ts[1].text().into(),
        },
        declarative_macro: Some(Evidence {
            invocation: expansion.invocation.anchor,
            definition: expansion.definition.anchor,
        }),
    })
}
