#!/usr/bin/env bash
set -euo pipefail
[[ $(git config --get core.hooksPath || true) == .githooks ]] || {
  echo 'Tracked hooks are not active: require core.hooksPath=.githooks; run make install-hooks.' >&2
  exit 1
}
for hook in pre-commit commit-msg post-commit; do
  [[ -f ".githooks/$hook" && -x ".githooks/$hook" ]] || {
    echo "Tracked hook .githooks/$hook is missing or not executable; run make install-hooks." >&2
    exit 1
  }
done
