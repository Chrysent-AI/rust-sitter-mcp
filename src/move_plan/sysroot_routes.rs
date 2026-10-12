//! Bounded literal module/include routes inside the one captured virtual root.
use super::{Controls, Coverage, PREFIX, graph::Profile, refuse};
use crate::{items, result::DomainError};
use ra_ap_syntax::{
    AstNode, AstToken, Edition, SyntaxNode,
    ast::{self, HasAttrs, HasName},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Component, Path},
};

pub(crate) fn predicate(
    pred: &ast::CfgPredicate,
    atom: &impl Fn(&str, &Option<String>) -> Option<bool>,
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
                atom(key.text().trim_start_matches("r#"), &value).ok_or("undeclared_cfg_atom")
            }
            ast::CfgPredicate::CfgComposite(composite) => {
                let keyword = composite.keyword().ok_or("unparseable_cfg")?;
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
fn atom(pred: &ast::CfgAtom) -> Option<(String, Option<String>)> {
    let key = pred
        .ident_token()?
        .text()
        .trim_start_matches("r#")
        .to_owned();
    let value = if pred.eq_token().is_some() {
        Some(
            ast::String::cast(pred.string_token()?)?
                .value()
                .ok()?
                .into_owned(),
        )
    } else {
        None
    };
    Some((key, value))
}
fn normalize(base: &str, relative: &str) -> Result<String, DomainError> {
    let path = Path::new(relative);
    if path.is_absolute() {
        return Err(refuse(
            "SYSROOT_LAYOUT_UNSUPPORTED",
            "literal route is absolute",
        ));
    }
    let mut components: Vec<_> = base
        .split('/')
        .filter(|part| !part.is_empty())
        .map(str::to_owned)
        .collect();
    for component in path.components() {
        match component {
            Component::Normal(s) => components.push(
                s.to_str()
                    .ok_or_else(|| {
                        refuse("SYSROOT_LAYOUT_UNSUPPORTED", "literal route is not UTF-8")
                    })?
                    .into(),
            ),
            Component::CurDir => {}
            Component::ParentDir if components.len() > 3 => {
                components.pop();
            }
            _ => {
                return Err(refuse(
                    "SYSROOT_LAYOUT_UNSUPPORTED",
                    "literal route escapes captured library",
                ));
            }
        }
    }
    let path = components.join("/");
    if !path.starts_with(PREFIX) {
        return Err(refuse(
            "SYSROOT_LAYOUT_UNSUPPORTED",
            "literal route escapes captured library",
        ));
    }
    Ok(path)
}
fn cfg_active(
    meta: &ast::Meta,
    profile: &Profile,
    crate_name: &str,
    depth: usize,
) -> Result<bool, DomainError> {
    if depth > 32 {
        return Err(refuse(
            "SYSROOT_CAPTURE_EXHAUSTED",
            "route attribute nesting cap exhausted",
        ));
    }
    let value = |p: &ast::CfgPredicate| {
        predicate(
            p,
            &|key, value| profile.value(crate_name, key, value),
            &mut |_, _| {},
            0,
        )
        .map_err(|reason| {
            refuse(
                "SYSROOT_CONFIGURATION_UNSUPPORTED",
                &format!("{reason}: {crate_name}: {p}"),
            )
        })
    };
    match meta {
        ast::Meta::CfgMeta(cfg) => {
            value(&cfg.cfg_predicate().ok_or_else(|| {
                refuse("SYSROOT_CONFIGURATION_UNSUPPORTED", "malformed route cfg")
            })?)
        }
        ast::Meta::CfgAttrMeta(cfg) => {
            if value(&cfg.cfg_predicate().ok_or_else(|| {
                refuse(
                    "SYSROOT_CONFIGURATION_UNSUPPORTED",
                    "malformed route cfg_attr",
                )
            })?)? {
                let values: Vec<_> = cfg
                    .metas()
                    .map(|m| cfg_active(&m, profile, crate_name, depth + 1))
                    .collect();
                Ok(values
                    .into_iter()
                    .collect::<Result<Vec<_>, _>>()?
                    .into_iter()
                    .all(|value| value))
            } else {
                Ok(true)
            }
        }
        _ => Ok(true),
    }
}
fn active(node: &SyntaxNode, profile: &Profile, crate_name: &str) -> Result<bool, DomainError> {
    let mut enabled = true;
    for ancestor in node.ancestors() {
        let attrs: Vec<_> = if let Some(item) = ast::Item::cast(ancestor.clone()) {
            item.attrs().collect()
        } else if let Some(file) = ast::SourceFile::cast(ancestor) {
            file.attrs().collect()
        } else {
            Vec::new()
        };
        for attr in attrs {
            enabled &= cfg_active(
                &attr.meta().ok_or_else(|| {
                    refuse(
                        "SYSROOT_CONFIGURATION_UNSUPPORTED",
                        "unparseable route attribute",
                    )
                })?,
                profile,
                crate_name,
                0,
            )?;
        }
    }
    Ok(enabled)
}
fn override_path(
    meta: &ast::Meta,
    profile: &Profile,
    crate_name: &str,
    depth: usize,
    paths: &mut Vec<String>,
) -> Result<(), DomainError> {
    if depth > 32 {
        return Err(refuse(
            "SYSROOT_CAPTURE_EXHAUSTED",
            "route attribute nesting cap exhausted",
        ));
    }
    if let ast::Meta::CfgAttrMeta(cfg) = meta {
        let condition = predicate(
            &cfg.cfg_predicate().ok_or_else(|| {
                refuse(
                    "SYSROOT_CONFIGURATION_UNSUPPORTED",
                    "invalid path condition",
                )
            })?,
            &|key, value| profile.value(crate_name, key, value),
            &mut |_, _| {},
            0,
        )
        .map_err(|reason| refuse("SYSROOT_CONFIGURATION_UNSUPPORTED", reason))?;
        if condition {
            for meta in cfg.metas() {
                override_path(&meta, profile, crate_name, depth + 1, paths)?;
            }
        }
    } else if meta.simple_name().as_deref() == Some("path") {
        let ast::Meta::KeyValueMeta(meta) = meta else {
            return Err(refuse(
                "SYSROOT_LAYOUT_UNSUPPORTED",
                "path route must be a literal string",
            ));
        };
        let ast::Expr::Literal(literal) = meta
            .expr()
            .ok_or_else(|| refuse("SYSROOT_LAYOUT_UNSUPPORTED", "missing path literal"))?
        else {
            return Err(refuse("SYSROOT_LAYOUT_UNSUPPORTED", "dynamic module route"));
        };
        let ast::LiteralKind::String(string) = literal.kind() else {
            return Err(refuse(
                "SYSROOT_LAYOUT_UNSUPPORTED",
                "non-string module route",
            ));
        };
        paths.push(
            string
                .value()
                .map_err(|_| refuse("SYSROOT_LAYOUT_UNSUPPORTED", "invalid module literal"))?
                .into_owned(),
        );
    }
    Ok(())
}
struct Audit<'a> {
    texts: &'a BTreeMap<String, String>,
    profile: &'a Profile,
    coverage: &'a mut Coverage,
    controls: Controls<'a>,
    seen: BTreeSet<(String, String)>,
    pending: BTreeSet<(String, String)>,
}
impl Audit<'_> {
    fn route(&mut self) -> Result<(), DomainError> {
        self.coverage.capture_usage.route_edges += 1;
        if self.coverage.capture_usage.route_edges > self.coverage.capture_usage.max_route_edges {
            return Err(refuse(
                "SYSROOT_CAPTURE_EXHAUSTED",
                "literal route-edge cap exhausted",
            ));
        }
        Ok(())
    }
    fn file(
        &mut self,
        path: &str,
        crate_name: &str,
        root: bool,
        depth: usize,
    ) -> Result<(), DomainError> {
        items::check(self.controls.0, self.controls.1)?;
        if depth > 32 {
            return Err(refuse(
                "SYSROOT_CAPTURE_EXHAUSTED",
                "literal route depth cap exhausted",
            ));
        }
        let key = (path.to_owned(), crate_name.to_owned());
        if self.pending.contains(&key) {
            return Err(refuse(
                "SYSROOT_LAYOUT_UNSUPPORTED",
                "literal source route cycle",
            ));
        }
        if !self.seen.insert(key.clone()) {
            return Ok(());
        }
        self.pending.insert(key.clone());
        let text = self.texts.get(path).ok_or_else(|| {
            refuse(
                "SYSROOT_LAYOUT_UNSUPPORTED",
                &format!("unresolved literal route: {path}"),
            )
        })?;
        let edition = self
            .profile
            .configuration
            .crates
            .iter()
            .find(|c| c.name == crate_name)
            .expect("graph root")
            .edition
            .parse::<Edition>()
            .expect("profile edition");
        let parsed = ra_ap_syntax::SourceFile::parse(text, edition);
        // The pinned parser does not recognize `const move` closures. Admit only
        // that exact diagnostic inside a written function body for route walking;
        // declaration/receiver proof contexts still reject any file recovery.
        for error in parsed.errors() {
            let at = usize::from(error.range().start());
            let const_move = error.to_string() == "expected fn, trait or impl"
                && text.get(at.saturating_sub(5)..at) == Some("const")
                && text
                    .get(at..)
                    .is_some_and(|tail| tail.starts_with(" move |"))
                && parsed
                    .tree()
                    .syntax()
                    .descendants()
                    .filter_map(ast::Fn::cast)
                    .filter_map(|function| function.body())
                    .any(|body| body.syntax().text_range().contains_range(error.range()));
            if !const_move {
                return Err(refuse(
                    "SYSROOT_LAYOUT_UNSUPPORTED",
                    &format!("source route syntax recovery: {path}: {error}"),
                ));
            }
            self.coverage.omissions.push(format!("route-only const-move parser diagnostic at {path}:{at}; recovered files cannot supply declaration/receiver facts"));
        }
        let directory = path.rsplit_once('/').expect("virtual source path").0;
        let stem = Path::new(path)
            .file_stem()
            .and_then(|s| s.to_str())
            .expect("UTF-8 source");
        let base = if root || stem == "mod" {
            directory.to_owned()
        } else {
            format!("{directory}/{stem}")
        };
        self.modules(
            parsed.tree().syntax(),
            &base,
            directory,
            crate_name,
            depth,
            !root && stem != "mod",
        )?;
        self.pending.remove(&key);
        Ok(())
    }
    fn modules(
        &mut self,
        node: &SyntaxNode,
        base: &str,
        directory: &str,
        crate_name: &str,
        depth: usize,
        non_dir_owner: bool,
    ) -> Result<(), DomainError> {
        for child in node.children() {
            items::check(self.controls.0, self.controls.1)?;
            if let Some(module) = ast::Module::cast(child.clone()) {
                if !active(module.syntax(), self.profile, crate_name)? {
                    continue;
                }
                let name = module
                    .name()
                    .ok_or_else(|| refuse("SYSROOT_LAYOUT_UNSUPPORTED", "nameless source module"))?
                    .text()
                    .to_string();
                let mut paths = Vec::new();
                for attr in module.attrs() {
                    if let Some(meta) = attr.meta() {
                        override_path(&meta, self.profile, crate_name, 0, &mut paths)?;
                    }
                }
                if paths.len() > 1 {
                    return Err(refuse(
                        "SYSROOT_LAYOUT_UNSUPPORTED",
                        "competing path attributes",
                    ));
                }
                if let Some(inline) = module.item_list() {
                    let next = if let Some(path) = paths.first() {
                        normalize(if non_dir_owner { directory } else { base }, path)?
                    } else {
                        normalize(base, name.trim_start_matches("r#"))?
                    };
                    self.modules(
                        inline.syntax(),
                        &next,
                        directory,
                        crate_name,
                        depth + 1,
                        false,
                    )?;
                } else {
                    let path = if let Some(path) = paths.first() {
                        normalize(if non_dir_owner { directory } else { base }, path)?
                    } else {
                        let flat = normalize(base, &format!("{name}.rs"))?;
                        let nested = normalize(base, &format!("{name}/mod.rs"))?;
                        match (
                            self.texts.contains_key(&flat),
                            self.texts.contains_key(&nested),
                        ) {
                            (true, false) => flat,
                            (false, true) => nested,
                            _ => {
                                return Err(refuse(
                                    "SYSROOT_LAYOUT_UNSUPPORTED",
                                    &format!("unresolved/competing module route: {base}/{name}"),
                                ));
                            }
                        }
                    };
                    self.route()?;
                    self.file(&path, crate_name, !paths.is_empty(), depth + 1)?;
                }
            } else if let Some(call) = ast::MacroCall::cast(child.clone()) {
                let name = call.path().map(|p| p.to_string()).unwrap_or_default();
                if matches!(name.as_str(), "include" | "include_str" | "include_bytes")
                    && active(call.syntax(), self.profile, crate_name)?
                {
                    let tree = call.token_tree().ok_or_else(|| {
                        refuse("SYSROOT_LAYOUT_UNSUPPORTED", "invalid source include")
                    })?;
                    let tokens: Vec<_> = tree
                        .syntax()
                        .descendants_with_tokens()
                        .filter_map(|n| n.into_token())
                        .filter(|t| !t.kind().is_trivia())
                        .collect();
                    if tokens.len() != 3 {
                        return Err(refuse(
                            "SYSROOT_LAYOUT_UNSUPPORTED",
                            "dynamic source include is not admitted",
                        ));
                    }
                    let string = ast::String::cast(tokens[1].clone()).ok_or_else(|| {
                        refuse(
                            "SYSROOT_LAYOUT_UNSUPPORTED",
                            "include must have one literal string",
                        )
                    })?;
                    let path = normalize(
                        directory,
                        &string.value().map_err(|_| {
                            refuse("SYSROOT_LAYOUT_UNSUPPORTED", "invalid include literal")
                        })?,
                    )?;
                    if !self.texts.contains_key(&path) {
                        return Err(refuse(
                            "SYSROOT_LAYOUT_UNSUPPORTED",
                            "unresolved or binary-only source include",
                        ));
                    }
                    self.route()?;
                    if name == "include" {
                        self.file(&path, crate_name, false, depth + 1)?;
                    }
                }
            } else if !ast::MacroRules::can_cast(child.kind())
                && !ast::MacroDef::can_cast(child.kind())
            {
                self.modules(&child, base, directory, crate_name, depth, non_dir_owner)?;
            }
        }
        Ok(())
    }
}
pub(super) fn audit(
    texts: &BTreeMap<String, String>,
    profile: &mut Profile,
    coverage: &mut Coverage,
    controls: Controls<'_>,
) -> Result<(), DomainError> {
    // Every identification-domain atom is individually named, even when OFF.
    // Token-tree generated routes never supply a written declaration proof.
    for (path, text) in texts.iter().filter(|(path, _)| path.ends_with(".rs")) {
        items::check(controls.0, controls.1)?;
        let parsed = ra_ap_syntax::SourceFile::parse(text, Edition::Edition2024);
        let roots: Vec<_> = coverage
            .consulted_roots
            .iter()
            .filter(|r| {
                let directory = r.manifest.rsplit_once('/').expect("manifest path").0;
                path.starts_with(&format!("{directory}/"))
                    || r.language_origin == "core"
                    || r.language_origin == "std"
            })
            .map(|r| r.name.clone())
            .collect();
        for pred in parsed
            .tree()
            .syntax()
            .descendants()
            .filter_map(ast::CfgAtom::cast)
        {
            if let Some((key, value)) = atom(&pred) {
                for root in &roots {
                    profile.observe_identification_atom(coverage, root, &key, value.clone())?;
                }
            }
        }
    }
    let seeds: Vec<_> = coverage
        .consulted_roots
        .iter()
        .map(|r| (r.root_file.clone(), r.name.clone()))
        .collect();
    let mut audit = Audit {
        texts,
        profile,
        coverage,
        controls,
        seen: BTreeSet::new(),
        pending: BTreeSet::new(),
    };
    for (path, name) in seeds {
        audit.file(&path, &name, true, 0)?;
    }
    Ok(())
}
