//! Positive-only written cfg evaluation shared by chain and resolution context gates.
use super::DeclaredCfg;
use ra_ap_syntax::{AstNode, AstToken, Edition, SourceFile, ast};

/// Admit only conditional wrappers. Other attributes retain their existing
/// written identity, expansion and prelude-control audits.
pub(crate) fn attribute(
    text: &str,
    cfg: Option<&DeclaredCfg>,
    require_active: bool,
) -> Result<(), &'static str> {
    let cfg = cfg.ok_or("declared_configuration_unproved")?;
    let parsed = SourceFile::parse(&format!("{text}\nfn __cfg_probe() {{}}"), Edition::CURRENT);
    if !parsed.errors().is_empty() {
        return Err("unparseable_attribute");
    }
    let attr = parsed
        .tree()
        .syntax()
        .descendants()
        .find_map(ast::Attr::cast)
        .ok_or("unparseable_attribute")?;
    let meta = attr.meta().ok_or("unparseable_attribute")?;
    if !matches!(meta, ast::Meta::CfgMeta(_) | ast::Meta::CfgAttrMeta(_)) {
        return Err("unsupported_attribute");
    }
    conditional_meta(&meta, cfg, require_active, 0)
}

fn conditional_meta(
    meta: &ast::Meta,
    cfg: &DeclaredCfg,
    require_active: bool,
    depth: usize,
) -> Result<(), &'static str> {
    if depth > 32 {
        return Err("attribute_depth_limit");
    }
    let enabled = |pred: ast::CfgPredicate| {
        predicate(
            &pred,
            &|key, value| cfg.contains(&(key.into(), value.clone())),
            &mut |_, _| {},
            0,
        )
    };
    match meta {
        ast::Meta::CfgMeta(attr) => {
            let active = enabled(attr.cfg_predicate().ok_or("unparseable_cfg")?)?;
            if require_active && !active {
                Err("inactive_written_binding")
            } else {
                Ok(())
            }
        }
        ast::Meta::CfgAttrMeta(attr) => {
            let active = enabled(attr.cfg_predicate().ok_or("unparseable_cfg")?)?;
            let metas: Vec<_> = attr.metas().collect();
            if metas.is_empty() {
                return Err("unparseable_cfg_attr");
            }
            if active {
                for nested in metas {
                    conditional_meta(&nested, cfg, require_active, depth + 1)?;
                }
            }
            Ok(())
        }
        // Metadata does not introduce bindings. Derives and prelude controls
        // still need the owning written/semantic fact-class audit.
        _ if super::identity_inert_metadata(meta.simple_name().as_deref())
            || matches!(
                meta.simple_name().as_deref(),
                Some("inline" | "cold" | "must_use")
            ) =>
        {
            Ok(())
        }
        _ => Err("unsupported_attribute"),
    }
}

/// All leading outer attributes must preserve an active written declaration.
/// Comments do not detach an attribute from its owner.
pub(crate) fn attributed(
    node: tree_sitter::Node<'_>,
    source: &str,
    cfg: Option<&DeclaredCfg>,
) -> bool {
    let mut previous = node.prev_named_sibling();
    while let Some(attr) = previous {
        match attr.kind() {
            "attribute_item"
                if cfg.is_none()
                    || (!super::context_independent_attribute(&source[attr.byte_range()])
                        && attribute(&source[attr.byte_range()], cfg, true).is_err()) =>
            {
                return true;
            }
            "attribute_item" | "line_comment" | "block_comment" => {}
            _ => break,
        }
        previous = attr.prev_named_sibling();
    }
    false
}

pub(crate) fn predicate(
    pred: &ast::CfgPredicate,
    atom: &impl Fn(&str, &Option<String>) -> bool,
    observe: &mut impl FnMut(&ast::CfgPredicate, Result<bool, &'static str>),
    depth: usize,
) -> Result<bool, &'static str> {
    let outcome = (|| {
        if depth > 32 {
            return Err("cfg_depth_limit");
        }
        match pred {
            ast::CfgPredicate::CfgAtom(value) => {
                if value.true_token().is_some() {
                    return Ok(true);
                }
                if value.false_token().is_some() {
                    return Ok(false);
                }
                let key = value.ident_token().ok_or("unparseable_cfg")?;
                let key = key.text().trim_start_matches("r#");
                let value = if value.eq_token().is_some() {
                    Some(
                        ast::String::cast(value.string_token().ok_or("unparseable_cfg")?)
                            .ok_or("unparseable_cfg")?
                            .value()
                            .map_err(|_| "unparseable_cfg")?
                            .into_owned(),
                    )
                } else {
                    None
                };
                if atom(key, &value) {
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
                // Evaluate every operand: short-circuiting cannot hide unknown atoms.
                let values: Vec<_> = composite
                    .cfg_predicates()
                    .map(|p| predicate(&p, atom, observe, depth + 1))
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
    observe(pred, outcome);
    outcome
}
