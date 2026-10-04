#!/usr/bin/env bash
set -euo pipefail
scratch=$(mktemp -d /tmp/rust-sitter-deps.XXXXXX)
trap 'rm -rf "$scratch"' EXIT
if git rev-parse --verify HEAD >/dev/null 2>&1; then
  git show HEAD:Cargo.toml > "$scratch/old.toml" 2>/dev/null || :
else
  : > "$scratch/old.toml"
fi
git show :Cargo.toml > "$scratch/new.toml"
awk -f scripts/dependency-tables.awk "$scratch/old.toml" | LC_ALL=C sort > "$scratch/old"
awk -f scripts/dependency-tables.awk "$scratch/new.toml" | LC_ALL=C sort > "$scratch/new"
if cmp -s "$scratch/old" "$scratch/new"; then exit 0; fi
# Compare entries, not diff hunk context: package/metadata edits are irrelevant.
awk -F '\t' '
  FILENAME==ARGV[1] { old[$1 SUBSEP $2]=$3; next }
  { id=$1 SUBSEP $2; new[id]=$3; if (old[id]!=$3) changed[id]=1 }
  END {
    for (id in old) if (!(id in new)) changed[id]=1
    for (id in changed) {
      split(id,key,SUBSEP)
      if (key[1]=="features") { features=1; continue }
      name=key[2]
      if (key[1] ~ /dependencies\./) { name=key[1]; sub(/^.*dependencies\./,"",name) }
      value=(id in new ? new[id] : old[id])
      if (match(value,/package="[^"]+"/)) { name=substr(value,RSTART+9,RLENGTH-10) }
      names[name]=1
    }
    if (features) {
      for (id in new) {
        split(id,key,SUBSEP)
        if (key[1] ~ /dependencies$/) names[key[2]]=1
        else if (key[1] ~ /dependencies\./) { name=key[1]; sub(/^.*dependencies\./,"",name); names[name]=1 }
      }
    }
    for (name in names) print name
  }
' "$scratch/old" "$scratch/new" > "$scratch/names"
[[ -s "$scratch/names" ]] || exit 0
git diff --cached --unified=0 -- docs/dependency-log.md | awk '/^\+[^+]/ { print substr($0,2) }' > "$scratch/notes"
git show :Cargo.lock > "$scratch/lock"
[[ -s "$scratch/notes" ]] || { echo 'Dependency changes require an added staged dependency-log entry.' >&2; exit 1; }
while IFS= read -r name; do
  version=$(awk -v name="$name" '
    /^\[\[package\]\]/ { found=0 }
    $0=="name = \"" name "\"" { found=1 }
    found && /^version = / { gsub(/\"/,"",$3); print $3; exit }
  ' "$scratch/lock")
  [[ -n "$version" ]] || { printf 'Cannot resolve %s from staged Cargo.lock; review the dependency change.\n' "$name" >&2; exit 1; }
  awk -v name="$name" -v version="$version" '
    /^## / { for (i=1; i<NF; i++) if ($i==name && $(i+1)==version) found=1 }
    END { exit !found }
  ' "$scratch/notes" || { printf 'Add a staged dependency-log heading naming %s %s.\n' "$name" "$version" >&2; exit 1; }
done < "$scratch/names"
