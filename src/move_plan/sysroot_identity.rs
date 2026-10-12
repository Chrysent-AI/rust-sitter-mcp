//! Local component release claims, not independent release authentication.
use super::{Controls, Coverage, Identity, Observed, read, refuse};
use crate::result::DomainError;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

pub(super) type Claims = BTreeMap<PathBuf, (Vec<u8>, Identity)>;
const INPUTS: &[&str] = &[
    "components",
    "multirust-channel-manifest.toml",
    "manifest-rust-src",
];
const RELEASE: &str = "1.97.1 (8bab26f4f 2026-07-14)";
fn text(value: &(Vec<u8>, Identity)) -> Result<&str, DomainError> {
    std::str::from_utf8(&value.0).map_err(|_| {
        refuse(
            "SYSROOT_IDENTITY_UNSUPPORTED",
            "installed component claims are not UTF-8",
        )
    })
}
fn release(manifest: &str, name: &str) -> Result<String, DomainError> {
    let section = format!("[pkg.{name}]");
    let mut inside = false;
    let mut values = Vec::new();
    for line in manifest.lines().map(str::trim) {
        if line.starts_with('[') {
            inside = line == section;
        }
        if inside && let Some(value) = line.strip_prefix("version = ") {
            values.push(serde_json::from_str::<String>(value).map_err(|_| {
                refuse(
                    "SYSROOT_IDENTITY_UNSUPPORTED",
                    "malformed local component release",
                )
            })?);
        }
    }
    if values.len() != 1 || values[0] != RELEASE {
        return Err(refuse(
            "SYSROOT_IDENTITY_UNSUPPORTED",
            "compiler and rust-src must claim the audited same release in local rustup metadata",
        ));
    }
    Ok(values.remove(0))
}
pub(super) fn capture(
    toolchain: &Path,
    coverage: &mut Coverage,
    controls: Controls<'_>,
    comparison: bool,
) -> Result<Claims, DomainError> {
    let mut claims = BTreeMap::new();
    for name in INPUTS {
        let path = toolchain.join("lib/rustlib").join(name);
        let value = read(&path, coverage.capture_usage.max_file_bytes, controls)?;
        let usage = &mut coverage.capture_usage;
        if comparison {
            usage.comparison_input_files += 1;
            usage.comparison_input_bytes += value.0.len();
        } else {
            usage.input_files += 1;
            usage.input_bytes += value.0.len();
        }
        let (files, bytes) = if comparison {
            (usage.comparison_input_files, usage.comparison_input_bytes)
        } else {
            (usage.input_files, usage.input_bytes)
        };
        if files > usage.max_entries || bytes > usage.max_source_bytes {
            return Err(refuse(
                "SYSROOT_CAPTURE_EXHAUSTED",
                "installed identity input count/byte cap exhausted",
            ));
        }
        claims.insert(path, value);
    }
    let components = text(&claims[&toolchain.join("lib/rustlib/components")])?;
    for required in [
        "rust-src",
        "rustc-aarch64-apple-darwin",
        "rust-std-aarch64-apple-darwin",
    ] {
        if components.lines().filter(|line| *line == required).count() != 1 {
            return Err(refuse(
                "SYSROOT_IDENTITY_UNSUPPORTED",
                "required installed component receipt absent or ambiguous",
            ));
        }
    }
    let manifest = text(&claims[&toolchain.join("lib/rustlib/multirust-channel-manifest.toml")])?;
    release(manifest, "rustc")?;
    coverage.component_release = release(manifest, "rust-src")?;
    coverage.identity_inputs = INPUTS
        .iter()
        .map(|name| format!("lib/rustlib/{name}"))
        .collect();
    Ok(claims)
}
pub(super) fn membership(
    toolchain: &Path,
    claims: &Claims,
    observed: &Observed,
) -> Result<(), DomainError> {
    let manifest = text(&claims[&toolchain.join("lib/rustlib/manifest-rust-src")])?;
    let prefix = "file:lib/rustlib/src/rust/library/";
    let mut expected = BTreeSet::new();
    for line in manifest.lines() {
        if let Some(name) = line.strip_prefix(prefix)
            && (name.split('/').any(|part| matches!(part, "" | "." | ".."))
                || !expected.insert(toolchain.join("lib/rustlib/src/rust/library").join(name)))
        {
            return Err(refuse(
                "SYSROOT_IDENTITY_UNSUPPORTED",
                "invalid or duplicated installed source membership claim",
            ));
        }
    }
    if expected.is_empty() || expected != observed.files.keys().cloned().collect() {
        return Err(refuse(
            "SYSROOT_IDENTITY_UNSUPPORTED",
            "captured library membership differs from installed rust-src component receipt",
        ));
    }
    Ok(())
}
