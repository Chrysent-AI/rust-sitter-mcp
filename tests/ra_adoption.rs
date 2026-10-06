//! Verify the pinned pure database seam without project discovery or execution.
use ra_ap_base_db::{
    CrateGraphBuilder, CrateOrigin, CrateWorkspaceData, FileChange, FileId, FileSet, SourceRoot,
    VfsPath,
};
use ra_ap_hir::{AsAssocItem, AssocItemContainer, HasSource, HasVisibility, Semantics};
use ra_ap_ide_db::RootDatabase;
use ra_ap_syntax::{
    AstNode, Edition,
    ast::{self, HasName},
};

#[test]
fn admitted_text_resolves_true_inherent_method_and_visibility() {
    let text = "pub struct Value; impl Value { pub fn read(&self) -> u32 { 7 } } fn user(value: Value) -> u32 { value.read() }";
    let file = FileId::from_raw(0);
    let mut set = FileSet::default();
    set.insert(file, VfsPath::new_virtual_path("/lib.rs".into()));
    let mut graph = CrateGraphBuilder::default();
    graph.add_crate_root(
        file,
        Edition::Edition2024,
        None,
        None,
        Default::default(),
        None,
        Default::default(),
        CrateOrigin::Local {
            repo: None,
            name: None,
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
    let mut change = FileChange::default();
    change.set_roots(vec![SourceRoot::new_local(set)]);
    change.change_file(file, Some(text.into()));
    change.set_crate_graph(graph);
    let mut db = RootDatabase::default();
    change.apply(&mut db);
    let sema = Semantics::new(&db);
    let syntax = sema.parse_guess_edition(file);
    let call = syntax
        .syntax()
        .descendants()
        .find_map(ast::MethodCallExpr::cast)
        .unwrap();
    let types = sema.type_of_expr(&call.receiver().unwrap()).unwrap();
    assert!(!types.original.contains_unknown());
    assert!(!types.adjusted().contains_unknown());
    let function = sema.resolve_method_call(&call).unwrap();
    let associated = function.as_assoc_item(&db).unwrap();
    let AssocItemContainer::Impl(implementation) = associated.container(&db) else {
        panic!("not an impl");
    };
    assert!(implementation.trait_(&db).is_none());
    let module = sema.scope(call.syntax()).unwrap().module();
    assert!(function.is_visible_from(&db, module));
    let source = function.source(&db).unwrap();
    assert_eq!(source.value.name().unwrap().text(), "read");
    assert_eq!(
        source.value.syntax().text().to_string(),
        "pub fn read(&self) -> u32 { 7 }"
    );
}
