# rust-sitter-mcp

A read-only [Model Context Protocol](https://modelcontextprotocol.io) server for structural Rust refactoring — built for coding agents, useful to anyone who edits Rust.

It finds and rewrites Rust **syntactically** (tree-sitter), preserves every comment, attribute, and byte outside the edit, and emits **git-apply-compatible patches** your agent (or you) can review and apply. The server never writes to your source tree.

## Tools

| Tool | What it does |
|---|---|
| `search` | Structural search with SSR-style sugar patterns: `$a.unwrap()`, `$a.expect($b)`, `pair($a, $a)` (repeated names must match byte-identical source) |
| `search_query` | Raw [tree-sitter queries](https://tree-sitter.github.io/tree-sitter/using-parsers/queries) for anything the sugar grammar doesn't cover |
| `replace` | Structural search-and-replace → dry-run plan: exact template substitution, all-or-nothing patches, trivia decisions surfaced with overrides |
| `move_item` | Lift & shift: move whole top-level items to existing files or new sibling modules (2018 layout, `mod x;` synthesis/reuse, file-creation patches) |
| `suggest_split` | Advisory file-splitting: complete item inventory, cohesion signals, 1–2 explained partition drafts — you edit and execute via explicit `move_item` batches |

Every mutating tool returns a **plan** (`applicable` / `blocked` / `incomplete`), a **unified diff**, and a **JSON edit list** that reconstructs the same bytes. Ambiguous situations (comment ownership, import/visibility rewrites under shadowing or glob imports, parse recovery) are surfaced as explicit, overridable decisions — never guessed. See [`docs/tools.md`](docs/tools.md) for the complete contracts.

## Safety model

- **Read-only.** No tool writes to your repository; patches are applied externally by you or your agent.
- **Lossless.** Untouched bytes stay byte-identical; moved items carry their comments with them; line endings and formatting are never normalized.
- **Honest.** Results report completeness, coverage, and skips precisely. Matching is syntactic: `semantic: "not_performed"` in every plan — the server does not claim your code compiles after a patch (though the test suite compiles fixture patches to verify them).

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
make install        # installs hooks + gate tools, verifies, builds and installs the binary
# or just the binary:
cargo install --locked --path .
```

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
