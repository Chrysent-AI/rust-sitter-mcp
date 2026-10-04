#!/usr/bin/env bash
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
scratch=$(mktemp -d /tmp/rust-sitter-build.XXXXXX)
trap 'rm -rf "$scratch"' EXIT
repo="$scratch/checkout with spaces"
mkdir -p "$repo/scripts"
cp -R "$root/src" "$repo/"
cp "$root/Cargo.toml" "$root/Cargo.lock" "$root/build.rs" "$root/rust-toolchain.toml" "$repo/"
cp "$root/scripts/bump-version.sh" "$repo/scripts/"
export CARGO_TARGET_DIR="$scratch/target"
cd "$repo"
git init -q
commit() { git -c user.name=Tooling -c user.email=tooling@example.invalid -c commit.gpgsign=false commit "$@"; }
git add Cargo.toml Cargo.lock build.rs rust-toolchain.toml src scripts/bump-version.sh
commit -qm 'build: seed build identity fixture'
identity() {
  local version=$1 sha=$2 actual
  cargo build --offline --locked --quiet
  actual=$("$CARGO_TARGET_DIR/debug/rust-sitter-mcp" --version)
  [[ "$actual" == "rust-sitter-mcp $version ($sha)" ]] || { printf 'Unexpected identity: %s\n' "$actual" >&2; exit 1; }
}
short_sha() { git rev-parse HEAD | cut -c1-12; }
identity 0.1.0 "$(short_sha)"
printf 'first identity\n' > receipt.txt
git add receipt.txt
commit -qm 'docs: advance HEAD without cleaning'
identity 0.1.0 "$(short_sha)"
git checkout -q --detach
identity 0.1.0 "$(short_sha)"
git checkout -q -b build-fixture
# Each bump must leave every dependency resolution unchanged.
cargo tree --offline --locked -e no-dev | awk 'NR>1' > "$scratch/tree-before"
for mode in patch minor major 2.3.4; do
  git restore --source=HEAD --worktree -- Cargo.toml Cargo.lock
  case "$mode" in patch) version=0.1.1 ;; minor) version=0.2.0 ;; major) version=1.0.0 ;; *) version=$mode ;; esac
  # Array and single-table metadata assignments must not be rewritten.
  printf '\n[[package.metadata.examples]]\nversion = "9.9.9"\n[package.metadata.fixture]\nversion = "8.8.8"\n' >> Cargo.toml
  bash scripts/bump-version.sh "$mode"
  grep -q '^version = "9.9.9"$' Cargo.toml
  grep -q '^version = "8.8.8"$' Cargo.toml
  identity "$version" "$(short_sha)"
  cargo tree --offline --locked -e no-dev | awk 'NR>1' > "$scratch/tree-after"
  cmp "$scratch/tree-before" "$scratch/tree-after"
done
git restore --source=HEAD --worktree -- Cargo.toml Cargo.lock
bash scripts/bump-version.sh '' 3.2.1
identity 3.2.1 "$(short_sha)"
git restore --source=HEAD --worktree -- Cargo.toml Cargo.lock
for invalid in invalid 1.2 01.2.3 1.2.3-beta '1.2.3;false'; do
  cp Cargo.toml "$scratch/before.toml"
  if bash scripts/bump-version.sh "$invalid" > "$scratch/invalid.log" 2>&1; then echo 'Invalid bump accepted.' >&2; exit 1; fi
  cmp Cargo.toml "$scratch/before.toml"
done
if bash scripts/bump-version.sh patch 3.2.1 > "$scratch/invalid.log" 2>&1; then echo 'Conflicting bump accepted.' >&2; exit 1; fi
# Fresh linked worktree: active branch ref belongs to its shared Git directory.
git worktree add -q -b linked-build "$scratch/linked checkout"
(cd "$scratch/linked checkout" && identity 0.1.0 "$(short_sha)"
 printf 'linked change\n' > receipt.txt
 git add receipt.txt
 commit -qm 'docs: advance linked worktree identity'
 identity 0.1.0 "$(short_sha)")
# Packed refs, followed by a new loose ref, must refresh identity as well.
git pack-refs --all
identity 0.1.0 "$(short_sha)"
printf 'packed ref change\n' > receipt.txt
git add receipt.txt
commit -qm 'docs: advance packed reference'
identity 0.1.0 "$(short_sha)"
# Git-less archive and an archive inside an unrelated checkout both use unknown.
archive="$scratch/archive"
mkdir "$archive"
cp -R src "$archive/"
cp Cargo.toml Cargo.lock build.rs rust-toolchain.toml "$archive/"
(cd "$archive" && identity 0.1.0 unknown)
mkdir "$repo/untracked archive"
cp -R "$archive/src" "$repo/untracked archive/"
cp "$archive/Cargo.toml" "$archive/Cargo.lock" "$archive/build.rs" "$archive/rust-toolchain.toml" "$repo/untracked archive/"
(cd "$repo/untracked archive" && identity 0.1.0 unknown)
printf '%s\n' 'Build fixtures passed: all bump modes, metadata array tables, invalid inputs, stable dependency locks, SHA refresh, detached HEAD, linked/packed refs, archive unknown.'
