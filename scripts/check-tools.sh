#!/usr/bin/env bash
set -euo pipefail
command -v gitleaks >/dev/null || { echo 'Missing required tool: gitleaks; brew install gitleaks / make install-tools (Go install with pinned module/version flags); or install a release binary from https://github.com/gitleaks/gitleaks/releases (Ubuntu).' >&2; exit 1; }
[[ $(gitleaks version) =~ ^v?8\.[0-9]+\.[0-9]+$ ]] || { echo 'Require gitleaks 8.x (tested 8.30.1); run make install-tools.' >&2; exit 1; }
for tool in git make bash awk rustup cargo; do
  command -v "$tool" >/dev/null || { printf 'Missing required tool: %s\n' "$tool" >&2; exit 1; }
done
[[ $(rustc --version) == 'rustc 1.98.1 '* ]] || { echo 'Use the pinned Rust 1.98.1 toolchain.' >&2; exit 1; }
cargo fmt --version >/dev/null
cargo clippy --version >/dev/null
[[ $(cargo deny --version) == 'cargo-deny 0.20.2' ]] || { echo 'Require cargo-deny 0.20.2; run make install-tools.' >&2; exit 1; }
[[ $(cargo machete --version) == '0.9.2' ]] || { echo 'Require cargo-machete 0.9.2; run make install-tools.' >&2; exit 1; }
