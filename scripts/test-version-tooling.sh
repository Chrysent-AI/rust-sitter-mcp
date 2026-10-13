#!/usr/bin/env bash
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
scratch=$(mktemp -d /tmp/rust-sitter-version-fixtures.XXXXXX)
trap 'rm -rf "$scratch"' EXIT
mkdir -p "$scratch/repo/scripts" "$scratch/repo/tests" "$scratch/repo/src" "$scratch/bin"
cp "$root/scripts/"{bump-version.sh,check-hooks.sh,check-test-versions.sh,test-version-literals.awk} "$scratch/repo/scripts/"
cp -R "$root/.githooks" "$scratch/repo/"
printf '[package]\nname = "version-fixture"\nversion = "1.2.3"\n' > "$scratch/repo/Cargo.toml"
printf 'since v1.2.3\n' > "$scratch/repo/history.md"
cat > "$scratch/bin/cargo" <<'STUB'
#!/usr/bin/env bash
printf '%s\n' "$*" >> "$VERSION_FIXTURE_LOG"
[[ ${VERSION_FIXTURE_FAIL:-} != "$1" ]]
STUB
chmod +x "$scratch/bin/cargo"
export PATH="$scratch/bin:$PATH"
export VERSION_FIXTURE_LOG="$scratch/cargo.log"
cd "$scratch/repo"
git init -q
reject() {
  if "$@" > "$scratch/rejection.log" 2>&1; then
    echo "Expected rejection: $*" >&2; exit 1
  fi
}
# Never deactivate hooks in the caller checkout.
reject bash scripts/check-hooks.sh
grep -q 'Tracked hooks are not active' "$scratch/rejection.log"
reject bash scripts/bump-version.sh patch
grep -q 'Tracked hooks are not active' "$scratch/rejection.log"
git config --local core.hooksPath elsewhere
reject bash scripts/check-hooks.sh
git config --local core.hooksPath .githooks
chmod -x .githooks/pre-commit
reject bash scripts/check-hooks.sh
grep -q 'missing or not executable' "$scratch/rejection.log"
chmod +x .githooks/pre-commit
mv .githooks/commit-msg "$scratch/commit-msg"
reject bash scripts/check-hooks.sh
mv "$scratch/commit-msg" .githooks/commit-msg
bash scripts/check-hooks.sh
for comparison in 'assert_eq!(actual, "1.2.3");' 'assert_eq! { actual, "1.2.3" };' 'assert_ne!["1.2.3", actual];' 'assert_ne!("1.2.3", actual);' 'assert!(actual == "1.2.3");' 'assert!("1.2.3" != actual);' 'assert_eq!(actual, r#"1.2.3"#);' 'assert_eq!(actual, concat!("1.2.3"));' 'assert_ne!(concat!["1.2.3"], actual);' 'assert_eq! { actual, stringify!("1.2.3") };' 'assert_eq!(actual, concat!("prefix", concat!("1.2.3")));'; do
  printf '%s\n' "$comparison" > tests/version.rs
  reject bash scripts/check-test-versions.sh
  grep -q 'hardcoded current package version' "$scratch/rejection.log"
  reject bash scripts/bump-version.sh minor
  grep -q 'hardcoded current package version' "$scratch/rejection.log"
  grep -q 'version = "1.2.3"' Cargo.toml
  # The new-version expectation must also fail after rewriting the manifest.
  printf '%s\n' "$comparison" | tr '123' '246' > tests/version.rs
  reject bash scripts/bump-version.sh '' 2.4.6
  grep -q 'Version verification failed' "$scratch/rejection.log"
  grep -q 'version = "2.4.6"' Cargo.toml
  printf '[package]\nname = "version-fixture"\nversion = "1.2.3"\n' > Cargo.toml
done
cat > tests/version.rs <<'RUST'
// assert_eq!(actual, "1.2.3"); since v1.2.3
/* nested /* comment */ assert_eq!(actual, "1.2.3"); */
assert_eq!(actual, env!("CARGO_PKG_VERSION"));
assert_eq!(actual, "9.8.7");
assert_eq!(fixture("1.2.3"), fixture("1.2.3"));
let fixture_version = concat!("1.2.3");
assert_eq!(fixture(concat!("1.2.3")), actual);
assert_eq!(actual, concat!(env!("CARGO_PKG_VERSION")));
assert_eq!(actual, concat!("9.8.7"));
assert_eq!(actual, stringify!("9.8.7"));
assert_eq!(actual, other, concat!("1.2.3"));
assert_eq!(actual, other, "1.2.3");
let punctuation = "{";
assert_eq!(actual, env!("CARGO_PKG_VERSION"));
let prose = r##"assert_eq!(actual, "1.2.3");"##;
RUST
bash scripts/check-test-versions.sh
printf '#[cfg(test)]\nmod tests {\n #[test]\n fn version() { assert_eq!(actual,\n "1.2.3"); }\n}\n' > src/lib.rs
reject bash scripts/check-test-versions.sh
grep -q 'src/lib.rs:5:' "$scratch/rejection.log"
printf '#[test]\nfn version() { assert!("1.2.3" == actual); }\n' > src/lib.rs
reject bash scripts/check-test-versions.sh
printf 'fn runtime() { assert_eq!(actual, "1.2.3"); }\n' > src/lib.rs
bash scripts/check-test-versions.sh
# Argument failures still precede mutations and hook checks.
for args in 'minor 2.0.0' 'invalid' '01.2.3'; do
  read -r first second <<< "$args"
  reject bash scripts/bump-version.sh "$first" "${second:-}"
  grep -Eq 'Choose BUMP or VERSION|Expected major, minor, patch' "$scratch/rejection.log"
done
export VERSION_FIXTURE_FAIL=metadata
reject bash scripts/bump-version.sh '' 2.4.6
grep -q 'Version verification failed' "$scratch/rejection.log"
grep -q 'version = "2.4.6"' Cargo.toml
unset VERSION_FIXTURE_FAIL
bash scripts/bump-version.sh '' 3.5.7 > "$scratch/success.log"
grep -q 'version literals re-checked' "$scratch/success.log"
grep -q '^metadata --offline --format-version 1$' "$VERSION_FIXTURE_LOG"
! git rev-parse --verify HEAD >/dev/null 2>&1
[[ -z $(git tag) ]]
printf '%s\n' 'Version fixtures passed: literal comparisons, historical prose, derived versions, fixture versions, inline tests, argument errors, visible failed bumps, no commit/tag, absent/misconfigured/non-executable hooks.'
