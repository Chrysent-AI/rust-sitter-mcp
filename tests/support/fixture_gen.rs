//! Disposable full-flow corpus. `src` compiles; unlinked `cases` isolate blockers.
use std::{
    collections::BTreeMap,
    fs,
    os::unix::fs::{MetadataExt, PermissionsExt, symlink},
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicUsize, Ordering},
};

static NEXT: AtomicUsize = AtomicUsize::new(0);
pub struct Fixture(pub PathBuf);
impl Fixture {
    fn empty() -> Self {
        let root = std::env::temp_dir().join(format!(
            "rust-sitter-fixture-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        Self(root)
    }
    pub fn generate() -> Self {
        let repo = Self::empty();
        for &(path, source) in FILES {
            repo.write(path, source);
        }
        repo.write("src/bytes.rs", "//! Scope docs\r\n#![allow(dead_code)]\r\npub fn bytes(x: Option<u8>) -> u8 {\r\n\tlet é = x;\n\té.unwrap() // Unicode trailing\r\n}\r\n");
        repo.write(
            "src/quoted path\t\"é\\.rs",
            "pub fn quoted(x: Option<u8>) -> u8 { x.unwrap() }",
        );
        repo.write(
            "cases/limits/many.rs",
            &format!(
                "fn many(x: Option<u8>) {{ {} }}\n",
                "x.unwrap();".repeat(501)
            ),
        );
        repo.write(
            "cases/limits/output.rs",
            &format!(
                "fn output(x: Option<u8>) {{ {} }}\n",
                "x.unwrap();".repeat(400)
            ),
        );
        repo.write("cases/limits/binary.rs", "\0");
        repo.write(
            "cases/limits/recovered.rs",
            "fn recovered(x: Option<u8>) { @ x.unwrap(); }\n",
        );
        repo.write(
            "cases/limits/overlap.rs",
            "fn overlap(x: Option<Option<u8>>) { x.unwrap().unwrap(); }\n",
        );
        fs::create_dir_all(repo.0.join("cases/paths/occupied_dir.rs")).unwrap();
        symlink("source.rs", repo.0.join("cases/paths/link.rs")).unwrap();
        symlink("../layout", repo.0.join("cases/paths/linked_dir")).unwrap();
        fs::set_permissions(
            repo.0.join("src/bytes.rs"),
            fs::Permissions::from_mode(0o755),
        )
        .unwrap();
        repo.git(&["init", "--quiet", "--template=", "--initial-branch=main"]);
        git(
            &repo.0.join("cases/paths/nested"),
            &["init", "--quiet", "--template=", "--initial-branch=main"],
        );
        repo.git(&[
            "add",
            "Cargo.toml",
            "Cargo.lock",
            ".gitignore",
            "src",
            "cases",
        ]);
        repo.git(&[
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "--quiet",
            "-m",
            "Initial fixture",
        ]);
        repo
    }
    pub fn write(&self, path: &str, source: &str) {
        let path = self.0.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, source).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o644)).unwrap();
    }
    pub fn git(&self, args: &[&str]) {
        git(&self.0, args);
    }
    pub fn copy(&self) -> Self {
        fn copy(from: &Path, to: &Path) {
            for entry in fs::read_dir(from).unwrap() {
                let entry = entry.unwrap();
                let src = entry.path();
                let dst = to.join(entry.file_name());
                let m = fs::symlink_metadata(&src).unwrap();
                if m.is_symlink() {
                    symlink(fs::read_link(src).unwrap(), dst).unwrap();
                } else if m.is_dir() {
                    fs::create_dir(&dst).unwrap();
                    copy(&src, &dst);
                    fs::set_permissions(dst, m.permissions()).unwrap();
                } else {
                    fs::copy(src, &dst).unwrap();
                    fs::set_permissions(dst, m.permissions()).unwrap();
                }
            }
        }
        let result = Self::empty();
        copy(&self.0, &result.0);
        result
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}
fn git(root: &Path, args: &[&str]) {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .env("GIT_AUTHOR_DATE", "2000-01-01T00:00:00Z")
        .env("GIT_COMMITTER_DATE", "2000-01-01T00:00:00Z")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[derive(Debug, PartialEq, Eq)]
pub struct Entry {
    pub bytes: Vec<u8>,
    pub mode: u32,
    pub modified: (i64, i64),
    pub link: Option<PathBuf>,
}
// Include directories, symlink targets and ALL Git data; never follow symlinks.
pub fn observe(root: &Path) -> BTreeMap<PathBuf, Entry> {
    fn walk(root: &Path, path: &Path, out: &mut BTreeMap<PathBuf, Entry>) {
        let m = fs::symlink_metadata(path).unwrap();
        out.insert(
            path.strip_prefix(root).unwrap().to_owned(),
            Entry {
                bytes: if m.is_file() {
                    fs::read(path).unwrap()
                } else {
                    vec![]
                },
                mode: m.mode(),
                modified: (m.mtime(), m.mtime_nsec()),
                link: if m.is_symlink() {
                    Some(fs::read_link(path).unwrap())
                } else {
                    None
                },
            },
        );
        if m.is_dir() {
            for e in fs::read_dir(path).unwrap() {
                walk(root, &e.unwrap().path(), out);
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(root, root, &mut out);
    out
}
// Source tree identity excludes Git administration and wall-clock observations.
pub fn tree(root: &Path) -> BTreeMap<PathBuf, Entry> {
    observe(root)
        .into_iter()
        .filter_map(|(path, mut entry)| {
            if path.components().any(|c| c.as_os_str() == ".git") {
                return None;
            }
            entry.modified = (0, 0);
            Some((path, entry))
        })
        .collect()
}

const FILES: &[(&str, &str)] = &[
    (
        "Cargo.toml",
        "[package]\nname = \"fixture-corpus\"\nversion = \"0.0.0\"\nedition = \"2024\"\n",
    ),
    (
        "Cargo.lock",
        "version = 4\n\n[[package]]\nname = \"fixture-corpus\"\nversion = \"0.0.0\"\n",
    ),
    (
        ".gitignore",
        "target/\ncases/paths/ignored.rs\ncases/paths/nested/\n",
    ),
    (
        "src/lib.rs",
        r#"//! Compilable baseline; cases outside src are intentionally unlinked.
#![allow(dead_code, unused_imports, unused_macros, uncommon_codepoints)]
mod inventory;
mod trivia;
mod bytes;
mod rewrite;
mod destination;
mod rich;
mod weak;
mod single;
mod empty;
mod legacy;
// A quoted, Unicode path also exercises external Git patch framing.
#[path = "quoted path\t\"é\\.rs"]
mod quoted;
pub fn entry(x: Option<u8>) -> u8 { inventory::take(x) + rewrite::caller() }
"#,
    ),
// Inventory: every supported top-level unit kind; nested/member lookalikes are not top-level units.
    (
        "src/inventory.rs",
        r#"//! Scope-owned prologue.
#![allow(dead_code)]
/** Attached block docs. */
#[inline]
pub(crate) fn take(x: Option<u8>) -> u8 { x.unwrap() } // same-line trailing
pub struct Record { pub value: u8 }
pub enum Choice { First, Second }
pub union Bits { pub number: u32, pub bytes: [u8; 4] }
pub trait Value { fn value(&self) -> u8; }
impl Record {
    #![allow(unused_variables)]
    pub fn member(&self) -> u8 { self.value }
}
impl Value for Record { fn value(&self) -> u8 { self.value } }
impl dyn Value { fn anonymous(&self) -> u8 { self.value() } }
pub type Count = usize;
pub const DEFAULT: u8 = 1;
pub static LABEL: &str = "obviously_fake_fixture_label";
fn nested_host() { fn nested() {} nested(); }
mod inline { pub fn hidden_member() {} }
use std::mem::size_of;
extern crate core;
unsafe extern "C" { fn fixture_foreign(); }
macro_rules! fixture_macro { () => { const GENERATED: u8 = 2; }; }
fixture_macro!();
"#,
    ),
// Bytes/trivia corpus: ordinary carry vs protected refusal across encodings and line endings.
    (
        "src/trivia.rs",
        r#"//! Inner docs stay with this file.
#![allow(dead_code)]
fn retained() {}
// Contiguous leading comment.
/// Attached docs.
#[inline]
pub fn with_body(x: Option<u8>) -> u8 {
    #![allow(unused_variables)]
    // Internal comment.
    x.unwrap()
} // trailing belongs to whole item
// --- ambiguous banner: keep by default, explicit ordinary carry case ---

/* Blank-separated block. */

pub(self) fn restricted() {}
"#,
    ),
// Module layouts: existing matching declarations, new siblings, and a legacy mod.rs tree.
    (
        "src/destination.rs",
        "//! Existing destination prologue.\nuse crate::rewrite::helper as reused;\nfn destination_keep() {}\n",
    ),
    ("src/empty.rs", ""),
    (
        "src/legacy/mod.rs",
        "mod source;\nmod destination;\npub(super) fn legacy() {}\n",
    ),
    ("src/legacy/source.rs", "pub(crate) fn legacy_source() {}\n"),
    ("src/legacy/destination.rs", "fn legacy_destination() {}\n"),
    ("cases/layout/lib.rs", "mod source;\nmod reused;\n"), // reused.rs intentionally absent
    ("cases/layout/source.rs", "fn clean() {}\n"),
// Mechanical rewrites: aliases/groups/relative paths,
    // private remaining caller, deduplicated binding reuse, batch co-location, descendant access.
    (
        "src/rewrite.rs",
        r#"use crate::inventory::take;
use crate::inventory::take as alias;
use crate::inventory::{DEFAULT, take as grouped};
use crate::inventory::{self as inv, Choice};
pub(crate) fn helper() -> u8 { 1 }
fn private() -> u8 { helper() }
pub(crate) fn caller() -> u8 { private() + self::helper() + super::inventory::DEFAULT + crate::inventory::DEFAULT }
fn dependencies() -> u8 { take(Some(DEFAULT)) + alias(Some(1)) + grouped(Some(1)) + inv::DEFAULT }
fn together_a() -> u8 { together_b() }
fn together_b() -> u8 { 2 }
mod child { fn descendant() -> u8 { super::private() } }
"#,
    ),
// Advisory split + edited two-sibling execution flow.
    // Retain const/use/impl; choose alpha/beta groups, then edit membership outside server.
    (
        "src/rich.rs",
        r#"use crate::rewrite::helper;
const RETAIN: u8 = 1;
// --- alpha section ---
fn alpha_read() -> u8 { alpha_parse() }
fn alpha_parse() -> u8 { helper() }
// --- beta section ---
fn beta_write() -> u8 { beta_flush() + alpha_parse() }
fn beta_flush() -> u8 { RETAIN }
struct Marker;
impl Marker { fn retained_member() {} }
impl Default for Marker { fn default() -> Self { Marker } }
"#,
    ),
    (
        "src/weak.rs",
        "fn apple() {}\nfn zebra() {}\nfn mountain() {}\nfn river() {}\n",
    ),
    ("src/single.rs", "fn only() {}\n"),
// Isolated ambiguity areas (one concern per file): use EACH file as its own explicit crate_root,
    // not src/lib.rs. Ordinary sibling declarations give dependencies real context.
    (
        "cases/ambiguity/binding_collision.rs",
        "mod collision_destination;\nfn selected() {}\nfn caller(selected: fn()) { selected(); }\n",
    ),
    (
        "cases/ambiguity/collision_destination.rs",
        "fn selected() {}\n",
    ),
    (
        "cases/ambiguity/bindings.rs",
        "pub fn take(x: Option<u8>) -> u8 { x.expect(\"fixture value\") }\n",
    ),
    (
        "cases/ambiguity/glob_dependency.rs",
        "mod bindings;\nuse crate::bindings::*;\nfn selected() -> u8 { take(Some(1)) }\n",
    ),
    (
        "cases/ambiguity/macro_dependency.rs",
        "macro_rules! call { ($f:ident) => { $f() }; }\nfn selected() {}\nfn caller() { call!(selected); }\n",
    ),
    (
        "cases/ambiguity/reexport_dependency.rs",
        "mod bindings;\npub use crate::bindings::take as public_take;\n",
    ),
    (
        "cases/ambiguity/module_path.rs",
        "#[path = \"other.rs\"] mod selected;\n",
    ),
    ("cases/ambiguity/other.rs", "fn selected() {}\n"),
    (
        "cases/ambiguity/module_conditional.rs",
        "#[cfg(any())] mod selected;\n",
    ),
    (
        "cases/ambiguity/module_competing.rs",
        "mod selected;\nmod selected;\n",
    ),
    (
        "cases/ambiguity/visibility_restricted.rs",
        "pub(super) fn selected() {}\n",
    ),
    (
        "cases/ambiguity/visibility_member.rs",
        "struct Hidden { value: u8 }\nfn selected(x: Hidden) -> u8 { x.value }\n",
    ),
    (
        "cases/ambiguity/visibility_type.rs",
        "fn selected<T: Iterator>(x: &mut T) { x.next(); }\n",
    ),
    (
        "cases/ambiguity/scope_cfg.rs",
        "#[cfg(any())] fn selected() {}\n",
    ),
    (
        "cases/ambiguity/scope_inherited.rs",
        "#![no_implicit_prelude]\nfn selected() {}\n",
    ),
    (
        "cases/ambiguity/scope_include.rs",
        "fn selected() -> &'static str { include_str!(\"relative.txt\") }\n",
    ),
    (
        "cases/ambiguity/relative.txt",
        "obviously fake fixture text\n",
    ),
    (
        "cases/ambiguity/trivia_ownership.rs",
        "fn keep() {}\n// --- disputed banner ---\n\nfn selected() {}\n",
    ),
    (
        "cases/ambiguity/unsupported_dependency_form.rs",
        "fn selected<T: Default>() -> T { <T as Default>::default() }\n",
    ),
    (
        "cases/ambiguity/unrelated_controls.rs",
        "mod bindings;\nuse crate::bindings::*;\nmacro_rules! unrelated { () => { 1 }; }\nfn independent() {}\n",
    ),
// New-path adversaries: occupied/competing/symlink/case-collision targets; request names/escapes are in the smoke map.
    ("cases/paths/lib.rs", "mod source;\n"),
    ("cases/paths/source.rs", "fn selected() {}\n"),
    ("cases/paths/occupied.rs", "fn occupied() {}\n"),
    ("cases/paths/competing/mod.rs", "fn competing() {}\n"),
    ("cases/paths/Clash.rs", "fn case_collision() {}\n"),
    ("cases/paths/ignored.rs", "fn ignored() {}\n"),
    ("cases/paths/filtered.rs", "fn filtered() {}\n"),
    (
        "cases/paths/nested/source.rs",
        "fn nested_repository() {}\n",
    ),
// Limits/lifecycle controls plus the full tool-flow smoke.
    (
        "cases/limits/trivia_loss.rs",
        "fn loss(x: Option<u8>) { x.unwrap(/* must preserve */); }\n",
    ),
];
