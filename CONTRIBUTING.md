# Contributing

This package currently provides the Rust library/binary skeleton and `--help` / `--version`, not an MCP server implementation.

## Developer setup

Use Git 2.39+, Make, Bash 3.2+, POSIX awk and rustup/Cargo. The tested toolchain is **Rust 1.97.1** (edition 2024), pinned in `rust-toolchain.toml` with rustfmt and Clippy. Build tooling targets macOS Apple Silicon and Ubuntu; native Windows is not claimed. Later parser builds also need a C11 compiler and archive tools.

```sh
make install-tools       # cargo-deny 0.20.2 and cargo-machete 0.9.2, locked installs
make install-hooks       # activate tracked .githooks; safe to run twice
make build
make version
```

`make install` sequentially ensures components/tools, installs hooks, fetches the locked graph, runs the full gate, builds and installs the binary with Cargo, and prints its version. Tool/dependency installation and the advisory database refresh may require network access. This is developer setup, not runtime behavior. `make help` lists all commands.

The hook installer sets only the local `core.hooksPath=.githooks`, ensures hook executable bits, and refuses an existing conflicting hook manager. Relative hooks resolve within the active checkout, including linked worktrees. It does not replace `.git/hooks` files. Fresh clones must install the hooks deliberately.

## Commits and required gates

Stage all intended changes before committing: hooks reject tracked unstaged changes, nonignored untracked paths and staged whitespace errors. Checks evaluate the disk tree, which must equal the staged tree. No documentation-only shortcut, automatic bypass, or missing-tool exemption exists.

Subjects follow `type(scope)!: description`: scope and `!` are optional; scope is lowercase alphanumeric plus `.`, `_`, `-`; description is 1–72 characters. Types: `feat`, `fix`, `docs`, `style`, `refactor`, `perf`, `test`, `build`, `ci`, `chore`, `revert`. Git-generated `Merge ...` and `Revert "...` subjects are exempt. Example: `build: establish Rust tooling and package skeleton`.

`make quality` and pre-commit share these fail-closed gates:

1. Pinned toolchain, rustfmt/Clippy, cargo-deny and cargo-machete availability/version checks.
2. `cargo fmt --all -- --check`.
3. `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`.
4. `cargo test --workspace --all-features --locked --quiet`.
5. `cargo deny check` (advisories, approved licenses, allowed sources; duplicate versions warn).
6. `cargo machete` (no blanket unused-dependency suppression).
7. Bash syntax checks and disposable tooling regression fixtures.

Pre-commit additionally checks staged dependency changes against added staged entries in `docs/dependency-log.md`. Package-version and metadata-only edits do not trigger it. Normal/dev/build/target-specific dependency entries, including their inline feature changes, are compared without hunk-context false positives; package feature-table changes conservatively require fresh notes for the affected graph. Added headings must name the crate and its resolved version in the staged lockfile. Unsupported dependency TOML syntax fails closed for review; prefer one-line entries or per-dependency subtables. This heuristic checks co-staging, not research quality. Review still checks each decision.

See [AGENTS.md](AGENTS.md) for the dependency policy and [the dependency log](docs/dependency-log.md) for research. No Cargo dependencies should be added speculatively; add them with first use and a matching decision. Tool installations are separate from application dependencies.

Post-commit prints one informational SHA receipt. It never cleans build output, edits files, creates tags, or performs network operations.

## Version changes and build identity

```sh
make bump-version                  # patch by default
make bump-version BUMP=minor       # or major / patch / x.y.z
make bump-version VERSION=0.2.0    # exact stable version; do not also set BUMP
```

Only the single `[package]` version is edited; dependency/table version assignments remain unchanged. `cargo check --offline` refreshes the package's lock entry without updating dependency resolutions. Invalid input fails before replacing the manifest. A failed Cargo check leaves a visible, recoverable manifest/lock diff. Inspect and commit both files deliberately; no tag, fetch, push, or release happens automatically.

`--version` prints `rust-sitter-mcp <package-version> (<first-12-hex-HEAD-digits>)`. The SHA is embedded at build time and denotes HEAD, not dirty source. HEAD, packed refs and the active branch ref are tracked so a later commit refreshes the identity without `cargo clean`. Detached HEAD and linked worktrees work. If Git metadata or a tracked package manifest is unavailable (including an archive inside an unrelated repository), the marker is `unknown`; no Git process runs at binary startup.

## Tooling tests

```sh
make test-tooling
make quality
```

The fast tooling suite uses isolated `/tmp` Git repositories, paths with spaces, and explicit local Cargo test doubles to exercise installation twice/conflicts, every failing/unavailable gate, staging parity, commit-message rejection, dependency-note changes and harmless receipts. It does not recursively run the parent suite. These test doubles test hook control flow, not actual Rust lint/audit correctness; the real gates run separately against this package.

The build-identity/version regression suite uses real Cargo in disposable copies:

```sh
bash scripts/test-build-identity.sh
```

It checks version bumps and stable lock resolutions, SHA refresh without cleaning, detached HEAD, linked worktrees and archive fallback. It is a focused acceptance test, separate from the fast pre-commit gate to avoid repeated fixture compilation.
