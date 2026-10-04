#!/usr/bin/env bash
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
# Only repositories created by this invocation are removed.
scratch=$(mktemp -d /tmp/rust-sitter-tooling.XXXXXX)
trap 'rm -rf "$scratch"' EXIT
repo="$scratch/checkout with spaces"
mkdir -p "$repo" "$scratch/bin"
cp -R "$root/.githooks" "$root/scripts" "$root/src" "$repo/"
cp "$root/Cargo.toml" "$root/Cargo.lock" "$root/rust-toolchain.toml" "$root/Makefile" "$repo/"
mkdir "$repo/docs"
cp "$root/docs/dependency-log.md" "$repo/docs/"
# A fixture-local final gate prevents recursive execution of this suite.
printf '%s\n' '#!/usr/bin/env bash' 'echo fixture-tooling-gate' > "$repo/scripts/test-tooling.sh"
cat > "$scratch/bin/cargo" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "$*" >> "$FAKE_LOG"
if [[ ${MISS_GATE:-} == "$1" ]]; then echo "fixture unavailable: $1" >&2; exit 127; fi
if [[ ${2:-} == --version ]]; then
  case "$1" in
    deny) echo 'cargo-deny 0.20.2' ;;
    machete) echo '0.9.2' ;;
    *) echo 'fixture component' ;;
  esac
  exit 0
fi
if [[ ${FAIL_GATE:-} == "$1" ]]; then echo "fixture failure: $1" >&2; exit 1; fi
STUB
cat > "$scratch/bin/gitleaks" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
if [[ ${1:-} == version ]]; then echo "${FAKE_GITLEAKS_VERSION:-8.30.1}"; exit 0; fi
[[ "$*" == 'protect --staged --redact --no-banner --log-level warn --exit-code 1' ]] || exit 126
printf 'gitleaks %s\n' "$*" >> "$FAKE_LOG"
if [[ -n ${FAIL_GITLEAKS:-} ]]; then echo 'fixture scanner error' >&2; exit "$FAIL_GITLEAKS"; fi
staged=$(git diff --cached --no-ext-diff --unified=0)
if printf '%s\n' "$staged" | grep -Eq '^\+.*AKIA[0-9A-Z]{16}'; then
  echo 'gitleaks: leaks found (fake AWS fixture, redacted)' >&2; exit 1
fi
STUB
chmod +x "$scratch/bin/cargo" "$scratch/bin/gitleaks"
export PATH="$scratch/bin:$PATH"
export FAKE_LOG="$scratch/cargo.log"
cd "$repo"
git init -q
git add .githooks/pre-commit .githooks/commit-msg .githooks/post-commit
make install-hooks > "$scratch/install.log"
make install-hooks >> "$scratch/install.log"
[[ $(git config --local --get core.hooksPath) == .githooks ]]
for hook in pre-commit commit-msg post-commit; do test -x ".githooks/$hook"; done
commit() { git -c user.name=Tooling -c user.email=tooling@example.invalid -c commit.gpgsign=false commit "$@"; }
reject() {
  local before
  before=$(git rev-parse HEAD)
  if "$@" > "$scratch/rejection.log" 2>&1; then
    echo "Expected rejection: $*" >&2; exit 1
  fi
  [[ $(git rev-parse HEAD) == "$before" ]]
}
git add Cargo.toml Cargo.lock rust-toolchain.toml Makefile .githooks scripts src docs/dependency-log.md
commit -qm 'build: seed disposable hook fixture' > "$scratch/seed.log" 2>&1
# Generate a fake key only inside the disposable repo, not in tracked test source.
printf 'aws_access_key_id = AKIA%s\n' 'BCDEFGHIJKLMNOPQ' > secret-fixture.txt
git add secret-fixture.txt
: > "$FAKE_LOG"
reject commit -qm 'test: reject a staged fake secret'
grep -q 'gitleaks: leaks found' "$scratch/rejection.log"
[[ $(< "$FAKE_LOG") == 'gitleaks protect --staged --redact --no-banner --log-level warn --exit-code 1' ]]
# A clean index passes even when the worktree holds an unstaged fake key.
printf 'clean fixture\n' > secret-fixture.txt
git add secret-fixture.txt
printf 'aws_access_key_id = AKIA%s\n' 'BCDEFGHIJKLMNOPQ' >> secret-fixture.txt
gitleaks protect --staged --redact --no-banner --log-level warn --exit-code 1
git restore --worktree -- secret-fixture.txt
: > "$FAKE_LOG"
commit -qm 'test: accept clean staged content' > "$scratch/clean.log" 2>&1
IFS= read -r first_gate < "$FAKE_LOG"
[[ "$first_gate" == 'gitleaks protect --staged --redact --no-banner --log-level warn --exit-code 1' ]]
grep -q '^machete$' "$FAKE_LOG"
grep -q 'fixture-tooling-gate' "$scratch/clean.log"
# Scanner failures other than the findings code must fail before Cargo checks.
for code in 2 126; do
  export FAIL_GITLEAKS="$code"
  : > "$FAKE_LOG"
  reject bash .githooks/pre-commit
  grep -q 'fixture scanner error' "$scratch/rejection.log"
  [[ $(< "$FAKE_LOG") == 'gitleaks protect --staged --redact --no-banner --log-level warn --exit-code 1' ]]
  unset FAIL_GITLEAKS
done
# A curated PATH really has no scanner; do not depend on host installations.
mkdir "$scratch/no-gitleaks"
for tool in bash git; do ln -s "$(command -v "$tool")" "$scratch/no-gitleaks/$tool"; done
for command in 'commit' 'check-tools'; do
  if [[ "$command" == commit ]]; then
    PATH="$scratch/no-gitleaks" reject commit -qm 'test: reject missing scanner'
  else
    PATH="$scratch/no-gitleaks" reject bash scripts/check-tools.sh
  fi
  grep -q 'Missing required tool: gitleaks' "$scratch/rejection.log"
  grep -q 'brew install gitleaks / make install-tools (Go install with pinned module/version flags)' "$scratch/rejection.log"
  grep -q 'https://github.com/gitleaks/gitleaks/releases (Ubuntu)' "$scratch/rejection.log"
done
export FAKE_GITLEAKS_VERSION=9.0.0
reject bash .githooks/pre-commit
grep -q 'Require gitleaks 8.x' "$scratch/rejection.log"
unset FAKE_GITLEAKS_VERSION
FAKE_GITLEAKS_VERSION=8.99.1 bash scripts/check-tools.sh
FAKE_GITLEAKS_VERSION=v8.30.1 bash scripts/check-tools.sh
printf '\n// fixture change\n' >> src/lib.rs
git add src/lib.rs
for gate in fmt clippy test deny machete; do
  export FAIL_GATE="$gate"
  reject commit -qm 'test: reject deliberately failing gate'
  grep -q "fixture failure: $gate" "$scratch/rejection.log"
  unset FAIL_GATE
  export MISS_GATE="$gate"
  reject commit -qm 'test: reject unavailable required gate'
  grep -q "fixture unavailable: $gate" "$scratch/rejection.log"
  unset MISS_GATE
done
reject commit -qm 'bad subject'
grep -q 'expected type' "$scratch/rejection.log"
# A successful receipt must not remove build output.
mkdir target
printf 'keep\n' > target/receipt-sentinel
# Ignore build output as the real repository does.
printf 'target/\n' > .gitignore
git add .gitignore
commit -qm 'test(tooling): accept valid subject and harmless receipt' > "$scratch/valid.log" 2>&1
test -f target/receipt-sentinel
# Parity failures follow the staged scan but precede any Cargo gate.
printf '\n// unstaged\n' >> src/lib.rs
: > "$FAKE_LOG"
reject bash .githooks/pre-commit
[[ $(< "$FAKE_LOG") == 'gitleaks protect --staged --redact --no-banner --log-level warn --exit-code 1' ]]
git restore --worktree -- src/lib.rs
printf 'untracked\n' > parity-file
reject bash .githooks/pre-commit
rm -f parity-file
# Conflict-aware installation must not replace a hook manager.
git config --local core.hooksPath elsewhere
reject make install-hooks
[[ $(git config --local --get core.hooksPath) == elsewhere ]]
git config --local core.hooksPath .githooks
# Direct subject checks cover optional scope/bang, exemptions and the boundary.
for subject in 'feat(cli)!: change the CLI' 'Merge branch example' 'Revert "feat: example"' 'Revert "feat: quote "example""' "docs: $(printf '%072d' 0)"; do
  printf '%s\n' "$subject" > "$scratch/message"
  bash .githooks/commit-msg "$scratch/message"
done
for subject in 'fixup! fix: example' 'feat(UPPER): example' 'fix:no space' 'Revert "unfinished' 'Revert ""' 'Revert "feat: example" trailing' "docs: $(printf '%073d' 0)"; do
  printf '%s\n' "$subject" > "$scratch/message"
  reject bash .githooks/commit-msg "$scratch/message"
done
# Package metadata/version-only changes must not need a dependency note.
awk '{ if ($0=="version = \"0.1.0\"") sub(/0.1.0/,"0.1.1"); print }' Cargo.toml > "$scratch/manifest"
cp "$scratch/manifest" Cargo.toml
printf '\n[package.metadata.fixture]\nversion = "9.9.9"\n[package.metadata.dependencies]\nversion = "metadata-only"\n[[package.metadata.examples]]\nname = "example"\n' >> Cargo.toml
git add Cargo.toml
bash scripts/check-dependency-log.sh
# A dependency feature edit without a fresh note must fail (hunk context is irrelevant).
awk '{ sub(/"error-context"/,"\"error-context\", \"suggestions\""); print }' Cargo.toml > "$scratch/manifest"
cp "$scratch/manifest" Cargo.toml
git add Cargo.toml
reject bash scripts/check-dependency-log.sh
printf '\nOld entry edited only.\n' >> docs/dependency-log.md
git add docs/dependency-log.md
reject bash scripts/check-dependency-log.sh
printf '\n## 2026-10-04 — clap 4.6.7\n\nFixture re-evaluation of features.\n' >> docs/dependency-log.md
git add docs/dependency-log.md
bash scripts/check-dependency-log.sh
# Target/build/dev and nested dependency tables are detected too.
for section in 'dev-dependencies' '"dev-dependencies"' 'build-dependencies' 'target.\047cfg(unix)\047.dependencies' 'dependencies.clap'; do
  git restore --source=HEAD --staged --worktree -- Cargo.toml docs/dependency-log.md
  if [[ "$section" == dependencies.clap ]]; then
    awk '!/^clap = /' Cargo.toml > "$scratch/manifest"
    cp "$scratch/manifest" Cargo.toml
    printf '\n[%s]\nversion = "4.6"\n' "$section" >> Cargo.toml
  else
    printf '\n[%b]\nclap = "4.6"\n' "$section" >> Cargo.toml
  fi
  git add Cargo.toml
  reject bash scripts/check-dependency-log.sh
done
git restore --source=HEAD --staged --worktree -- Cargo.toml docs/dependency-log.md
printf '\n[features]\nfixture = ["clap/help"]\n' >> Cargo.toml
git add Cargo.toml
reject bash scripts/check-dependency-log.sh
git restore --source=HEAD --staged --worktree -- Cargo.toml
# Dependency ordering is irrelevant, but real additions/removals still need notes.
deps_repo="$scratch/dependency checkout"
mkdir -p "$deps_repo/scripts" "$deps_repo/docs"
cp "$root/scripts/check-dependency-log.sh" "$root/scripts/dependency-tables.awk" "$deps_repo/scripts/"
cp "$root/Cargo.toml" "$root/Cargo.lock" "$deps_repo/"
(
  cd "$deps_repo"
  git init -q
  printf 'clap_builder = "4.6"\n' >> Cargo.toml
  printf 'Fixture dependency log.\n' > docs/dependency-log.md
  git add Cargo.toml Cargo.lock scripts/check-dependency-log.sh scripts/dependency-tables.awk docs/dependency-log.md
  commit -qm 'test: seed dependency ordering fixture'
  awk '/^clap = / { clap=$0; next } /^clap_builder = / { print; print clap; next } { print }' Cargo.toml > "$scratch/manifest"
  cp "$scratch/manifest" Cargo.toml
  git add Cargo.toml
  bash scripts/check-dependency-log.sh
  for change in add remove; do
    git restore --source=HEAD --staged --worktree -- Cargo.toml
    if [[ "$change" == add ]]; then
      printf 'clap_derive = "4.6"\n' >> Cargo.toml
    else
      awk '!/^clap_builder = /' Cargo.toml > "$scratch/manifest"
      cp "$scratch/manifest" Cargo.toml
    fi
    git add Cargo.toml
    reject bash scripts/check-dependency-log.sh
    grep -q 'Dependency changes require an added staged dependency-log entry.' "$scratch/rejection.log"
  done
)
# An unavailable Git receipt still succeeds and does not mutate artifacts.
mkdir "$scratch/no-git"
printf '%s\n' '#!/usr/bin/env bash' 'exit 127' > "$scratch/no-git/git"
chmod +x "$scratch/no-git/git"
PATH="$scratch/no-git:$PATH" bash .githooks/post-commit > "$scratch/post.log" 2>&1
grep -q 'unknown committed' "$scratch/post.log"
test -f target/receipt-sentinel
# Linked worktrees use tracked local hooks, not main-checkout symlinks.
git worktree add -q -b fixture-linked "$scratch/linked checkout"
(cd "$scratch/linked checkout" && make install-hooks && make install-hooks) > "$scratch/linked.log"
test -x "$scratch/linked checkout/.githooks/pre-commit"
printf '%s\n' 'Tooling fixtures passed: staged secrets/clean content, scanner errors/missing/version, idempotency/conflicts, five failing and missing Cargo gates, parity, messages, dependency ordering/notes, receipts, linked worktrees.'
