# AGENTS.md — rust-sitter-mcp

Guidance for all agents (and humans) working in this repository.

## Dependency policy (binding for all work in this repo)

Every change that adds or re-pins a dependency requires a short research step BEFORE the Cargo.toml change lands. No exceptions for "small" or "obvious" deps — the note can be one line, but it must exist.

1. **Verify currency and maintenance** — `cargo info <crate>` plus crates.io metadata (updated_at, release cadence, downloads, yanked versions). A crate untouched for ~12 months, or superseded by a maintained alternative, is presumed rejected until proven otherwise. Do not copy pins from other projects (including our reference repos) without re-verification — pins age fast.
2. **Know the alternatives** — identify the top 2–3 candidates for the slot (use web search where available). Record why the chosen crate wins and what the runner-up loses. "No real alternative exists" is an acceptable, recordable answer; "we didn't look" is not.
3. **Check the fine print** — license compatibility, MSRV vs our toolchain, the minimal feature-flag set (disable the rest), transitive weight (`cargo tree -e no-dev`), and pin strategy with rationale (exact/tilde/caret; 0.x crates get a pin + deliberate-upgrade policy — e.g. tree-sitter, whose 0.x minors break ABI/API).
4. **Log it** — append an entry to `docs/dependency-log.md` in the SAME commit that changes the dependency: date, crate, pinned version, slot/purpose, candidates considered (with links), and the decision rationale. One entry per dependency decision; dev-dependencies follow the same bar with a shorter note.

**Mechanical enforcement** (wired at tooling setup): `cargo-deny` (advisories/licenses/sources) and `cargo-machete` (unused deps) run as pre-commit gates. A Cargo.toml dependency change without a matching `docs/dependency-log.md` entry in the same commit is rejected (pre-commit heuristic + review convention).

**Preferences:**
- Well-known, actively maintained crates; official/organization-backed where a genuine choice exists (e.g. `rmcp` as the official MCP SDK).
- Prefer std or zero-dependency solutions where genuinely sufficient — every dependency is paid for again in review, build time, and supply-chain risk.
- 0.x dependencies only when they are the ecosystem standard for the slot (tree-sitter qualifies); pin and upgrade deliberately with a recorded reason.
