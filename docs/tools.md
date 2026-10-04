# Structural Rust search, replacement and move plans

`rust-sitter-mcp` runs a read-only stdio MCP service with no startup repository argument. It advertises primary `search`, the raw `search_query` escape hatch, dry-run `replace`, and explicitly selected `move_item` plans. It does not launch rust-analyzer, Cargo, a formatter, a network listener, or an external parser. Runtime requires local Git 2.39+; build/install prerequisites and gates are in [CONTRIBUTING.md](../CONTRIBUTING.md). Platform acceptance remains pending Ubuntu execution and the large-repository benchmark; local verification is on Darwin arm64.

A stdio launch configuration is:

```json
{"mcpServers":{"rust-sitter":{"command":"rust-sitter-mcp","args":[]}}}
```

Build with `cargo build --locked`; `target/debug/rust-sitter-mcp --version` reports the package version and embedded build revision. No Git version command runs at startup. Logging uses stderr, controlled by `RUST_LOG`; source/query/capture bodies are not logged. Stdout is exclusively newline-delimited MCP JSON-RPC while serving.

## Calls

Call primary `search` for written zero-argument unwrap calls:

```json
{"repo_path":"/absolute/project","pattern":"$a.unwrap()","paths":["src"]}
```

`$name` captures a single expression node; field/method names are concrete. Comments/whitespace between tokens do not prevent matching, but an explicit argument list has exact arity: the pattern above does not match `foo.unwrap(x)`. Captures are occurrence arrays under user names, with original ranges/text (no internal query captures). Repeated names require byte-identical source. Patterns are one expression without a statement semicolon; sequence placeholders, `@capture` annotations and `$$` escapes are unsupported. Dollars inside Rust literals/comments remain literal. The supported forms and positions are enumerated below; broader authored syntax uses `search_query`.

### Sugar grammar (`search`)

Exactly one supported Rust expression, without a trailing statement semicolon, is parsed using the pinned `tree-sitter-rust@0.24.2` grammar. Its native metavariable tokens are validated; strings/comments are never rewritten into placeholders.

| Authored roots and compositions | Metavariable positions |
|---|---|
| `$name`, concrete identifiers, `self`, concrete scoped paths; integer, float, boolean, char, string and raw-string literals (including byte/C prefixes) | Whole expression; dollars inside literals/comments are literal |
| Call/field expressions: `pair(...)`, `$a.unwrap()`, `$a.field`, `$a.0` | Whole callee, receiver/base, each argument; field/method/path names must be concrete |
| Parenthesized, unit, tuple, array and repeat-array expressions | Each expression child; repeat value/count are separate slots, never a list capture |
| Unary, reference, try, await, index expressions | Each expression operand/base/index; operators, mutability and punctuation are concrete |
| Binary, assignment, compound-assignment and range expressions | Each present expression operand; operator and omitted endpoint presence are concrete |

Only these forms can be authored, including within compositions. Authored blocks, control flow, closures, structs, casts, generics/turbofish, macros, and item/type/statement/pattern roots are rejected. A metavariable **may bind any one concrete expression node** from source, including a whole block, closure, generic call, cast or macro invocation. Macro token-tree contents are not interpreted or expanded. Concrete identifier/literal searches select expression positions, not binding names, field/path segments or macro tokens.

Names are ASCII `[A-Za-z_][A-Za-z0-9_]*`, excluding the internal `__ssr_` prefix. `$`, `${name}`, `$...`, sequence/optional placeholder syntax, partial-token placeholders and binding/type/lifetime/attribute/token-tree placeholder positions are rejected. `$a?` is ordinary Rust try syntax; `*$a` is dereference. There is no sugar `@capture` annotation. Literal dollars use ordinary Rust literals/comments; **there is no `$$` escape outside them**. Unsupported literal-dollar token-tree matching belongs to raw queries.

Structural matching compares the complete significant subtree: kind, ordered/field-labelled children, child presence, concrete leaf bytes and punctuation. Only comment extras and whitespace gaps are ignored. Literals are atomic and source-exact, including raw-string delimiters, prefixes, escapes and internal whitespace. Explicit lists require exact arity. Written trailing commas are significant: `pair($a,$b)` does not match `pair(x,y,)`; author `pair($a,$b,)` for that syntax. No semantic equivalence is inferred.

Worked calls to `search`:

```json
{"repo_path":"/absolute/project","pattern":"$a.unwrap()","paths":["src"]}
{"repo_path":"/absolute/project","pattern":"$a.expect($b)","paths":["src"]}
{"repo_path":"/absolute/project","pattern":"pair($a, $a)"}
```

Minimum fixtures:

| Pattern | Matches | Does not match |
|---|---|---|
| `$a.unwrap()` | `thing.unwrap()`, `thing.unwrap(/* comment */)` | `thing.unwrap(x)`, `thing.unwrap::<T>()`, source-looking text in comments/strings/macro tokens |
| `$a.expect($b)` | `thing.expect("why")`, capturing `a = thing`, `b = "why"` | `thing.expect()`, `thing.expect(x,y)` |
| `pair($a,$a)` | `pair(x,x)`, `pair(1,1)`, `pair((x /*c*/),(x /*c*/))` | `pair(x,y)`, `pair((x + y),(x+y))`, captures with different comments |
| `pair($a,$b)` | `pair(x,y)` **and** `pair(1,1)` | Lists with other arity/punctuation |

Repeated names unify using full original source bytes, without normalizing internal spacing/comments/spelling. Distinct names bind independently; bindings never carry between matches or calls. Capture arrays preserve occurrence order under the user's names, retain full original ranges, and expose no internal compiler captures. Replacement consumers can use the first unified occurrence's exact bytes; display omission does not change internal capture equality.

Malformed patterns use `INVALID_PATTERN`; unsupported authored forms use `UNSUPPORTED_PATTERN_FORM`; illegal metavariable positions use `UNSUPPORTED_PLACEHOLDER_POSITION`. These bounded errors include `field: "pattern"` and a zero-based UTF-8 `byte_offset` into the supplied pattern (EOF is a valid error position). Validation precedes repository discovery.

### Raw queries (`search_query`)

Call `search_query` for the broader raw syntax surface:

```json
{"repo_path":"/absolute/project","query":"(call_expression) @match","paths":["src"]}
```

Zero-argument written `.unwrap()` calls, ignoring comments in the arguments:

```json
{"repo_path":"/absolute/project","query":"((call_expression function: (field_expression value: (_expression) @a field: (field_identifier) @method) arguments: (arguments) @args) @match (#eq? @method \"unwrap\") (#rust-arity? @args \"0\"))","context":{"before_lines":2,"after_lines":2},"page_size":100}
```

Each top-level pattern must be rooted and designate exactly one result with `@match`. The result may be a descendant; other captures retain their own original ranges, including ranges outside that result. Repeated capture names produce occurrence arrays, **not** sugar-style equality. Explicit comment/string queries are legal; embedded source-looking text is not parsed as an expression. The grammar sees written syntax, not inferred types, symbols, evaluated cfg, or expanded macros. Macro bodies are token trees; an empty result is not proof of absence in generated code.

Supported Tree-sitter 0.27 syntax includes nodes, fields, literals, wildcards, alternations, quantified children, anchors and negated fields. Supported predicates are `#eq?`, `#not-eq?`, `#any-eq?`, `#any-not-eq?`, `#match?`, `#not-match?`, `#any-match?`, `#any-not-match?`, `#any-of?`, and `#not-any-of?`. Text and regex predicates use the pinned Rust binding's byte-based comparison/regex semantics: ordinary quantified eq/match requires all captured occurrences, `any-` uses existential matching. `#rust-arity? @capture "N"` requires a single `arguments` capture and decimal u32 count, ignores actual comment extras, and rejects malformed punctuation, recovery, attributes and non-expression children. Wrong node kind is an error. Property operations (`#is?`, `#is-not?`, `#set!`) and all other custom operations are rejected before scanning.

## Replacement (`replace`)

```json
{"repo_path":"/absolute/project","pattern":"$a.unwrap()","replacement":"$a.expect(\"reason\")","paths":["src"]}
```

The pattern and template use the same expression grammar above. Each bound `$name` substitutes the **first unified capture's exact bytes**, regardless of display limits. Unknown names yield `UNBOUND_METAVARIABLE`; unsupported template syntax yields `INVALID_TEMPLATE`, with `field: "replacement"` and a byte offset. Dollars in literals/comments are literal; no `$$` escape or `${name}` interpolation. No formatting, dedent, normalization or automatic parentheses occur. Author necessary parentheses yourself; syntax checking is not semantic-equivalence checking.

An omitted `selection` selects all matches in scope. `selection: []` selects none. Explicit selection entries are `{path, range:{start_byte,end_byte}, expected_text}` from original matches; anchors must be unique and their complete text byte-identical. Unknown, changed or out-of-scope anchors yield `STALE_SELECTION`. `max_matches` defaults to 500 (1–5000), counts scope matches **before selection**, and withholds artifacts when exceeded—even if one anchor was selected. Replacement has no cursor; narrow paths/globs to reduce work.

The common execution metadata accompanies `plan`:

- `state: applicable|blocked|incomplete`, `applicable`, exact `selected_count` or null when unknown;
- bounded `matches`, `trivia_decisions`, `blockers`, `proposed_edits` previews and explicit omission counts;
- `edits` and `patch`: non-null only for a complete applicable plan;
- `base_files`: original byte lengths and Git modes (`100644`/`100755`), tied to the envelope's corpus `snapshot_id`;
- `integrity: {grammar:"tree-sitter-rust@0.24.2",syntax:"checked|blocked|not_checked",semantic:"not_performed"}`.

Default trivia disposition is `keep_in_place`: no automatic range enlargement. Same-line comments belong to preceding syntax; contiguous leading comments/outer docs/attributes belong to following syntax. Banners, blank-separated blocks and competing neighbors surface as decisions. Inner docs/attributes and prologues belong to their containing scope. Consumed comments/attributes need **capture-origin provenance**, not merely equal-looking newly authored text. A lost second occurrence of a unified capture is independently accounted for. Unknown protected attachment after reparsing blocks all artifacts.

A decision identifies its span/text, classification/owner, reason, suggested dispositions, default/selected disposition, preservability and affected match IDs. Override ordinary unretained or external ambiguous comments with original anchors:

```json
{"repo_path":"/absolute/project","pattern":"$a.unwrap()","replacement":"$a.expect(\"reason\")","trivia_overrides":[{"trivia":{"path":"src/a.rs","range":{"start_byte":17,"end_byte":27},"expected_text":"/* note */"},"disposition":"before_match","target_match":{"path":"src/a.rs","range":{"start_byte":8,"end_byte":28},"expected_text":"x.unwrap(/* note */)"}}]}
```

These illustrative offsets must be replaced with actual source anchors. `before_match`/`after_match` require an explicitly selected same-file target (including matches selected by omission). Movement copies exact comment bytes, leaves outside whitespace untouched, and adds only a block-comment space or a nearest-source LF/CRLF line-comment separator. Multiple comments are ordered by original span and folded into the target splice. Docs, attributes, scope prologues, already-copied trivia, cross-file/unselected targets and unknown/changed anchors cannot move. `keep_in_place` forbids a target. Discard is never a disposition.

Overlapping/nested edits, pre-existing ERROR/MISSING anywhere in a selected file, newly introduced recovery, unverifiable trivia/attachment, detected corpus changes, eligibility/work limits or an oversized complete response withhold **all** applicable artifacts. A blocked/incomplete preview explicitly says `applicable:false`; preview text may be omitted and is not reconstructable. Complete zero-selection/zero-change plans return `edits:[]`, `patch:""` (successful no-op).

### Agent dry-run/review flow

1. Search and inspect original captures/trivia; narrow scope or supply full selection anchors when appropriate.
2. Call `replace`. Require `plan.applicable:true`, not merely a successful MCP call or `status:complete`.
3. Review all decisions, JSON edits and the complete patch. No edit/patch bytes are truncated. Edits are sorted ascending by UTF-8 path/start/end and refer to original half-open byte coordinates; reconstruct each file by applying its list in reverse after verifying `original_text` against base bytes.
4. Recheck the base: the final corpus freshness check is not an atomic snapshot, and later external writes can invalidate artifacts. Save the returned patch **outside the source tree** if desired, then independently run `git apply --check /path/to/patch` at the repository root. An empty no-op patch is exempt.
5. Only the caller chooses whether to externally apply a reviewed patch. The server never writes, applies, formats or compiles anything. Git patches modify existing files only, with three real context lines, C-quoted paths and no mode changes. Disposable fixtures verify external patch bytes equal JSON reconstruction, including CRLF/mixed endings and absent final newlines.

## Whole-item relocation (`move_item`)

`move_item` is a read-only, simultaneous plan for **dependency-free whole written top-level items**. It does not repair imports, paths, visibility or public API exposure. A needed repair or uncertain affected context blocks the entire batch; a successful syntax check is not compilation or semantic equivalence.

For these exact initial files:

```rust
// src/lib.rs
mod source;
mod destination;
// src/source.rs
fn selected() {}
fn other() {}
// src/destination.rs
fn keep() {}
```

The file labels above are explanatory, not part of the file bytes. Obtain complete anchors with raw search, for example `(source_file (function_item) @match)` and `paths:["src/source.rs"]`. The range covers the syntax item, not preceding docs/attributes. When `span.text_omitted` is true, obtain the full original bytes before constructing `expected_text`; never use a display snippet or a snapshot-local ID as an anchor.

A single existing-file move (one entry in the required list):

```json
{"repo_path":"/absolute/project","crate_root":"src/lib.rs","paths":["src"],"moves":[{"item":{"path":"src/source.rs","range":{"start_byte":0,"end_byte":16},"expected_text":"fn selected() {}"},"destination":{"kind":"existing","path":"src/destination.rs"}}]}
```

A batch creating one sibling, with a privately synthesized parent declaration:

```json
{"repo_path":"/absolute/project","crate_root":"src/lib.rs","paths":["src"],"moves":[{"item":{"path":"src/source.rs","range":{"start_byte":17,"end_byte":30},"expected_text":"fn other() {}"},"destination":{"kind":"new_sibling","path":"src/moved.rs","parent_path":"src/lib.rs"}},{"item":{"path":"src/source.rs","range":{"start_byte":0,"end_byte":16},"expected_text":"fn selected() {}"},"destination":{"kind":"new_sibling","path":"src/moved.rs","parent_path":"src/lib.rs"}}]}
```

These are independent calls against the initial files, not sequential application instructions. Both items reach the new file in original source order, despite reversed request entries. For multiple sources at the same insertion point, source-file groups follow their first appearance in `moves`, then items follow original byte order. Existing destinations default to actual top-level EOF. Optional `before_item` is a full unselected destination-item anchor; insertion precedes its attached leading run, never detaches its attributes/docs. A file can simultaneously supply and receive items; all edits use original coordinates.

### Supported units and ordinary layout

Functions, structs, enums, unions, traits, whole impls (including anonymous impls), type aliases, consts and statics have `supported_unit` inventory eligibility. Their dependencies may still block relocation. Whole modules have `module_context` eligibility reasons; use/extern/foreign constructs have `scope_dependency`; macro definitions/invocations have `macro_dependency`; other significant units are explicitly unsupported. Attributes/docs are associated constituents, not independently selectable inventory units. Nested/body/member/partial selections are rejected.

`crate_root` is an admitted existing Rust file chosen as the analysis root, **not** an inferred Cargo target. Source/destination identities require unique, ordinary written `mod name;` chains within admitted scope. Missing/conditional/competing/`#[path]` mappings and inherited context uncertainty block. References are scoped to the admitted corpus; no absence claim covers generated code or unexamined build targets.

A new destination must be an absent literal `.rs` sibling of every assigned source, in an existing directory. Its basename is an ASCII identifier, not `_`, `mod`, a raw identifier, or any Rust strict/reserved/contextual keyword (including `gen`, `raw`, `safe`, `union` and `macro_rules`). No directory creation or same-file reordering is supported. Symlinks, hard exclusions, nested repositories, ignores, caller filters, existing entries, case-folded aliases, competing `name/mod.rs` layouts and declaration conflicts fail closed. Use an admitted existing directory in `paths`, not an absent path.

Ordinary children of the root or an existing `mod.rs` reside in that file's directory. Children of non-root `foo.rs` reside in `foo/`. Thus moving `src/source.rs` → `src/moved.rs` usually needs parent `src/lib.rs`, **not** `src/source.rs`. Existing legacy layouts are supported with ordinary evidence, but `mod.rs` is never created or restructured. One matching unconditioned declaration is reused without a parent edit; reused restricted visibility or a public exposure requiring a decision blocks rather than being repaired. Incompatible layouts cannot be acknowledged away.

### Trivia, synthesis and replay

Moves carry internal bytes, associated outer attributes/docs, contiguous owned leading comments and same-line trailing comments. Entire internal scopes travel intact, including their inner forms. File/module prologues, inner docs/attributes outside the item, ordinary first-scope headers and ambiguous banners/blank-separated blocks stay in place by default. No header, formatter, dedent, newline normalization or blank-line cleanup is invented. Each original interval has path-qualified, exact-once retained/carried provenance, including protected owner checks after reparsing.

`trivia_overrides` use `{trivia:SourceAnchor,disposition:"keep_in_place"|"carry_with_item",target_item?:SourceAnchor}`. Carry requires a fully anchored selected target and ordinary ambiguous trivia relevant to a selected source item; a different selected file may supply that target. Keep forbids a target. Unknown/stale/unrelated/duplicate anchors fail. Protected/owned/internal trivia cannot be independently detached, retargeted or discarded; unsafe choices withhold all artifacts.

Every synthesized module declaration and separator is an itemized `rewrites[]` record with exact before/after bytes, syntactic confidence, contributing items and precise edit/create-content linkage. Boundary separators use nearest LF/CRLF without changing copied bytes. Separator targets expose `boundary_role:"before_payload"|"after_payload"`; their optional `binding` is an opaque original-run/declaration/insertion boundary identity, not a Rust binding. Fragmented runs have distinct identities. Replay the **entire published `target`**, not the display rewrite ID. Entries in `rewrite_overrides` are `{target:<published object>,action:"accept_default"|"retain"|"replace",replacement_text?:string}`. Only replace accepts text (≤64 KiB). Supported alternatives at this stage are whitespace separators containing a safe newline, or the same private ordinary declaration; no code/header/API injection is accepted. Retaining required synthesis leaves its unresolved need visible and blocks. Every choice is reparsed and attachment/byte safety is rechecked. Unknown, stale, duplicate or conflicting targets fail; no server-side plan handle is required.

### Complete artifacts and typed failures

`plan` contains `state`, `applicable`, `selected_count`, `moves`, `trivia_decisions`, `decisions`, `rewrites`, `origins`, `base_files`, `blockers`, `edits`, `created_files`, `patch` and `integrity`. Every outcome discloses `semantic:"not_performed"`. Only an applicable plan has all three non-null artifacts; blocked/failed/incomplete results set **edits, creations and patch to null**, never a safe subset. `moves:[]` is an explicit checked no-op with empty edits/creations/patch and no virtual syntax claim; stray overrides are rejected.

Creations have complete `content`, `must_be_absent:true`, mode `100644`, parent and provenance links. `declaration_link` is discriminated: `{kind:"synthesized",rewrite_id}` resolves to a module rewrite, or `{kind:"reused",path,span}` resolves to the original declaration. Reuse alone adds no parent base/edit or fictitious rewrite. The optional declaration-visibility rewrite link is absent in dependency-free plans. Source files remain present even when emptied.

Existing-file edits are sorted by path/start/end, with additive item/rewrite links (absent from replacement results). Reconstruct existing files in reverse original-coordinate order after matching `original_text`; create files separately from complete `created_files[].content` only after rechecking absence. Never interpret creation as insertion into a fictional empty base. Review all audit records together, then externally run `git apply --check` and apply on unchanged disposable copies. Creation sections use C-quoted Git paths, `/dev/null`, `b/<path>` and `new file mode 100644`; existing modes are unchanged and there are no deletion/rename/mode transitions. JSON reconstruction and patch application yield identical paths, bytes and canonical modes.

Common limits/admission/cancellation apply. `max_moves` defaults to 500 (1–5,000) and counts the whole explicit list. Fixed work guards are 100,000 inventory descriptors, 100,000 relevant reference candidates and 128 MiB aggregate analysis descriptors, including conservatively accounted transient binding/evidence records. Effective guards, observed counts and reference coverage are reported; query-state limits do not apply to direct CST analysis. Mandatory anchors/artifacts/audit must fit the complete duplicated wire response. `text_bytes:0` omits descriptive slices, **not** freshness checks or artifact bytes. On overflow, all artifacts are withheld and preview-array omissions are explicit. Calls allocate no search cursor/series.

Definite bad requests use `INVALID_ITEM_SELECTION`, `DUPLICATE_MOVE`, `STALE_SELECTION`, `INVALID_DESTINATION`, `INVALID_NEW_FILE_NAME`, `INVALID_DECLARATION_PARENT`, `DESTINATION_ALREADY_EXISTS`, `MODULE_DECLARATION_CONFLICT`, `STALE_DESTINATION`, `INVALID_MOVE_TRIVIA_OVERRIDE`, `INVALID_REWRITE_OVERRIDE` or `STALE_REWRITE_OVERRIDE`, with actionable input fields. Uncertain identity uses `CRATE_IDENTITY_UNCERTAIN`, not a guessed cross-crate assertion. Dependency decisions distinguish binding, glob, macro, re-export, module, visibility, inherited scope and unsupported-form categories with evidence and narrowing actions. Pre-existing/new syntax recovery, unsafe attachment, overlap, cancellation, work/output limits, source changes or `CREATION_RACE` withhold the entire batch. Final byte/mode/ignore/absence rechecks are observational, not atomic or application-time guarantees.

## Scope

`repo_path` is required and existing; explicit relative paths resolve against the captured launch directory, never a fallback search location. Git resolves the canonical working-tree root, including linked worktrees. A supplied file only locates the root; use `paths` to narrow to it.

Discovery considers regular case-sensitive `.rs` files; hidden files are included. Only in-root `.gitignore` files participate, with ordinary scoped precedence/negation. Global ignores, parent ignore files, `.ignore`, and `.git/info/exclude` do not participate. `target` and `.git` basenames are excluded at every depth. Symlinks are not followed and nested repositories/submodules are pruned; querying inside one separately resolves its own root.

`paths` is a union of existing root-relative file/directory restrictions. Normalize redundant `.`/separators; `.` means root. Absolute paths, `..`, NUL, symlink components and named hard-excluded paths are errors. `globs` is a positive root-anchored gitignore-style inclusion union, applied **after** discovery. `*`/`?` do not cross `/`, `**` spans directories, classes/backslash escapes are supported. `*.rs` selects only root files; `**/*.rs` selects all depths. Negation/comments, braces, leading slash/home expansion, `..`, and trailing directory slash are invalid. Simultaneous restrictions combine as discovery AND path union AND glob union; explicit empty arrays are invalid.

Eligibility precedence: oversized → binary (NUL in first 8,192 bytes) → non-UTF-8; failed reads are unreadable. Reads are capped at file limit+1, with observed handle/path identity, mode, length and modification metadata consistency. This is not a hostile-filesystem sandbox or a repository-wide atomic snapshot. Every result range/text/capture comes from one immutable source buffer.

## Bounds and results

| Setting | Default | Allowed maximum |
|---|---:|---:|
| `page_size` | 100 | 1,000 (minimum 1) |
| `context.before_lines`, `after_lines` | 2 each | 20 each (minimum 0) |
| `limits.max_file_bytes` | 2 MiB | 16 MiB |
| `limits.max_files` | 20,000 | 100,000 |
| `limits.max_source_bytes` | 256 MiB | 512 MiB |
| `limits.time_budget_ms` | 60,000 | 300,000 (minimum 1) |
| `limits.query_state_limit` | 4,096 | 65,536 |
| `limits.response_bytes` | 2 MiB | 16 MiB (minimum 64 KiB) |
| `limits.diagnostic_count` | 64 | 256 (minimum 0) |
| `limits.text_bytes` | 8 KiB | 64 KiB (minimum 0) |

Other bounds are positive. Partial option objects use defaults for omitted settings; unknown fields are rejected. Raw query size is at most 64 KiB, 64 patterns/64 capture names. Sugar size is at most 64 KiB, 4,096 significant IR nodes and 64 metavariable occurrences. Decoded arguments are at most 8 MiB. Per-file query execution permits at most 100,000 candidate matches and 128 MiB of capture/range descriptors. Unfinished file matches are discarded. One engine call is admitted at a time (`BUSY` instead of unbounded queueing); blocking work uses at most four worker-local parsers/cursors. Cancellation/deadlines are cooperative; OS/Git I/O can outlast them.

Structured results also include a JSON text fallback. Wire accounting includes both representations, JSON escaping and a framing reserve. The schema-version-1 envelope carries `tool`, canonical `root` (null if unresolved), `snapshot_id`, `status`, `coverage`, `counts`, effective `limits`, `truncation_reasons`, fixed-key `skipped`, bounded `diagnostics` and omissions, typed `error`, `matches`, `next_cursor` and `has_more`. Invalid/domain calls set MCP `isError:true` and return a failed envelope; successful empty results are not errors. Input deserialization failures use `INVALID_PARAMS`, not an opaque router response.

`scan_exhausted` concerns traversal; `eligible_scan_complete` means a complete fingerprintable source manifest was built; `scope_exhaustive` additionally requires no eligible-file failures or unfinished matching. Hard/ignore/filter exclusions define scope, while policy boundary counts disclose symlink/nested exclusions. Skip records contain exact/saturating counts, at most three examples per reason, and `examples_omitted`. Non-UTF-8 path examples have null actionable path and bounded `path_bytes_hex`, never lossy path strings. Diagnostics/skip examples share a display budget.

Each match has `id`, `path`, `span`, capture arrays, syntax flags, context and relevant keep-in-place trivia ambiguity records. `span`/captures/context entries contain the **full original half-open byte range**, start/end positions, `text`, `text_bytes` and `text_omitted`. Lines are one-based and byte columns zero-based; LF advances the line, CR/tabs/Unicode remain original bytes. Context is exact complete source lines with ranges and clipped/omitted line counts. Oversized field text is null; a wire-oversized match advances pagination as a full-range descriptor with explicit occurrence/context/trivia omission counts. Display limits do not change matching. IDs are snapshot-local, not stable syntax identities.

Recovery diagnostics scan every node, including anonymous zero-width missing tokens. Syntax flags distinguish file, subtree and enclosing non-file recovery. Such matches do not certify valid Rust or compiler correctness.

## Continuation and counts

Repeat the same effective tool/pattern-or-query/root/scope/context/limits/page-size request with its returned `next_cursor`. Ordering is path UTF-8 bytes, start/end bytes, pattern index and sorted capture signature; completed files are sorted/deduplicated before paging. A lookahead determines whether a count-limited page really has remaining matches. Retry tokens are reusable, deterministic and process-local. Each series has one fixed 15-minute lifetime; any number of pages share it. At most 32 active series are retained; capacity is an explicit error, not silent eviction.

A cursor contains series ID, file/match position and a process-keyed corruption checksum. On continuation, re-discover/read/fingerprint the entire scope using read-only `git hash-object --stdin --no-filters` (never `-w`) before resuming matching. Invalid/unknown, expired, scope-mismatched and stale cursors produce distinct errors. Restart invalidates all tokens. `snapshot_id` is an object-format/corpus hash, not a commit, persisted Git object, authorization token or atomic snapshot.

Discovery/snapshot stoppage yields partial results and **null cursor** with a narrowing reason. Matching stoppage after a complete snapshot can resume at the unfinished file; a repeatedly non-progressing file needs a narrower query. `has_more:null` means work could not establish exhaustion. Counts distinguish discovered/scanned/eligible files, files whose matching completed, matches observed and matches emitted. `total_matches` is series-wide and exact only when the whole original matching scope was exhausted in this call. A terminal continued page that did not count its prefix reports null/`total_is_exact:false`; no extra scan/cache is introduced just to manufacture totals.
