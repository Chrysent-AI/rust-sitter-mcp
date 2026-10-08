//! Attributes that preserve ordinary file/module identity, not general item admission.
use ra_ap_syntax::{AstNode, Edition, SourceFile, ast};
use std::collections::BTreeSet;

pub(crate) type DeclaredCfg = BTreeSet<(String, Option<String>)>;

pub(crate) fn admits(text: &str, cfg: Option<&DeclaredCfg>) -> bool {
    let parsed = SourceFile::parse(&format!("{text}\nmod __chain_probe;"), Edition::CURRENT);
    if !parsed.errors().is_empty() {
        return false;
    }
    let Some(attr) = parsed
        .tree()
        .syntax()
        .descendants()
        .find_map(ast::Attr::cast)
    else {
        return false;
    };
    attr.meta().is_some_and(|meta| active(&meta, cfg, 0))
}

// Unknown conditions are harmless only when every possible payload is inert
// for this fact class. Never execute doc/include expressions or read their files.
fn inert(meta: &ast::Meta, depth: usize) -> bool {
    if depth > 32 {
        return false;
    }
    if let ast::Meta::CfgAttrMeta(attr) = meta {
        let metas: Vec<_> = attr.metas().collect();
        return attr.cfg_predicate().is_some_and(|p| {
            super::super::cfg::predicate(&p, &|_, _| true, &mut |_, _| {}, 0).is_ok()
        }) && !metas.is_empty()
            && metas.iter().all(|m| inert(m, depth + 1));
    }
    if super::super::identity_inert_metadata(meta.simple_name().as_deref()) {
        return true;
    }
    // This compiler feature only enables documentation metadata. Other feature
    // gates are deliberately not assumed to preserve the written module tree.
    meta.simple_name().as_deref() == Some("feature")
        && meta.as_simple_call().is_some_and(|(_, tree)| {
            let tokens: Vec<_> = tree.syntax().descendants_with_tokens()
                .filter_map(|n| n.into_token())
                .filter(|t| !t.kind().is_trivia())
                .map(|t| t.text().to_owned()).collect();
            matches!(tokens.as_slice(), [a, b, c] if a == "(" && b == "doc_cfg" && c == ")")
                || matches!(tokens.as_slice(), [a, b, c, d] if a == "(" && b == "doc_cfg" && c == "," && d == ")")
        })
}

fn active(meta: &ast::Meta, cfg: Option<&DeclaredCfg>, depth: usize) -> bool {
    if depth > 32 {
        return false;
    }
    if inert(meta, depth) {
        return true;
    }
    let enabled = |pred: ast::CfgPredicate| {
        super::super::cfg::predicate(
            &pred,
            &|key, value| cfg.is_some_and(|atoms| atoms.contains(&(key.into(), value.clone()))),
            &mut |_, _| {},
            0,
        )
    };
    match meta {
        ast::Meta::CfgMeta(attr) => {
            cfg.is_some() && attr.cfg_predicate().is_some_and(|p| enabled(p) == Ok(true))
        }
        ast::Meta::CfgAttrMeta(attr) => {
            let metas: Vec<_> = attr.metas().collect();
            if metas.is_empty() || cfg.is_none() {
                return false;
            }
            match attr.cfg_predicate().map(enabled) {
                Some(Ok(false)) => true,
                Some(Ok(true)) => metas.iter().all(|m| active(m, cfg, depth + 1)),
                _ => false,
            }
        }
        _ => false,
    }
}
