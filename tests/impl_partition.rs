#[path = "support/fixture_gen.rs"]
mod fixture_gen;
#[path = "support/move_artifacts.rs"]
mod move_artifacts;
use fixture_gen::Fixture;
use move_artifacts::{anchor, apply};
use rust_sitter_mcp::{engine::Engine, move_plan::MoveRequest};
use serde_json::{Value, json};
use std::{fs, sync::atomic::AtomicBool};

fn run(repo: &Fixture, args: Value) -> Value {
    let request: MoveRequest = serde_json::from_value(args).unwrap();
    serde_json::to_value(
        Engine::new(repo.0.clone())
            .unwrap()
            .move_item(request, &AtomicBool::new(false)),
    )
    .unwrap()
}

#[test]
fn unchanged_generic_where_header_and_verbatim_members_move_to_sibling() {
    let repo = Fixture::generate();
    repo.write(
        "cases/layout/lib.rs",
        "mod source; pub trait Bound {} pub struct Record<T>(T);",
    );
    let header = "impl<T> Record<T> where T: crate::Bound ";
    let method = "fn identity(value: T) -> T { value }";
    repo.write(
        "cases/layout/source.rs",
        &format!("use crate::Record;\n{header}{{\n    /// Preserve bytes.\n    {method}\n}}\n"),
    );
    let result = run(
        &repo,
        json!({"repo_path":repo.0,"crate_root":"cases/layout/lib.rs","paths":["cases/layout"],"moves":[{"item":anchor(&repo,"cases/layout/source.rs",method),"enclosing_impl":anchor(&repo,"cases/layout/source.rs",header),"destination":{"kind":"new_sibling","path":"cases/layout/moved.rs","parent_path":"cases/layout/lib.rs"}}]}),
    );
    assert_eq!(result["plan"]["applicable"], true, "{result}");
    let copy = apply(&repo, &result);
    let text = fs::read_to_string(copy.0.join("cases/layout/moved.rs")).unwrap();
    assert!(text.contains(header));
    assert!(
        text.contains(
            "    /// Preserve bytes.\n    pub(crate) fn identity(value: T) -> T { value }"
        ),
        "{text:?}"
    );
}

#[test]
fn trait_and_conditional_members_are_not_movable() {
    for (header, method) in [
        ("impl Trait for Record ", "fn value(&self) {}"),
        ("impl Record ", "#[cfg(test)] fn value(&self) {}"),
        ("unsafe impl Trait for Record ", "fn value(&self) {}"),
    ] {
        let repo = Fixture::generate();
        repo.write(
            "cases/layout/lib.rs",
            "mod source; pub struct Record; pub trait Trait {}",
        );
        repo.write(
            "cases/layout/source.rs",
            &format!("use crate::Record; {header}{{ {method} }}"),
        );
        let result = run(
            &repo,
            json!({"repo_path":repo.0,"crate_root":"cases/layout/lib.rs","paths":["cases/layout"],"moves":[{"item":anchor(&repo,"cases/layout/source.rs","fn value(&self) {}"),"enclosing_impl":anchor(&repo,"cases/layout/source.rs",header),"destination":{"kind":"new_sibling","path":"cases/layout/moved.rs","parent_path":"cases/layout/lib.rs"}}]}),
        );
        assert_eq!(result["plan"]["applicable"], false, "{result}");
        assert!(result["plan"]["patch"].is_null());
    }
}

#[test]
fn one_method_between_identical_impl_headers() {
    let repo = Fixture::generate();
    repo.write(
        "cases/layout/lib.rs",
        "mod source;\nmod destination;\nstruct Record;\n",
    );
    repo.write("cases/layout/source.rs", "use crate::Record;\nimpl Record {\n    /// Exact method docs.\n    fn value(&self) -> u8 { 7 }\n}\n");
    repo.write(
        "cases/layout/destination.rs",
        "use crate::Record;\nimpl Record {\n}\n",
    );
    let result = run(
        &repo,
        json!({
            "repo_path":repo.0,"crate_root":"cases/layout/lib.rs","paths":["cases/layout"],
            "moves":[{
                "item":anchor(&repo,"cases/layout/source.rs","fn value(&self) -> u8 { 7 }"),
                "enclosing_impl":anchor(&repo,"cases/layout/source.rs","impl Record "),
                "destination":{"kind":"existing_impl","path":"cases/layout/destination.rs","implementation":anchor(&repo,"cases/layout/destination.rs","impl Record {\n}")}
            }]
        }),
    );
    assert_eq!(result["plan"]["applicable"], true, "{result}");
    let copy = apply(&repo, &result);
    let text = fs::read_to_string(copy.0.join("cases/layout/destination.rs")).unwrap();
    assert!(text.contains("/// Exact method docs."));
    assert!(text.contains("fn value(&self) -> u8 { 7 }"));
    assert_eq!(text.matches("impl Record").count(), 1);
}
