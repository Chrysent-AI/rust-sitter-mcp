use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const SKILL: &str = ".pi/skills/split-rust-code";

fn references(directory: &Path, paths: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(directory).expect("read skill references") {
        let entry = entry.expect("read reference entry");
        let kind = entry.file_type().expect("reference file type");
        assert!(!kind.is_symlink(), "skill references must not be symlinks");
        if kind.is_dir() {
            references(&entry.path(), paths);
        } else {
            assert!(kind.is_file(), "skill reference must be a regular file");
            paths.push(entry.path());
        }
    }
}

fn digest(bytes: &[u8]) -> String {
    // Git's blob hash includes its standard type/length header, not just bytes.
    // No repository, object write, filter or network is involved.
    let mut child = Command::new("git")
        .args([
            "-c",
            "extensions.objectFormat=sha1",
            "hash-object",
            "--stdin",
        ])
        .current_dir(std::env::temp_dir())
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_COMMON_DIR")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("Git is a build prerequisite");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(bytes)
        .expect("hash skill bytes");
    let output = child.wait_with_output().expect("wait for Git blob hash");
    assert!(output.status.success(), "hash skill bytes failed");
    let hash = String::from_utf8(output.stdout).expect("ASCII blob hash");
    let hash = hash.trim();
    assert!(hash.len() == 40 && hash.bytes().all(|b| b.is_ascii_hexdigit()));
    format!("git-blob-sha1:{hash}")
}

pub fn embed_skill(root: &Path) {
    let base = root.join(SKILL);
    println!("cargo:rerun-if-changed={}", base.display());
    let mut paths = Vec::new();
    references(&base.join("references"), &mut paths);
    paths.sort();
    paths.insert(0, base.join("SKILL.md"));
    let mut generated = String::from("static DOCUMENTS: &[EmbeddedDocument] = &[\n");
    let mut concatenated = String::new();
    for path in paths {
        println!("cargo:rerun-if-changed={}", path.display());
        let relative = path
            .strip_prefix(&base)
            .unwrap()
            .to_str()
            .expect("UTF-8 skill path")
            .replace('\\', "/");
        let id = if relative == "SKILL.md" {
            "guide"
        } else {
            &relative
        };
        let source = format!("{SKILL}/{relative}");
        let content = std::fs::read_to_string(&path).expect("UTF-8 canonical Markdown");
        let hash = digest(content.as_bytes());
        if !concatenated.is_empty() {
            concatenated.push_str("\n\n");
        }
        concatenated.push_str(&format!(
            "# Bundled document: {id}\n\nSource: `{source}`\n\nDigest: `{hash}`\n\n"
        ));
        concatenated.push_str(&content);
        generated.push_str(&format!("EmbeddedDocument {{ id: {id:?}, path: {source:?}, digest: {hash:?}, content: {content:?} }},\n"));
    }
    generated.push_str("];\n");
    generated.push_str(&format!(
        "static CONCATENATED: &str = {concatenated:?};\nstatic CONCATENATED_DIGEST: &str = {:?};\n",
        digest(concatenated.as_bytes())
    ));
    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("Cargo OUT_DIR"));
    std::fs::write(out.join("skill_guide.rs"), generated).expect("write embedded guide");
}
