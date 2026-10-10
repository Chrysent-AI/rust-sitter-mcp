#!/usr/bin/env bash
set -euo pipefail
version=$(awk '
  /^[ \t]*\[/ { package=($0 ~ /^[ \t]*\[package\][ \t]*(#.*)?$/) }
  package && /^[ \t]*version[ \t]*=/ {
    value=$0; sub(/^[^"]*"/,"",value); sub(/".*$/,"",value); print value
  }
' Cargo.toml)
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || {
  echo 'Cannot read stable [package] version for test-version guard.' >&2; exit 1
}
# Tokenize Rust instead of grepping prose or entire assertion statements. Only
# direct literal operands are rejected; nested fixture builders remain valid.
while IFS= read -r -d '' source; do
  awk -v version="$version" -f scripts/test-version-literals.awk "$source"
done < <(find tests src -type f -name '*.rs' -print0)
