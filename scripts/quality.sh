#!/usr/bin/env bash
set -euo pipefail
if [[ ${1:-} == --precommit ]]; then
  git diff --quiet || { echo 'Stage all tracked changes before committing.' >&2; exit 1; }
  [[ -z $(git ls-files --others --exclude-standard) ]] || { echo 'Stage or deliberately ignore untracked files before committing.' >&2; exit 1; }
  git diff --cached --check
  bash scripts/check-dependency-log.sh
elif [[ $# != 0 ]]; then
  echo 'Usage: quality.sh [--precommit]' >&2
  exit 1
fi
# Staged checks need the committing index; disposable fixtures must not inherit it.
unset GIT_ALTERNATE_OBJECT_DIRECTORIES GIT_OBJECT_DIRECTORY GIT_DIR GIT_WORK_TREE \
  GIT_IMPLICIT_WORK_TREE GIT_GRAFT_FILE GIT_INDEX_FILE GIT_NO_REPLACE_OBJECTS \
  GIT_REPLACE_REF_BASE GIT_PREFIX GIT_INTERNAL_SUPER_PREFIX GIT_SHALLOW_FILE GIT_COMMON_DIR
bash scripts/check-tools.sh
printf '%s\n' 'Gate: formatting'
cargo fmt --all -- --check
printf '%s\n' 'Gate: lint'
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
printf '%s\n' 'Gate: tests'
cargo test --workspace --all-features --locked --quiet
printf '%s\n' 'Gate: advisories, licenses, sources'
cargo deny check
printf '%s\n' 'Gate: unused dependencies'
cargo machete
printf '%s\n' 'Gate: tooling syntax and regression fixtures'
for script in .githooks/* scripts/*.sh; do bash -n "$script"; done
bash scripts/test-tooling.sh
