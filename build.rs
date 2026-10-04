use std::path::{Path, PathBuf};
use std::process::Command;

fn git_output(root: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .env_remove("GIT_COMMON_DIR")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let output = String::from_utf8(output.stdout).ok()?;
    let output = output.strip_suffix('\n').unwrap_or(&output);
    (!output.is_empty()).then(|| output.to_owned())
}

fn watch_git_path(root: &Path, name: &str) {
    if let Some(path) = git_output(root, &["rev-parse", "--git-path", name]) {
        let path = PathBuf::from(path);
        let path = if path.is_absolute() {
            path
        } else {
            root.join(path)
        };
        println!("cargo:rerun-if-changed={}", path.display());
    }
}

fn revision(manifest_dir: &Path) -> Option<String> {
    let root = PathBuf::from(git_output(manifest_dir, &["rev-parse", "--show-toplevel"])?);
    let root = root.canonicalize().ok()?;
    let manifest_dir = manifest_dir.canonicalize().ok()?;
    if !manifest_dir.starts_with(&root) {
        return None;
    }
    watch_git_path(&manifest_dir, "HEAD");
    watch_git_path(&manifest_dir, "packed-refs");
    if let Some(head_ref) = git_output(&manifest_dir, &["symbolic-ref", "-q", "HEAD"]) {
        watch_git_path(&manifest_dir, &head_ref);
    }
    // An unpacked archive inside another repository must not borrow its SHA.
    git_output(
        &manifest_dir,
        &["ls-files", "--error-unmatch", "--", "Cargo.toml"],
    )?;
    let sha = git_output(&manifest_dir, &["rev-parse", "--verify", "HEAD"])?;
    if !matches!(sha.len(), 40 | 64) || !sha.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    Some(sha[..12].to_owned())
}

fn main() {
    println!("cargo:rerun-if-changed=Cargo.toml");
    let manifest_dir = PathBuf::from(
        std::env::var_os("CARGO_MANIFEST_DIR").expect("Cargo provides CARGO_MANIFEST_DIR"),
    );
    let sha = revision(&manifest_dir).unwrap_or_else(|| "unknown".to_owned());
    println!("cargo:rustc-env=GIT_SHA={sha}");
    println!(
        "cargo:rustc-env=VERSION_FULL={} ({sha})",
        env!("CARGO_PKG_VERSION")
    );
}
