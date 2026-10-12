//! Origin-scoped installed-source context admission. No caller/external waiver.
use super::{
    Inputs,
    context::{self, FactClass},
};
use ra_ap_hir::{MacroKind, Module, Semantics};
use ra_ap_ide_db::RootDatabase;
use ra_ap_syntax::{
    AstNode, AstToken, SyntaxNode,
    ast::{self, HasAttrs, HasName},
};

pub(super) fn crate_name<'a>(
    inputs: &'a Inputs,
    sema: &Semantics<'_, RootDatabase>,
    module: Module,
    node: &SyntaxNode,
) -> Option<&'a str> {
    let file = sema.hir_file_for(node).file_id()?.file_id(&inputs.db);
    if !inputs
        .paths
        .get(&file)?
        .starts_with(super::super::sysroot::PREFIX)
    {
        return None;
    }
    let name = inputs
        .roots
        .get(&module.krate(&inputs.db).root_file(&inputs.db))?;
    inputs
        .sysroot
        .as_ref()?
        .origins
        .contains_key(name)
        .then_some(name.as_str())
}
// Compiler-owned metadata/codegen controls are finite, never a rustc_* prefix.
const METADATA: &[&str] = &[
    "stable",
    "unstable",
    "rustc_const_stable",
    "rustc_const_unstable",
    "rustc_diagnostic_item",
    "rustc_conversion_suggestion",
    "rustc_confusables",
    "rustc_allow_const_fn_unstable",
    "track_caller",
];
fn inert_name(name: &str) -> bool {
    crate::items::identity_inert_metadata(Some(name))
        || matches!(name, "inline" | "cold" | "must_use" | "no_std" | "no_core")
        || METADATA.contains(&name)
}
pub(super) fn metadata(
    inputs: &Inputs,
    sema: &Semantics<'_, RootDatabase>,
    module: Module,
    meta: &ast::Meta,
) -> Option<Result<(), &'static str>> {
    let crate_name = crate_name(inputs, sema, module, meta.syntax())?;
    let name = meta.simple_name()?;
    let origin = &inputs.sysroot.as_ref()?.origins[crate_name];
    let outcome = if name == "derive" {
        match sema.resolve_derive_macro(meta) {
            Some(providers)
                if !providers.is_empty()
                    && providers.iter().all(|provider| {
                        provider
                            .as_ref()
                            .is_some_and(|m| m.builtin_derive_kind(&inputs.db).is_some())
                    }) =>
            {
                Ok(())
            }
            _ => Err("sysroot_derive_provider_unproved"),
        }
    } else if name == "feature" || name == "needs_allocator" || name == "rustc_preserve_ub_checks" {
        let root_attribute = meta
            .parent_attr()
            .and_then(|attr| attr.syntax().parent())
            .is_some_and(|node| ast::SourceFile::can_cast(node.kind()));
        if root_attribute
            && (name != "rustc_preserve_ub_checks" || matches!(origin.as_str(), "core" | "alloc"))
        {
            Ok(())
        } else {
            Err("sysroot_root_provider_unproved")
        }
    } else if name == "lang" {
        let attr = meta.parent_attr()?;
        let owner = attr.syntax().parent().and_then(ast::Struct::cast);
        let exact = owner
            .as_ref()
            .is_some_and(|s| s.name().is_some_and(|n| n.text() == "String"))
            && origin == "alloc"
            && sema
                .hir_file_for(meta.syntax())
                .file_id()
                .is_some_and(|file| {
                    inputs
                        .paths
                        .get(&file.file_id(&inputs.db))
                        .is_some_and(|p| p == "@sysroot/0/library/alloc/src/string.rs")
                })
            && meta
                .syntax()
                .descendants_with_tokens()
                .filter_map(|node| node.into_token())
                .filter_map(ast::String::cast)
                .any(|string| string.value().is_ok_and(|value| value == "String"));
        if exact {
            Ok(())
        } else {
            Err("sysroot_lang_identity_unproved")
        }
    } else if inert_name(&name) {
        Ok(())
    } else {
        Err("unsupported_attribute")
    };
    // A same-named procedural attribute provider is not compiler metadata.
    if outcome.is_ok() && name != "derive" {
        let mut replacing = false;
        if let Some(scope) = sema.scope(meta.syntax()) {
            scope.process_all_names(&mut |binding, definition| {
                if binding.as_str() == name.as_str() && matches!(definition, ra_ap_hir::ScopeDef::ModuleDef(ra_ap_hir::ModuleDef::Macro(m)) if m.is_attr(&inputs.db)) { replacing = true; }
            });
        }
        if replacing {
            return Some(Err("sysroot_attribute_provider_unproved"));
        }
    }
    Some(outcome)
}
fn tokens(node: &SyntaxNode) -> Result<(), &'static str> {
    let mut count = 0;
    let mut depth = 0usize;
    for token in node
        .descendants_with_tokens()
        .filter_map(|node| node.into_token())
        .filter(|token| !token.kind().is_trivia())
    {
        count += 1;
        if count > 4096 {
            return Err("sysroot_macro_token_limit");
        }
        match token.text() {
            "(" | "[" | "{" => {
                depth += 1;
                if depth > 32 {
                    return Err("sysroot_macro_depth_limit");
                }
            }
            ")" | "]" | "}" => {
                depth = depth.checked_sub(1).ok_or("sysroot_macro_unparseable")?;
            }
            _ => {}
        }
    }
    if depth != 0 {
        return Err("sysroot_macro_unparseable");
    }
    Ok(())
}
fn output_attr(
    inputs: &Inputs,
    crate_name: &str,
    meta: &ast::Meta,
    depth: usize,
) -> Result<(), &'static str> {
    if depth > 32 {
        return Err("sysroot_macro_depth_limit");
    }
    let profile = inputs
        .sysroot
        .as_ref()
        .ok_or("sysroot_profile_unavailable")?;
    let value = |pred: &ast::CfgPredicate| {
        super::super::sysroot::predicate(
            pred,
            &|key, value| profile.value(crate_name, key, value),
            &mut |_, _| {},
            0,
        )
    };
    match meta {
        ast::Meta::CfgMeta(cfg) => {
            value(&cfg.cfg_predicate().ok_or("unparseable_cfg")?)?;
            Ok(())
        }
        ast::Meta::CfgAttrMeta(cfg) => {
            if value(&cfg.cfg_predicate().ok_or("unparseable_cfg")?)? {
                for meta in cfg.metas() {
                    output_attr(inputs, crate_name, &meta, depth + 1)?;
                }
            }
            Ok(())
        }
        _ if meta.simple_name().is_some_and(|name| inert_name(&name)) => Ok(()),
        _ => Err("sysroot_macro_attribute_unproved"),
    }
}
fn inspect_macro(
    inputs: &Inputs,
    sema: &Semantics<'_, RootDatabase>,
    module: Module,
    call: &ast::MacroCall,
    fact_class: FactClass,
) -> Result<(), &'static str> {
    let crate_name =
        crate_name(inputs, sema, module, call.syntax()).ok_or("sysroot_origin_unproved")?;
    let provider = sema
        .resolve_macro_call(call)
        .ok_or("sysroot_macro_provider_unproved")?;
    if provider.is_proc_macro()
        || provider.kind(&inputs.db) != MacroKind::Declarative
        || provider.module(&inputs.db).krate(&inputs.db) != module.krate(&inputs.db)
    {
        return Err("sysroot_macro_provider_unproved");
    }
    let source = sema
        .source(provider)
        .ok_or("sysroot_macro_definition_unproved")?;
    let Some(ast::Macro::MacroRules(definition)) = source.value.left() else {
        return Err("sysroot_macro_definition_unproved");
    };
    let declaration = inputs
        .declaration(
            provider.module(&inputs.db),
            definition.syntax(),
            source.file_id,
        )
        .ok_or("sysroot_macro_definition_unproved")?;
    if !declaration
        .anchor
        .path
        .starts_with(super::super::sysroot::PREFIX)
    {
        return Err("sysroot_macro_origin_unproved");
    }
    tokens(definition.syntax())?;
    tokens(call.syntax())?;
    for attr in definition.attrs() {
        if !context::safe_attr(inputs, sema, provider.module(&inputs.db), &attr, fact_class) {
            return Err("sysroot_macro_attribute_unproved");
        }
    }
    for attr in call.attrs() {
        if !context::safe_attr(inputs, sema, module, &attr, fact_class) {
            return Err("sysroot_macro_attribute_unproved");
        }
    }
    let id = sema
        .to_def(call)
        .ok_or("sysroot_macro_expansion_unproved")?;
    let expanded = sema.expand(id);
    if expanded.err.is_some() || id.parse_macro_expansion_error(&inputs.db).is_some() {
        return Err("sysroot_macro_expansion_unproved");
    }
    tokens(&expanded.value)?;
    let mut count = 0;
    for item in expanded.value.children().filter_map(ast::Item::cast) {
        let ast::Item::Impl(implementation) = item else {
            return Err("sysroot_macro_namespace_unproved");
        };
        if implementation.trait_().is_none() || implementation.self_ty().is_none() {
            return Err("sysroot_macro_inherent_output");
        }
        for attr in implementation.attrs() {
            output_attr(
                inputs,
                crate_name,
                &attr.meta().ok_or("sysroot_macro_unparseable")?,
                0,
            )?;
        }
        for item in implementation
            .assoc_item_list()
            .ok_or("sysroot_macro_unparseable")?
            .assoc_items()
        {
            if !matches!(
                item,
                ast::AssocItem::Fn(_) | ast::AssocItem::Const(_) | ast::AssocItem::TypeAlias(_)
            ) {
                return Err("sysroot_macro_namespace_unproved");
            }
            for attr in item.attrs() {
                output_attr(
                    inputs,
                    crate_name,
                    &attr.meta().ok_or("sysroot_macro_unparseable")?,
                    0,
                )?;
            }
        }
        count += 1;
    }
    if count == 0 {
        return Err("sysroot_macro_namespace_unproved");
    }
    context::record(
        inputs,
        sema,
        provider.module(&inputs.db),
        definition.syntax(),
        fact_class,
        (
            "sysroot_macro_definition",
            "admitted",
            None,
            "bounded_trait_impl_only_output",
        ),
    );
    Ok(())
}
pub(super) fn module_macro(
    inputs: &Inputs,
    sema: &Semantics<'_, RootDatabase>,
    module: Module,
    call: &ast::MacroCall,
    fact_class: FactClass,
) -> bool {
    if crate_name(inputs, sema, module, call.syntax()).is_none() {
        return false;
    }
    let outcome = inspect_macro(inputs, sema, module, call, fact_class);
    context::record(
        inputs,
        sema,
        module,
        call.syntax(),
        fact_class,
        (
            "sysroot_module_macro",
            if outcome.is_ok() {
                "admitted"
            } else {
                "skipped"
            },
            None,
            outcome.err().unwrap_or("bounded_trait_impl_only_output"),
        ),
    );
    outcome.is_ok()
}
