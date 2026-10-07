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

pub(super) const BASIS: &str = "declaration identity established through bounded declarative-macro expansion of written tokens; original/final invocation and in-crate macro_rules definition anchors match; no generated impl, method, field, constructor or variant facts; no proc-macro execution, compilation or equivalence checking";
const CONTEXT_BASIS: &str = "bounded declarative-macro written-token admission only; declaration identity additionally requires matching original/final invocation and definition anchors; generated members and proc macros are not admitted";
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

fn unconditional(node: &SyntaxNode) -> Result<(), &'static str> {
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
            if !matches!(
                attr.simple_name().as_deref(),
                Some("allow" | "warn" | "deny" | "forbid" | "doc" | "macro_export")
            ) {
                return Err("declarative_attribute_unproved");
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
            unconditional(declaration.value.syntax())?;
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
        unconditional(sema.parse_guess_edition(file).syntax())?;
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

/// Generated attributes can rewrite the declaration. Only inert metadata and
/// additive derives qualify; the exact transparent serde helper is metadata.
fn type_attributes(item: &ast::Item) -> bool {
    let attrs: Vec<_> = item.attrs().collect();
    let tree = |a: &ast::Attr| a.meta()?.as_simple_call().map(|(_, t)| t);
    let has_serde_derive = attrs
        .iter()
        .filter(|a| a.simple_name().as_deref() == Some("derive"))
        .filter_map(tree)
        .any(|t| {
            tokens(t.syntax()).is_ok_and(|ts| {
                ts.iter()
                    .any(|t| matches!(t.text(), "Serialize" | "Deserialize"))
            })
        });
    attrs.iter().all(|a| match a.simple_name().as_deref() {
        Some("allow" | "warn" | "deny" | "forbid" | "doc") => true,
        Some("derive") => tree(a)
            .and_then(|t| tokens(t.syntax()).ok())
            .is_some_and(|ts| {
                ts.len() > 2 && super::context::nominal_derive_paths(&ts[1..ts.len() - 1]).is_ok()
            }),
        Some("serde") if has_serde_derive => tree(a)
            .and_then(|t| tokens(t.syntax()).ok())
            .is_some_and(|ts| {
                ts.iter().map(|t| t.text()).collect::<Vec<_>>() == ["(", "transparent", ")"]
            }),
        _ => false,
    })
}

fn inspect(
    inputs: &Inputs,
    sema: &Semantics<'_, RootDatabase>,
    module: Module,
    call: &ast::MacroCall,
) -> Result<Expansion, &'static str> {
    unconditional(call.syntax())?;
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
    let definition_admission = unconditional(def.syntax())
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
    for item in syntax.children().filter_map(ast::Item::cast) {
        match &item {
            ast::Item::Struct(s) if s.generic_param_list().is_none() && type_attributes(&item) => {}
            ast::Item::Enum(e) if e.generic_param_list().is_none() && type_attributes(&item) => {}
            ast::Item::Impl(_) => {}
            _ => return Err("declarative_output_unproved"),
        }
    }
    Ok(Expansion {
        invocation,
        definition,
        argument,
        syntax,
    })
}

pub(super) fn context_admitted(
    inputs: &Inputs,
    sema: &Semantics<'_, RootDatabase>,
    module: Module,
    call: &ast::MacroCall,
) -> bool {
    let outcome = inspect(inputs, sema, module, call);
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
            evaluation.basis = CONTEXT_BASIS.into();
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
    if source.file_id.macro_expansion_depth(&inputs.db) != 1 {
        return None;
    }
    let site = source.file_id.call_node(&inputs.db)?;
    let file = site.file_id.file_id()?.file_id(&inputs.db);
    let root = sema.parse_guess_edition(file);
    let call = root
        .syntax()
        .descendants()
        .filter_map(ast::MacroCall::cast)
        .find(|c| c.syntax().text_range() == site.value.text_range())?;
    let module = sema.scope(call.syntax())?.module();
    if !context_admitted(inputs, sema, module, &call) {
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
