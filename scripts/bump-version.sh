#!/usr/bin/env bash
set -euo pipefail
bump=${1:-}
exact=${2:-}
if [[ -n "$bump" && -n "$exact" ]]; then
  echo 'Choose BUMP or VERSION, not both.' >&2
  exit 1
fi
choice=${exact:-${bump:-patch}}
case "$choice" in
  major|minor|patch) ;;
  *) [[ "$choice" =~ ^(0|[1-9][0-9]{0,8})\.(0|[1-9][0-9]{0,8})\.(0|[1-9][0-9]{0,8})$ ]] || { echo 'Expected major, minor, patch, or stable x.y.z.' >&2; exit 1; } ;;
esac
tmp=$(mktemp ./Cargo.toml.bump.XXXXXX)
trap 'rm -f "$tmp"' EXIT
awk -v choice="$choice" '
  /^[ \t]*\[/ {
    package=($0 ~ /^[ \t]*\[package\][ \t]*(#.*)?$/)
    if (package) packages++
  }
  package && /^[ \t]*version[ \t]*=/ {
    versions++
    if ($0 !~ /^[ \t]*version[ \t]*=[ \t]*"[0-9]+\.[0-9]+\.[0-9]+"[ \t]*(#.*)?$/) { bad=1; next }
    old=$0; sub(/^[^"]*"/,"",old); sub(/".*$/,"",old)
    split(old,v,".")
    if (choice=="major") nextversion=(v[1]+1) ".0.0"
    else if (choice=="minor") nextversion=v[1] "." (v[2]+1) ".0"
    else if (choice=="patch") nextversion=v[1] "." v[2] "." (v[3]+1)
    else nextversion=choice
    sub(/"[0-9]+\.[0-9]+\.[0-9]+"/,"\"" nextversion "\"")
  }
  { print }
  END {
    if (packages!=1 || versions!=1 || bad) {
      print "Expected exactly one literal stable [package] version." > "/dev/stderr"
      exit 1
    }
  }
' Cargo.toml > "$tmp"
chmod 644 "$tmp"
mv "$tmp" Cargo.toml
# Refresh only the package lock entry, without re-resolving dependencies.
# Failure deliberately leaves a visible diff; no automatic commit or tag.
cargo check --offline --quiet
printf '%s\n' 'Version updated; inspect Cargo.toml and Cargo.lock before committing.'
