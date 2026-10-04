# Structural Rust search

`rust-sitter-mcp` runs a read-only stdio MCP service with no startup repository argument. The current increment advertises primary `search` and the raw `search_query` escape hatch; replacement is not yet implemented. It does not launch rust-analyzer, Cargo, a formatter, a network listener, or an external parser. Runtime requires local Git 2.39+; build/install prerequisites and gates are in [CONTRIBUTING.md](../CONTRIBUTING.md). Platform acceptance remains pending Ubuntu execution and the large-repository benchmark; local verification is on Darwin arm64.

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
