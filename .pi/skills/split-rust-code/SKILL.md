---
name: split-rust-code
description: Split or reorganize Rust files into modules using the rust-sitter-mcp MCP server. Use when asked to break up a monolithic Rust file, extract functions into their own module, move items between files, or reorganize crate structure. Handles the full suggest-split, review, move, verify patch workflow.
---

# Split Rust Code into Modules

Use the rust-sitter-mcp MCP server to split, extract, or reorganize Rust code. Discover `suggest_split` and `move_item` before use. In Pi, MCP exposure depends on configuration: use `tool_search` when the tools are not already exposed (deferred/codemode tools can be loaded this way); in other harnesses, follow that harness's guidance for loading MCP tools. Invoke the exact names and schemas exposed by the client (prefixes vary). The server is **read-only**: advice has no patch, and applicable move plans return patches you review and apply externally.

**Not for:** semantic renames (requires type knowledge), inline `mod x { ... }` body extraction, cross-crate moves, macro-body refactoring, or same-file reordering. By default the engine is syntactic only — it never claims your code compiles. Opt-in evidence flags (Step 3) add bounded, explicitly-configured resolution without ever claiming compilation or equivalence.

**Set expectations first.** On idiomatic Rust — functions calling methods, prelude types (`Option`, `String`), `#[derive]`/proc-macro attributes, macro invocations (`println!`, `vec!`, `format!`), or consumers inside inline `#[cfg(test)]` modules — most selections block with `unsupported_in_engine` routes. For such files the realistic deliverable is a **design report** (inventory + grouping), not an applicable patch; the engine's sweet spot is macro-free library units (plain consts, undecorated items). `supported_unit` eligibility counts overstate what is actually movable. A file whose bulk is one large `impl` cannot be split by whole-item moves at all — methods are not items.

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

Use `paths` to keep discovery to the smallest useful scope; include `crate_root`, the source, and any files needed for module-chain or written-binding evidence. Omitting `paths` permits repository-wide discovery (large repos can return enormous decision lists); narrowing away required evidence makes the advice incomplete. For a `new_sibling` destination, admit its **directory literal** — e.g. creating `src/tui/new.rs` means `paths: ["src/lib.rs", "src/tui"]` — never the absent filename (`PATH_NOT_FOUND`, `INVALID_DESTINATION`). Note: cross-module binding decisions are usually a minority; most decisions on a big file are intra-file, so narrowing `paths` often does NOT shrink the response.

**Response budget:** current advice preserves full drafts and root/draft summaries at default budgets, including on large files. Still inspect `status`, coverage, omissions, and truncation reasons. If a response explicitly withholds data for `response_bytes`, retry with `limits: {"response_bytes": 8388608}` (up to 16 MiB). Distinguish server omissions from a client's truncated preview — inspect the saved full response.

## Step 2: Triage and edit the advice

Before choosing a draft, inspect `status`, `coverage`, `counts.omissions`, `truncation_reasons`, and `draft_eligibility`. Failed or incomplete advice requires recovery or explicit acknowledgment of its limits — do not treat it as a complete partition.

- `inventory[]`: every top-level unit with `id`, `kind`, `name`, `span.range` (byte offsets). Unnamed impls have `name: null` — handle them by ID.
- `drafts[]`: up to two proposals. Inspect `groups[].rationale`, `groups[].confidence` (confidence is per group, not per draft), sizes, and warnings.
- `decisions[]`: items needing attention (cross-references, visibility, ambiguity).

With no drafts, read `draft_eligibility.reasons` and `chain_diagnostics`: `response_bytes` means raise the response budget (above); a `no_admitted_nonconflicting_name…` reason usually means the file is already modularized or has too few movable units — a no-draft result there is a legitimate "nothing to do". A `destination: null` group in a draft is the **retain set** — those items stay; omit them from `moves`.

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
root = advice["root"]  # canonical Git root; if null, the response was truncated — recover the budget first
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

Run the extraction as a script — **never transcribe `expected_text` from display output**; truncation and multibyte characters will corrupt it (`STALE_SELECTION`).

If `span.text` is `null` or `text_omitted: true`, you must obtain complete bytes this way — display text may be truncated.

**Probe before large batches.** Submit 1–2 representative items first. If the probe blocks with any `unsupported_in_engine` route — `member_or_constructor_unproved`, `external_or_missing_binding`, `macro_context_unexamined`, `lexical_context_unproved`, `glob_binding_unproved`, `conditional_or_inherited_context` — on the items' own bodies (method calls, prelude/external types, macro invocations, `#[derive]` attributes) **or their consumers** (call sites inside macros, `tokio::select!`, inline test modules), every similar item will block too: that is the syntactic-only boundary, not a recoverable error. Rethink the selection or produce a design report before spending a large call.

Submit all moves as **one batch** (the server plans them together, all-or-nothing):

**Optional `move_item` evidence flags** (default off): `assume_standard_prelude` proves only unshadowed type-position `Option`/`Result`/`Box`/`Vec`/`String` and bare built-in derives; inspect `standard_prelude` / `standard_builtin_derive` proofs and coverage. `resolve_semantic` requires an explicit `semantic_configuration` (admitted crate graph with editions/features/cfg — no sysroot or external paths); bounded rust-analyzer resolution can prove eligible inherent methods, fields, constructors, and access at both source and final overlay — never compilation or equivalence. `acknowledge_test_consumers` discloses only consumers in the moved file's exact inline `#[cfg(test)]` module (including macro arguments) as nonblocking risks; it does not validate tests. Review proof/coverage and acknowledgment decisions; unrelated blockers still block.

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
- `binding_proofs[]` and coverage: review `standard_prelude`, `standard_builtin_derive`, or `ra_resolved` evidence; `integrity.semantic: "resolution_performed"` means bounded resolution only, never compilation/equivalence.
- `decisions[]` / `blockers[]`: inspect routes, fields, purposes, and choices. A `test_consumer_acknowledged` decision is visible but nonblocking, not proof tests pass; run relevant tests. `lexical_uncertainty.witness_relation: "hoist_possibility"` marks a positional same-block statement-macro witness, not a pattern containing the anchor (it may occur later). The existing test-consumer opt-in can acknowledge that risk for a bare reference with a written super import and no other written binding uncertainty; the witness stays visible. Other lexical uncertainties, known binding conflicts, stale anchors, and structural failures still block.
- `rewrites[]`: review every import/path/visibility repair (`before_text`/`after_text`).
- `removal_gap` decisions: bytes stay as-is by default; collapse only via the explicit supported override after reviewing the exact whitespace.
- `trivia_decisions[]`: review comment/banner ownership.
- `patch`: review the complete git-apply diff and insertion locations.

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
- **Some MCP clients stringify object parameters** — if a call fails validation with `must be object` (or you see `"limits": "null"` in the error echo), your client serialized `context`/`limits`/`globs` or an explicit `null` into a JSON string. Omit optional object parameters entirely; server defaults apply. Never pass explicit nulls.
- **A complete draft is not move safety** — membership completeness and eligibility say nothing about cross-references, visibility, or macro context. That analysis happens in `move_item`.
- **Chain refusals may name `macro_generated_module_tree` or `root_attribute_chain_uncertainty`** — these are advice-only causes, not override opportunities. Correct the root/scope where possible or stop at a design report.
- **Macro-generated crate roots** (`crate_root!()` / `OUT_DIR` includes) cannot supply a proved ordinary module chain; treat these selections as advice-only.
- **Creating `tests/*.rs` siblings can add Cargo integration-test crates** — module validity cannot see autotest discovery; review the build graph externally.
- **Macro-invocation-as-item files are unsplittable like one-large-impl files** — generated items are not written inventory units; use advice, not an applicable-split claim.

## Reference files

- `references/failure-recovery.md` — read when a call fails, a plan is blocked, or advice is incomplete: triage rule, common error codes, typed decision routing, override syntax
- `references/rust-modularization.md` — read before choosing destination names or grouping: naming, visibility, layout conventions, capability caveats
