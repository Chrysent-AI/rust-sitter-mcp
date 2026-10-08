//! Positive-only written cfg evaluation shared by chain and resolution context gates.
use ra_ap_syntax::{AstToken, ast};

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
