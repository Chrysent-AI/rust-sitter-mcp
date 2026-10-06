---
name: split-rust-code
description: Split or reorganize Rust files into modules using the rust-sitter-mcp MCP server. Use when asked to break up a monolithic Rust file, extract functions into their own module, move items between files, or reorganize crate structure. Handles the full suggest-split, review, move, verify patch workflow.
---

# Split Rust Code into Modules

Use the rust-sitter-mcp MCP server to split, extract, or reorganize Rust code. Discover `suggest_split` and `move_item` before use. In Pi, MCP exposure depends on configuration: use `tool_search` when the tools are not already exposed (deferred/codemode tools can be loaded this way); in other harnesses, follow that harness's guidance for loading MCP tools. Invoke the exact names and schemas exposed by the client (prefixes vary). The server is **read-only**: advice has no patch, and applicable move plans return patches you review and apply externally.

**Not for:** semantic renames (requires type knowledge), inline `mod x { ... }` body extraction, cross-crate moves, macro-body refactoring, or same-file reordering. The engine is syntactic only — it never claims your code compiles.

## Workflow checklist

- [ ] 1. Call `suggest_split` on the target file → get inventory + drafts
- [ ] 2. Triage advice completeness, review drafts, edit groups/filenames
- [ ] 3. Construct `move_item` batch with exact item anchors
- [ ] 4. Review the plan (rewrites, decisions, trivia)
- [ ] 5. If `applicable: true`, save and apply the patch externally
- [ ] 6. Run `cargo check` from the affected Cargo project

## Step 1: Get split advice

```json
{
  "repo_path": "/absolute/path/to/project",
  "crate_root": "src/lib.rs",
  "source_path": "src/big_file.rs"
}
```

- `repo_path`: absolute path to the repository (must be a Git worktree)
- `crate_root`: a Rust source file that declares the module tree (NOT Cargo.toml). For a library crate use `src/lib.rs`; for a binary use `src/main.rs`. If the file sits in a subdirectory, use the root that actually declares its chain.
- `source_path`: the file to analyze (Git-root-relative)

Use `paths` to keep discovery to the smallest useful scope; include `crate_root`, the source, and any files needed for module-chain or written-binding evidence. Omitting `paths` permits repository-wide discovery (large repos can return enormous decision lists); narrowing away required evidence makes the advice incomplete.

## Step 2: Triage and edit the advice

Before choosing a draft, inspect `status`, `coverage`, `counts.omissions`, `truncation_reasons`, and `draft_eligibility`. Failed or incomplete advice requires recovery or explicit acknowledgment of its limits — do not treat it as a complete partition.

- `inventory[]`: every top-level unit with `id`, `kind`, `name`, `span.range` (byte offsets). Unnamed impls have `name: null` — handle them by ID.
- `drafts[]`: up to two proposals. Inspect `groups[].rationale`, `groups[].confidence` (confidence is per group, not per draft), sizes, and warnings.
- `decisions[]`: items needing attention (cross-references, visibility, ambiguity).

With no drafts, read `draft_eligibility.reasons` and `chain_diagnostics`: this can mean recovery, insufficient items, or unsupported layout — not necessarily the absence of a useful conceptual split.

Drafts are advisory; you choose the final grouping:
- Join `drafts[].groups[].item_ids` to `inventory[].id` (IDs, not names, are keys)
- Move items between groups, change destination names, or retain items
- Retain a unit by omitting it from `moves`. `eligibility: "supported_unit"` proves only its kind, not move safety. Context-sensitive uses/modules/macros are inventoried, not automatically movable.
- Name modules after the actual concept (`parser`, `imports`, `sync`), not `part_2` or `utils`

Read `references/rust-modularization.md` before choosing destination names or deciding what to group.

## Step 3: Construct move anchors

For each item to move, build its anchor from the **exact original bytes** and byte range in the inventory:

```python
from pathlib import Path
by_id = {item["id"]: item for item in advice["inventory"]}
root = Path(advice["root"])  # canonical Git root returned by the tool
moves = []
for item_id, destination in chosen_groups:
    item = by_id[item_id]
    r = item["span"]["range"]
    raw = (root / item["path"]).read_bytes()
    text = raw[r["start_byte"]:r["end_byte"]].decode("utf-8")
    moves.append({
        "item": {"path": item["path"], "range": r, "expected_text": text},
        "destination": destination
    })
```

If `span.text` is `null` or `text_omitted: true`, you must obtain complete bytes this way — display text may be truncated.

Submit all moves as **one batch** (the server plans them together, all-or-nothing):

```json
{
  "repo_path": "/absolute/path/to/project",
  "crate_root": "src/lib.rs",
  "paths": ["src"],
  "moves": [ ... ]
}
```

Destination rules:
- `{"kind": "existing", "path": "src/destination.rs"}` — append to an existing admitted file; optional `before_item` anchor from that file
- `{"kind": "new_sibling", "path": "src/new_module.rs", "parent_path": "src/lib.rs"}` — create a new file. The path must be a literal `.rs` sibling of **every** assigned source, in a directory that already exists.
- `parent_path` is the ordinary declaring parent, not automatically the source: for `src/source.rs` → `src/moved.rs` it is normally `src/lib.rs`; non-root `foo.rs` declares children in `foo/`.
- Scope (`paths`) must include root, source, destination/parent, and chain/binding evidence. Admit a new file through its existing directory, never its absent filename.
- Build destinations from the accepted request fields only. Advisory `group.destination` also contains `parent_module` and `existing_declaration`; do not copy it verbatim — the strict schema rejects unknown fields.

## Step 4: Review the plan

**Require `plan.applicable: true` AND non-null `edits`, `created_files`, `patch`.** A `status: "complete"` envelope does NOT mean the plan is applicable; blocked plans withhold all three artifacts, and preview rewrites are informational only.

Check:
- `rewrites[]`: every import/path/visibility repair (review `before_text`/`after_text`)
- `decisions[]` and `blockers[]`: read each `action.route`, `action.field`, `action.purpose` — see `references/failure-recovery.md` for routing
- `trivia_decisions[]`: comment/banner ownership choices
- `patch`: the git-apply diff

## Step 5: Apply the patch

Save the complete decoded `plan.patch` unchanged (outside the source tree), then from the returned Git root:

```bash
git apply --check patch.diff
git apply patch.diff
```

Keep or recheck original base bytes/modes plus `created_files[].must_be_absent` before application. `git apply --check` verifies patch applicability, not that the whole analyzed base is unchanged — if anything shifted, regenerate the plan; never discard user edits. Finally run `cargo check` (and relevant tests) from the affected Cargo project or workspace.

## Gotchas

- **`crate_root` is NOT `Cargo.toml`** — it's a `.rs` source file. A wrong root (e.g., `main.rs` when the tree hangs off `lib.rs`) surfaces in `chain_diagnostics` and linked decisions; inspect them.
- **Anchors go stale after any source edit** — `STALE_SELECTION` means re-read current bytes and rebuild anchors. Never guess offsets.
- **Items inside inline `mod x { ... }` blocks cannot be extracted** — only direct top-level units are movable.
- **The engine serializes calls** — a second concurrent call returns `BUSY`. Wait, then retry.
- **Trailing newlines are not part of an item's syntax anchor** — inventory byte ranges exclude them.
- **A complete draft is not move safety** — membership completeness and eligibility say nothing about cross-references, visibility, or macro context. That analysis happens in `move_item`.

## Reference files

- `references/failure-recovery.md` — read when a call fails, a plan is blocked, or advice is incomplete: triage rule, common error codes, typed decision routing, override syntax
- `references/rust-modularization.md` — read before choosing destination names or grouping: naming, visibility, layout conventions, capability caveats
