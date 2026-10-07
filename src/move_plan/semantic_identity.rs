//! Written bindings that additive derive output cannot silently replace.
use super::{Inputs, context::FactClass, scoped_context};
use ra_ap_hir::{Adt, HasVisibility, Module, ModuleDef, PathResolution, Semantics};
use ra_ap_ide_db::RootDatabase;
use ra_ap_syntax::{
    AstNode, SyntaxNode,
    ast::{self, HasAttrs, HasName},
};

/// Every named module/enum prefix must have a written declaration or explicit
/// import, including the source of a re-export. RA identity alone is insufficient:
/// generated explicit imports can override names supplied by written globs.
/// Configured extern-prelude roots retain the stricter generated-item gate.
pub(super) fn stable_path(
    inputs: &Inputs,
    sema: &Semantics<'_, RootDatabase>,
    path: &ast::Path,
    depth: usize,
    fact_class: &mut FactClass,
) -> Option<()> {
    if depth > 32 {
        return None;
    }
    let PathResolution::Def(definition) = sema.resolve_path(path)? else {
        return None;
    };
    if !definition.is_visible_from(&inputs.db, sema.scope(path.syntax())?.module()) {
        return None;
    }
    let segment = path.segment()?;
    let qualifier = qualifier(path);
    if let Some(prefix) = &qualifier {
        stable_path(inputs, sema, prefix, depth + 1, fact_class)?;
    }
    match segment.kind()? {
        ast::PathSegmentKind::CrateKw
        | ast::PathSegmentKind::SelfKw
        | ast::PathSegmentKind::SuperKw => {
            return matches!(definition, ModuleDef::Module(_)).then_some(());
        }
        ast::PathSegmentKind::Name(_) => {}
        _ => return None,
    }
    let name = segment.name_ref()?.text().to_string();
    if let Some(prefix) = qualifier {
        match sema.resolve_path(&prefix)? {
            PathResolution::Def(ModuleDef::Module(module)) => {
                return stable_binding(
                    inputs,
                    sema,
                    &module_syntax(inputs, sema, module)?,
                    &name,
                    definition,
                    depth,
                    fact_class,
                );
            }
            PathResolution::Def(ModuleDef::Adt(Adt::Enum(parent))) => {
                // Variant segments are members of the already-stable named enum,
                // not names looked up through the surrounding module's globs.
                return matches!(definition, ModuleDef::EnumVariant(v) if v.parent_enum(&inputs.db) == parent).then_some(());
            }
            _ => return None,
        }
    }
    // Absolute external/prelude roots have no admitted written binding here.
    if path.coloncolon_token().is_some() {
        return None;
    }
    for scope in path.syntax().ancestors() {
        if ast::Module::can_cast(scope.kind()) {
            break;
        }
        if ast::StmtList::can_cast(scope.kind()) {
            if has_binding(&scope, &name) {
                return stable_binding(inputs, sema, &scope, &name, definition, depth, fact_class);
            }
            // Do not borrow an outer explicit binding across a nearer lexical glob.
            if scope.children().filter_map(ast::Use::cast).any(|u| {
                u.syntax()
                    .descendants()
                    .filter_map(ast::UseTree::cast)
                    .any(|t| t.star_token().is_some())
            }) {
                return None;
            }
        }
    }
    let module = sema.scope(path.syntax())?.module();
    let scope = module_syntax(inputs, sema, module)?;
    if has_binding(&scope, &name) || scope.children().any(|n| ast::MacroCall::can_cast(n.kind())) {
        return stable_binding(inputs, sema, &scope, &name, definition, depth, fact_class);
    }
    // Preserve the existing configured-dependency route, but never extend custom
    // derive admission to its extern-prelude root. It is not a written binding,
    // and a written glob could instead supply the same module name.
    if scope.children().filter_map(ast::Use::cast).any(|u| {
        u.syntax()
            .descendants()
            .filter_map(ast::UseTree::cast)
            .any(|t| t.star_token().is_some())
    }) {
        return None;
    }
    let ModuleDef::Module(dependency) = definition else {
        return None;
    };
    if dependency != dependency.krate(&inputs.db).root_module(&inputs.db) {
        return None;
    }
    let root = module.krate(&inputs.db).root_file(&inputs.db);
    let config = inputs
        .configuration
        .crates
        .iter()
        .find(|c| inputs.ids.get(&c.root_file) == Some(&root))?;
    let origin = inputs
        .roots
        .get(&dependency.krate(&inputs.db).root_file(&inputs.db))?;
    if !config
        .dependencies
        .iter()
        .any(|d| same_name(&d.name, &name) && &d.crate_name == origin)
    {
        return None;
    }
    scoped_context(inputs, sema, path.syntax(), FactClass::GeneratedItems)?;
    *fact_class = FactClass::GeneratedItems;
    Some(())
}

/// Failed RA resolution can be caused by an over-budget expansion with no ADT.
/// Follow only written named routes to disclose the reached macro veto; this
/// records evidence and never supplies a missing identity or follows a glob.
pub(super) fn disclose_missing_path(
    inputs: &Inputs,
    sema: &Semantics<'_, RootDatabase>,
    path: &ast::Path,
    depth: usize,
) {
    if depth > 32 {
        return;
    }
    let module = if let Some(prefix) = qualifier(path) {
        match sema.resolve_path(&prefix) {
            Some(PathResolution::Def(ModuleDef::Module(m))) => m,
            _ => return,
        }
    } else if let Some(scope) = sema.scope(path.syntax()) {
        scope.module()
    } else {
        return;
    };
    let Some(scope) = module_syntax(inputs, sema, module) else {
        return;
    };
    for call in scope.children().filter_map(ast::MacroCall::cast) {
        super::declarative::context_admitted(inputs, sema, module, &call);
    }
    let Some(name) = path
        .segment()
        .and_then(|s| s.name_ref())
        .map(|n| n.text().to_string())
    else {
        return;
    };
    for tree in scope.children().filter_map(ast::Use::cast).flat_map(|u| {
        u.syntax()
            .descendants()
            .filter_map(ast::UseTree::cast)
            .collect::<Vec<_>>()
    }) {
        if leaf_name(&tree).is_some_and(|n| same_name(&n, &name))
            && let Some(path) = tree.path()
        {
            disclose_missing_path(inputs, sema, &path, depth + 1);
        }
    }
}

/// Nested use trees contribute implicit qualifiers: `use crate::{a::{Enum}}`.
fn qualifier(path: &ast::Path) -> Option<ast::Path> {
    path.qualifier().or_else(|| {
        let tree = path.syntax().ancestors().find_map(ast::UseTree::cast)?;
        tree.syntax()
            .ancestors()
            .skip(1)
            .filter_map(ast::UseTree::cast)
            .find_map(|t| t.path())
    })
}

fn module_syntax(
    inputs: &Inputs,
    sema: &Semantics<'_, RootDatabase>,
    module: Module,
) -> Option<SyntaxNode> {
    let source = module.definition_source(&inputs.db);
    let file = source.file_id.file_id()?.file_id(&inputs.db);
    let parsed = sema.parse_guess_edition(file);
    match source.value {
        ra_ap_hir::ModuleSource::SourceFile(_) => Some(parsed.syntax().clone()),
        ra_ap_hir::ModuleSource::Module(m) => parsed
            .syntax()
            .descendants()
            .filter_map(ast::Module::cast)
            .find(|n| n.syntax().text_range() == m.syntax().text_range())?
            .item_list()
            .map(|l| l.syntax().clone()),
        ra_ap_hir::ModuleSource::BlockExpr(_) => None,
    }
}

fn leaf_name(tree: &ast::UseTree) -> Option<String> {
    if !tree.is_simple_path() {
        return None;
    }
    if let Some(rename) = tree.rename() {
        return rename.name().map(|n| n.text().to_string());
    }
    let path = tree.path()?;
    let segment = path.segment()?;
    let name = if segment.self_token().is_some() {
        qualifier(&path)?.segment()?.name_ref()?.text().to_string()
    } else {
        segment.name_ref()?.text().to_string()
    };
    Some(name)
}

fn same_name(a: &str, b: &str) -> bool {
    a.trim_start_matches("r#") == b.trim_start_matches("r#")
}
fn has_binding(scope: &SyntaxNode, name: &str) -> bool {
    scope
        .children()
        .filter_map(ast::Item::cast)
        .any(|item| match item {
            ast::Item::Use(u) => u
                .syntax()
                .descendants()
                .filter_map(ast::UseTree::cast)
                .any(|t| leaf_name(&t).is_some_and(|n| same_name(&n, name))),
            ast::Item::Enum(e) => e.name().is_some_and(|n| same_name(n.text(), name)),
            ast::Item::Struct(s) => s.name().is_some_and(|n| same_name(n.text(), name)),
            ast::Item::Module(m) => m.name().is_some_and(|n| same_name(n.text(), name)),
            _ => false,
        })
}

fn stable_binding(
    inputs: &Inputs,
    sema: &Semantics<'_, RootDatabase>,
    scope: &SyntaxNode,
    name: &str,
    definition: ModuleDef,
    depth: usize,
    fact_class: &mut FactClass,
) -> Option<()> {
    for item in scope.children().filter_map(ast::Item::cast) {
        let written = match &item {
            ast::Item::Enum(e) if e.name().is_some_and(|n| same_name(n.text(), name)) => {
                sema.to_def(e).map(|e| ModuleDef::Adt(Adt::Enum(e)))
            }
            ast::Item::Struct(s) if s.name().is_some_and(|n| same_name(n.text(), name)) => {
                sema.to_def(s).map(|s| ModuleDef::Adt(Adt::Struct(s)))
            }
            ast::Item::Module(m) if m.name().is_some_and(|n| same_name(n.text(), name)) => {
                sema.to_def(m).map(ModuleDef::Module)
            }
            ast::Item::Use(u) => {
                for tree in u.syntax().descendants().filter_map(ast::UseTree::cast) {
                    if leaf_name(&tree).is_some_and(|n| same_name(&n, name)) {
                        let path = tree.path()?;
                        if !matches!(sema.resolve_path(&path), Some(PathResolution::Def(d)) if d == definition)
                        {
                            return None;
                        }
                        let module =
                            scoped_context(inputs, sema, u.syntax(), FactClass::NominalIdentity)?;
                        if u.attrs().any(|a| {
                            !super::context::active_binding_attr(
                                inputs,
                                sema,
                                module,
                                &a,
                                FactClass::NominalIdentity,
                            )
                        }) {
                            return None;
                        }
                        return stable_path(inputs, sema, &path, depth + 1, fact_class);
                    }
                }
                None
            }
            ast::Item::MacroCall(_) if matches!(definition, ModuleDef::Adt(_)) => {
                let ModuleDef::Adt(adt) = definition else {
                    unreachable!()
                };
                let file = sema
                    .hir_file_for(item.syntax())
                    .file_id()?
                    .file_id(&inputs.db);
                if let Some(declaration) = super::declarative::declaration(inputs, sema, adt)
                    && declaration.declarative_macro.as_ref().is_some_and(|e| {
                        e.invocation.range.start_byte
                            == usize::from(item.syntax().text_range().start())
                            && e.invocation.path == inputs.paths[&file]
                    })
                {
                    return Some(());
                }
                None
            }
            _ => None,
        };
        if written == Some(definition) {
            let module = scoped_context(inputs, sema, item.syntax(), FactClass::NominalIdentity)?;
            if item.attrs().any(|a| {
                !super::context::active_binding_attr(
                    inputs,
                    sema,
                    module,
                    &a,
                    FactClass::NominalIdentity,
                )
            }) {
                return None;
            }
            return Some(());
        }
    }
    None
}
