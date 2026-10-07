#!/usr/bin/env bash
# Gate: internal tracking identifiers and proprietary names never reach a commit.
# Scans ADDED lines of staged content and the commit message for:
#   1. tracker identifiers (EPIC-|STORY-|TASK-|BUG- + digits)
#   2. protected names listed in .githooks/protected-names.local (untracked,
#      maintainer-maintained; one name per line, case-sensitive, matched with
#      non-identifier boundaries — list case variants explicitly (lower and upper)
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

fail=0
# Scan only ADDED lines: removing leaked content must always be allowed.
added() { git diff --cached -U0 | grep -E '^\+[^+]' || true; }
matches=$(added | grep -nE '\b(EPIC|STORY|TASK|BUG)-[0-9]{2,}\b' || true)
if [[ -n "$matches" ]]; then
  printf '%s\n' 'Gate: internal tracking identifiers found in staged content:' "$matches" >&2
  fail=1
fi
if [[ -n "${2:-}" && -f "$2" ]]; then
  msg_matches=$(grep -nE '\b(EPIC|STORY|TASK|BUG)-[0-9]{2,}\b' "$2" || true)
  if [[ -n "$msg_matches" ]]; then
    printf '%s\n' 'Gate: internal tracking identifiers found in commit message:' "$msg_matches" >&2
    fail=1
  fi
fi

list=.githooks/protected-names.local
if [[ -f "$list" ]]; then
  while IFS= read -r name; do
    [[ -n "$name" && $name != \#* ]] || continue
    pattern="(^|[^A-Za-z0-9_])${name}([^A-Za-z0-9_]|$)"
    staged=$(added | grep -nE "$pattern" || true)
    if [[ -n "$staged" ]]; then
      printf '%s\n' "Gate: protected name found in staged content: $name" "$staged" >&2
      fail=1
    fi
    if [[ -n "${2:-}" && -f "$2" ]]; then
      msg_name=$(grep -nE "$pattern" "$2" || true)
      if [[ -n "$msg_name" ]]; then
        printf '%s\n' "Gate: protected name found in commit message: $name" "$msg_name" >&2
        fail=1
      fi
    fi
  done < "$list"
else
  printf '%s\n' "Note: .githooks/protected-names.local absent; proprietary-name scan skipped (maintainer maintains this untracked list)." >&2
fi

exit $fail
