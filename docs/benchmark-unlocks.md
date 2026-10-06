# Frozen move-batch unlock measurements

## Reproduction

The stdlib-only Python 3.9+ rig compares two trusted stdio server binaries.
Run from any directory; the default corpus is resolved relative to the script:

```sh
scripts/replay-unlocks ~/.cargo/bin/rust-sitter-mcp target/debug/rust-sitter-mcp
scripts/replay-unlocks ~/.cargo/bin/rust-sitter-mcp target/debug/rust-sitter-mcp opi
python3 scripts/test-replay-unlocks.py
```

The optional positional filter matches batch IDs, codebases and human labels.
`--json` emits the same measurements as structured JSON, including original
category/reason counts, coverage and display omissions. `--timeout 120` bounds
each JSON-RPC request. `--corpus DIR` selects a corpus directory. Use
`--repo target=/absolute/path/to/opi` (repeatable for other codebases) to relocate
the original snapshots without regenerating anchors. Missing repositories,
changed frozen source hashes, stale anchors, incomplete analysis, RPC failures
and timeouts exit nonzero: they are **not** reported as zero unlocks.

Each checked-in JSON under `tests/fixtures/replay-corpus/` contains a human
label, codebase, recorded source commit, absolute repository path inside the
request, frozen SHA-256 inputs and the full original `move_item` arguments.
The 30-item headless request was copied unchanged from
`/tmp/target-args-moves.json`; the two-item bounded variant is its exact subset.
Other anchors were constructed from `suggest_split` inventory byte ranges,
then sliced directly from the original UTF-8 bytes, not inventory display text.
Every request was verified by actual read-only replay against both binaries.
No corpus patches are applied and no Cargo command runs on target repositories.

| Codebase | Frozen batch | Selected items | Selected bytes |
| --- | --- | ---: | ---: |
| Target app | headless argument extraction | 30 | 9,352 |
| Target app | bounded_required + bounded_optional | 2 | 273 |
| Target app | app redraw helpers | 3 | 531 |
| Target app | app Drop implementation + signal helper | 2 | 848 |
| Target app | cmd_config + cmd_ticket + cmd_spec | 3 | 8,988 |
| Target app | cmd_mcp + cmd_mcp_status + cmd_mcp_config | 3 | 6,849 |
| rust-sitter-mcp | Template + CopyOrigin + Expansion | 3 | 254 |
| duct.rs | executable-path, relative-path and error helpers | 3 | 2,631 |

Target app source commit: `ba6b1003dac17139100ace3377a096f92a07bff5`.
Engine source commit: `7fa1564bf19f36e0b2af9c88a4ffa4e5513daa7a`.
External proc-macro-light `duct.rs` source commit:
`ba535d51f5e9912cc3a6bfbd8be80d7516305f8b`.
The corpus records exact scopes/globs, including admitted test consumers.
The original observation below samples eight batches, not every possible split
or a random population. Three additional serde negative controls are frozen at
`6693a89cca77e0151437da1c7f890090b9ebf04c`: `get_lit_str` + `get_lit_str2`
(1,085 bytes), `is_str` + `is_slice_u8` (229 bytes), and `missing_field` from
`serde/src/private/de.rs` (1,136 bytes). Replay just these with the `serde` filter
(or relocate with `--repo serde=/absolute/path/to/serde`). Their optional
`negative_control` metadata asserts zero applicability and exact blocking
category/reason classes for both binaries; candidate runs also assert the named
chain-reason set. Baselines may predate those additive names. Counts are measured,
not frozen: the rig reports named-chain diagnostic counts separately and fails
nonzero on a silent unlock or class drift. External snapshots are required;
focused rig tests do not pretend to replay absent repositories.

## What is counted and observed

Four variants run for every batch: baseline-off, baseline-on, candidate-off,
candidate-on. “On” changes **only** `assume_standard_prelude` to `true`; “off”
explicitly sets it to `false`. Stdin stays open through the response, avoiding
server cancellation on EOF. Each variant initializes a fresh real server and
verifies structured/text response equality. Versions and binary SHA-256 hashes
identify the measured executables; replacement during replay fails the run.

Applicability is whole-batch `plan.applicable`, never inferred from decreasing
blocker counts. Applicable items/bytes are estimates equal to the request's
selected items/anchor bytes only for applicable plans; blocked batches contribute
zero. These byte estimates exclude carried trivia and generated/import repairs.
Counts sum `count` from **blocking decision groups**, not compact exemplars or
the number of groups. Segments are member/constructor, macro, external-binding,
conditional/derive, cfg-test/module-context, glob, and other. Module-chain
failures and category `module_context` take precedence over the conditional
segment. The engine's shared member/constructor reason does not distinguish
method calls from field reads, so the rig does not invent that distinction.
Residual blocker combinations count batches exhibiting each combination.
Deltas mean candidate minus baseline at the same flag setting; newly-applicable
and regressed batches are counted separately. Diagnostic-count and trivia-scope
omissions affect display only; all other truncations and non-complete analysis
are rejected. There is no compiler/semantic evidence: `semantic: not_performed`.

Before and after **each** request the rig hashes all Git-listed Rust source and
repository `.gitignore` files, plus explicit source, destination, declaration
parent and crate-root paths and ancestor ignore files (including absence).
It also compares read-only Git porcelain status. Returned base-file paths must
be covered by the observation. Added/deleted source files, source mutations,
ignore changes and creation of even an ignored proposed destination fail the
run immediately. Frozen hashes also detect added/removed files in recorded
path scopes. These checks detect persistent changes; they do not sandbox an
untrusted binary or detect write-then-restore. Run against quiescent repositories.
Ignored build products, credentials, Git administrative files and unrelated
ignored non-input files are not hashed. The rig never writes target repos.

## First observation: native macOS arm64, 2026-10-06

Baseline installed binary: `rust-sitter-mcp 0.2.0 (331959e578e5)`.
Candidate debug binary, rebuilt before replay:
`rust-sitter-mcp 0.2.0 (7fa1564bf19f)`.

| Batch | baseline-off | baseline-on | candidate-off | candidate-on |
| --- | --- | --- | --- | --- |
| duct-path-helpers-3 | false | false | false | false |
| opi-app-lifecycle-2 | false | false | false | false |
| opi-app-redraw-3 | false | false | false | false |
| opi-headless-args-30 | false | false | false | false |
| opi-headless-bounded-2 | false | false | false | false |
| opi-main-config-3 | false | false | false | false |
| opi-main-mcp-3 | false | false | false | false |
| rust-sitter-template-types-3 | false | false | false | false |

Target app: **0/6 applicable** in all four variants. Across all three codebases:
**0/8 applicable**, zero newly-applicable batches, zero regressed batches and
zero applicable-item/byte estimates. All 32 requests preserved the observed
source/ignore hashes, destination absence and Git status.

| Blocking occurrences, all batches | baseline-off | baseline-on | candidate-off | candidate-on | delta off | delta on |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| member/constructor | 259 | 259 | 259 | 259 | 0 | 0 |
| macro | 87 | 87 | 87 | 87 | 0 | 0 |
| external-binding | 190 | 190 | 190 | 185 | 0 | -5 |
| conditional/derive | 33 | 33 | 33 | 32 | 0 | -1 |
| cfg-test/module-context | 20 | 20 | 20 | 20 | 0 | 0 |
| glob | 6 | 6 | 4 | 4 | -2 | -2 |
| other | 157 | 157 | 157 | 157 | 0 | 0 |

Only the engine template sample changes blocker counts. Its member/constructor
blockers remain, so even fewer external/derive/glob blockers unlock no batch.
These observations are evidence for the pre-committed lane decision, not an
estimate of all Rust code or semantic correctness and not an automatic product
change. The 30-move and two-move target rows reproduce the known zero baseline.
