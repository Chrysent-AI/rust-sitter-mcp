# Structural Rust search, move plans and split advice

`rust-sitter-mcp` runs a read-only stdio MCP service with no startup repository argument. It advertises primary `search`, the raw `search_query` escape hatch, dry-run `replace`, explicitly selected `move_item` plans, and advisory-only `suggest_split` inventories/partitions. It does not launch rust-analyzer, Cargo, a formatter, a network listener, or an external parser. Runtime requires local Git 2.39+; build/install prerequisites and gates are in [CONTRIBUTING.md](../CONTRIBUTING.md). Platform acceptance remains pending Ubuntu execution and the large-repository benchmark; local verification is on Darwin arm64.

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

An omitted `selection` selects all matches in scope. `selection: []` selects none. Explicit selection entries are `{path, range:{start_byte,end_byte}, expected_text}` from original matches; anchors must be unique and their complete text byte-identical. Unknown, changed or out-of-scope anchors yield `STALE_SELECTION`. `max_matches` defaults to 500 (1–5000) and caps **selected matches**, not unselected scope matches. This replaces the former pre-selection scope-match gate: one explicit anchor can be planned even when the scope contains more than 500 matches. Exceeding the selected-match cap still withholds all artifacts; omitted selection selects the whole scope and remains subject to the cap. Replacement has no cursor; narrow paths/globs to reduce work.

An admitted selection still scans the entire scope within the existing discovery, query and time limits. After complete matching, `counts.total_matches` is the exact scope-wide count and `total_is_exact:true`, independent of the selection size (including `[]`). Calls whose scans stop early at the selected-match cap or a work limit retain honest partial counts: `observed_matches` reports completed matching work, `total_matches:null` and `total_is_exact:false`. The cap bounds selected response detail and plan eligibility, not the totals for an admitted selection. Freshness, stale-anchor checks and all-or-nothing artifact checks are unchanged.

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

`move_item` is a read-only, simultaneous plan for whole written top-level items, with **itemized import, path and visibility repairs** when ordinary module and written-binding evidence is unique. It does not infer types, expand macros, repair public API exposure or apply its plan. A required unsupported/uncertain dependency blocks the entire batch; a successful syntax check is not compilation or semantic equivalence.

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

### Opt-in standard-prelude assumptions

`assume_standard_prelude` is a request-level boolean, default `false`. Omission or
`false` preserves the written-only response bytes. There is no corresponding
`suggest_split` option. Set it to `true` for a batch to discharge otherwise-missing
**type-position** `Option`, `Result`, `Box`, `Vec` and `String` references using a
hand-authored, edition-independent subset of the standard prelude:
https://doc.rust-lang.org/std/prelude/v1/index.html

This is a caller assumption, not written-import or semantic proof. Each discharged
occurrence appears separately in `plan.binding_proofs` with `class:"standard_prelude"`,
a complete source `anchor`, `item_ids`, `destination_path`, `standard_path` and
per-occurrence `basis`. `coverage.standard_prelude` counts those occurrences, not
imports or items. Both fields are omitted when no occurrence is discharged;
schema version remains 2 because the fields are additive. The count survives
response fitting; omitted proof records are disclosed in `counts.omissions.binding_proofs`,
and a response that cannot carry the full applicable audit withholds all artifacts.

The source **and** destination/batch visible module chains must be complete and
free of competing explicit use-leaves, relevant globs and same-name written
declarations. Inline-module evidence is scoped to each reference's enclosing
module chain: a child or sibling module's imports, declarations and outer
attributes (including `#[cfg(test)] mod tests { use super::*; }`) do not shadow a
parent reference. Existing destinations are checked at their top-level insertion
scope. Selected item bodies are scanned conservatively for same-name generic,
local and pattern bindings and for conditional/recovered or macro context;
simultaneous arrivals and planned written-import repairs are also checked.
Inherited `#![no_implicit_prelude]`, `#![no_std]`, `#![no_core]`, conditional attributes,
non-built-in or shadowed derives and unexamined macros refuse the type fallback.
Refusal preserves existing typed needs; it is not an override. A proven written
binding still uses ordinary repair logic, including explicit standard-library
imports, never a prelude proof.

The same flag also admits **bare, unshadowed compiler-built-in derive names**:
`Debug`, `Clone`, `Copy`, `PartialEq`, `Eq`, `PartialOrd`, `Ord`, `Hash` and `Default`.
Their standard-defined expansion introduces no module-scope bindings. Each name
must be unshadowed at both ends, including source/destination ancestors, enclosing
inline scopes, simultaneous arrivals and planned imports. Any same-spelling
`macro_rules!`, item or explicit use-leaf (including aliases), or any visible glob,
refuses that derive. Child/sibling scopes do not leak bindings to a parent.
Path forms such as `#[derive(foo::Debug)]` or `#[derive(serde::Serialize)]` never
qualify. In `#[derive(Debug, Args)]`, an unshadowed `Debug` is separately discharged,
but `Args` retains the attribute veto and the whole batch remains blocked.
`#[serde(...)]`, `#[expect(...)]` and all other non-allowlisted attributes still veto.
Advice remains strict because it has no prelude flag; advice, move guards and
required-binding repairs share the existing context-independent attribute predicate.
Required-binding repairs still refuse inherited inner attribute context and
unexamined attributed bindings; a derive proof cannot stand in for an unfinished
import/visibility repair.

Derive evidence is additive: `plan.binding_proofs` uses
`class:"standard_builtin_derive"` with a complete original-coordinate name anchor,
contributing `item_ids`, `destination_path`, standard macro `standard_path` and an
explicit caller-assumption `basis`. `coverage.standard_builtin_derive` counts
name occurrences separately from the five type names. Contextual derives on
retained or destination declarations are recorded too when they relieve a veto;
the same occurrence/destination is counted once and its contributing items are
merged. Unknown names never receive proof records. Counts survive response fitting;
`counts.omissions.binding_proofs` discloses omitted records, and oversized audits
withhold all artifacts. Empty derive proof/count fields are omitted.

No import is synthesized: the destination is assumed to have the same implicit
prelude. Constructors, associated calls, methods and arbitrary macro expansion
remain outside the bridge. All other collisions, missing context and blockers still apply to the
entire batch. Original bytes remain lossless, no `Cargo.toml` is read, and
`semantic:"not_performed"` is unchanged. This option does not make idiomatic Rust
relocation generally executable.

### Supported units and ordinary layout

Functions, structs, enums, unions, traits, whole impls (including anonymous impls), type aliases, consts and statics have `supported_unit` inventory eligibility. Their dependencies may still block relocation. Whole modules have `module_context` eligibility reasons; use/extern/foreign constructs have `scope_dependency`; macro definitions/invocations have `macro_dependency`; other significant units are explicitly unsupported. Attributes/docs are associated constituents, not independently selectable inventory units. Nested/body/member/partial selections are rejected.

`crate_root` is an admitted existing Rust file chosen as the analysis root, **not** an inferred Cargo target. Source/destination identities require unique, ordinary written `mod name;` chains within admitted scope. Missing/conditional/competing/`#[path]` mappings and inherited context uncertainty block. References are scoped to the admitted corpus; no absence claim covers generated code or unexamined build targets.

A new destination must be an absent literal `.rs` sibling of every assigned source, in an existing directory. Its basename is an ASCII identifier, not `_`, `mod`, a raw identifier, or any Rust strict/reserved/contextual keyword (including `gen`, `raw`, `safe`, `union` and `macro_rules`). No directory creation or same-file reordering is supported. Symlinks, hard exclusions, nested repositories, ignores, caller filters, existing entries, case-folded aliases, competing `name/mod.rs` layouts and declaration conflicts fail closed. Use an admitted existing directory in `paths`, not an absent path.

Ordinary children of the root or an existing `mod.rs` reside in that file's directory. Children of non-root `foo.rs` reside in `foo/`. Thus moving `src/source.rs` → `src/moved.rs` usually needs parent `src/lib.rs`, **not** `src/source.rs`. Existing legacy layouts are supported with ordinary evidence, but `mod.rs` is never created or restructured. One matching unconditioned declaration is reused without a parent edit unless a necessary visibility repair is separately linked. Relative/re-scoped restrictions, unverified access and public exposure requiring an API decision remain blockers. Incompatible layouts cannot be acknowledged away.

### Known structural limitations

- **Macro-generated crate roots:** `crate_root!()`-style roots can emit module declarations from macro bodies; `include!(concat!(env!("OUT_DIR"), ...))` can supply build-generated declarations. The syntactic stage neither expands macros nor executes build scripts, so it cannot discover or prove these module trees. Such selections are advice-only, not applicable moves. Literal declaration-shaped tokens can name the macro cause; an opaque include without written declarations remains unproved rather than guessed.
- **Cargo autotest discovery:** creating a sibling directly under `tests/*.rs` can create a new integration-test crate under Cargo's default discovery rules. Ordinary module validity cannot see this build-graph change; review the target layout and Cargo test discovery externally before applying a patch.
- **Macro-invocation-as-item files:** when most of a file consists of macro invocations generating implementations, the inventory exposes those invocations, not their expanded items. This archetype is unsplittable by whole-item moves, like a file dominated by one large `impl`; use advice rather than treating generated methods/items as selectable written units.

Serialize MCP probes: a parallel call returns `BUSY`; wait for the active call to finish before retrying, rather than treating the busy result as an applicability measurement.

### Diagnosing an unproved module chain

Both tools expose additive `chain_diagnostics[]`: at the top level of `suggest_split`,
and inside `move_item`'s `plan`. A related `module_context` decision links them through
`chain_diagnostic_ids[]`. These mandatory explanations are independent of the generic
scan diagnostic cap. Per-occurrence decision links can be omitted by the advice/move
detail cap, with exact link omissions and group routing retained. Legacy no-draft reasons, blocker codes and
bad-request error codes are unchanged.

Each diagnostic echoes the caller's `crate_root`, `requested_path` and `role`
(`source`, `destination` or `declaration_parent`). `at_file_path` names the observed
failure boundary; optional `declaration:{path,range}` uses original half-open byte
coordinates. For named macro/attribute refusals, `declaration` locates the observed macro
module tokens or inner attribute, not a proved ordinary declaration.
`evidenced_prefix_paths`, ordinary `candidate_paths`, typed
`origin_reasons` and `parent_candidates` disclose only observed context. Locations
contain no source text and are **not execution freshness anchors**. IDs such as
`chain/0` are deterministic within the response, not persistent handles.

The snake_case `reason` distinguishes:

- `source_not_in_root_chain`: no relevant failed hop was evidenced under this root.
- `chain_file_missing` or `chain_file_unadmitted`: neither ordinary file exists,
  or the existing candidate was not admitted (without guessing why it was excluded).
- `conditional_declaration`, `path_attribute`, `competing_declarations`,
  `competing_file_layout`, `inline_module_layout` or
  `unexamined_declaration_attributes`: the evidenced declaration/layout is not an
  ordinary unique unconditioned edge.
- `inherited_uncertainty`: recovery, scope attributes or competing inclusion at a
  named origin also affects the traversed descendant (the existing veto is unchanged).
- `macro_generated_module_tree`: literal module declaration-shaped tokens occur
  in a top-level macro invocation input or macro rule output body. The recorded
  token range and ordinary candidate paths locate the possible obstruction; they
  do not prove expansion, cfg activation or inclusion. Unrelated macros do not
  veto a clean ordinary chain.
- `root_attribute_chain_uncertainty`: a non-allowlisted inner attribute appears
  in a file on the traversed chain, including nested scope attributes. Its exact
  attribute range identifies the cause; the existing `inherited_uncertainty`
  diagnostic remains alongside it. This names the current file-wide conservative
  veto, not a claim that a nested attribute has crate-wide Rust semantics.
  Both named refusals explain the advice-only consequence and route to
  `unsupported_in_engine`; no acknowledgment proves the missing context.
- `ambiguous_parent`, `no_ordinary_sibling_parent` or `ordinary_layout_mismatch`:
  candidate parent evidence does not prove the requested ordinary sibling layout.

Read `relation` before interpreting a hop. `direct` associates an observed edge or
inherited origin with the request; `possible_ancestor` only locates a possible
obstruction by ordinary candidate-directory prefix, not proof of an untraversed
chain. `root_search_exhausted` names the supplied root when no relevant failed
edge is evidenced. For example, a binary `main.rs` that only imports its library
cannot prove the library's scheduler tree: the boundary is `main.rs`, not an
invented scheduler declaration. Repeating the request with an admitted library
root that actually declares that tree can produce drafts or an applicable move.
No alternate Cargo target, cfg outcome or root recommendation is inferred.

An unadmitted root still fails `STALE_SELECTION`; invalid new-file parents and
conflicting declarations still fail with their existing error and field. A
validated creation reusing a dangling declaration is not a missing-chain failure,
and unrelated failed edges do not veto a provable chain. If mandatory diagnostics
and decision links cannot fit, the response becomes incomplete, withholds drafts
or all move artifacts, and records omissions while removing linked arrays together.

### Trivia, synthesis and replay

Moves carry internal bytes, associated outer attributes/docs, contiguous owned leading comments and same-line trailing comments. Entire internal scopes travel intact, including their inner forms. File/module prologues, inner docs/attributes outside the item, ordinary first-scope headers and ambiguous banners/blank-separated blocks stay in place by default. No header, formatter, dedent, newline normalization or blank-line cleanup is invented. Each original interval has path-qualified, exact-once retained/carried provenance, including protected owner checks after reparsing.

`trivia_overrides` use `{trivia:SourceAnchor,disposition:"keep_in_place"|"carry_with_item",target_item?:SourceAnchor}`. Carry requires a fully anchored selected target and ordinary ambiguous trivia relevant to a selected source item; a different selected file may supply that target. Keep forbids a target. Unknown/stale/unrelated/duplicate anchors fail. Protected/owned/internal trivia cannot be independently detached, retargeted or discarded; unsafe choices withhold all artifacts.

Every path/use change, import insertion or leaf extraction, visibility repair, synthesized module declaration and separator is an itemized `rewrites[]` record with exact before/after bytes, syntactic confidence, contributing items and precise edit/create-content linkage. Boundary separators use nearest LF/CRLF without changing copied bytes. Created files receive an audited EOF separator and end with one newline by default. An EOF line comment is terminated with synthesized LF: if its copied CST bytes already include CR, that completes CRLF without adding a second CR; otherwise LF avoids altering the copied comment span. Separator targets expose `boundary_role:"before_payload"|"after_payload"`; their optional `binding` is an opaque original-run/declaration/insertion boundary identity, not a Rust binding. Fragmented runs have distinct identities. Replay the **entire published `target`**, not the display rewrite ID. Entries in `rewrite_overrides` are `{target:<published object>,action:"accept_default"|"retain"|"replace",replacement_text?:string}`. Only replace accepts text (≤64 KiB). Supported alternatives are complete simple paths to the same evidenced final target, explicit private non-glob imports (including evidenced alias/reference repairs), private/`pub(crate)` visibility with access rechecked, safe newline separators, or the unchanged ordinary module declaration. A synthesized binding can also be replaced by a supported explicit path at its anchored written references. No arbitrary code, comments, header, wildcard, public API shim or new `pub` exposure may be injected. Source `retain` preserves the exact original span; synthesis `retain` emits no bytes. Rejecting a required repair leaves a linked unresolved binding/access/declaration decision and blocks every artifact. Every choice is reparsed and attachment/byte safety is rechecked. Unknown, stale, duplicate or conflicting targets fail; no server-side plan handle is required.

### Written-binding repairs: repaired versus blocked

New private `mod name;` declarations are inserted after the last sibling file-module declaration, otherwise before the first `#[cfg(test)]` item and its attached docs/attributes, otherwise at EOF. Placement includes attached trailing comments and existing line endings, with the item/boundary recorded in the rewrite anchors. Required imports go after the last whole top-level `use` and its trailing attachments, or before the first item's leading attachments when there are no imports. Existing use groups stay adjacent and no existing code is reordered.

Removal gaps stay byte-identical by default. A nonblocking `decisions[]` entry with `category:"removal_gap"`, `reason:"removal_gap_choice"` offers optional collapse when a pure engine-owned removal component leaves two or more consecutive blank lines. Its `removal_gap` contains exact residual `before_text`, collapsed `after_text`, `default_disposition:"keep_in_place"` and `selected_disposition:"keep_in_place"|"collapse"`. The full original source region, including removed items, is anchored in `action.target`; `before_text` is the whitespace remaining after those items are removed. To accept, replay the entire published target in `rewrite_overrides` with `action:"replace"` and `replacement_text` equal to `removal_gap.after_text`. Omission, `accept_default` and `retain` all preserve the gap. No arbitrary whitespace or code replacement is accepted. Acceptance produces a `removal_gap` rewrite linked to the combined original-coordinate removal splice, with complete original `before_text`, collapsed `after_text` and the decision ID. The first line terminator and one complete blank line are kept (one blank line at BOF), retaining their LF/CRLF bytes and the next item's indentation. Unrelated blank-line runs are untouched; gaps shared with another repair or insertion are not offered. Retained comment/attribute bytes are excluded, and attachment/syntax checks still apply.

For example, using the actual published decision `d` (not a display ID):

```json
{"rewrite_overrides":[{"target":"<d.action.target object>","action":"replace","replacement_text":"\n\n"}]}
```

The placeholder must be replaced by the full object and the replacement by the exact `d.removal_gap.after_text`; mixed endings or BOF gaps may have different bytes.

Defaults use complete CST paths and use leaves, not equal-looking text in comments, strings or token trees. Explicit aliases survive; grouped imports use an unchanged prefix, a shared prefix edit, or comment-free leaf extraction with unrelated leaves/trivia preserved. Local and inline-module import extraction keeps the original binding scope. Only necessary imports are synthesized; equivalent destination bindings are reused and shared needs are deduplicated. Items moving together do not create artificial cross-module imports or visibility changes.

Complete `crate`/`self`/`super` paths and at most one uniquely evidenced explicit module alias can identify a written target. Moved code is interpreted in its old lexical module, then mapped into the final batch. Recognized local functions, parameters, simple local bindings and generics are independent bindings, not leftover callers. Ordinary tuple/slice/constructor/struct binding positions are compared against the queried spelling: disjoint written names do not create uncertainty for every spelling in that scope. Explicit `mut`/`ref` binders and ordinary struct shorthand can prove independent locals; a matching plain identifier in a composite or refutable pattern still requires binding-versus-constant evidence. For/match/if-let/while-let patterns are checked only where their bindings are in scope, not in initializers, other arms or an if-let's else branch. Unsupported patterns, let chains, relevant local imports, macros and recovered/conditional lexical contexts remain anchored uncertainty. No macro expansion or semantic resolution is performed. Prelude assumptions are limited to the explicit opt-in type-position bridge described above. External explicit imports retain their written spelling, not a symbol-resolution guarantee; missing names are not guessed to be prelude imports on the default path.

Visibility checks inspect the item and each ordinary module declaration from its declaring parent scope. Already sufficient visibility and proven unchanged absolute ancestor restrictions survive. Only a proven insufficient access receives `pub(crate)`. Relative restrictions whose scope changes, private fields/tuple constructors, members, associated/type-directed references, chained re-exports, relevant globs/macros, conditional imports/modules and unsupported dependency forms remain decisions rather than guessed repairs. An unrelated glob/macro outside the relevant binding context is not by itself a veto. Consumer-glob candidates are checked against written module routes: file identity plus inline-module anchors, scoped module aliases, grouped use prefixes, and exact default-layout parent declarations. A same-named module in another subtree is not the source module. Proven different modules and enum-variant globs are excluded; a caller-selected file root does not inherit aliases from every unrelated `crate` import. Unknown/recovered routes, competing inclusions, local alias shadows, and possible wildcard/re-export/same-name forwarding remain conservative candidates. This is admitted written-syntax evidence, not Cargo discovery, cfg evaluation, macro expansion, or semantic name resolution. Optional `coverage.glob_exclusions` counts excluded selected-item/glob pairs by reason (`different_written_module`, `enum_variants`, `no_written_route`); it is not a count of files skipped or semantic absence proofs. Test-consumer advice uses the same written-route classifier, including aliases that glob the immediate test parent; it infers no edge from an unknown route.

Each alternative is checked against the same target and final binding/access constraints before assembly; newly colliding aliases block. Caller-authored bytes and associated reference repairs remain `origin:"caller_override"`, never compiler-verified. Always echo a complete published target: synthesized import `binding` identities can include a scope anchor, and separator identities include their boundary role. Neither display IDs nor a cached server plan authorize a choice.

For an independent example with exactly `mod source;\n` in `src/lib.rs` and these source bytes:

```rust
fn private() {}
fn caller() { private(); }
```

Request the move against that unchanged fixture:

```json
{"repo_path":"/absolute/project","crate_root":"src/lib.rs","paths":["src"],"moves":[{"item":{"path":"src/source.rs","range":{"start_byte":0,"end_byte":15},"expected_text":"fn private() {}"},"destination":{"kind":"new_sibling","path":"src/moved.rs","parent_path":"src/lib.rs"}}]}
```

The default plan carries the function, widens it only for the remaining cross-module caller, adds `use crate::moved::private;` in the source and links a private root `mod moved;`. Review each repair and its artifact link. To choose an alias, repeat the **same move request** and add an override whose target is the source import record's published target, for example:

```json
{"target":{"kind":"synthesis","path":"src/source.rs","slot":"import","items":[{"path":"src/source.rs","range":{"start_byte":0,"end_byte":15},"expected_text":"fn private() {}"}],"boundary_role":null,"parent_path":null,"binding":"private"},"action":"replace","replacement_text":"use super::moved::private as retained;"}
```

Place that object in `rewrite_overrides:[…]`, not in a separate apply call. The plan rewrites the necessary anchored bare caller to `retained()` and labels the authored import/reference bytes. Alternatively, choose `action:"retain"` (with no replacement text) on a required repair to inspect its unresolved need: `applicable:false`, with edits, creations and patch all null. Do not apply a preview or an older applicable batch to stand in for that rejected choice.

### Complete artifacts and typed failures

For decision field meanings and display-only labels, see [Consuming decisions and member labels](#consuming-decisions-and-member-labels).

`plan` contains `state`, `applicable`, `selected_count`, `moves`, `trivia_decisions`, `decisions`, `decision_groups`, `chain_diagnostics`, `rewrites`, `origins`, `base_files`, `blockers`, `edits`, `created_files`, `patch` and `integrity`. Every outcome discloses `semantic:"not_performed"`. Only an applicable plan has all three non-null artifacts; blocked/failed/incomplete results set **edits, creations and patch to null**, never a safe subset. `moves:[]` is an explicit checked no-op with empty edits/creations/patch and no virtual syntax claim; stray overrides are rejected.

Creations have complete `content`, `must_be_absent:true`, mode `100644`, parent and provenance links. `declaration_link` is discriminated: `{kind:"synthesized",rewrite_id}` resolves to a module rewrite, or `{kind:"reused",path,span}` resolves to the original declaration. Reuse alone adds no parent base/edit or fictitious rewrite. `declaration_visibility_rewrite_id`, when present, resolves to the separately audited required visibility repair on either a synthesized or reused declaration. Root-private modules ordinarily already admit descendant consumers and are not blanket-widened. Source files remain present even when emptied.

Existing-file edits are sorted by path/start/end, with additive item/rewrite links (absent from replacement results). Reconstruct existing files in reverse original-coordinate order after matching `original_text`; create files separately from complete `created_files[].content` only after rechecking absence. Never interpret creation as insertion into a fictional empty base. Review all audit records together, then externally run `git apply --check` and apply on unchanged disposable copies. Creation sections use C-quoted Git paths, `/dev/null`, `b/<path>` and `new file mode 100644`; existing modes are unchanged and there are no deletion/rename/mode transitions. JSON reconstruction and patch application yield identical paths, bytes and canonical modes.

Blocked, failed and incomplete move previews are counts-first: `decision_groups`
contains complete analyzed counts, blocking flags and one routing summary for
identical cause/route/consequence records. By default, `decisions` contains one
full anchored exemplar per group, up to the existing 64-record diagnostic cap.
Repeated routing/consequence text lives in the group, not each compact exemplar.
Group and move/rewrite decision IDs still name the full analyzed set, so some IDs
may have no detail record in a capped response. `counts.omissions.decisions`
reports exactly how many detail records were withheld, with `diagnostic_count` in
`truncation_reasons`; omitted per-decision chain links are counted separately.
Display omissions do not by themselves make a completed analysis partial.
Raise `limits.diagnostic_count` to obtain more detail, up to 100,000 for moves
(the existing whole-call decision guard); raise `response_bytes` too if needed.
For example, `limits:{"diagnostic_count":512}` can return all 326 decisions from
a blocked batch. This does not resolve any blocker or guarantee output will fit.
Applicable plans retain all decisions, regardless of the diagnostic display cap.

Preview `rewrites`, `moves` and `origins` default to **4 exemplars each** on
blocked, failed and incomplete plans. Their exact remainder is reported in
`counts.omissions`; `selected_count` and `counts.selected_items` still cover the
whole batch. The same single `limits.diagnostic_count` knob controls expansion:
when explicitly supplied, its value caps each array independently and decisions
return in their original order with their full routing/consequence text.
For example, `diagnostic_count:5` returns up to 5 of each preview record type;
`diagnostic_count:64` explicitly requests up to 64 of each. Increasing the explicit value monotonically expands
detail; `diagnostic_count:100000` requests all observed rewrites and decisions
within the whole-call guards (raise `response_bytes` too if necessary). Omitting
`diagnostic_count`, including when other limits are supplied, keeps the separate
4-record audit previews and grouped decision exemplars. No separate expansion
parameter exists.

`counts.rewrites` reports all observed rewrites, while `plan.rewrites` returns
the first capped exemplars in their existing order.
`counts.omissions.rewrites` records the remainder, with `diagnostic_count` in
`truncation_reasons`. Move rewrite IDs still name the full observed set, so
omitted IDs need not resolve in a capped preview. `diagnostic_count:0` returns
counts without decision or rewrite exemplars. These are informational previews,
not applicable artifacts. Applicable plans always keep their full byte-exact
rewrite audit, regardless of this display cap.

By default, blocked previews surface `trivia_decisions` only for selected items,
their immediate gaps, or explicitly overridden trivia. Attachments and internal
trivia owned by unselected items are excluded even if that item is a neighbor.
Set `include_unselected_trivia:true` to inspect the full source-file trivia
preview. Scoped-out entries are counted in `counts.omissions.trivia_decisions`
with `trivia_scope` in `truncation_reasons`. This is response shaping only:
analysis, ownership, override validation and group counts still cover the same
source. Applicable plans keep the full existing trivia audit and byte-complete
artifacts; neither this flag nor diagnostic/text limits remove artifact bytes.

Move decisions retain authoritative full original `anchors[].expected_text`.
An `evidence[]` slice duplicating the same anchored range and bytes is removed
(the field remains, possibly `[]`); distinct evidence is retained. This removes
redundancy, not unique evidence, and therefore adds no omission count. Replay
anchors inside `action` retain their existing wire shape.

Output fitting drops display text and preview detail before complete decision-group
membership. `root`, `snapshot_id`, coverage and omission counts remain available
through the last tier. If even membership cannot fit, its exact group/reference
omissions are disclosed; it is never approximated by a bounding range.

The syntactic stage delivers design advice plus proven-complete patches for a
narrow class of written Rust. Executable relocation of idiomatic Rust is the
responsibility of the deferred semantic stage, not a guarantee of this stage.

Common limits/admission/cancellation apply. `max_moves` defaults to 500 (1–5,000) and counts the whole explicit list. Fixed work guards are 100,000 inventory descriptors, 100,000 relevant reference candidates and 128 MiB aggregate analysis descriptors, including conservatively accounted transient binding/evidence records. Effective guards, observed counts and reference coverage are reported; query-state limits do not apply to direct CST analysis. Mandatory anchors/artifacts/audit must fit the complete duplicated wire response. `text_bytes:0` omits descriptive slices, **not** freshness checks or artifact bytes. On overflow, all artifacts are withheld and preview-array omissions are explicit. Calls allocate no search cursor/series.

Definite bad requests use `INVALID_ITEM_SELECTION`, `DUPLICATE_MOVE`, `STALE_SELECTION`, `INVALID_DESTINATION`, `INVALID_NEW_FILE_NAME`, `INVALID_DECLARATION_PARENT`, `DESTINATION_ALREADY_EXISTS`, `MODULE_DECLARATION_CONFLICT`, `STALE_DESTINATION`, `INVALID_MOVE_TRIVIA_OVERRIDE`, `INVALID_REWRITE_OVERRIDE` or `STALE_REWRITE_OVERRIDE`, with actionable input fields. Uncertain identity uses `CRATE_IDENTITY_UNCERTAIN`, not a guessed cross-crate assertion. Dependency decisions distinguish binding, glob, macro, re-export, module, visibility, inherited scope and unsupported-form categories with evidence and narrowing actions. Pre-existing/new syntax recovery, unsafe attachment, overlap, cancellation, work/output limits, source changes or `CREATION_RACE` withhold the entire batch. Final byte/mode/ignore/absence rechecks are observational, not atomic or application-time guarantees.

## Advisory file splitting (`suggest_split`)

```json
{"repo_path":"/absolute/project","crate_root":"src/lib.rs","source_path":"src/rich.rs","paths":["src"],"limits":{"text_bytes":0}}
```

This call inspects **one existing admitted file**. `crate_root` supplies the same ordinary written module context as `move_item`; it is not Cargo target discovery. Optional `max_items` defaults to 500 (1–5000). `paths`, `globs`, `context` and `limits` have the usual meanings. There is no cursor, saved plan, execution handle, or implicit selection. Advice consumes no search-series capacity and shares the same admission/cancellation/shutdown lifecycle as other tools.

The result has `advisory:true`, `source`, `inventory[]`, `item_contexts[]`, `impl_contexts[]`, `scope_trivia[]`, `signals[]`, `decisions[]`, `chain_diagnostics[]`, `drafts[]` and `draft_eligibility`. It has **no patch, edits, creation content or move plan**, even when a draft is complete.

- Inventory is in original source order and includes every significant top-level written unit. Anonymous impl blocks have IDs and written type/optional trait spans in `impl_contexts`; they do not disappear because `name` is null. Outer attributes/docs are associated spans, not extra units. Each unit reports full original coordinates, kind/name, raw visibility, byte/line sizes, syntax flags, unit-kind eligibility/reasons and `signal_ids`.
- `supported_unit` means the unit kind is supported, **not** that a move is dependency-free or safe. Modules, uses, extern/foreign and macro constructs remain inventoried with context-sensitive reasons and are retained in complete partitions.
- Unattached scope/prologue/ambiguous trivia has separate descriptors and a `keep_in_place` default. Banner links and `banner_section` signals mean adjacency only, never banner ownership. Internal scopes belong to their whole unit. Item contexts are original neighboring complete lines; the entire-source context has no lines outside the file.
- Signals expose snake_case/CamelCase word prefixes (≥3 characters, ≥2 names), counted directed bare/simple-path candidates with occurrence spans, banner sections, adjacent shared outer attributes/doc headings and item sizes. `get`, `set`, `new`, duplicate/common names, uncertain lexical bindings, unexpanded macros and type-directed accesses are not strong evidence. Same-spelled recognized locals/parameters/generics are not file-level edges. Inline-module bodies, macro tokens, strings/comments and unrelated qualified suffixes do not supply file-level reference proof. Candidates are **not a resolved call graph**.

### Reading and editing partitions

Each draft has a nonempty retain group and up to two sibling groups. Every inventoried ID appears **exactly once** in each complete draft. A sibling destination supplies a proposed absent path, an evidenced ordinary declaration parent/module chain, and an existing matching declaration descriptor when reused. The server probes the base name then `_2` through `_99`, checking scope admission, ignores, filesystem/case/layout occupancy and declaration/import collisions. It never guesses a parent from the source basename.

The primary proposal seeds clusters from prefixes/sections/outer-text adjacency, joins strongly connected candidate references, and ranks by descending internal reference occurrences, same-prefix members, then section/attribute/doc agreements; earliest original span breaks ties. Other units stay in source, retaining the earliest eligible unit if necessary. A distinct alternative retains all context units and the earliest eligible unit, then chooses the original-order boundary closest to half the remaining bytes (earlier boundary wins ties). Weak cohesion uses that balanced arrangement with explicitly low confidence. Equivalent alternatives are not duplicated.

Group `facts`, rationale and confidence expose integer organization evidence, not a probability of compiler correctness. High confidence requires at least two independent signal families and complete layout/membership; execution risks remain unresolved. Sizes count written item bytes/lines, not a prediction of generated file size. The 64-KiB/256-line group aim is **soft advice**: whole oversized items remain indivisible and get warnings. Cross-group candidate IDs and warnings identify import/path/visibility review needs.

Every group, including retain groups and groups with zero observed risks, also has:

```json
{
  "expected_to_block": {
    "lower_bound": true,
    "counts": {
      "member_call": 2,
      "macro_context": 1,
      "external_binding": 1,
      "conditional_or_derive": 0,
      "cfg_test_consumer": 1,
      "other_local": 0
    },
    "decision_ids": ["d/0", "d/1", "d/2", "d/3", "d/4"],
    "note": "observed lower bound, not a move plan or safe-move verdict; move_item adds consumer/destination/batch/module-chain/trivia checks and may deduplicate or repair written dependencies"
  },
  "test_coupled": true,
  "assessment_scope": {
    "assessed": "local_only",
    "not_assessed": ["other_consumers", "destination", "batch", "module_chain", "trivia_ownership"],
    "note": "only written source-local risks and observed same-file cfg(test) consumers; zero counts mean no observed local risks, not a safe move"
  }
}
```

The counts are **observed advice decision records**, counted once per cause per
participating group, not guesses at dependencies or the number of blocked items.
All five named causes and `other_local` are present even at zero; their sum equals
`decision_ids.length`. Every ID belongs to a same-response `decision_groups` run;
individual detail records may be omitted and counted. Item-free
inherited source concerns apply to every group. Nonblocking trivia choices and
batch-dependent cross-group reference reviews are excluded. `member_call` includes
written field/member access; `external_binding` covers bare references with no
inventoried declaration/import and a lexical assessment of absence. Qualified
unknown paths, impl `Self` and unknown bare spellings with uncertain lexical
context are not counted as external bindings by this local projection; existing
uncertainty decisions for inventoried candidate bindings are preserved.
Conditional/derive classification uses the same attribute predicate as `move_item`.

The lower bound describes **risk coverage**, not a guaranteed minimum count of
final move blockers: `move_item` can repair or deduplicate written dependencies and
adds destination, module-chain, other-consumer, batch and trivia checks. Proposed
sibling paths/parent evidence are layout advice, not destination applicability
analysis. No planner is run per draft. Even a high-cohesion, zero-local-risk group
explicitly leaves those concerns `not_assessed`; it is never a safe-move verdict.

Per-item `cfg_test_consumer` signals are linked through the target inventory entry's
`signal_ids`. They have `from_item_id` naming the test module, `to_item_id` naming
the candidate target, source occurrence evidence and the label
"test-coupled: consumers in this file's test module will block relocation".
`item_ids` names only the affected target, so test consumers do not become cohesion
edges or test-module relocation proposals. Detection is limited to directly
inventoried inline modules with the written exact `#[cfg(test)]` form (whitespace
is immaterial), direct `super::name` paths, and observed references through direct
`super` imports (including aliases and `super::*`). Same-spelled recognized locals
and module declarations are excluded. No macro expansion, nested-module inference,
external test-file scan or cfg evaluation is performed; absence is not proof of no
test coupling. Duplicate target spellings remain labeled ambiguous candidates.
These signals project the existing conditional-consumer decision reason;
**execution behavior is unchanged**: same-file cfg(test) consumers still block
relocation in `move_item`. No cfg override or safe relocation mode is introduced.

Risk fields and signal kinds are unchanged by counts-first response shaping;
`semantic:"not_performed"` is unchanged. `suggest_split` publishes schema version 2
because decision-group IDs now use exact runs rather than strings.

By default, `decisions[]` contains one full anchored exemplar per identical
cause/route/consequence group, bounded by `limits.diagnostic_count` (default 64).
The groups retain all analyzed IDs and counts, with a consequence and one routing
summary each. `counts.omissions.decisions` counts withheld detail records exactly;
`diagnostic_count` appears in `truncation_reasons` when detail is capped. This
presentation cap alone does **not** make analysis incomplete or withhold full
drafts: memberships, `expected_to_block` counts and links, and `assessment_scope`
are computed from the complete collected evidence before capping.

An **explicit** `limits.diagnostic_count` (0–100,000 for advice) returns the first N
full decisions in original ID order, retaining their original fields and anchor
bytes. Even an explicit 64 requests 64 records instead of one exemplar per group;
omitting this field, including when other limits are provided, keeps the compact
profile. Use `limits:{"diagnostic_count":100000,"response_bytes":16777216}` for
large full-detail responses. Overall response/work limits still apply. Counts-only
`diagnostic_count:0` keeps complete group routing and exact omission counts.

Every `unresolved_decision_ids[]` entry belongs to a same-response `decision_groups`
run. Expanded `decisions[]` records provide the per-occurrence anchored display
descriptors, evidence, unresolved consequence and next action; consult omissions
rather than assuming that every detail record is present. `choice_available` identifies supported review/anchored-choice paths (including retained ordinary banners); `request_change_required` cannot be cleared by acknowledgment. `blocks_applicability` describes a prospective execution concern, not an executable advisory result. Actual `move_item` analysis determines which repairs/choices are supported for the caller's edited batch.

When output fitting withholds full drafts, `draft_summaries[]` retains each draft's
ID, source snapshot, group kind/destination path, complete `item_ids` and unresolved
decision IDs. These are non-executable membership summaries, not complete drafts;
the omitted decision records need not resolve locally. Display inventory is trimmed
before these summaries or decision groups; root/snapshot/coverage/omissions survive.
Only the final tier may omit summaries, with exact summary/reference counts.

Empty/singleton/recovered or unsupported-layout files return inventory and an explicit no-draft reason. A work, discovery, membership, freshness or mandatory evidence/output limit gives incomplete advice and withholds **all complete drafts**. Counts distinguish observed inventory/descriptors/candidates from returned items; `counts.omissions` records suppressed arrays and membership/decision links. Dropping only source/context display text preserves complete drafts when full coordinates and required evidence links fit. `span.text:null` is not a shortened `expected_text`: obtain the complete current original bytes before execution. Integrity is input-only (`input_checked`, `input_recovered` or `not_checked`), always `semantic:"not_performed"`.

### Consuming decisions and member labels

These decision fields apply to both `move_item`'s `plan.decisions[]` and
`suggest_split`'s `decisions[]`:

- `unresolved_consequence` describes the consequence of leaving the concern
  unresolved. This is the actual wire field; there is no `consequence` alias.
  Implicit counts-first move exemplars hoist this field, `next_action` and
  action instructions to their group; explicit diagnostic expansion restores
  the full original decision fields without changing anchors.
- `reason` is a typed snake_case cause, independent of the legacy `category`.
  Category alone is not evidence of repairability.
- `lexical_uncertainty`, when present, explains an unproved lexical context with
  `spelling`, typed `reason`, `scope:{path,range,kind}`, and optional
  `pattern:{path,range,kind}`. Its reasons are `unsupported_pattern`,
  `identifier_pattern_binding_or_constant`, `value_binding_in_type_position`,
  `relevant_local_import`, `conditional_local_context` and `syntax_recovery`.
  A same-spelled value binding does not prove a type reference independent;
  that namespace uncertainty remains blocked rather than silently discarding
  the type dependency. The main decision retains the original occurrence
  anchor. These witness coordinates locate evidence;
  they are not replay targets or new request fields. Legacy
  `category:"binding_collision"`/`BINDING_COLLISION` can still accompany
  `reason:"lexical_context_unproved"`: that means uncertainty, not a proven
  collision. Witnesses survive `text_bytes:0`; if mandatory evidence cannot fit,
  the result is incomplete and decisions/artifacts are withheld with omissions.
- `action.route` provides one of three additive routes:
  - `request_field` names `tool`, the actual `field`, supported `choices`, and
    `purpose` (`resolve_decision`, `review_default` or `submit_for_analysis`).
    Rewrite choices carry the full replay `target`; trivia choices can carry
    original `trivia` and selected `target_item` anchors. Repeat the original
    batch with the named override; never use a display `r/N` ID as a target.
    Advice has display descriptors only: obtain full current anchors before
    submission. An analysis field such as `crate_root`, `paths` or `moves`
    requests reanalysis, not a promise of an applicable result.
  - `selection_change_required` names `fields` to change and an `instruction`,
    for example a colliding `moves[index].destination` or unsupported selection.
  - `unsupported_in_engine` names the anchored `construct` and an `instruction`.
    No supported override proves that concern. In particular there is no
    binding-evidence, cfg, public-API or semantic override, and accepting unrelated
    default rewrites does not clear such blockers.
- `next_action` renders the same typed route as concise caller guidance, not an
  executable request or proof that the whole batch is repairable. Review the
  related anchors and supported choices.
- `resolution:"choice_available"` means a supported review/anchored choice is
  available; `resolution:"request_change_required"` cannot be cleared by acknowledgment.
- `selected_choice` is the current choice, or `null` when none is selected. A
  reviewable default such as `keep_in_place` does not clear a separate blocker.
- `blocks_applicability` says whether the concern blocks a move (or represents a
  prospective execution concern in advice). Neither a choice nor a complete
  advice envelope implies an applicable move. Require `plan.applicable:true` and
  non-null `edits`, `created_files` and `patch` before treating a move as applicable.

Both tools also publish `decision_groups[]` (inside `plan` for moves). Each group
contains `category`, `reason`, `route`, `blocks_applicability`, `decision_ids` and
`count`. These sorted cause summaries count decision records, not guessed
references. Both move and advice groups encode exact
contiguous runs, e.g. `decision_ids:[{"first_id":"d/0","count":2},
{"first_id":"d/3","count":1}]` names only `d/0`, `d/1`, `d/3`, not `d/2`.
A run increments the numeric suffix of the first ID; run counts sum to group count.
This run encoding applies to **every** `move_item` response (blocked and
applicable) and **every** `suggest_split` response. Both publish `schema_version: 2`
(v1 listed group decision IDs as plain strings). Search tools remain on schema
version 1. Draft/risk decision IDs remain individual strings and can be joined to
the exact runs even when per-occurrence detail is omitted.
Both tools' groups additionally contain `unresolved_consequence` and `actions[]`: one
routing summary for the group, including the request tool/field/purpose/choices,
selection-change fields/instruction, or unsupported construct/instruction.
These are routing summaries, **not replay targets**: request-field actions have
null `target`, `trivia` and `target_item`. Raise the tool's diagnostic limit and
obtain full original anchors to replay a particular choice. This guidance remains
available even with `diagnostic_count:0`, including when different chain causes
share a group. Groups merge only identical category/cause/route/blocking state,
routing summary and consequence, never just a broad category.

Groups are finalized before either tool's detail capping and even when advice has no
drafts. Follow returned detail records for anchors, unique evidence, item links
and chain diagnostics; consult omission counts when detail is capped. Group
counts and IDs cover all analyzed decisions, including omitted detail. Mandatory
evidence or overall response overflow marks the result incomplete and withholds
artifacts/full drafts. Complete groups outlive preview detail and are omitted only
at the final tier with exact group/reference omissions. Existing individual decision
fields remain available through explicit detail expansion.

Draft groups carry `item_ids`, not member-name strings. Join every ID to
`inventory[].id`, including the retain group and context-sensitive units. Count
membership by IDs: every inventoried unit appears exactly once per complete draft.
A valid unnamed unit has `name:null`, not an empty-string name. Display its `kind`,
`path` and `span` coordinates instead of filtering it out or substituting an empty
wire-level name. Names can repeat; they are not membership keys.

For example, this **caller-side** Python displays every group from a saved advice
JSON response, preserving unnamed members:

```python
import json, pathlib, sys
advice = json.loads(pathlib.Path(sys.argv[1]).read_text())
by_id = {unit["id"]: unit for unit in advice["inventory"]}
impl_by_id = {record["item_id"]: record for record in advice["impl_contexts"]}

def label(unit):
    span = unit["span"]
    r = span["range"]
    location = (f'{unit["path"]}:{span["start"]["line"]}-{span["end"]["line"]}'
                f' bytes {r["start_byte"]}..{r["end_byte"]}')
    if unit["name"] is not None:
        return unit["name"]
    impl = impl_by_id.get(unit["id"], {})
    type_text = (impl.get("written_type") or {}).get("text")
    trait_text = (impl.get("written_trait") or {}).get("text")
    heading = unit["kind"]
    if type_text:
        heading += f' {trait_text} for {type_text}' if trait_text else f' {type_text}'
    return f'{heading} @ {location}'

for draft in advice["drafts"]:
    seen = set()
    for group in draft["groups"]:
        print(group["kind"])
        for item_id in group["item_ids"]:
            assert item_id not in seen
            seen.add(item_id)
            print("  " + label(by_id[item_id]))
    assert seen == set(by_id)
```

`impl_contexts[].item_id` links optional `written_type`/`written_trait` spans to
inventory units. Their text is written syntax, **not semantic impl identity**.
Null spans or omitted text still leave a nonempty kind/location label. Display
text can be omitted (`text:null`, `text_omitted:true`) while membership,
coordinates and required evidence remain complete; check status and omission
counts rather than treating a missing display string as missing membership.
Neither IDs nor labels/snippets are `SourceAnchor.expected_text`. Obtain complete
current original bytes for execution anchors, even after a complete advice result.

### Explicit edited-batch flow

1. Inspect the inventory, decisions and one or two drafts; edit/choose/ignore them **outside the server**.
2. Obtain full current source bytes, select the intended whole-item ranges, and explicitly choose each destination/parent. Retain items by not listing them in `moves`, not by asking the server to execute a draft.
3. Submit one `move_item` batch. Optional `draft_provenance` only echoes display metadata: it never authorizes membership or bypasses stale-anchor checks.
4. Require `plan.applicable:true`, review all rewrites/trivia decisions and complete edits/creations/patch, then externally recheck/apply against unchanged bases. No server-side application occurs.

For the shipped rich fixture, the following **caller-side** Python builds a real edited batch from an advice JSON response saved outside the source tree. It changes both memberships and filenames: `beta_write` joins the alpha functions, `beta_flush` goes separately, and other units remain in source. Run with the fixture root and the advice file as arguments; stdout is the explicit `move_item` arguments, not an execution handle:

```python
import json, pathlib, sys
root = pathlib.Path(sys.argv[1]).resolve()
advice = json.loads(pathlib.Path(sys.argv[2]).read_text())
assert advice["advisory"] and advice["draft_eligibility"]["state"] == "drafted"
source = (root / advice["source"]["path"]).read_bytes()
by_name = {item["name"]: item for item in advice["inventory"] if item["name"]}
moves = []
for name, path in [("alpha_read", "src/edited_a.rs"),
                   ("alpha_parse", "src/edited_a.rs"),
                   ("beta_write", "src/edited_a.rs"),
                   ("beta_flush", "src/edited_b.rs")]:
    item = by_name[name]
    r = item["span"]["range"]
    moves.append({"item": {"path": item["path"], "range": r,
                           "expected_text": source[r["start_byte"]:r["end_byte"]].decode("utf-8")},
                  "destination": {"kind": "new_sibling", "path": path,
                                  "parent_path": "src/lib.rs"}})
print(json.dumps({"repo_path": str(root), "crate_root": "src/lib.rs",
                  "paths": ["src"], "moves": moves,
                  "draft_provenance": {"draft_id": advice["drafts"][0]["id"],
                                       "source_snapshot_id": advice["snapshot_id"]}}))
```

This example assumes unchanged advice bytes; `move_item` still performs its own full-anchor checks. The runnable end-to-end demonstration generates the fixture, calls real stdio advice, edits those memberships, requests the batch, externally checks/applies the patch, independently reconstructs JSON bytes/modes and cargo-checks a disposable copy:

```sh
cargo test --locked --offline --test fixture_smoke stdio_advice_and_caller_edited_split -- --nocapture
cargo test --locked --offline --test suggest_split
```

Cargo compilation here is **test-only**, not a server semantic check. Advice-only ten-run workload methodology and platform qualifications are in [benchmark-advice.md](benchmark-advice.md).

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
| `limits.diagnostic_count` | 64 | 256; 100,000 for `move_item` and `suggest_split` detail (minimum 0) |
| `limits.text_bytes` | 8 KiB | 64 KiB (minimum 0) |

Other bounds are positive. Partial option objects use defaults for omitted settings; unknown fields are rejected. Raw query size is at most 64 KiB, 64 patterns/64 capture names. Sugar size is at most 64 KiB, 4,096 significant IR nodes and 64 metavariable occurrences. Decoded arguments are at most 8 MiB. Per-file query execution permits at most 100,000 candidate matches and 128 MiB of capture/range descriptors. Unfinished file matches are discarded. One engine call is admitted at a time (`BUSY` instead of unbounded queueing); blocking work uses at most four worker-local parsers/cursors. Cancellation/deadlines are cooperative; OS/Git I/O can outlast them.

Structured results also include a JSON text fallback. Wire accounting includes both representations, JSON escaping and a framing reserve. The schema-version-1 envelope carries `tool`, canonical `root` (null if unresolved), `snapshot_id`, `status`, `coverage`, `counts`, effective `limits`, `truncation_reasons`, fixed-key `skipped`, bounded `diagnostics` and omissions, typed `error`, `matches`, `next_cursor` and `has_more`. Invalid/domain calls set MCP `isError:true` and return a failed envelope; successful empty results are not errors. Input deserialization failures use `INVALID_PARAMS`, not an opaque router response. When a string is supplied where an object is expected, or the literal string `"null"` causes a type error, the message adds a client-stringification hint: omit optional object parameters instead of passing null. Values are still rejected, never parsed again or coerced; required object parameters must be actual JSON objects.

`scan_exhausted` concerns traversal; `eligible_scan_complete` means a complete fingerprintable source manifest was built; `scope_exhaustive` additionally requires no eligible-file failures or unfinished matching. Hard/ignore/filter exclusions define scope, while policy boundary counts disclose symlink/nested exclusions. Skip records contain exact/saturating counts, at most three examples per reason, and `examples_omitted`. Non-UTF-8 path examples have null actionable path and bounded `path_bytes_hex`, never lossy path strings. Diagnostics/skip examples share a display budget.

Each match has `id`, `path`, `span`, capture arrays, syntax flags, context and relevant keep-in-place trivia ambiguity records. `span`/captures/context entries contain the **full original half-open byte range**, start/end positions, `text`, `text_bytes` and `text_omitted`. Lines are one-based and byte columns zero-based; LF advances the line, CR/tabs/Unicode remain original bytes. Context is exact complete source lines with ranges and clipped/omitted line counts. Oversized field text is null; a wire-oversized match advances pagination as a full-range descriptor with explicit occurrence/context/trivia omission counts. Display limits do not change matching. IDs are snapshot-local, not stable syntax identities.

Recovery diagnostics scan every node, including anonymous zero-width missing tokens. Syntax flags distinguish file, subtree and enclosing non-file recovery. Such matches do not certify valid Rust or compiler correctness.

## Continuation and counts

Repeat the same effective tool/pattern-or-query/root/scope/context/limits/page-size request with its returned `next_cursor`. Ordering is path UTF-8 bytes, start/end bytes, pattern index and sorted capture signature; completed files are sorted/deduplicated before paging. A lookahead determines whether a count-limited page really has remaining matches. Retry tokens are reusable, deterministic and process-local. Each series has one fixed 15-minute lifetime; any number of pages share it. At most 32 active series are retained; capacity is an explicit error, not silent eviction.

A cursor contains series ID, file/match position and a process-keyed corruption checksum. On continuation, re-discover/read/fingerprint the entire scope using read-only `git hash-object --stdin --no-filters` (never `-w`) before resuming matching. Invalid/unknown, expired, scope-mismatched and stale cursors produce distinct errors. Restart invalidates all tokens. `snapshot_id` is an object-format/corpus hash, not a commit, persisted Git object, authorization token or atomic snapshot.

Discovery/snapshot stoppage yields partial results and **null cursor** with a narrowing reason. Matching stoppage after a complete snapshot can resume at the unfinished file; a repeatedly non-progressing file needs a narrower query. `has_more:null` means work could not establish exhaustion. Counts distinguish discovered/scanned/eligible files, files whose matching completed, matches observed and matches emitted. `total_matches` is series-wide and exact only when the whole original matching scope was exhausted in this call. A terminal continued page that did not count its prefix reports null/`total_is_exact:false`; no extra scan/cache is introduced just to manufacture totals.

Both `search` and `search_query` already apply `page_size` and wire limits to returned matches, not scope-match admission. Their shared cursor pipeline does not reject a page because the scope has more matches; it reports a continuation and honest counts for the work actually completed. Unlike replacement's admitted explicit selection, a paged search need not count the whole original scope in one call. The common rule is unchanged: response limits bound payload, and exact totals are claimed only after complete scope counting.
