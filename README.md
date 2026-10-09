# rust-sitter-mcp

A read-only [Model Context Protocol](https://modelcontextprotocol.io) server for structural Rust refactoring — built for coding agents, useful to anyone who edits Rust.

It finds and rewrites Rust **syntactically** (tree-sitter), preserves every comment, attribute, and byte outside the edit, and emits **git-apply-compatible patches** your agent (or you) can review and apply. Whole-item moves can additionally opt into bounded rust-analyzer resolution over explicitly configured, admitted source texts. The server never writes to your source tree.

## Tools

| Tool | What it does |
|---|---|
| `search` | Structural search with SSR-style sugar patterns: `$a.unwrap()`, `$a.expect($b)`, `pair($a, $a)` (repeated names must match byte-identical source) |
| `search_query` | Raw [tree-sitter queries](https://tree-sitter.github.io/tree-sitter/using-parsers/queries) for anything the sugar grammar doesn't cover |
| `replace` | Structural search-and-replace → dry-run plan: exact template substitution, all-or-nothing patches, trivia decisions surfaced with overrides |
| `move_item` | Lift & shift: move whole top-level items or supported inherent members to existing files, new siblings or explicit private children (ordinary layout, `mod x;` synthesis/reuse, complete file-creation patches). Child-containing plans use schema 3 with linked directory preconditions, including one absent conventional flat-parent directory; the server creates nothing. Opt-in discharge flags: `assume_standard_prelude` (std prelude names + built-in derives, both-ends shadow refusal) and `resolve_semantic` (rust-analyzer `ra_resolved` proofs over an explicit crate graph; no Cargo discovery or caller-code execution) |
| `suggest_split` | Advisory file-splitting: complete item inventory, cohesion signals and explained partition drafts with consequence/test-coupling scope; opt-in compact manifests and immutable retention — you edit and execute via explicit `move_item` batches |
| `get_split_detail` | Bounded repeatable historical pages and complete original unit/header anchors from explicitly retained advice, or release its process-local record; no live freshness check, reanalysis or move submission |
| `export_move_request` | One inactive ordinary exact-anchor move request from explicit retained unit/destination entries after observational source/scope/policy freshness checks; no implicit companions, submission or applicability claim |

Since **v0.5.0**, `suggest_split` omitting `include_balanced` or setting it false
excludes byte-balanced alternatives. Set `include_balanced:true` to request the
existing distinct low-confidence original-order alternative when structural
advice and layout are valid; there is no two-proposal promise or balanced primary
for a no-credible result. Incomplete analysis and unsupported layout remain
separate, and advice never relaxes move proofs. Inspect status, coverage and
omissions: complete analysis, capped display and genuine partial advice differ.

Explicit advice retention defaults to one active record, 134,217,728 aggregate
accounted bytes (128 MiB), and a fixed non-sliding 900-second lifetime from
publication. Preparations and in-flight readers count against capacity;
conservative reservation can refuse before final stored charges would fit.
The cap is not allocated upfront or a repository-size, response-byte or process-RSS
limit. Release is explicit, there is no silent eviction, and restart loses records.

Every mutating tool returns a **plan** (`applicable` / `blocked` / `incomplete`), a **unified diff**, and a **JSON edit list** that reconstructs the same bytes. Ambiguous situations (comment ownership, import/visibility rewrites under shadowing or glob imports, parse recovery) are surfaced as explicit, overridable decisions — never guessed. See [`docs/tools.md`](docs/tools.md) for the complete contracts.

This repository also ships a pi agent skill at [`.pi/skills/split-rust-code/`](.pi/skills/split-rust-code/SKILL.md) — pi users get a guided end-to-end workflow (advice triage, anchor construction, failure recovery) automatically.

## Safety model

- **Read-only.** No tool writes to your repository; patches are applied externally by you or your agent.
- **Lossless.** Untouched bytes stay byte-identical; moved items carry their comments with them; line endings and formatting are never normalized.
- **Honest.** Results report completeness, coverage, and skips precisely. Default plans keep `semantic: "not_performed"`. Opt-in move resolution uses `ra_resolved` occurrence proofs and `semantic: "resolution_performed"`, scoped to one explicit crate graph/configuration and the exact final virtual batch. Neither mode claims compilation or semantic equivalence (the test suite separately compiles fixture patches). No Cargo discovery, caller build scripts or proc macros are executed.

## Install

Requires: Rust toolchain per [`rust-toolchain.toml`](rust-toolchain.toml), Git 2.39+, and a C11 compiler for the bundled tree-sitter grammars.

Install the binary with cargo (from this repository):

```sh
cargo install --locked --git https://github.com/sdkks/rust-sitter-mcp
```

Or from a checkout:

```sh
git clone https://github.com/sdkks/rust-sitter-mcp.git
cd rust-sitter-mcp
make install        # builds and installs the release binary (no tests, no gate tools)
```

Contributors: run `make setup` instead — it additionally installs the pinned gate
tools and Git hooks and runs the full verification suite before installing.

The server speaks MCP over stdio and takes no arguments — every tool call names its target via `repo_path`.

## Configure (MCP client)

The server is a plain stdio binary (`rust-sitter-mcp`), so any MCP client that launches local servers works. Pick your harness:

### pi

```sh
pi mcp add rust-sitter-mcp -- rust-sitter-mcp
```

Or edit `~/.pi/agent/mcp.json` (user) or `.pi/mcp.json` (project):

```json
{
  "mcpServers": {
    "rust-sitter-mcp": { "command": "rust-sitter-mcp" }
  }
}
```

### Codex CLI

```sh
codex mcp add rust-sitter-mcp -- rust-sitter-mcp
```

Or in `~/.codex/config.toml`:

```toml
[mcp_servers.rust-sitter-mcp]
command = "rust-sitter-mcp"
```

### Claude Code

```sh
claude mcp add rust-sitter-mcp -- rust-sitter-mcp
```

### Cursor

Edit `~/.cursor/mcp.json` (global) or `.cursor/mcp.json` (per project):

```json
{
  "mcpServers": {
    "rust-sitter-mcp": { "command": "rust-sitter-mcp" }
  }
}
```

### Zed

In `~/.config/zed/settings.json`:

```json
{
  "context_servers": {
    "rust-sitter-mcp": {
      "command": "rust-sitter-mcp",
      "args": []
    }
  }
}
```

### Windsurf

In `~/.codeium/windsurf/mcp_config.json`:

```json
{
  "mcpServers": {
    "rust-sitter-mcp": { "command": "rust-sitter-mcp" }
  }
}
```

### VS Code (GitHub Copilot)

In `.vscode/mcp.json` (workspace):

```json
{
  "servers": {
    "rust-sitter-mcp": {
      "type": "stdio",
      "command": "rust-sitter-mcp"
    }
  }
}
```

### Any other stdio MCP client

Any client that launches local stdio servers works — the command is `rust-sitter-mcp` with no arguments. Client schemas differ (e.g. VS Code uses `servers` + `type`; Cursor/Windsurf use `mcpServers`); follow your client's local-server documentation for the entry shape.

Every tool call takes an explicit `repo_path` (any file or directory inside the target Git worktree); the server resolves the canonical root and never falls back to ambient directories.

## Quick example

Ask your agent:

> Find every `x.unwrap()` in this repo and replace with `x.unwrap_or_default()`, show me the patch first.

Under the hood: `search` with pattern `$a.unwrap()` → inspect matches with byte-exact ranges → `replace` with pattern `$a.unwrap()`, replacement `$a.unwrap_or_default()` → review the returned diff → `git apply`.

## Platform support

| Platform | Status |
|---|---|
| macOS (Apple Silicon) | Validated locally (full suite + gates) |
| Linux amd64 | Validated in CI |
| Linux arm64 | Validated in CI (and in Ubuntu 24.04 containers) |

Windows is not supported. macOS is intentionally **not** built in CI (cost); it is covered by the same gated workflow locally.

## Development

See [CONTRIBUTING.md](CONTRIBUTING.md) — short version: `make install-hooks`, commit through the gates, keep the dependency log current. The repository enforces formatting, clippy `-D warnings`, tests, `cargo-deny`, `cargo-machete`, secret scanning, and a dependency-research log via Git hooks.

## License

[MIT](LICENSE)
