# Structural Rust search, move plans and split advice

`rust-sitter-mcp` runs a read-only stdio MCP service with no startup repository argument. It advertises `read_skill_guide` for embedded usage guidance, primary `search`, the raw `search_query` escape hatch, dry-run `replace`, explicitly selected `move_item` plans, advisory-only `suggest_split` inventories/partitions, `get_split_detail` for explicitly retained historical evidence/release, and `export_move_request` for inactive exact-anchor requests from an explicit retained selection. It does not launch rust-analyzer, a formatter, a network listener, or an external parser. Opt-in path-dependency discovery runs only offline locked Cargo metadata in an ephemeral server-owned Cargo home; it never builds caller code. Runtime requires local Git 2.39+; build/install prerequisites and gates are in [CONTRIBUTING.md](../CONTRIBUTING.md). Platform acceptance remains pending Ubuntu execution and the large-repository benchmark; local verification is on Darwin arm64.

A stdio launch configuration is:

```json
{"mcpServers":{"rust-sitter":{"command":"rust-sitter-mcp","args":[]}}}
```

Build with `cargo build --locked`; `target/debug/rust-sitter-mcp --version` reports the package version and embedded build revision. No Git version command runs at startup. Logging uses stderr, controlled by `RUST_LOG`; source/query/capture bodies are not logged. Stdout is exclusively newline-delimited MCP JSON-RPC while serving.

## Bundled usage guidance (`read_skill_guide`)

This read-only tool requires no repository. It serves only canonical Markdown
embedded when the binary was built: `SKILL.md` first, then **all** files under
`references/**` in lexicographic relative-path order. Reference IDs are their
relative paths, including `references/`; the guide ID is `guide`. Examples:

```json
{}
```

```json
{"document":"references/failure-recovery.md","limits":{"response_bytes":2097152}}
```

The schema-1 response has `tool:"read_skill_guide"`, `status`, `documents`,
`bytes`, `digest`, `document`, `content_bytes`, `content_digest`, `content`,
`omitted_bytes`, `omission_reason`, `limits`, `integrity` and `error`.
Every response, including errors and omissions, includes the complete
`documents` index: each record contains `id`, source `path`, `digest` and exact
UTF-8 `bytes`. Top-level `bytes`/`digest` always identify the **full
concatenation**, even on a slice response. `content_bytes`/`content_digest`
identify the complete chosen content before fitting. Digests use
`git-blob-sha1:<40 lowercase hex digits>`: SHA-1 over Git's blob header
(`blob <byte-length>` followed by a NUL) plus exact bytes. They identify content,
not a signature or authentication guarantee.

With `document` omitted, `content` contains the concatenation. Each segment starts
with `# Bundled document: <id>`, a blank line, `Source: \`<path>\``, a blank line,
`Digest: \`<digest>\``, and a blank line, followed by the verbatim document bytes.
Two additional LF bytes separate segments. With an exact `documents[].id`,
`content` is that document alone, byte-for-byte, without a segment header. The
server performs no Markdown conversion, newline normalization, runtime skill
filesystem reads, network access or caller-repository inspection. Build-time
embedding watches the entire skill tree, including newly added references;
the drift test compares working-tree inventory, exact bytes and digests with the
compiled copy. Editing the tree triggers a rebuild; running the already-built
test executable against an edited tree fails the guard.

`limits` accepts **only** `response_bytes`, from 65536 through 16777216 inclusive
(default 2097152). The budget counts duplicated text/structured JSON with escaping
and a 4096-byte framing reserve, not just the Markdown. If chosen content exceeds
it, the response keeps the complete index and exact sizes/digests but returns
`status:"partial"`, `content:null`, `omitted_bytes:content_bytes` and
`omission_reason:"response_bytes"`. No silent prefix or cut is returned. Raise the
limit or select a document; a large slice can itself be omitted. Complete content
has `status:"complete"`, `omitted_bytes:0` and `omission_reason:null`.

Unknown IDs, unknown request/limit fields and invalid limits return
`INVALID_PARAMS`, `status:"failed"`, `content:null` and the complete index.
Failed requests use default limits and no selected content digest. Calls share
the server's admission slot; `BUSY` starts no work. The tool changes no files and
performs no analysis; `integrity.semantic` is always `"not_performed"`.

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

`move_item` is a read-only, simultaneous plan for whole written top-level items, with **itemized import, path and visibility repairs** when ordinary module and written-binding evidence is unique. By default it does not infer types or expand macros; opt-in bounded resolution below admits a narrow declarative-macro declaration-identity tier. It never repairs public API exposure or applies its plan. A required unsupported/uncertain dependency blocks the entire batch; a successful syntax check is not compilation or semantic equivalence.

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

### Written re-export routes

Consumer-side repairs can follow unique named `pub use` and
`pub(crate) use` leaves that are unconditioned or active under the declared
configuration to a directly inventoried declaration in an admitted
ordinary module. Renamed leaves preserve the consumer's binding spelling:
`pub use crate::implementation::B as C` produces a destination import such as
`use crate::implementation::B as C;` when that canonical path is accessible. Relative
paths use the declaring module's written context. Transitive routes are bounded
to **eight re-export hops** and reject cycles, competing leaves, missing terminal
declarations, globs and unproved/inactive attributed leaves (including unknown
`cfg` and `cfg_attr`). Terminal attribute, constructor, visibility and collision checks
still apply; following a route does not prove type/member behavior. Before
rewriting a canonical route, every intermediate module must have written visibility
from the final destination: `pub`/`pub(crate)` permit crate access; private and
`pub(self)` modules permit their declaring parent and descendants; `pub(super)`
permits that parent's parent and descendants; `pub(in crate::...)`, `pub(in self...)`
and `pub(in super...)` must name an evidenced ancestor containing the destination.
Whitespace/comments in visibility modifiers do not alter this check. Inaccessible,
missing or unprovable canonical edges trigger a search of admitted written public
re-export routes to the same terminal declaration. Each candidate retains the same
unique-leaf, condition, cycle and eight-hop checks, and every module edge of the
chosen public path must be visible from the final destination. The exporting modules
must also have written access to their respective hop targets. Selection is
**fewest re-export hops, then lexicographic absolute path**; the canonical path
remains preferred when accessible. No canonical module visibility is widened for a
fallback. If no accessible admitted route exists, the retained
`visibility_scope_unproved` decision has `refusal_basis.class:"inaccessible_route:<segment>"`,
with `name` naming the segment and an original module-declaration anchor when
available. Synthesized or missing declarations omit the range. This is written
reachability, not compilation; terminal attribute and moved-item API vetoes remain.

These repairs retain their ordinary `kind` (`import_insert`, `use_path`, `path`
or `import_leaf_extract`) and add `"written_reexport"` to `rewrites[].evidence`.
The same rewrite's `anchors` includes each complete original re-export
`use` declaration and the terminal declaration, alongside the consumer/import
and insertion-boundary anchors when applicable. This is syntactic evidence,
not a `binding_proofs` semantic class. No new request flag or schema version is
introduced, and `semantic:"not_performed"` remains unchanged without semantic
resolution. A fallback additionally discloses `written_reexport_route_fallback`
in `rationale`: the rejected canonical path and inaccessible segment, chosen public
path, candidate count, hop count and ordering rule. Its anchors retain the original
and chosen re-export hops, terminal declaration, rejected module declaration and
chosen path's written module edges. This is additive rationale vocabulary, not a
new evidence/proof class. Review these anchors and the chosen `after_text` before
accepting or replacing an ordinary rewrite.

Without `parent_facade`, this is **consumer-side import resolution only**.
Moving an exposed declaration still retains `public_path_change`; no compatibility
shim is inferred. Explicit `parent_facade:true` supports only the path-preserving
private-child case described below. An unsupported consumer route remains a
written-binding or conditional-context blocker, not permission to bypass it.

### Opt-in standard `matches!` consumers

`standard_matches_assertions` is an optional list of whole-invocation
`SourceAnchor` objects: `{path, range:{start_byte,end_byte}, expected_text}`.
The range is half-open in the captured original file, excluding an enclosing
semicolon or method chain. Omission, null or `[]` keeps conservative macro
handling. The caller asserts only that each exact invocation uses the standard
`matches!` contract, not that its spelling establishes macro identity. Stale,
duplicate, overlapping, non-invocation or out-of-scope anchors return
`INVALID_PARAMS` with indexed `error.field`.

The bounded adapter accepts bare parenthesized `matches!` with a simple
identifier/path/literal expression (optionally parenthesized), a written unit
enum-variant path or top-level `|` alternatives, and an optional trailing comma.
Wildcard/literal alternatives introduce no binding references. It parses copied
argument slices solely to classify roles, maps sites back byte-exactly, verifies
the written enum owner and unit variant, and uses ordinary import/path, lexical,
module, access and collision checks. Only the enum-owner prefix can be spliced;
`::Variant` remains exact. Strings/comments do not create reference sites.

Known local/module/ancestor macro shadows, renamed imports, visible globs,
`macro_use`, unknown providers and final destination shadows still refuse with
original witnesses. Qualified macro names, guards, binders, tuple/record
constructors, generics/associated paths, nested/token-generating macros and
malformed/recovered syntax are outside the subset. Assertions are invocation
specific: another macro's lexical or imported-alias veto is not waived. All
independent API, attribute, module, trivia and final moved-binder blockers remain.

Successful affected occurrences have `plan.binding_proofs` class
`assumed_standard_matches`, containing original occurrence and invocation
anchors, `item_ids`, actual `destination_path`, declaration and optional variant
anchors, `written_route` and an explicit caller-assumption `basis`.
`coverage.assumed_standard_matches` separately counts unique discharged
occurrences, including omitted proof records; it never increments `ra_resolved`.
With `resolve_semantic:false`, `semantic:"not_performed"` remains unchanged.
There is no expansion, hygiene, compilation or equivalence verification. Review
the full plan, externally apply it only when applicable, and run the caller's
compiler/tests. Advisory macro token candidates never become proof merely from
spelling.

### Opt-in standard-prelude assumptions

`assume_standard_prelude` is a request-level boolean, default `false`. Omission or
`false` preserves the written-only response bytes. There is no corresponding
`suggest_split` option. Set it to `true` for a batch to discharge otherwise-missing
**type-position** `Option`, `Result`, `Box`, `Vec` and `String` references using a
hand-authored, edition-independent subset of the standard prelude:
https://doc.rust-lang.org/std/prelude/v1/index.html
The same type proof covers exactly the written `std::`-rooted paths
`std::option::Option`, `std::result::Result`, `std::boxed::Box`, `std::vec::Vec`
and `std::string::String` in type position. Their basis names the qualified form:
qualification bypasses the terminal-name binding, so explicit same-name imports
(such as a local `use ...::Result`) do not shadow `std::result::Result` and require
no terminal-name audit. The `std` root must still be unshadowed: written competing
`mod std` declarations and `extern crate ... as std` aliases in the reference's
module/block scope chain veto proof at either overlay. Extern-crate declarations
bind their alias when present, otherwise their crate name; renaming `std` to
`something` does not introduce a competing `std` binding. Original/final root,
context, derive, macro and prelude-control audits remain in force. Other qualified
paths, relative paths, associated calls and qualified constructors do not qualify.
No semantic configuration is required for this
caller-assumed coverage.

This is a caller assumption, not written-import or semantic proof. Each discharged
occurrence appears separately in `plan.binding_proofs` with `class:"standard_prelude"`,
a complete source `anchor`, `item_ids`, `destination_path`, `standard_path` and
per-occurrence `basis`. `coverage.standard_prelude` counts those occurrences, not
imports or items. Both fields are omitted when no occurrence is discharged;
schema version remains 2 because the fields are additive. The count survives
response fitting; omitted proof records are disclosed in `counts.omissions.binding_proofs`,
and a response that cannot carry the full applicable audit withholds all artifacts.

The same assumption also covers **bare value-position `Some` and `None`** from
`std::option::Option`. Each gets `class:"standard_prelude_constructor"` with the
same anchor/item/destination/path/basis shape, and
`coverage.standard_prelude_constructor` counts occurrences separately from type
and derive proofs. Both the constructor spelling and `Option` must be unshadowed
under the original/final module, batch, lexical, derive and prelude-control audits.
The basis says **"constructor identity assumed with the type; derive-generated
imports are not modeled; not macro hygiene proved"**. This is a caller assumption,
not an RA resolution claim about imports emitted by custom derives. Qualified
paths (`Option::Some`), patterns, other constructors and associated calls do not qualify. Empty proof/count fields are omitted; counts
survive response fitting with the same omission/withholding rules as type proofs.

The source **and** destination module identity chains must be complete. For bare
spellings, competing explicit use-leaves, globs and same-name written declarations
are checked in the reference's own module, lexically enclosing inline modules and
the destination's
final insertion scope (departures removed, arrivals and planned imports included).
Filesystem ancestors' terminal-name imports and declarations do not inherit into
child modules; competing `std` root bindings remain relevant to qualified paths
throughout the module identity chain.
An attributed declaration contributes its written name conservatively even when
its presence is conditional: `#[cfg(test)] mod tests;` shadows `tests`, not `Option`.
Child/sibling scopes' finite names and derives do not leak to a parent reference.
Generic/local/pattern bindings are checked at each occurrence. A signature outside
an enclosing block ignores body macros. Standalone macros in a reference's block
or any enclosing block veto proof regardless of source order: both direct
invocations and expression-statement wrappers may expand to block items visible
even to earlier references and nested-item signatures. Written syntax does not
prove expression-only output. Unexamined outer attributes directly in those
blocks also veto, even on a later, differently named local item: an attribute
macro can emit a competing hoisted item. The refusal discloses the attribute's
original range as `lexical_uncertainty` with `conditional_local_context`; it is
not a standalone-macro witness eligible for test-consumer risk acknowledgment.
The existing strict context-independent predicate exempts written attributes
(`allow(...)`, `inline`, `inline(always)`, `inline(never)`, `repr(...)`); doc comments
are inert comment syntax. Declared configuration can additionally discharge
conditional wrappers as described below. Explicit `doc`/`expect`, unknown
conditional/custom attributes and local derives remain unexamined: bare derive spelling alone does not prove
compiler-built-in macro identity in that block. Expansion sites confined to sibling
blocks do not veto, and body attributes do not veto a signature outside their blocks.
This audits direct block macro invocations, macro expression-statement wrappers
and outer attribute siblings, not expansion or macro hygiene.
`no_implicit_prelude` vetoes its own module and descendants, not its parent or
siblings; a file-root directive is inherited by its filesystem children. These are
prelude-audit scoping rules: the independent ordinary module-chain audit still
refuses non-allowlisted scope attributes, including inline inner attributes.
Chain-wide vetoes remain for crate-root controls `no_std`, `no_core`,
`prelude_import`, syntax recovery/unparseable attributes, module-scope item macro
invocations and unexamined module-declaration attribute macros (including
`cfg_attr`, whose payload may introduce one). These unbounded vetoes also survive
sibling inline module scopes. A same-spelling textual macro definition remains relevant
downward through the chain; unrelated definitions do not veto every type.
Derive-related type refusals are limited to same-spelling competition for a
contextual built-in derive or its conditional/unexamined companion attributes
in relevant own/enclosing inline/final scopes, not unrelated filesystem ancestors.
Unknown derives retain their own attribute blockers, not a blanket type-name veto.
Refusal preserves existing typed needs; it is not an override. A proven written
binding still uses ordinary repair logic, including explicit standard-library
imports, never a prelude proof.

The same flag also admits **bare, unshadowed compiler-built-in derive names**:
`Debug`, `Clone`, `Copy`, `PartialEq`, `Eq`, `PartialOrd`, `Ord`, `Hash` and `Default`.
Their standard-defined expansion introduces no module-scope bindings. Each name
must be unshadowed at both ends in their own/enclosing inline scopes, simultaneous
arrivals and planned imports; filesystem ancestors contribute textual macros and
chain controls, not ordinary imports or declarations. Any same-spelling
`macro_rules!`, item or explicit use-leaf (including aliases), or any visible glob,
refuses that derive. Child/sibling scopes do not leak bindings to a parent.
Path forms such as `#[derive(foo::Debug)]` or `#[derive(serde::Serialize)]` never
qualify. In `#[derive(Debug, Args)]`, an unshadowed `Debug` is separately discharged,
but `Args` retains the attribute veto and the whole batch remains blocked.
`#[serde(...)]`, `#[expect(...)]` and all other non-allowlisted attributes still
retain their own move/context needs. Unexamined companion attributes, including unknown or unsupported
`cfg_attr` payloads, veto an unconditional built-in derive proof on that declaration.
Known conditional wrappers can discharge only their context veto under the
same declared configuration; derive identity still needs its scoped audit.
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
merged. Unknown names never receive proof records under this flag. Counts survive response fitting;
`counts.omissions.binding_proofs` discloses omitted records, and oversized audits
withhold all artifacts. Empty derive proof/count fields are omitted.

No import is synthesized: the destination is assumed to have the same implicit
prelude. Except for the bare value-position `Some`/`None` assumption above,
constructors, associated calls, methods and arbitrary macro expansion remain
outside the bridge. All other collisions, missing context and blockers still apply to the
entire batch. Original bytes remain lossless, no `Cargo.toml` is read, and
`semantic:"not_performed"` is unchanged. This option does not make idiomatic Rust
relocation generally executable.

### Conditional written derive relocation

`derived_attribute_relocations` is optional (omission/null/`[]` keeps the existing
`derive_veto`). Each entry asserts one exact **outer bare `derive(...)` attribute**
attached to a selected written struct/enum, its registered helpers, and unchanged
provider bindings:

```text
{
  invocation: {path, range:{start_byte,end_byte}, expected_text},
  helpers: [{anchor:{path,range,expected_text}, derive_name:"Serialize"}],
  bindings: [{
    name:"Serialize", provider_path:"serde::Serialize",
    original_import:{path,range,expected_text},
    destination_import:{path,range,expected_text}
  }]
}
```

Every anchor uses full captured original bytes. The caller asserts that the named
invocation and helper payloads preserve their dependency interpretation (including
imports, relative paths and helper strings), that helpers are registered by the
named derive rather than replacing attribute providers, and that relocation
introduces no conflicting namespace bindings, including generated imports. This
is **not** an assertion of expansion inertness, generated-member existence, or
engine-classified macro identity. No allowlist of serde or other derive spellings
is used and no procedural macro/build script is executed.

Each custom derive name needs exactly one explicit binding story: the original
and destination anchors must each name an unchanged, unconditional file-root
`use` with one named leaf whose written external route equals `provider_path` and
whose bound name equals `name`. Both files must already contain these imports;
this bridge does not synthesize imports or admit absent destinations. Renamed
imports qualify only with this explicit matching route/alias story. Qualified
derive names, local `crate`/`self`/`super` routes, globs, missing/ambiguous imports
and unasserted names remain refused. Existing separately audited
`standard_builtin_derive` name proofs can cover compiler-built-in names in a
mixed invocation without another binding entry; otherwise an explicit route is
required for them too. Provider identity behind the written external route is a
caller premise, not rust-analyzer evidence.

Helper anchors must name exact attached outer attributes or attributes inside the
selected type (fields/variants), with `derive_name` naming a derive in this exact
invocation. Control attributes cannot be declared helpers. Known competing
written helper imports, same-name provider declarations, macros, inner attributes,
recovery, competing import bindings, globs and other unasserted custom attribute/
derive providers in source/destination files, their captured filesystem ancestors
or other arriving items' source files conservatively retain refusal. This bounded
bridge does not analyze another provider's generated imports. Attributes/imports
cannot be edited by a repair, or kept behind/reattached by a trivia override, while
using this assertion. All ordinary access, origin/attachment, independent body,
module-chain, API, cfg/provider and final binder audits remain in force. Unknown
or inactive cfg still blocks; this assertion clears no cfg predicate.

Stale/non-node/non-type-attached invocation anchors, duplicate invocations,
invalid helper anchors/registration associations and descriptor-cap violations
return indexed `INVALID_PARAMS` at `derived_attribute_relocations[i]`. Assertions
are bounded by `max_moves × 64`, 4,096 helpers and 1,024 binding stories per entry,
and a 128-MiB text/descriptor guard. Missing or unmatched binding stories and
namespace/repair vetoes retain the ordinary anchored context refusal instead of
silently accepting the assertion.

Successful discharge records `plan.binding_proofs.class:"assumed_derive_relocation"`
with the invocation, helpers, original/destination binding anchors, selected
`item_ids`, `destination_path` and explicit conditional `basis`.
`coverage.assumed_derive_relocation` counts whole discharged invocations, including
omitted proof records, separately from `ra_resolved` and written-binding evidence.
This field needs neither `resolve_semantic` nor `assume_declared_helpers` and does
not change their contracts. With `resolve_semantic:false`, semantic labeling stays
`not_performed`; generated methods/constructors/trait implementations and dependent
semantic facts remain unproved. Apply and compile/check the complete patch
externally. Exported inactive scaffolds never supply this assertion implicitly.

### Declared configuration in written audits

`semantic_configuration` also supplies positive cfg evidence to written
module-chain, inline-module, import, required-binding, selected-item, lexical and
standard-prelude context audits, without requiring `resolve_semantic:true`.
One uniquely configured entry must match `crate_root`, with a valid edition and
bounded cfg/features. Listed features can use `features` or exact
`cfg` entries `{key:"feature",value:"name"}`; both encodings agree. No configuration
means no new admission, and advice has no configuration opt-in.

Written and resolution audits share the same predicate evaluator: `all`, `any`
and single-operand `not` check every operand, including unknown atoms behind a
Boolean determining operand. Unlisted features, keys and values remain unknown,
not OFF. Active `cfg` can discharge a conditional veto; selected items, required
bindings and module edges must remain active. Retained context may be known-OFF,
but its written shadow names are kept conservatively rather than erased.
`cfg_attr` recurses through active cfg/metadata payloads up to the existing depth
limit; a provably inactive condition skips its payload. Unknown conditions,
malformed predicates and arbitrary providers still refuse. Active derive payloads
and prelude controls retain their independent identity/control audits.

This is written configuration evidence, not symbol resolution or compilation.
`integrity.semantic` stays `"not_performed"` on written-only calls; no RA proof or
`resolution_coverage` is synthesized. Module-evidence assumptions and prelude
proof bases disclose the declared atoms. Remaining conditional refusals keep
original anchors and state in their consequence that the declared configuration
was consulted; expand `diagnostic_count` to see full decision prose. Independent
macro, access, collision, API, chain and repair blockers remain.

### Opt-in external library method evidence

Set `external_path_dependencies:true` on `move_item` together with
`resolve_semantic:true` and an explicit `semantic_configuration` for the admitted
caller root. Omission/false preserves existing behavior. The caller crate name,
edition and `features` must match its Cargo target and resolved feature set,
including `default` when Cargo activates that named feature. Do not add external
roots or automatic dependency edges to the explicit caller configuration: the
server discovers and appends them in a separate semantic-only namespace.

Discovery runs exactly `cargo metadata --format-version 1 --offline --locked`
from the caller manifest directory. An existing lockfile is mandatory. The
subprocess receives a scrubbed environment, a server-selected installed Rust
toolchain (no auto-install), and a new private `CARGO_HOME` under the server's
state tree; the scratch home is removed when discovery ends, including on failure.
Both output streams are capped at 8 MiB and cancellation kills/reaps discovery.
Caller repositories and the user's normal Cargo home are not written. Source-only
Cargo configuration is captured from the actual caller ancestors and Cargo home:
ordinary `[source.<name>]` string entries (`directory`, `registry`, `replace-with`)
and `[registries.<name>] index` are admitted. Cargo performs source replacement
and configuration precedence; the server does not resolve Cargo versions, features
or edges itself. Directory paths use Cargo's config-relative base; global source
configuration is copied into scratch with absolute directory paths. Inline/dotted
configuration tables, git/local registries, wrappers, credentials, build/target
settings and injected environment refuse before Cargo. The admitted form has
one header or string assignment per line; multiline values also refuse. Both
config filenames are checked conservatively, even when one shadows the other.

For locked registry sources, the server captures only already-present lock-selected
sparse-index entries, matching checked-out package directories and their cached
archives. Registry lock entries require a 64-character lowercase SHA-256 checksum.
The compressed `.crate` bytes are hashed before staging and must match a captured
lock entry. Only independent index/archive copies enter the ephemeral Cargo home:
no checked-out sources or `.cargo-ok` marker are seeded there. Offline Cargo must
unpack the selected archive afresh in private state. Every resulting regular file
and its package-relative membership must match the captured checkout, including
Cargo's marker; extra, missing or altered checkout files refuse. The selected
package name/version/original source is then bound to Cargo's authoritative
workspace lock, and its selected sparse-index row and archive must match that
exact checksum. A staged-copy equality check alone is not integrity evidence.

The server never links to caller storage, downloads, fills the real cache, unpacks
into the real home or admits a package available only as an archive. A missing
checkout/index/archive or insufficient comparison budget refuses; obtain the
needed inputs outside the server before retrying. Only existing version-3 sparse
cache encodings are supported, not Git-protocol registry indices. Cargo selects
source replacement, registry and package graph; the engine only checks the
selected identity/checksum row, not version resolution. Directory/vendor sources
are read in place, including a `vendor/` inside the caller repository. Their
captured manifest name/version and `.cargo-checksum.json` package checksum must
match the selected locked identity. Every regular file other than that checksum
metadata must have exactly one valid SHA-256 map entry matching its raw bytes;
missing, malformed, duplicate, extra or mismatched entries refuse. This follows
Cargo's directory-source checksum contract, not authentication against malicious
rewrites of both files and their checksum map. Consultation grants no edit
authority. All candidate registry/vendor roots and their possible manifest
closures are checked before scratch creation.

Captured manifests, locks, configuration, index, compressed archives and complete
package-file trees share `max_files`, `max_file_bytes` and `max_source_bytes`.
Fresh archive correspondence additionally charges unpacked directory entries and
regular-file bytes against the same capture budgets (defaults: 20,000 files/
entries, 2 MiB per file or archive, 256 MiB aggregate). These are comparison and
capture caps, not a scratch-disk quota on Cargo's unpacker. Cargo remains bounded
by the request deadline/cancellation and output caps; unreadable or non-regular
files, symlinks, failed unpacking and exhausted comparison budgets fail closed.
All captured inputs are reobserved before scratch creation and at publication;
publication rediscovery repeats the archive/checksum and membership checks.

State must be disjoint from the repository, the real Cargo home and every
admitted dependency root, including retained-advice spill locations.
Before any scratch directory creation or Cargo execution, a bounded lexical
manifest-input closure starts with the caller manifest and all ancestor
`Cargo.toml` files from its manifest directory and already-admitted crate-root
directories, **including enclosing workspace manifests outside the selected
repository**. Every captured manifest's literal `path = "…"` (or literal-string
equivalent) declarations become candidate package roots, resolved relative to
that manifest's directory. Their manifests, enclosing ancestor manifests and
own path declarations join the same walk. Literal `members = [...]` entries
also become candidates relative to the declaring workspace root, including
unconfigured sibling members and their dependencies. A visited set deduplicates
ordinary workspace/ancestor revisits; path-declaration cycles refuse with
`EXTERNAL_ROOT_UNSUPPORTED` before Cargo.

Each upward ancestor search includes at most 32 directories (starting directory
through filesystem root); candidate walks and path-declaration chains are also
bounded at 32 manifests. Bound exhaustion returns `EXTERNAL_CORPUS_INCOMPLETE`,
never an unchecked metadata invocation. All captured candidate manifests and
adjacent existing lockfiles share the request's file-count and aggregate-byte
pre-check budget, with identity-checked regular-file reads under per-file and
deadline limits. The closure bytes are reobserved before scratch creation and
retained for publication freshness. Unreadable, non-regular, linked or undecodable
inputs fail closed before Cargo. An external enclosing workspace is supported
only when its manifest and existing lockfile were captured at this pre-check.
Explicit `package.workspace` string pointers (including non-ancestor workspace
routes) refuse before Cargo; `dep.workspace = true` inheritance remains supported.

Every candidate manifest directory and declared root must be disjoint from server
state; overlap returns `EXTERNAL_STATE_OVERLAP`. This lexical pre-check does not
interpret Cargo tables, excludes, features or dependency activation, and
intentionally over-refuses: unrelated ancestor manifests, optional/inactive
paths, excluded/unconfigured members and unrelated `path`/`members` assignments
can all trigger refusal. For `workspace = true` inheritance, cycle/hop checking
conservatively treats every captured ancestor path declaration as a possible
edge, without resolving dependency names or tables; this can reject otherwise
valid inherited layouts. Directory candidates must have safe readable manifests.
Literal source-file targets such as `[lib] path = 'src/lib.rs'` are checked as
regular-file candidates, not mistaken for package directories; missing or unsafe
source-file candidates still refuse. Workspace-member globs (`*`, `?`, bracket/brace patterns), non-string
entries, malformed/ambiguous member lists and explicit workspace pointers use
`EXTERNAL_ROOT_UNSUPPORTED`; missing candidate manifests use
`EXTERNAL_DEPENDENCY_UNREADABLE`, unsafe files or exhausted budgets use
`EXTERNAL_CORPUS_INCOMPLETE`. The lexical selector is not a Cargo/TOML resolver.
The authoritative post-metadata workspace and resolved-root identity/overlap
checks remain in place. Publication rediscovery repeats the complete pre-check,
with all closure manifests and locks bound by byte reobservation. Visiting an
unrelated path candidate or sibling member checks discovery safety only. After
Cargo resolves the graph, the normal-dependency closure starts at the selected
caller and follows `resolve.nodes[].deps` by package ID, including transitive
path, cached registry and directory-source packages. Each resolved manifest seeds
another bounded candidate walk and must match inputs captured **before** Cargo;
no newly discovered unchecked root is admitted after state creation. A missing
resolved manifest or changed candidate closure refuses rather than loading it.

Declared direct out-of-tree **path libraries**, exact already-cached **registry
libraries**, **directory/source-replaced libraries**, and their complete supported
normal-edge transitive closure are admitted. Package
identity is name + version + source (or path identity), not bare crate name.
Renamed dependency edges bind to the resolved package ID; two versions of one
package get distinct library graph nodes/namespaces. Duplicate directory-source
name/version candidates refuse rather than accepting Cargo's arbitrary selection.
Git dependencies (cached or not), target-conditional edges,
build scripts and proc-macro packages remain unsupported. Dev/build-only edges
are omitted from the method-evidence graph, not relabeled as normal dependencies;
conservative offline discovery/preflight can still require their local inputs.
The closure deduplicates package IDs at joins while preserving each owner's
renamed aliases and distinct versions/sources. The combined graph has at most
32 crates (including explicit caller roots) and 32 dependencies per crate.
Resolved-library cycles refuse with `EXTERNAL_ROOT_UNSUPPORTED`; graph caps use
`EXTERNAL_CORPUS_INCOMPLETE`. All required roots must fit in one shared capture
budget; exhaustion halfway through a closure never publishes a partial corpus.
Missing/stale locks and
uncached dependencies fail closed at preflight or offline discovery. Dependency
`#[path]`, filesystem `include!` variants, symlinks, syntax recovery and missing
or competing ordinary module layouts also refuse. Capturing package Rust files
is bounded by the request's file, entry and source-byte limits and deadline;
exhaustion never produces a partial-success corpus. Captured dependency features
come from Cargo; arbitrary caller cfg atoms are not copied into libraries.

The same immutable corpus enters distinct library source roots in both pure RA
resolution overlays. This opt-in proves only eligible concrete written inherent
methods/functions on dependency types, with existing receiver identity,
trait/generic exclusion, macro/provider and both-overlay access gates intact.
External fields, nominal type/direct ADT constructor/variant facts are not discharged by this tier.
Only actual successful both-overlay proofs count in `coverage.ra_resolved`.
No compilation, build script or proc-macro execution is performed by the server.

Inspect additive `coverage.external_dependencies`: `complete`,
`consulted_roots` (package ID/name/version, original source identity, provenance
`path`/`registry_cache`/`source_replacement`, absolute root, namespace,
edition/features and file/byte counts), `corpus_digest`, `omissions`, and
`capture_usage`. The latter reports `max_files`, `max_file_bytes`,
`max_source_bytes`, pre-source `input_files`/`input_bytes`, deduplicated final
`observed_files`/`observed_bytes`, `archive_comparison_entries`/
`archive_comparison_bytes`, `source_files_per_overlay`/`source_bytes_per_overlay`,
and `graph_edges` (normal resolved edges). Observed inputs plus archive comparison
charges share the request caps; source totals count each namespaced corpus entry,
even when distinct package/source identities read the same physical directory.
The original and final RA databases each receive those same source files/bytes:
raw external text materialization across the two overlays is twice the per-overlay
total, not a heap/RSS estimate or Cargo-unpacker disk quota. Advice supplies the
same capture disclosure but constructs neither overlay. A mid-source-capture
budget refusal includes partial root/file/byte charges and the active limits in
`error.message`; earlier preflight/comparison refusals identify their exhausted
cap. Disclosure remains present when discovery succeeds but no decision discharges.
`@external/N/...` declaration anchors are read-only evidence, never selection
anchors, destinations or ordinary consumer-scan files. Semantic input/final
coverage digests bind the corpus and discovered graph. Captured bytes, source
membership, package-root identity and discovery configuration are reobserved
before publishing an applicable plan; stale evidence withholds artifacts.
If that external freshness/cancellation gate fails after resolution, all
already-computed RA proof records and resolution coverage are withheld, their
real/assumed counters are cleared, and external coverage reports `complete:false`.
`integrity.semantic` may still say `resolution_performed` on that failed response:
queries did run, but no resulting proof is published.
No persistent dependency cache is created.

For example, a caller can use `dep::Thing::new().configure()` when its direct
library contains `pub use leaf::Thing;` and the resolved transitive library
contains the concrete written `Thing` implementation. Both libraries appear in
`consulted_roots`, while inherent declaration anchors identify the leaf's own
namespace/package ID. Re-export discovery itself is not a resolved proof.

`suggest_split` accepts the same flag and explicit caller configuration for
source consultation/disclosure. It stays advisory: no final overlay, no method
proof, no applicability or compilation claim, and `semantic:"not_performed"`.
This configuration does not enable written cfg-chain admission in advice.
Re-enable the opt-in explicitly on any subsequent ordinary move request.

Typed refusals include `EXTERNAL_DISCOVERY_FAILED`,
`EXTERNAL_DEPENDENCY_UNSUPPORTED`, `EXTERNAL_CONFIGURATION_MISMATCH`,
`EXTERNAL_CONFIGURATION_UNSUPPORTED`, `EXTERNAL_ROOT_UNSUPPORTED`,
`EXTERNAL_DEPENDENCY_UNREADABLE`, `EXTERNAL_CORPUS_INCOMPLETE`,
`EXTERNAL_SOURCE_CHANGED`, `EXTERNAL_STATE_UNAVAILABLE`,
`EXTERNAL_STATE_OVERLAP`, `EXTERNAL_NAMESPACE_CONFLICT`,
`EXTERNAL_PACKAGE_UNAVAILABLE` and `EXTERNAL_PACKAGE_AMBIGUOUS`; normal cancellation
and deadline refusals remain unchanged. An otherwise resolved non-method external
fact retains its original decision with `external_fact_unsupported` refusal basis.
`EXTERNAL_PACKAGE_UNAVAILABLE` includes missing/malformed/mismatched package
checksums or archive/checkout correspondence; Cargo may instead return
`EXTERNAL_DISCOVERY_FAILED` while rejecting inconsistent locked/index/vendor
inputs. Neither case publishes proofs or move artifacts. Missing unreadable files
and unsafe forms retain their existing unreadable/root/corpus refusal codes.
Correct the input/scope and re-plan; no override admits an incomplete corpus.

### Opt-in bounded resolution

`resolve_semantic` defaults to **false**. Omission or false leaves
`semantic:"not_performed"`; a supplied configuration can still affect the written
audits above. To request resolution, include an explicit `semantic_configuration`
in the same `move_item` request, for example:

```json
{
  "resolve_semantic": true,
  "semantic_configuration": {
    "crates": [{
      "name": "application",
      "root_file": "src/lib.rs",
      "edition": "2024",
      "features": [],
      "cfg": [],
      "dependencies": []
    }]
  }
}
```

This fragment augments the ordinary anchored move request; it is not a separate
tool or stored plan. Each root must be an existing admitted Rust file; one root
must equal `crate_root`. All admitted texts and the assembler's exact final
virtual texts are ingested into request-owned pure rust-analyzer databases.
By default no Cargo.toml/lockfile, sysroot, filesystem watcher, dependency cache,
build script, executable override or proc-macro expander is loaded. Configure
repository-local dependency sources explicitly in admitted `paths`/`globs`, or
use the separate direct path-library opt-in below. Never put out-of-tree roots
in repository `paths`/`globs`. A dependency edge is
`{"name":"external","crate_name":"dependency_root_name"}`; `name` is its extern
prelude spelling and `crate_name` names another entry in `crates`. Crate names
and roots must be unique; graphs are acyclic and capped at 32 crates with 32
edges per crate. Editions are `2015`, `2018`, `2021` or `2024`.
Crate names and dependency aliases are Rust crate identifiers, not Cargo package
names: pass `my_package`, not `my-package`; no implicit normalization occurs.
With `resolve_semantic:true`, unusable supplied configurations return
`INVALID_PARAMS` with an indexed `error.field` (for example,
`semantic_configuration.crates[0].name`) and a bounded offending-value preview
plus the reason. This includes unadmitted/missing roots, duplicate names/roots,
missing dependency targets, cycles, invalid editions and configuration limits.
Validation occurs even when there are no eligible resolution needs. Invalid
configuration never starts resolution or produces `resolution_coverage`;
`integrity.semantic` remains `not_performed`. The written-cfg path with
`resolve_semantic:false` is unchanged.

Each crate declares its selected `features`, `cfg` atoms and dependencies, with
no implicit defaults. A cfg atom is `{"key":"unix","value":null}` or
`{"key":"target_os","value":"linux"}`; each selected feature also sets
`feature="name"`. These are caller-provided inputs, not discovered Cargo facts.
There is no build-script environment, target data layout or implicit standard
library source. Absent configuration or incomplete binding evidence does not
acknowledge away a blocker: the original typed needs remain. Unusable supplied
configuration instead returns the field-specific input error described above.

The resolution context gate evaluates written `cfg` and `cfg_attr` predicates:
bare atoms, string-valued atoms (including `feature="name"`), `true`/`false`,
`all(...)`, `any(...)` and single-operand `not(...)`, nested up to 32 levels.
Exact atoms listed in that crate's configuration are ON; unlisted atoms (including
unlisted features or a different value for a declared key) are **unknown**, not
implicitly OFF. OFF is provable through `not` of an ON predicate, `false`, or
`any()`. Every operand is checked: a known result cannot short-circuit an unknown
atom away. Unsupported predicates, malformed attributes and unknown atoms retain
the veto. A known-OFF sibling is configuration information, not proof an inactive
selected occurrence resolves. No new false-atom request field is introduced.

Attached `cfg`/`cfg_attr` attributes on selected top-level items also reach this
gate under `resolve_semantic`: `context_attribute` proofs compare the exact attribute bytes
at original and final anchors, including attributes carried with the move outside
the item's syntax range. Feature atoms can be supplied either in `features` or as
`cfg` entries with `key:"feature"` and a string `value`; the encodings agree.
A context-only proof requires an active selected item or required binding:
known-OFF `cfg`, including active `cfg_attr` payloads that disable the item,
retains `inactive_written_binding`. Unknown predicates retain an anchored
`conditional_context` refusal and reached `undeclared_cfg_atom` disclosure.
Selected predicate checks use the strict `generated_items` context gate even on
types: active custom/qualified derive payloads retain their veto. Direct derives
and other non-predicate attributes do not enter this selected-item route; existing
derive/macro-binding audits remain responsible for them. This clears only that
predicate need, not independent prelude/lexical, macro, inline-module, access, API
or repair vetoes, and is not compilation evidence.

The derive guard is scoped by the fact being proved:

- **Nominal identity** of written types/enum variants admits well-formed custom
  derives only when the path's written bindings are stable against additive
  output: a locally written declaration or explicit named import, with a written
  non-glob route through its module prefixes and any re-exports. Aliases and
  grouped use leaves qualify only when their `cfg`/`cfg_attr` leaves the binding
  active; known-OFF imports are harmless context, not binding evidence. Variant
  segments resolve inside the already-named enum. Both original and final
  overlays must establish this rule. A derive appends items without replacing
  its annotated declaration:
  https://doc.rust-lang.org/reference/procedural-macros.html#macro.proc.derive.output
  But a generated explicit import **can override a glob-resolved name without a
  compile error**. Glob-dependent or unestablished written routes therefore
  withhold nominal path proofs, even if unexpanded RA identities agree. The basis
  says **"nominal identity via stable written declaration or explicit import
  route"** and discloses the generated-import limit. This is not an inert-expansion
  claim or a compilation check. Direct required-binding attribute context remains
  independently nominal; its admission does not prove a dependent path's stability.
  Configured dependency roots from the extern prelude retain the conservative
  `generated_items` context gate, not custom-derive admission.
- **Generated-item-dependent facts**, including method/field receiver inference
  and function resolution, keep the existing gate: bare derives containing only
  `Clone`, `Copy`, `Debug`, `Default`, `Eq`, `PartialEq`, `Ord`, `PartialOrd` and
  `Hash` are admitted as compiler-built-in trait-impl-only context; known competing
  macro bindings, qualified names and other derive names retain the veto.
  Generated impl presence and trait-method resolution are not proved.

For both classes, malformed derives and arbitrary attribute macros still veto.
Module-level macro invocations retain their veto except for the bounded nominal
namespace audit described below; generated-item-dependent context stays strict. For `cfg_attr`, an ON condition requires each payload
attribute to be admitted recursively for the same fact class; an OFF condition
records the payload as inactive without evaluating it. An undeclared condition
still vetoes everything it wraps, including nominal identity. Existing inert
lint/doc/inline/registration attributes remain admitted. No proc-macro execution
or macro-hygiene proof is introduced.

#### Declarative-macro declaration identity

`resolve_semantic` also admits type-position paths generated by an **in-crate,
written `macro_rules!` definition and invocation in admitted files**, under the
same explicit configuration. Declarative macros substitute written tokens
without executing code; procedural macros execute arbitrary code and never
qualify or run under any flag. The initial budget is one `ident` argument, one
arm with matcher `($name:ident)`, one expansion step, 4,096 significant tokens
in the definition/output and token nesting ≤32. Repetition, recursive/nested
macro calls, other fragments and over-budget output refuse. Conditional macro
definitions, invocations or enclosing modules veto even when their cfg is ON.
Builtin macros, generated definitions and definitions in another crate refuse.

The pinned RA tier expands vetted tokens and verifies that the resolved struct
or enum name maps exactly to the written argument, not a coincidentally matching
literal or a fallback call-site range. Output may contain structs/enums and
impls, but no imports, modules or other namespace-producing items. Generated
type attributes permit only the existing inert `allow`/`warn`/`deny`/`forbid`/`doc`
metadata and unshadowed bare compiler-built-in derives by default. Other attributes, including
`#[serde(transparent)]` even beside `Serialize`/`Deserialize`, refuse as
provider-uncertain. A derive token spelling does not establish helper registration:
the same admitted tokens can denote an inert helper or an attribute macro that
replaces the declaration. Custom/qualified derives also retain the refusal in this
tier; written `macro_rules!` definitions cannot supply attribute/derive providers.
The definition, invocation and enclosing written owners remain audited; unknown
owner attributes refuse and conditional owners remain vetoed. No provider is loaded
or executed and no registration evidence is guessed.
This namespace audit removes the blanket module-macro veto **only for nominal
facts**; generated-item context keeps it. No generated fields, constructors,
variants, impls or methods are proved or become selectable inventory units.

Successful records have additive `class:"declarative_macro_identity"`,
`classification:"declaration_identity"` and basis text stating **"declaration
identity established through bounded declarative-macro expansion of written
tokens"**. The basis additionally states that resolved ADT identity and written
argument provenance are checked at both overlays, provider-uncertain attributes
are refused, and helper registration is not inferred from derive spelling. These
checks do not observe unmodeled procedural expansion or verify rustc compilation.
Both declaration objects anchor the identifier argument and carry
`declarative_macro:{invocation,definition}` with exact written anchors. All three
anchors normalize through the assembler's origin map and must match at both
overlays, with the same crate origin and positive access. Receiver fields are
null: unadmitted field types are irrelevant to this identity-only fact, not
inferred or assumed. These proofs count in `coverage.ra_resolved` as part of the
same configured RA tier. The schema version is unchanged.

`assume_declared_helpers` is a request-level boolean, default `false`, requiring
`resolve_semantic:true` and the same explicit `semantic_configuration`. It is an
assertion by the caller that each provider-uncertain attribute on a written or generated
struct/enum belongs to a **registered derive helper**, not a replacing attribute
provider. The engine does not classify or verify that assertion, infer registration
from a derive spelling, or execute any provider. Well-formed companion custom or
qualified derive paths on that helper-bearing type are admitted syntactically only;
helper-free custom/qualified derives retain their refusal. Control attributes
(`cfg`, `cfg_attr`, prelude controls) and attributes on written definitions,
invocations or enclosing owners cannot be turned into helpers by this flag.

A discharged occurrence relying on this assumption uses the distinct
`class:"assumed_declared_identity"`, never `declarative_macro_identity` or
`ra_resolved`. Its basis states **"declaration identity assumed with the type,
not engine-classified"**, names the caller assertion, and disclaims procedural
expansion, macro hygiene, generated-member, compilation and equivalence claims.
Generated declarations keep `classification:"declaration_identity"`, null receivers
and all three original/final identity anchors. Nominal facts about written types
that depend on an assumed sibling macro's namespace admission are also conditional
and carry this class, preserving their own declaration/receiver anchors.
`coverage.assumed_declared_identity` counts these occurrences separately, including
omitted records; they do **not** inflate `coverage.ra_resolved`.
`resolution_coverage.decisions` counts the sum of real and assumed resolution
occurrences, not just real proofs. No counter is emitted when its count is zero.
Omission or `false` preserves the containment response bytes. Both-overlay
identity/access, cfg, shape/token/nesting caps, procedural-macro construction and
all generated impl/member/field/constructor/variant vetoes remain in force.
Direct ordinary written struct/enum helpers also qualify for nominal identity only,
with exact original/final declaration and reached attribute anchors. Attributes on
other written owners and generated-item facts remain blocked.

A bare type imported through an existing accessible written public named facade
can carry that same facade import provisionally into a sibling. The rewrite
rationale says `pending both-overlay declaration identity`; it neither guesses a
generated canonical terminal nor clears the semantic need. Conditional,
competing, missing or inaccessible facade evidence still blocks, as do failed
original/final identities. Independent API/chain/trivia/repair gates still apply.

`context_evaluations` adds `kind:"declarative_macro"` invocation records and
`kind:"declarative_macro_definition"` definition records, retaining the
`nominal_identity` fact class. Their basis describes bounded admission, not an
already-matched identity. Reasons include `bounded_declarative_namespace`,
`bounded_declarative_definition`, `assumed_declared_helpers`, `declarative_definition_unproved`,
`declarative_external_definition`, `declarative_invocation_unproved`,
`declarative_conditional_context`, `declarative_attribute_provider_uncertain`,
`declarative_fragment_limit`, `declarative_recursion_limit`,
`declarative_token_limit`, `declarative_nesting_limit`,
`declarative_expansion_unproved`, `declarative_unparseable` and
`declarative_output_unproved`. A provider-uncertain attribute is named in the
skipped evaluation's basis; generated attributes anchor the whole written macro
definition, never invented generated-file coordinates. Written owner attributes
have their own exact anchors, with the reached invocation also marked skipped.
An assumed admission records `reason:"assumed_declared_helpers"` with an explicit
caller-assumption basis; definition records name the reached generated helper
attribute, anchored to the whole written definition. Admission is still not a
matched-identity claim. Unsupported expansion retains
`semantic_source_fact_unproved` or `semantic_final_fact_unproved`; changed or
unmappable pairs retain the identity/mapping refusal. Missing RA resolution may
follow bounded written named routes solely to disclose a reached macro refusal;
that traversal never invents identity or follows a glob. Reached checks are not
an exhaustive macro inventory. Design: `declarative-macro-identity.md`.

Only concrete, known receivers with preserved original **and adjusted** type
identities qualify for the other resolution facts. The first scope resolves actual inherent functions (including
associated calls), named fields, type/constructor paths, enum-variant paths in
value and pattern positions, and ordinary written function bindings. A
`classification:"variant_path"` proof anchors the terminal variant declaration
and its parent enum identity at both revisions, with original/final access checks.
Written variant/field attributes are audited even on cfg-hidden siblings; generic
or unknown parent identities still refuse. This is RA resolution, not a new
written-route proof or general pattern/binder-versus-constant solver. It checks the resolved declaration's actual impl ownership,
not candidate iteration, and positive visibility from both the original and
final lexical modules. Constructor fields must all be accessible. Receiver
identities use reference adjustments, builtin names, written declaration anchors
and concrete type arguments, not pretty-printed type strings or revision-local
IDs. Generic function contexts, trait functions/trait objects, unknown inference,
unsupported receiver shapes or const/lifetime substitutions, expanded/generated
member nodes and unadmitted macro/attribute contexts retain their existing typed
blockers; the identity-only exception above never admits such receivers.
Macro-bearing function bodies unrelated to the selected occurrence and ordinary
proc-macro registration elsewhere do not key refusal on the crate's kind.
Independent macro/module-chain/cfg(test), API, trivia and required-repair vetoes
are unchanged; the attribute refinement applies only to this resolution gate.
This is not general trait resolution, semantic rename or a compiler.

Each discharged occurrence adds a discriminated
`plan.binding_proofs[]` record with `class:"ra_resolved"`: full original and final
anchors, contributing item IDs, original/adjusted receivers at both ends,
original/final declaration identities (crate origin + stable written name
anchor), classification, a fact-class-specific `basis`, `source_access:true`,
`final_access:true` and explicit configuration coverage. A plain function binding has null receiver fields.
A required-binding attribute veto can separately produce a `ra_resolved` record
with `classification:"context_attribute"`, an attribute declaration anchor and
null receivers: this proves the required attribute context for the disclosed
fact class at both revisions, **not** resolution of a dependent pattern/body or
generated implementation. For nominal path facts, admission requires stable
written bindings rather than an assumption that additive output preserves glob
resolution. Attribute-only evidence concerns the direct written declaration.
Written access checks and required visibility/module repairs still run before
this context-only veto can discharge.
`coverage.ra_resolved` counts these occurrences separately from prelude proofs,
including records omitted during output fitting. Type identities/declarations
are compared across revisions through the assembler's exact byte-origin map.
The resolver neither edits source nor assembles an independent overlay; caller
choices, imports, visibility repairs and simultaneous arrivals are included in
the final view. Unmappable or changed identities refuse rather than guessing.

When queries are performed, `plan.integrity.semantic` is
`"resolution_performed"`. `plan.resolution_coverage.statement` says
**"resolution performed for N decisions under one explicit configuration;
compilation/equivalence not performed"**. Its configuration, analyzer version,
source `snapshot_id`, `semantic_input_digest`, `final_overlay_digest` and explicit
omissions scope every proof. `context_evaluations[]` adds deduplicated anchored
records for reached context checks: `revision` (`original`/`final`), `crate_name`,
`anchor` (exact path/range/text), `kind` (`attribute`, `cfg_predicate`,
`module_macro`, `binding`, `declarative_macro` or `declarative_macro_definition`), `status` (`admitted`, `evaluated`, `inactive` or `skipped`),
nullable predicate `value`, `reason`, `fact_class` (`nominal_identity` or
`generated_items`) and the explicit fact-class `basis`. Records are deduplicated
per anchor **and fact class**: the same custom derive may be nominally admitted
and skipped for a method fact, and both evaluations remain visible.
`binding` checks disclose `stable_written_identity` or
`stable_written_identity_unproved` at the exact original/final path; the latter
retains the semantic source/final-fact refusal and may also disclose a reached
non-builtin-derive veto. An inactive candidate import/declaration additionally
records `inactive_written_binding` on its attribute.
`configured_dependency_root_with_conservative_context` identifies a configured
external root audited without custom-derive admission.
`written_identity_under_declared_configuration` describes nominal attribute admission;
`inert_under_declared_configuration` remains the generated-item context admission
reason. Other reasons identify undeclared atoms, unsupported predicates,
malformed/non-built-in derives, parser recovery or module macros; known predicates
disclose both true and false values. The same
coverage accompanies proofs and zero-proof results. Omissions explicitly state
that checks after a veto and inactive payloads were not evaluated; this is not an
exhaustive attribute inventory. Zero proved occurrences never means verified code.
The semantic digest binds the source snapshot plus graph, editions, features,
cfg atoms and analyzer identity; the overlay digest additionally binds all final
virtual source bytes. Manifests are not inputs and are not freshness claims.
All admitted source/ignore/mode/absence inputs are rechecked before applicability.
This is an observational snapshot, not atomic or application-time freshness.

Cancellation/deadlines use the existing request flag and admission permit, with
a joined controller cancelling both live Salsa databases. Cancellation is
cooperative, not a hard-latency promise; `BUSY` remains until owned work settles.
Oversized proofs/coverage withhold all artifacts and account for omitted
`binding_proofs`/`resolution_coverage`; the occurrence count survives. The
unmodified default path performs no resolution and gains no new output fields.

### Opt-in test-consumer acknowledgment

`move_item.acknowledge_test_consumers` defaults to `false`. Omission and explicit
`false` have identical serialized responses and retain the blocking policy.
With `true`, residual
consumer decisions whose only uncertainty is within the moved items' own file's
directly inventoried exact inline `#[cfg(test)]` module become **disclosed risks**,
not applicability blockers. The exact attribute accepts whitespace variations;
this is written evidence, not cfg evaluation. Detection shares the test-module
and super-import/glob classifier used by split advice; move acknowledgment also
covers macro-argument candidates such as `assert!` and `assert_eq!` without
expanding or rewriting their tokens. It also covers bare test references carrying
a `statement_macro_before_reference` witness at-or-before the reference or a
`block_macro_may_introduce_items` witness for a later standalone macro in its block
or an enclosing block, when the written super import reaches the selected file, the reference's
written type/value namespace is compatible with the selected item, and a separate
written-binding audit finds no competing local or other lexical uncertainty.
The witness remains disclosed on the acknowledged decision; this is acceptance
of expansion risk, not a proof that the macro cannot introduce bindings.

Each risk stays in `plan.decisions` with reason `test_consumer_acknowledged`,
`resolution:"risk_acknowledged"`, `blocks_applicability:false`, its complete
original source anchor and affected item IDs. Nonblocking decision groups retain
the IDs/counts. `coverage.test_consumers_acknowledged` counts acknowledged
consumer decisions (an import and an invocation can be separate decisions).
`plan.test_consumer_disclosure` states: "N consumers under cfg(test) acknowledged
by the caller; behavior under test is validated by the caller's test run, not by
this engine". Both fields are absent when no consumers were acknowledged. No
binding proof or test-result claim is added, and `semantic:"not_performed"`
remains unchanged on the syntactic path.

The engine does **not** repair these consumers. Review their anchors and run the
appropriate tests yourself after external patch application. The flag never
acknowledges cfg(feature), cfg(unix), other unexamined attributes, nested-module
uncertainty, macros outside this test module, other lexical/binding ambiguity,
known competing written bindings, namespace mismatches, syntax recovery,
member/constructor blockers, selected-item context, or module/root-chain vetoes.
Stale anchors and structural failures still block.
A mixed batch still withholds every artifact for its non-test residue. All
acknowledged risk records survive diagnostic-count preview caps even in blocked
plans; response-byte fitting can still omit counted detail and withhold artifacts.
Patch/edit/creation shapes and read-only behavior are unchanged.

### Supported units and ordinary layout

Functions, structs, enums, unions, traits, whole impls (including anonymous impls), type aliases, consts and statics have `supported_unit` inventory eligibility. Their dependencies may still block relocation. Whole modules have `module_context` eligibility reasons; use/extern/foreign constructs have `scope_dependency`; macro definitions/invocations have `macro_dependency`; other significant units are explicitly unsupported. Attributes/docs are associated constituents, not independently selectable inventory units. Nested/body/partial selections are rejected. Whole written inherent functions and
associated consts are selectable with `moves[].enclosing_impl`, an exact header-only
`SourceAnchor` ending immediately before `{`. Inventory `enclosing_impl` contains
that anchor, the whole impl `range`, unchanged `header`, nominal `written_type` and
`exclusions`; it never repeats the impl body. Trait/unsafe/negative impls, attributed
impls, cfg/unexamined member attributes, generic members and macro-generated members
remain excluded. Generic/where impl headers can be copied unchanged when their
written dependencies and type identity are preserved; no header transformation is
supported. Existing/new-sibling/new-child file destinations synthesize audited `impl_wrapper`
rewrites without reindentation. `{"kind":"existing_impl","path":"src/target.rs",
"implementation":<whole impl anchor>,"before_item":<optional member anchor>}`
merges only into a byte-identical header resolving to the same written type; an
insertion anchor must be an unselected member of that impl. Method visibility repairs
use the narrowest sufficient ancestor region (`pub(super)`, `pub(in crate::path)`
or `pub(crate)`), preserving the moved member's original access region. Choices
are explicit and rechecked against all proven callers; unproved field access still blocks.
Associated-item batches publish reached context evaluations once in
`plan.resolution_coverage.context_evaluations`; per-proof coverage retains metadata
with an empty context array.

`crate_root` is an admitted existing Rust file chosen as the analysis root, **not** an inferred Cargo target. Workspace members use their Git-root-relative source root (for example `member/src/lib.rs`), with every required chain file admitted; workspace manifests are not loaded. Source/destination identities require unique, ordinary written `mod name;` chains within admitted scope. Missing/competing/`#[path]` mappings and inherited context uncertainty block. References are scoped to the admitted corpus; no absence claim covers generated code or unexamined build targets.

The ordinary chain audit uses the same identity-inert lint/doc classifier as configured written-context checks (including prelude cfg payload checks) and admits bare built-in `allow`/`warn`/`deny`/`forbid` and `doc` metadata. This applies equally to crate-root and non-root module-file inner attributes and ordinary module-declaration outer attributes, including plain `doc = "..."` and `doc = include_str!(...)`, without executing expressions or reading documentation files. Nested `cfg_attr` with only those payloads or the exact documentation feature `feature(doc_cfg)` is harmless to module identity even with an unknown condition. Only that exact documentation feature is exempt: `no_std`, `no_core`, `macro_use`, `recursion_limit`, other feature gates and unknown/qualified attribute providers retain anchored chain refusals (unless a declared cfg wrapper proves its payload inactive). This exemption is **chain evidence only**: semantic fact, prelude, selected-item and binding audits remain independent; the stricter direct item/lexical attribute predicate is unchanged.

For `move_item`, one uniquely configured `semantic_configuration` entry matching `crate_root` can also admit active `cfg`/`cfg_attr` chain attributes, without requiring semantic resolution, under its explicitly listed ON features/cfg atoms. All operands must be known; unlisted atoms remain unknown, not OFF, even in a Boolean expression with a determining operand. Known-OFF declarations do not supply edges. Invalid matching edition/cfg data or duplicate matching roots, unknown/unsupported payloads, path remapping, prelude controls, parser recovery and competing declarations/layouts retain their vetoes. The root's `ModuleEvidence.assumptions` discloses positive cfg use; this is not Cargo discovery or compilation. `suggest_split` has no written cfg-chain admission opt-in and still refuses condition-dependent edges; its external-library configuration is consultation-only.

A `new_sibling` destination must be an absent literal `.rs` sibling of every assigned source, in an existing directory. Its basename is an ASCII identifier, not `_`, `mod`, a raw identifier, or any Rust strict/reserved/contextual keyword (including `gen`, `raw`, `safe`, `union` and `macro_rules`). The server never creates directories or reorders items within one file; only `new_child` can disclose the bounded external directory prerequisite below. Symlinks, hard exclusions, nested repositories, ignores, caller filters, existing entries, case-folded aliases, competing `name/mod.rs` layouts and declaration conflicts fail closed. Use an admitted existing directory in `paths`, not an absent path.

Ordinary children of the root or an existing `mod.rs` reside in that file's directory. Children of non-root `foo.rs` reside in `foo/`. Thus moving `src/source.rs` → `src/moved.rs` usually needs parent `src/lib.rs`, **not** `src/source.rs`. Existing legacy layouts are supported with ordinary evidence, but `mod.rs` is never created or restructured. One matching declaration with admitted chain attributes is reused without a parent edit unless a necessary visibility repair is separately linked. Relative/re-scoped restrictions, unverified access and public exposure requiring an API decision remain blockers. Incompatible layouts cannot be acknowledged away.

### Private direct-child extraction (`new_child`)

```json
{"repo_path":"/absolute/project","crate_root":"src/lib.rs","paths":["src"],"moves":[{"item":{"path":"src/worker.rs","range":{"start_byte":0,"end_byte":14},"expected_text":"fn helper() {}"},"destination":{"kind":"new_child","parent_path":"src/worker.rs","path":"src/worker/detail.rs"}}]}
```

Every assigned unit must originate in the existing admitted `parent_path`. The
path must exactly match that parent's evidenced ordinary child geometry:
`foo.rs` → `foo/detail.rs`, `foo/mod.rs` → `foo/detail.rs`, or a child beside
an evidenced crate root. The basename rules above still apply. No new `mod.rs`,
inline parent, layout conversion, invented ancestor chain or unrelated-source
aggregation is supported. Admit an existing higher directory in `paths`, such as
`src`, not the absent `src/worker` or future filename. Positive globs must admit
the future file. Generic `new_sibling` admission remains strict.

For a non-root flat-file parent, exactly one absent conventional directory can
be disclosed; its higher ancestors must already safely exist. Existing directories
also work. In-root inherited ignore rules are checked prospectively for both the
directory and leaf; negation cannot reopen a pruned ancestor. Hard exclusions,
symlinks, non-directories, nested repositories, case aliases, competing
`detail.rs`/`detail/mod.rs` layouts and batch conflicts fail closed. An absent
directory is never read, traversed or created by the server.

The planner synthesizes one private ordinary `mod detail;`, or reuses one unique
compatible private dangling declaration under the existing admitted attribute and
positive configuration rules. Public/restricted, inline, remapped, ambiguous or
unproved declarations refuse rather than convert. The child module is never
widened to support consumers outside its parent's access region. By default, retain
public facades explicitly by omitting them from moves; no re-export is inferred.
With explicit `parent_facade:true`, the bounded synthesis below can preserve the
original named binding without widening the child declaration.
Child code does not inherit parent imports: only evidenced explicit import/path
repairs and audited verbatim inherent-member wrappers are generated. The same
original/final binding, public-path, syntax, trivia, module and narrow-visibility
audits apply. Field/constructor/concrete-type/receiver and macro/trait/generic
uncertainty remains blocking; common historical extractions are not promised to
be applicable merely because this layout is expressible.

### Explicit path-preserving parent facade

`parent_facade` is a top-level `move_item` boolean, default `false`. It applies
only to `new_child` selections; other destinations retain their usual behavior.
The caller still chooses every item and destination. For example, to relocate
`pub fn helper() {}` from `src/worker.rs` into its private child while preserving
`crate::worker::helper`, add `"parent_facade":true` to the ordinary request.
The plan synthesizes `pub use self::detail::helper;` in the parent and keeps
`mod detail;` private. Existing callers and supported named re-export aliases
keep their old paths rather than importing the hidden child.

Each facade and moved declaration retain **exactly the original access region**.
An originally private item gets private `use` in the parent and `pub(super)` in
its child (or the equivalent crate-root region); `pub(super)`, `pub(crate)` and
`pub(in ...)` restrictions are translated to the same evidenced ancestor, never
a wider region. A genuinely public binding uses `pub use`. This is not a generic
`pub(crate)` repair and never widens `mod child;`.

Inspect `rewrites[].kind:"parent_facade"`, its full synthesis target with
`slot:"parent_facade"`, `written_reexport` evidence, original declaration,
module-chain, insertion-boundary and discharged consumer anchors, and supporting
visibility evidence. The `parent_facade` rationale records the unchanged binding.
These are written-route witnesses, not a semantic `binding_proofs` class or a
compilation claim. Existing schema 3 applies; no additional schema version.
`integrity.semantic` stays `not_performed` without resolution. Retaining any
required facade/visibility rewrite blocks all artifacts; replacement cannot
rename a facade, inject code or widen its original region.

Admission is conservative: only named top-level functions, consts, statics,
type aliases, structs with no fields or wholly public fields, and enums qualify.
Unsupported namespaces/associated selections, private or restricted fields,
ambiguous aliases, conflicting parent names, wildcard competition in the parent,
conditional item/module/re-export routes (even declared ON), generated namespace
context and incomplete original routes refuse. Existing typed decision reasons
remain; `parent_facade_unproved` identifies an anchored synthesis-admission
failure. The existing named re-export machinery supplies unique terminal identity,
access and eight-hop/cycle checks; no parallel resolver is used.

A `public_path_change` or glob need is removed only when **that particular old
route is proved unchanged**. For globs the supported subset is one direct simple
wildcard route to the original parent, with a complete unchanged export scope,
no competing wildcard/named candidate or unexamined local/provider context, and
original access covering the consumer. Grouped/forwarded, external, conditional,
competing or unproved glob routes retain their existing blockers. This does not
synthesize globs, waive semantic/test acknowledgment rules, or authorize narrowing
away consumers. `export_move_request.move_options` does not accept this flag;
review and explicitly edit the exported ordinary request before submission.

Derive/helper attributes, receiver/concrete-type facts, private-field access,
macro uncertainty, attachment/origin checks and the final moved-binder audit are
independent and unchanged. Out-of-line tests can keep proven named routes, but
conditional test context or macro calls may still block; there is no expanded
test-consumer acknowledgment. Apply the full patch in a disposable copy and run
the affected Cargo project's `cargo check --all-targets` externally.

### Known structural limitations

- **Macro-generated crate roots:** `crate_root!()`-style roots can emit module declarations from macro bodies; `include!(concat!(env!("OUT_DIR"), ...))` can supply build-generated declarations. The syntactic stage neither expands macros nor executes build scripts, so it cannot discover or prove these module trees. Such selections are advice-only, not applicable moves. Literal declaration-shaped tokens can name the macro cause; an opaque include without written declarations remains unproved rather than guessed.
- **Cargo autotest discovery:** creating a sibling directly under `tests/*.rs` can create a new integration-test crate under Cargo's default discovery rules. Ordinary module validity cannot see this build-graph change; review the target layout and Cargo test discovery externally before applying a patch.
- **Macro-invocation-as-item files:** when most of a file consists of macro invocations generating implementations, the inventory exposes those invocations, not their expanded items. This archetype is unsplittable by written-item moves; unlike ordinary inherent impl members, generated members are not selectable; use advice rather than treating generated methods/items as selectable written units.

Serialize MCP probes: a parallel call returns `BUSY` because the analysis slot is
occupied. The rejected call starts no analysis and is not queued. Wait for the
active call to finish or cancellation to settle, then retry serially; no retry
deadline is promised. A busy result is not an applicability measurement.

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
  ordinary unique edge under the admitted chain attributes and any explicit positive cfg declaration.
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
No alternate Cargo target, undeclared cfg outcome or root recommendation is inferred.

An unadmitted root still fails `STALE_SELECTION`; invalid new-file parents and
conflicting declarations still fail with their existing error and field. A
validated creation reusing a dangling declaration is not a missing-chain failure,
and unrelated failed edges do not veto a provable chain. If mandatory diagnostics
and decision links cannot fit, the response becomes incomplete, withholds drafts
or all move artifacts, and records omissions while removing linked arrays together.

### Trivia, synthesis and replay

Moves carry internal bytes, associated outer attributes/docs, contiguous owned leading comments and same-line trailing comments. Entire internal scopes travel intact, including their inner forms. File/module prologues, inner docs/attributes outside the item, ordinary first-scope headers and ambiguous banners/blank-separated blocks stay in place by default. No header, formatter, dedent, newline normalization or blank-line cleanup is invented. Each original interval has path-qualified, exact-once retained/carried provenance, including protected owner checks after reparsing.

`trivia_overrides` use `{trivia:SourceAnchor,disposition:"keep_in_place"|"carry_with_item",target_item?:SourceAnchor}`. Carry requires a fully anchored selected target and ordinary ambiguous trivia relevant to a selected source item; a different selected file may supply that target. Keep forbids a target. Unknown/stale/unrelated/duplicate anchors fail. Protected/owned/internal trivia cannot be independently detached, retargeted or discarded; unsafe choices withhold all artifacts.

Every path/use change, import insertion or leaf extraction, visibility repair, synthesized module declaration and separator is an itemized `rewrites[]` record with exact before/after bytes, syntactic confidence, contributing items and precise edit/create-content linkage. Boundary separators use nearest LF/CRLF without changing copied bytes. Created files receive an audited EOF separator and end with one newline by default. An EOF line comment is terminated with synthesized LF: if its copied CST bytes already include CR, that completes CRLF without adding a second CR; otherwise LF avoids altering the copied comment span. Separator targets expose `boundary_role:"before_payload"|"after_payload"`; their optional `binding` is an opaque original-run/declaration/insertion boundary identity, not a Rust binding. Fragmented runs have distinct identities. Replay the **entire published `target`**, not the display rewrite ID. Entries in `rewrite_overrides` are `{target:<published object>,action:"accept_default"|"retain"|"replace",replacement_text?:string}`. Only replace accepts text (≤64 KiB). Supported alternatives are complete simple paths to the same evidenced final target, explicit private non-glob imports (including evidenced alias/reference repairs), private, `pub(self)`, `pub(super)`, `pub(in ...)` or `pub(crate)` visibility with ancestor scope and all required access rechecked, safe newline separators, or the unchanged ordinary module declaration. A synthesized binding can also be replaced by a supported explicit path at its anchored written references. No arbitrary code, comments, header, wildcard, public API shim or new `pub` exposure may be injected. Source `retain` preserves the exact original span; synthesis `retain` emits no bytes. Rejecting a required repair leaves a linked unresolved binding/access/declaration decision and blocks every artifact. Every choice is reparsed and attachment/byte safety is rechecked. Unknown, stale, duplicate or conflicting targets fail; no server-side plan handle is required.

### Post-move import review

After assembling the simultaneous source overlay (removals, incoming items and
binding repairs included), the planner scans surviving private `use` declarations
in files supplying moves. A binding with zero remaining written-name occurrences
outside its own declaration receives a nonblocking `post_move_import_review`
decision in category `post_move_import`. Counts use Rust CST identifier tokens,
normalize raw identifier spelling, and exclude strings, character literals and
comments; names in other imports, macro token trees, binders and unrelated scopes
count conservatively. This is a whole-file spelling scan, not binding resolution.

Each advisory anchors the complete **original** import first, followed by the
source selections, and names the zero-reference bindings in
`unresolved_consequence`. `resolution:"advisory"`, `selected_choice:"retain"`,
`blocks_applicability:false`, empty `supported_choices` and
`action.route:"unsupported_in_engine"` mean review externally, **not** change
selection or submit an override. All import bytes remain unchanged by this scan,
including single names, renamed/grouped imports and partially unused groups.
Globs receive an explicit `glob bindings unenumerated: true` advisory rather than
invented imported names. Public/restricted `pub use` re-exports are excluded.
Entirely synthesized imports have no original declaration and are not cleanup
targets; destination-only files are not scanned, nor are recovered overlays.
Other applicable-plan decisions and blocked-preview limits still apply.

Token absence cannot prove an import unused: traits can enable method lookup
without a written trait name, and cfg/macro expansion is not modeled here. Imports
mentioned only in comments/strings are therefore flagged but retained. This scan
never deletes a binding, claims semantic unused-import proof, or guarantees a
warning-free build, including under configured semantic resolution. After applying
a reviewed plan externally, run the caller's compiler, remove only confirmed unused
bindings, preserve surviving group leaves, then run `cargo check` again. Review of
the advisory does not clear independent blockers or weaken lossless byte splicing.

### Written-binding repairs: repaired versus blocked

New private `mod name;` declarations are inserted after the last sibling file-module declaration, otherwise before the first `#[cfg(test)]` item and its attached docs/attributes, otherwise at EOF. Placement includes attached trailing comments and existing line endings, with the item/boundary recorded in the rewrite anchors. Required imports go after the last whole top-level `use` and its trailing attachments, or before the first item's leading attachments when there are no imports. Existing use groups stay adjacent and no existing code is reordered.

In multi-item `move_item` batches, pure engine-owned interior removal components leaving more than two newlines collapse to one blank line by default, only when retained syntax bounds both sides. Gaps touching BOF or EOF default to `keep_in_place`; explicit collapse and retain remain available. Single-item serialized plans remain byte-identical to their pre-batch-collapse shape, including decision strings and explicit-collapse audit evidence/rationale, even for oversized gaps. Single/double newlines remain unchanged. A nonblocking `decisions[]` entry with `category:"removal_gap"`, `reason:"removal_gap_choice"` publishes exact residual `before_text`, collapsed `after_text`, and `default_disposition`/`selected_disposition` (`"keep_in_place"|"collapse"`). The full original source region, including removed items, is anchored in `action.target`; `before_text` is the whitespace remaining after those items are removed. Omission and `accept_default` follow the disclosed default. To preserve the exact residual gap bytes, replay the entire published target in `rewrite_overrides` with `action:"retain"`; this keeps the removals, not the removed items. To collapse explicitly, use `action:"replace"` and `replacement_text` equal to `removal_gap.after_text`. No arbitrary whitespace or code replacement is accepted.

Every collapse produces a `removal_gap` rewrite linked to the combined original-coordinate removal splice, with complete original `before_text`, collapsed `after_text` and the decision ID. Batch defaults have `origin:"synthesized"`, `default_action:"accept_default"`; explicit replacements have `origin:"caller_override"`. The first line terminator and one complete blank line are kept (one blank line at BOF), retaining their LF/CRLF bytes and the next item's indentation. At BOF the collapse choice can also be offered for two blank lines, preserving the existing single-item override behavior. Unrelated blank-line runs are untouched; gaps overlapping another repair or sharing a boundary with an assembled payload insertion are not offered. A separate repair may start at the gap's end (for example, a visibility insertion on the next retained token) without consuming gap whitespace. Start-boundary edits remain excluded. Retained comment/attribute bytes are excluded, and attachment/syntax checks still apply. No whole-file formatting takes place.

For example, using the actual published decision `d` (not a display ID):

```json
{"rewrite_overrides":[{"target":"<d.action.target object>","action":"replace","replacement_text":"\n\n"}]}
```

The placeholder must be replaced by the full object and the replacement by the exact `d.removal_gap.after_text`; mixed endings or BOF gaps may have different bytes.

Visibility rewrites additionally carry `visibility` evidence (absent on other rewrite kinds):

- `consumers[]` contains each exact original `anchor:{path,range,expected_text}`, its
  `access_reason`, and `original_module_region` / `final_module_region`.
  Reasons are `written_binding`, `written_path`, `written_import`,
  `impl_header_dependency`, `impl_self_type`, or `same_type_inherent_access`:
  these identify the written binding/path/import, header occurrence, or uniquely
  matched same-type inherent access that actually required access checking.
  Shared accesses are deduplicated, including their final region, and sorted.
  An import anchor identifies its written path leaf, not inferred downstream calls.
- `original_defining_region` and `final_defining_region` locate the declaration;
  the original region is `null` only for a synthesized module declaration.
  Regions are module-segment arrays relative to `crate`; `[]` is the crate root.
- `preserved_access_region` is the existing restriction or final private defining
  region that the repair must continue to cover. `preserved_original_access_regions`
  separately records original defining regions retained for moved private members.
- `narrowest_covering_region` is the computed deepest common ancestor of those
  preserved requirements and the observed final consumer regions. It describes
  the default requirement, even if a caller selects a broader supported override;
  `after_text` remains the selected visibility bytes.
- `basis` and `rationale` explain the covering calculation. A preservation-only
  repair has empty `consumers` and explicitly says **no new observed caller**.
  It does not invent callers beyond the admitted observations, prove exhaustive
  access discovery, or add semantic verification. Preview evidence never clears
  independent private-field, constructor, concrete-type, or receiver refusals.

Defaults use complete CST paths and use leaves, not equal-looking text in comments, strings or token trees. Explicit aliases survive; grouped imports use an unchanged prefix, a shared prefix edit, or comment-free leaf extraction with unrelated leaves/trivia preserved. Local and inline-module import extraction keeps the original binding scope. Only necessary imports are synthesized; equivalent destination bindings are reused and shared needs are deduplicated. Items moving together do not create artificial cross-module imports or visibility changes.

Complete `crate`/`self`/`super` paths and at most one uniquely evidenced explicit module alias can identify a written target. Moved code is interpreted in its old lexical module, then mapped into the final batch. Recognized local functions, parameters, simple local bindings and generics are independent bindings, not leftover callers. Ordinary tuple/slice/constructor/struct binding positions are compared against the queried spelling: disjoint written names do not create uncertainty for every spelling in that scope. Explicit `mut`/`ref` binders and ordinary struct shorthand can prove independent locals; a matching plain identifier in a composite or refutable pattern still requires binding-versus-constant evidence. For/match/if-let/while-let patterns are checked only where their bindings are in scope, not in initializers, other arms or an if-let's else branch. Supported if/while `&&` let chains admit each pattern's bindings only in later operands and the success body, never its own initializer, earlier operands, else branches or post-condition code. Later bindings shadow earlier ones; disjoint supported patterns permit outer lookup. Parenthesized Boolean operands do not introduce bindings. Nested closures may capture locals, but nested items do not inherit them. Unsupported patterns, let-chain match guards, relevant local imports, macros and recovered/conditional lexical contexts remain anchored uncertainty; pattern constant competition and constructor identity checks remain independent. For spelling-specific admitted-glob evidence and the final written binder identity audit of all moved items, see the lexical uncertainty contract below. No macro expansion or semantic resolution is performed. Prelude assumptions are limited to the explicit opt-in type-position and bare value-position Option-constructor bridge described above. External explicit imports retain their written spelling, not a symbol-resolution guarantee; missing names are not guessed to be prelude imports on the default path.

Visibility checks inspect the item and each ordinary module declaration from its declaring parent scope. Already sufficient visibility and proven unchanged ancestor restrictions survive. A proven insufficient access receives the deepest common ancestor region containing the declaration's preserved access region and all proven final callers: `pub(super)` for a non-root immediate parent, `pub(in crate::path)` for a more distant non-root ancestor, or `pub(crate)` only when the root is the smallest covering region. Multiple callers merge into one repair and can escalate a previously narrower requirement. Moved private inherent members also preserve their original defining region's access, without inventing callers outside the admitted corpus. Retained relative restrictions can be widened when their parent scope is unchanged; moved relative/re-scoped restrictions still block. Visibility alternatives must denote an evidenced ancestor and cover the merged requirement; insufficient choices block artifacts with `visibility_scope_unproved`, while public exposure, non-ancestor paths, comments and injected tokens are rejected. Private fields/tuple constructors, unproved associated/type-directed references, relevant globs/macros, unadmitted conditional imports/modules and unsupported dependency forms remain decisions rather than guessed repairs. An unrelated glob/macro outside the relevant binding context is not by itself a veto. Consumer-glob candidates are checked against written module routes: file identity plus inline-module anchors, scoped module aliases, grouped use prefixes, and exact default-layout parent declarations. A same-named module in another subtree is not the source module. Proven different modules and enum-variant globs are excluded; a caller-selected file root does not inherit aliases from every unrelated `crate` import. Unknown/recovered routes, competing inclusions, local alias shadows, and possible wildcard/re-export/same-name forwarding remain conservative candidates. This is admitted written-syntax evidence, not Cargo discovery, cfg evaluation, macro expansion, or semantic name resolution. Optional `coverage.glob_exclusions` counts excluded selected-item/glob pairs by reason (`different_written_module`, `enum_variants`, `no_written_route`); it is not a count of files skipped or semantic absence proofs. Test-consumer advice uses the same written-route classifier, including aliases that glob the immediate test parent; it infers no edge from an unknown route.

Bare selected-name references in other admitted files are checked against visible glob imports, including `use super::super::*`, grouped/module-alias routes and possible chained forwarding. A reachable or unproved route emits `glob_consumer_unrepaired` in category `glob_dependency`, with `action.route:"selection_change_required"`, `blocks_applicability:true`, and no acknowledgment or rewrite choice. Its first anchor is the original reference identifier in the consumer file; subsequent anchors contain the complete original candidate glob statements. `refusal_basis` retains their `glob_import` ranges and affected name, plus any lexical uncertainty witness. This is conservative written reachability, not proof of glob binding identity. Named bindings in the same scope and proven independent locals are not glob-only consumers; sibling/parent-module imports are not inherited. Existing explicit-import and same-file repair paths are unchanged. These third-file risks cannot be waived by `acknowledge_test_consumers` or semantic
resolution. Explicit `parent_facade` may instead prove the narrow direct old-route
preservation case above; all other such blockers keep every artifact null. Keep the affected item in place or change the selected move; never narrow away admitted consumers just to obtain an applicable plan.

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

`plan` contains `state`, `applicable`, `selected_count`, `moves`, `trivia_decisions`, `decisions`, `decision_groups`, `chain_diagnostics`, `rewrites`, `origins`, `base_files`, `blockers`, `edits`, `created_files`, `patch` and `integrity`. Default outcomes disclose `semantic:"not_performed"`; explicitly configured opt-in resolution uses the scoped evidence/label described above. Legacy-only batches use schema 2 and its unchanged serialized shape: an applicable plan has all three non-null artifacts, and unsuccessful results set edits, creations and patch to null. Every typed batch containing `new_child` uses **schema 3**, including mixed, blocked, failed, cancelled and incomplete results, selected before analysis. Schema 3 adds `directory_preconditions` and `directory_diagnostics` to `plan`, and `directory_precondition_ids` to each created file. Applicable schema-3 results have all four artifact members non-null; unsuccessful results set **edits, created_files, directory_preconditions and patch to null together**. Reject unknown/unsupported artifact versions before interpreting or applying any artifact; never partially apply a mixed batch. `moves:[]` is an explicit checked no-op with empty edits/creations/patch and no virtual syntax claim; stray overrides are rejected.

Creations have complete `content`, `must_be_absent:true`, mode `100644`, parent and provenance links. `declaration_link` is discriminated: `{kind:"synthesized",rewrite_id}` resolves to a module rewrite, or `{kind:"reused",path,span}` resolves to the original declaration. Reuse alone adds no parent base/edit or fictitious rewrite. `declaration_visibility_rewrite_id`, when present, resolves to the separately audited required visibility repair on either a synthesized or reused declaration. Root-private modules ordinarily already admit descendant consumers and are not blanket-widened. Source files remain present even when emptied.

Schema-3 `directory_preconditions` are deduplicated records with `id`, root-relative
`path` (`.` for the repository root), `observed_state:absent|existing_directory`,
`required_state:absent_then_directory|existing_directory`,
`basis:{kind,parent_path,crate_root,parent_module_segments}`, and
`dependent_created_file_ids`. Basis kinds are `flat_file_child`, `mod_rs_child`,
`crate_root_child`, or `new_sibling` for a legacy creation in a mixed batch.
Created-file `directory_precondition_ids` and dependent creation IDs resolve in
both directions. There is **no directory mode, content, source range or empty-directory
Git artifact**; permissions are caller policy. Existing-directory records are
preconditions, not recreation instructions. Failure `directory_diagnostics`
contain `id`, `reason`, `path`, `parent_path`, `expected_state` and `observed_state`,
without any creation instruction or invented range. Directory state/identity
races use `CREATION_RACE` with `directory_creation_race`; file appearance uses
`file_creation_race`, and competing layouts use `competing_file_layout`. Unsupported
admission uses existing typed errors with `directory_context_unproved` diagnostics;
missing higher ancestors disclose `directory_creation_unsupported_layout`.

Existing-file edits are sorted by path/start/end, with additive item/rewrite links (absent from replacement results). Reconstruct existing files in reverse original-coordinate order after matching `original_text`; create files separately from complete `created_files[].content` only after rechecking absence. For schema 3, first recheck source/parent bytes and modes, directory captured states and identities, effective in-root ignore inputs, and competing layouts; then externally create only the disclosed `absent_then_directory` paths with caller-chosen permissions. Any appearance of a previously absent directory invalidates the plan, even if now safe. A Git patch instead creates the nested file (and necessary directory) without a separate directory artifact; `git apply --check` alone does not prove these freshness inputs. The server observationally repeats these checks before publication, never executes Cargo, formatters, builds or caller scripts, and creates nothing. Never interpret creation as insertion into a fictional empty base. Review all audit records together, then externally run `git apply --check` and apply on unchanged disposable copies. Creation sections use C-quoted Git paths, `/dev/null`, `b/<path>` and `new file mode 100644`; existing modes are unchanged and there are no deletion/rename/mode transitions. JSON reconstruction and patch application yield identical paths, bytes and canonical modes.

### Blocking summary

`plan.blocked_summary` is additive in both move schemas 2 and 3. Each entry is
`{reason,class,count,top_anchor}`: `reason` is the existing typed decision reason;
`class` is an additional refusal-basis string (or null when no basis exists),
not a new taxonomy. `count` counts distinct blocking decisions for that
reason/class pair. Every distinct class on a decision is represented, not just
its first refusal basis; repeated same-class witnesses count once. Counts across
classes are not additive. `top_anchor` is the first original witness in analysis
order, `{path,range?}`, or null if no original anchor exists; it is diagnostic,
not a byte-exact replay selection.

Aggregation uses the full internal blocking decision set before grouping or
preview caps. Nonblocking decisions do not contribute; nonblocked plans return
`[]`. `counts.blocked_decisions` is the observed distinct blocking-decision total
and `counts.blocked_summary` is the total number of reason/class entries before
budget fitting. Incomplete analysis still reports only observed decisions, never
unexamined blockers. Existing decisions, groups, blockers and their omission
counts retain their meaning and remain the detailed evidence.

Under response pressure only summary entries are removed to fit this additive
disclosure, without changing applicability, artifacts or any existing preview.
`counts.omissions.blocked_summary` counts omitted entries;
`counts.omissions.blocked_summary_decisions` counts their decision/class
memberships (not distinct decisions across classes). The totals remain and
`truncation_reasons` includes `blocked_summary_response_bytes`. An empty summary
with omissions does not mean the plan is unblocked. Like existing mandatory
response metadata, totals and omission disclosure survive even when the budget
cannot accommodate that irreducible metadata.

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

The default syntactic stage delivers design advice plus proven-complete patches
for a narrow class of written Rust. Opt-in bounded resolution widens that class
only for proved occurrences; general idiomatic Rust relocation is not guaranteed.

Common limits/admission/cancellation apply. `max_moves` defaults to 500 (1–5,000) and counts the whole explicit list. Fixed work guards are 100,000 inventory descriptors, 100,000 relevant reference candidates and 128 MiB aggregate analysis descriptors, including conservatively accounted transient binding/evidence records. Effective guards, observed counts and reference coverage are reported; query-state limits do not apply to direct CST analysis. Mandatory anchors/artifacts/audit must fit the complete duplicated wire response. `text_bytes:0` omits descriptive slices, **not** freshness checks or artifact bytes. On overflow, all artifacts are withheld and preview-array omissions are explicit. Calls allocate no search cursor/series.

Definite bad requests use `INVALID_ITEM_SELECTION`, `DUPLICATE_MOVE`, `STALE_SELECTION`, `INVALID_DESTINATION`, `INVALID_NEW_FILE_NAME`, `INVALID_DECLARATION_PARENT`, `DESTINATION_ALREADY_EXISTS`, `MODULE_DECLARATION_CONFLICT`, `STALE_DESTINATION`, `INVALID_MOVE_TRIVIA_OVERRIDE`, `INVALID_REWRITE_OVERRIDE` or `STALE_REWRITE_OVERRIDE`, with actionable input fields. Uncertain identity uses `CRATE_IDENTITY_UNCERTAIN`, not a guessed cross-crate assertion. Dependency decisions distinguish binding, glob, macro, re-export, module, visibility, inherited scope and unsupported-form categories with evidence and narrowing actions. Pre-existing/new syntax recovery, unsafe attachment, overlap, cancellation, work/output limits, source changes or `CREATION_RACE` withhold the entire batch. Final byte/mode/ignore/absence rechecks are observational, not atomic or application-time guarantees.

## Advisory file splitting (`suggest_split`)

```json
{"repo_path":"/absolute/project","crate_root":"src/lib.rs","source_path":"src/rich.rs","paths":["src"],"limits":{"text_bytes":0}}
```

This call inspects **one existing admitted file**. `crate_root` supplies the same ordinary written module context as `move_item`; it is not Cargo target discovery. Since **v0.5.0**, optional `include_balanced` resolves to false on omission: omitted/false returns no byte-balanced alternative; explicit `true` requests the existing distinct low-confidence original-order alternative only when structural advice and layout are valid. Pre-v0.5.0 releases included a distinct balanced alternative by default when structural advice existed; clients wanting that alternative now must send `include_balanced:true`. No call is promised two proposals, and the flag never relaxes ownership, exclusions, membership or move-proof policies. Optional `max_items` defaults to 500 (1–5000). `paths`, `globs`, `context` and `limits` have the usual meanings. There is no search cursor, saved executable plan, execution handle, or implicit selection. Historical retention and its separate detail tokens are opt-in (below). Advice consumes no search-series capacity and shares the same admission/cancellation/shutdown lifecycle as other tools.

The result has `advisory:true`, `source`, `inventory[]`, `overlaps[]`, `item_contexts[]`, `impl_contexts[]`, `scope_trivia[]`, `signals[]`, `decisions[]`, `chain_diagnostics[]`, `drafts[]` and `draft_eligibility`. It has **no patch, edits, creation content or move plan**, even when a draft is complete.

- Inventory is in original source order and includes every significant top-level written unit. Anonymous impl blocks have IDs and written type/optional trait spans in `impl_contexts`; they do not disappear because `name` is null. Outer attributes/docs are associated spans, not extra units. Each unit reports full original coordinates, kind/name, raw visibility, byte/line sizes, syntax flags, unit-kind eligibility/reasons and `signal_ids`.
- `supported_unit` means the top-level unit kind is supported, **not** that a move is dependency-free or safe. Modules, uses, extern/foreign and macro constructs remain inventoried with context-sensitive reasons and are retained in complete partitions. Direct written impl members remain `context_sensitive`: selection requires their exact header-only `enclosing_impl.anchor`. Members with empty `reasons` can enter draft sibling groups; members with nonempty exclusion reasons and their overlapping enclosing impls stay in the retain set. `counts.eligible_items` counts supported top-level units plus these non-excluded associated units, not proven safe moves. A `supported_unit`-only batch still excludes members; construct drafted member moves with the associated-item anchor shape described above.
- Unattached scope/prologue/ambiguous trivia has separate descriptors and a `keep_in_place` default. Banner links and `banner_section` signals mean adjacency only, never banner ownership. Internal scopes belong to their whole unit. Item contexts are original neighboring complete lines; the entire-source context has no lines outside the file.
- Signals expose snake_case/CamelCase word prefixes (≥3 characters, ≥2 names), counted directed bare/simple-path candidates with occurrence spans, banner sections, adjacent shared outer attributes/doc headings and item sizes. `get`, `set`, `new`, duplicate/common names, uncertain lexical bindings, unexpanded macros and type-directed accesses are not strong evidence. Same-spelled recognized locals/parameters/generics are not file-level edges. Inline-module bodies, macro tokens, strings/comments and unrelated qualified suffixes do not supply file-level reference proof. Candidates are **not a resolved call graph**.

### Selection completeness and extraction payoff

Full schema-2 advice and compact schema-1 manifests add `inspection_partitions`
and `extraction_payoff`; existing schema versions and field meanings are unchanged.
Every draft/membership group and inspection group has `selection_completeness`:
`state` is `incomplete`, `undetermined`, or `no_observed_missing_obligations`,
with `missing_item_ids`, `uncertain_item_ids`, and `applicability:"not_assessed"`.
Only the selected nominal's local written type/impl associations are assessed.
A whole impl or all its supported members can represent an observed obligation;
a subset, excluded trait/generic impl, or excluded member cannot. Ambiguous
written association is undetermined, not invented ownership. The absence of an
observed missing obligation says nothing about generated/outside-file obligations,
semantic completeness or move applicability.

When any extraction group's selection is incomplete/undetermined, the partition
appears in `inspection_partitions`, **not** `drafts`. The same happens for a
file with a dominant indivisible item when an extraction lacks a supported prefix
name: no uninformative `_part.rs` destination draft is emitted. Inspection records
have `id` (`inspection/N`), `source_snapshot_id`, `rationale`, `groups`, and an explicit
inspection-only `applicability` note. Groups have `kind:"retain"` or
`"extraction_core"`, exact-once `item_ids`, `selection_completeness`, original
`sizes`, `rationale`, advisory `confidence`, uncapped `consequence_summary`, and
`warnings`, but **no destination**. These are not execution
requests, byte-disjoint partitions, or local blocker forecasts. Candidates and
companions remain independently visible. `draft_eligibility.state:"no_draft"`
with `inspection_only_selection_or_payoff` explains this presentation; completed
analysis can still have `partition_outcome:"credible_written_candidates"`.
Existing destination-bearing drafts, their null-destination retain sets and
name-prefix/fallback collision ladder retain their meaning. Conceptual architecture
is never inferred from a proposed prefix.

Candidate `alternatives` additionally offer `kind:"complete_supported_bundle"`
when all observed local impl obligations have a supported explicit representation,
otherwise `"supported_bundle_subset"`. Both include the core, supported inherent
members (or a supported whole impl with no inventoried members), visible
`excluded_item_ids`, `overlap_ids`, and an inspection-only `applicability` note.
Trait/generic/ambiguous impls and unsupported members stay excluded. The additive
`impl_owner_bundle.facts.generic_impl` identifies generic/where headers or a
parameterized written owner for bundle review, without changing the existing
`excluded_impl` fact, companion traversal or member eligibility. Whole impl
IDs excluded in favor of members remain visible as overlap exclusions; they do
not alone make a supported member bundle incomplete. Review the alternative and
explicitly select it: neither it nor a companion changes the primary core.

`extraction_payoff.completed` indicates completed written payoff analysis.
`source_bytes`, `largest_items` (the largest three top-level units in descending
byte size/source order, with `item_id`, `bytes`, `lines`, and
`supported_member_partition`), and `dominant_item_ids` describe source shape.
Dominance uses the disclosed threshold of at least half of source bytes in one
indivisible unit. An impl with supported member partitioning is not counted as
indivisible, but a giant individual member can be; an impl with only excluded
members remains indivisible for this advice. Resolve IDs against the inventory.
`expected_useful_extraction` (`items`, unique original-range `bytes`, written
`lines`) and `extraction_item_ids` describe the primary's core-only selection,
not companions, balanced alternatives or generated file size. This estimate is
independent of destination availability and is zero when no supported core exists.
`basis` and `limitations` disclose these rules. Dominance never changes a negative
into a positive or vice versa: `no_credible_written_partition` still means no
supported core established, not that the file is already modular. A dominated
file with a real smaller core preserves that core for inspection.

Compact manifests carry both disclosures without fetching group detail. Retained
full group records and fitted `draft_summaries` retain group completeness;
inspection partitions are supplied directly, not a new detail-selector collection.
Response-budget fallback withholds inspection partitions/payoff with exact
`inspection_partitions`/`extraction_payoff` omission counts and sets payoff
completion false; incomplete work cannot establish absence of obligations or
useful extraction. No compilation, semantic verification or execution change is
introduced.

### Naming advice and destination availability

Naming is separate from structural partition credibility. Additive vocabulary in
`draft_eligibility.reasons` distinguishes observed `destination_scope_rejected`
(path/glob/ignore or other destination-admission restriction),
`destination_name_collision` (occupied path/case alias, competing module layout,
reserved sibling, declaration or import binding), and
`unsupported_destination_name_or_layout` (invalid module spelling or unsupported
child spelling). Multiple causes can coexist; no arbitrary dominant cause is
chosen. The legacy `no_admitted_nonconflicting_name_in_base_or_suffix_2_through_99`
remains as the bounded-search exhaustion marker, accompanied by these causes.
It is never evidence that the source is already modularized or has nothing to do.
Structural `partition_outcome`, candidates and extraction payoff remain unchanged.

Full draft/inspection group `warnings` separately report
`no_credible_concept_name` when the selection lacks a supported shared name prefix.
This is a bounded naming observation, not proof that no human concept exists:
no domain lexicon or architectural name inference is used. Names come from the
existing written-name prefix signals; type/signature relationships continue to
support cores without inventing names. The ordinary prefix → file-based `_part`
fallback → `_2` through `_99` ladder is unchanged. A fallback is explicitly a
**placeholder, not a concept name**; suffixes avoid collisions, not discover
concepts. Supply a caller-chosen explicit destination in `move_item` after review;
`suggest_split` has no caller-name request field. When search fails without a
supported prefix, `draft_eligibility.reasons` also includes
`no_credible_concept_name`. Successful groups disclose skipped scope/collision/
unsupported candidate counts in `warnings`; naming exhaustion preserves every
observed cause in eligibility reasons. Destination withholding for incomplete
selections or dominant indivisible items remains unchanged. None of these naming
outcomes means no useful extraction or establishes move applicability.

### Reading and editing partitions

Each draft has a nonempty retain group and up to two sibling groups. Every inventoried ID appears **exactly once** in each complete draft. A sibling destination supplies a proposed absent path, an evidenced ordinary declaration parent/module chain, and an existing matching declaration descriptor when reused. The server probes the base name then `_2` through `_99`, checking scope admission, ignores, filesystem/case/layout occupancy and declaration/import collisions. It never guesses a parent from the source basename.

The primary proposal uses structurally supported `ownership_candidates`, never unions from prefixes, banners, headings, shared attributes or adjacency. Seeds are unambiguous written-reference strongly connected components, unique local nominal-type/impl associations (`impl_owner_bundle`, basis `written_type`), and supported acyclic member relationships within one written owner. Competing written nominal/import bindings leave the type/impl association ambiguous and cannot seed a core. A singly consumed member or a private signature payload attributable only to that owner's subset can support a member seed; a shared owner, helper, constant or orchestrator cannot join otherwise separate cores. Unsupported impls are disclosed, not newly eligible moves.

`ownership_candidates[]` exists independently of draft layout. Each record has a response-local `id` (`candidate/N`, in ranked order), `core_item_ids`, `structural_signal_ids`, `companions`, `alternatives`, `ranking`, `observation_scope` and `selection_policy`. Each companion has a unique response-local `id` (`companion/N`, ordered by target path/range/kind, then candidate order for repeated targets), target inventory `item_id`, `role`, `review_obligation`, `classification`, original evidence through signal/observation IDs, observed consumer IDs, exclusions and stopped-edge reasons. Boundary records' `ownership_candidate_ids` link to these `candidate/N` IDs. IDs are deterministic for the same analysis inputs, not persistent identities or execution anchors. Worklist traversal uses a visited set and existing guards; it stops at imports, exposed/public boundaries, shared nodes, unsupported contexts and uncertain identity. Companions never change core membership. Whole-impl and member alternatives expose overlap IDs and excluded IDs; explicitly choose one shape, never both. A type core does not silently select all of its methods.

Companion `review_obligation` derives from candidate-to-companion association evidence, independently of consumer `classification`, traversal stops and move eligibility:

- `selection_completeness`: a supported written type/impl association requires review of the complete-impl versus explicit-member selection shape, retaining any exclusions.
- `boundary_dependency`: an established written dependency may remain accessible in the parent, subject to independent move audits. Written references across shared/exposed/import boundaries and ordinary payload/helper/constant references use this value; even observed-exclusive consumers do not prove mandatory co-location.
- `association_unproved`: no supported written evidence establishes the candidate-to-companion association, for example because its binding identity is ambiguous. Do not infer completeness or a retainable dependency from this association. Uncertain consumer attribution alone does not erase a supported association.

None of these values selects a companion, proves access/repairability or waives exclusions.

Companion `classification` is `observed_exclusive`, `observed_shared` or `undetermined`. Exclusive means only the completed admitted written observations attribute consumers to the inspected component, not architectural ownership. Ambiguity, unsupported contexts and incomplete work preclude exclusive claims. Coverage excludes unadmitted/generated/build-target contexts even when classification is observed-exclusive.

`ranking.version:1` publishes integer `structural_support`, `distinct_internal_relationships`, `reference_occurrences`, `boundary_relationships`, `uncertain_relationships`, `weak_agreements` and `source_order`. Order is descending structural support, descending distinct unit relationships, ascending boundary costs/uncertainties, descending weak agreements, then original source order. Structural support distinguishes complete-owner member cohesion (3), reference/member subsets (2), and nominal bundles (1). Repeated calls count once for relationships; occurrences are separately disclosed. No probability or move-safety score is supplied.

`boundary_observations` has its own `coverage` and `records`: admitted/observed paths, completion, unsupported-context counts/examples, uncertainty and explicit unassessed domains. Records link incoming consumers or outgoing dependencies to inventory/candidate IDs, exact original occurrence `anchor`, optional enclosing unit/counterpart, and `route_evidence`. `certainty:"written_route"` means an ordinary written module path or explicit file-level import to an admitted declaration, not resolved identity. Glob/ambiguous routes are `candidate`; forwarded named routes, inline modules, macros and arbitrary receivers remain outside this bounded observation channel. This does not run a move planner or prove repairability. Legacy `assessment_scope.assessed:"local_only"`, its `other_consumers` unassessed entry, and `expected_to_block` meanings remain unchanged.

`partition_outcome` distinguishes `credible_written_candidates`, `no_credible_written_partition`, `insufficient_input`, `unsupported_layout`, `unsupported_input`, `incomplete_analysis` and `failed_analysis`. Only completed structural analysis may establish no credible partition; it retains inventory and weak evidence but emits no balanced primary. With a credible core, `include_balanced:true` may additionally return the labeled low-confidence original-order byte-balanced alternative, never a natural semantic boundary or an implicit primary. Omitted/false excludes it. True does not override no-credible, insufficient-input, incomplete-analysis or unsupported-layout outcomes. Other units remain in source, retaining the earliest eligible unit if necessary; equivalent alternatives are not duplicated.

Group `facts`, rationale and confidence expose integer organization evidence, not a probability of compiler correctness. High confidence requires at least two independent signal families and complete layout/membership; execution risks remain unresolved. Sizes count written item bytes/lines, not a prediction of generated file size. Exact-once ID membership is **not a disjoint byte partition**: whole impl descriptors overlap their members, even across groups. Member `enclosing_impl_id` links to the whole impl inventory ID, independently of the exact header-only execution anchor in `enclosing_impl.anchor`.

`overlaps[]` emits one original-range containment edge per member whose enclosing impl is inventoried, in member source order. Each record has a response-local `id` (`overlap/N`), `relation:"member_contained_in_impl"`, `path`, two `item_ids` (impl first, member second), and corresponding original byte `ranges`. Its `draft_groups[]` links each draft by `draft_id` and zero-based `impl_group_index`/`member_group_index`; different indices explicitly identify cross-group overlap. Inventory entries and participating draft groups carry `overlap_ids`, and overlapping entries/groups carry:

```json
"size_interpretation": {
  "basis": "original_descriptor_ranges",
  "additivity": "non_additive",
  "generated_module_size": "not_estimated"
}
```

A group is labeled even when the other descriptor is in another group. Existing item `bytes`/`lines` and group `sizes.items`/`bytes`/`lines` remain unchanged descriptor values/sums, not a range union or a generated module estimate. Absence of `size_interpretation` is not an additive-size or move-safety guarantee. Containment is an execution alternative: explicitly choose a whole impl **or** its members, never both; advice never prunes either descriptor or changes membership. The 64-KiB/256-line group aim is **soft advice**: whole oversized items remain indivisible and get warnings. Cross-group candidate IDs and warnings identify import/path/visibility review needs.

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
When members have separate inventory descriptors, candidate and body-risk walks run
on those members rather than again on the retained enclosing impl. Member risks,
including excluded members, remain visible; forecasts do not double-count the
same body occurrence merely because its impl also appears in the inventory.

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
These legacy signals remain separate from the broader advisory projection below.
These signals project the existing conditional-consumer decision reason;
**default execution behavior is unchanged**: same-file cfg(test) consumers still
block relocation in `move_item`. The separate explicit `acknowledge_test_consumers`
option above discloses supported consumer risks instead; advice itself never
acknowledges them. No cfg evaluation or safe-relocation guarantee is introduced.

### Consequence routing

Every `ownership_candidates[]` record and every draft group (including retain
sets) carries `consequence_summary`, computed once from finalized **uncapped**
evidence before diagnostic shaping and response fitting:

```json
{
  "basis": "uncapped_written_evidence",
  "count_unit": "distinct_decision_records_per_class",
  "non_additive": true,
  "classes": {
    "advice_only_review_risk": {"count": 1, "decision_refs": [{"collection": "advice_decisions", "id_runs": [{"first_id": "ad/0", "count": 1}]}]},
    "selection_incomplete": {"count": 1, "decision_refs": [{"collection": "advice_decisions", "id_runs": [{"first_id": "ad/0", "count": 1}]}]},
    "written_context_unprovable": {"count": 0, "decision_refs": []},
    "public_path_change": {"count": 0, "decision_refs": []},
    "consumer_outside_supported_repair": {"count": 0, "decision_refs": []},
    "missing_evidence": {"count": 0, "decision_refs": []}
  },
  "unmapped": {"count": 0, "reasons": [], "decision_refs": []},
  "destination_and_batch_applicability": "not_assessed"
}
```

Counts are distinct decision **records per class**, not additive blocker/unit
counts. One record can contribute to multiple classes and to `unmapped` alongside
known reasons. Exact runs never bridge interleaved causes; collection names
separate legacy `d/N` decisions from new response-local `ad/N` advice decisions.
Unknown reasons appear in `unmapped.reasons` with exact links, never silently zero.
This is routing, not a safe/unsafe badge or an execution selection. Zero counts
still leave destination/batch applicability unassessed; legacy `expected_to_block`
lower bounds and `assessment_scope` are unchanged.

`advice_decisions[]` projects each companion, boundary record, test record or test
limitation without adding local blockers. Records carry `id`, `reasons[]`, affected
`item_ids`, `evidence_refs:[{collection,id}]` and `applicability:"not_assessed"`.
Evidence collections are `companions`, `boundary_observations`, `test_observations`
and `test_limitations`. IDs resolve to the original records; `test_limitations`
uses the zero-based array index and boundary-wide unsupported context uses
`{collection:"boundary_observations",id:"coverage"}`. A companion record concerns
its candidate's core IDs; boundary/test records concern their linked target IDs.
Empty item IDs mean source-wide uncertainty and apply to every candidate/group.
Candidate summaries use core membership, draft summaries use group membership;
companions/alternatives are not silently added. Selection-completeness routing
asks for explicit whole-impl/member review, not proof the caller's edited batch
omits a required item. A missing-evidence decision counts one reported limitation,
not the number of missing files or unseen consumers.

The reason-to-class mapping is explicit (multiple rows can apply to a record):

| Reasons | Classes |
|---|---|
| `selection_completeness` | advice-only review risk + selection incomplete |
| `boundary_dependency`, `boundary_observation_review`, `test_coupling_review`, `cross_group_reference_review`, `test_consumer_acknowledged`, `ordinary_trivia_choice`, `removal_gap_choice`, `post_move_import_review`, `public_or_exposed_boundary`, `import_boundary`, `observed_shared_node` | advice-only review risk |
| `association_unproved`, `uncertain_identity`, `unsupported_context`, `boundary_identity_unproved`, `attributed_import`, `competing_written_binding`, `ambiguous_import_routes`, `glob_binding_candidate`, `lexical_macro_context_unproved`, `field_attributes_unproved`, `scope_attributes_or_syntax_unproved`, `ambiguous_target_identity`, `target_unresolved`, `macro_tokens_not_expanded_or_bound`, `enclosing_attributes_unproved`, `method_identity_unproved`, `token_shape_only` | advice-only review risk + written context unprovable |
| `unexamined_context`, `boundary_context_unexamined`, `unlinked_test_root`, `unsupported_attributes_or_remapping`, `competing_layouts`, `competing_module_routes`, `unsupported_test_cfg`, `unsupported_scope_attributes` | advice-only review risk + written context unprovable + missing evidence |
| `missing_test_file`, `unadmitted_test_file` | advice-only review risk + missing evidence |
| `module_chain_failure`, `external_or_missing_binding` | written context unprovable + missing evidence |
| `lexical_context_unproved`, `source_binding_ambiguous`, `destination_binding_conflict`, `final_alias_conflict`, `member_or_constructor_unproved`, `visibility_scope_unproved`, `conditional_or_inherited_context`, `macro_context_unexamined`, `glob_binding_unproved`, `unsupported_construct`, `trivia_preservation_unproved` | written context unprovable |
| `public_path_change` | public-path change |
| `glob_consumer_unrepaired` | written context unprovable + consumer outside supported repair |
| `unsupported_unit_kind`, `required_rewrite_retained` | selection incomplete |
| Any other reason | unmapped, preserving the reason and membership |

Only the existing typed `glob_consumer_unrepaired` decision maps to consumer
outside supported repair, prospectively under that execution contract. Arbitrary
outside/test observations (including glob candidates) never establish that a
caller-selected batch needs an unsupported repair; their repairability stays
unassessed. Severity/category prose does not select classes.

`counts.advice_decisions` reports the uncapped projection total. Display returns
at most `limits.diagnostic_count` advice records in ID order (default 64), with
exact `counts.omissions.advice_decisions`; zero/default/full expansion cannot
change summary counts or links. Omitted records are not currently retrievable by
handle: expand diagnostic and response budgets for details. Response fitting
preserves each group's summary in `draft_summaries` if full drafts are withheld.
At final overflow tiers candidates/membership summaries may themselves be
withheld with their existing omission counts; surviving summaries are never
recomputed from the reduced display, and partial output is not complete evidence.

### Advisory test observations

`test_observations` is a separate schema-2 advice object with `coverage`, `routes`,
`records` and `limitations`. It does not modify ownership/boundary records, legacy
`cfg_test_consumer` signals, local forecasts or execution acknowledgement.

- `coverage` reports `completed`, admitted/traversed paths, `unlinked_test_roots`,
  `not_assessed`, token-shape coverage and an explicit zero-observation caveat.
  Completion means the admitted written scan finished, not complete test coverage.
- `routes[]` links exact module declarations and attached attribute anchors to
  conventional flat-file and `mod.rs` candidates and their `admitted`, `unadmitted`
  or `missing` states. Exact `#[cfg(test)]` establishes conditional discovery only,
  never an execution-admissible cfg edge. Supported routes have status
  `conditional_written_route` or `inline_written`; uncertain inline scopes use
  `inline_uncertain`. Missing/unadmitted files, competing layouts and unsupported
  attributes/remapping are explicit limitations, not zero-consumer reassurance.
  Admitted files with exact `#[test]` or positive `#[cfg(test)]` markers (ignoring
  whitespace) but no supported route are disclosed as `unlinked_test_root`;
  directory names never establish Cargo targets/library aliases. Other cfg expressions
  mentioning `test`, including `cfg(not(test))` and `cfg_attr(test, ...)`, yield anchored
  `unsupported_test_cfg` limitations, not test-root or coupling evidence.
- `records[]` has a response-local `test-observation/N` ID, `channel`, `access_kind`,
  original occurrence `anchor`, known enclosing function/modules, optional written
  module segments, `route_ids`, `route_evidence`, candidate `target_item_ids`,
  `attribution`, typed `uncertainty`, non-exclusive `labels`, `candidate_couplings`
  and `possible_companion`. Route IDs are `test-route/N`, not execution anchors.
- The `cst` channel observes path/field/method accesses independently of line wrapping.
  Field attribution requires a simple lexical receiver with a written nominal
  parameter/let type or literal struct initializer and a matching written field
  on that target. Receiver chains, inferred/generic types and method identity remain
  unproved; a same-spelled field on an unrelated type never establishes a link.
  Visible glob imports contribute candidates with `glob_binding_candidate`, not
  certain binding identity. Block macros retain lexical uncertainty even where a
  simple written parameter/let candidate can be inspected.
- `macro_token_candidate` observes bounded written identifier paths and simple
  `identifier.field` token shapes in assertion/other macro inputs, excluding comments
  and literals. Anchors cover the complete observed token shape. These are not
  expanded expressions, binding proofs or assertion-equivalence evidence.
- Labels are `facade`, `moved_implementation`, `private_state`, `mixed`, `unresolved`.
  They are non-exclusive; candidate-relative labels link to ownership candidate IDs
  without changing their membership. `possible_companion:true` only asks the caller
  to inspect a test, never selects or mandates relocation. No assertions, placements,
  visibility or test acknowledgement eligibility change. Out-of-line/third-file glob
  consumers still block where the move contract requires, with either flag value.

The projection uses the shared reference/descriptor guards, cancellation and time
budget. Response fitting strips display text first, then records exact
`counts.omissions.test_observations`, `test_routes` and `test_limitations` if the
projection must be withheld; stopped/withheld projection coverage is not complete.
`counts.test_observations` is the analyzed record count, not a test count or safety score.

Risk fields and signal kinds are unchanged by counts-first response shaping;
`semantic:"not_performed"` is unchanged. Full `suggest_split` publishes schema version 2
because decision-group IDs now use exact runs rather than strings; the opt-in compact
manifest is separately identified as schema 1.

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

Under full-response budget pressure, after display text and neighboring item
contexts have been fitted, `item_size` and `name_prefix` signals may have
`evidence:[]` instead of repeating declaration-display spans. This tier requires
complete analysis and complete same-response inventory, and verifies that every
original descriptor equals the inventory span for the corresponding ordered
`signal.item_ids` entry, including ranges, positions, text bytes and omission flags.
`counts.omissions.duplicate_declaration_display_spans` counts the removed duplicate
**display descriptors**, not missing analysis or fewer observed references.

Clients that read direct declaration evidence arrays must join the existing IDs
back to `inventory[].span` in `item_ids` order when this count is present:

```python
by_id = {item["id"]: item for item in advice["inventory"]}
shared = advice["counts"]["omissions"].get("duplicate_declaration_display_spans", 0)
for signal in advice["signals"]:
    evidence = signal["evidence"]
    if shared and signal["kind"] in ("item_size", "name_prefix") and not evidence:
        evidence = [by_id[item_id]["span"] for item_id in signal["item_ids"]]
    # Inspect evidence as display descriptors, never as complete move anchors.
```

No retained handle or follow-up request is needed for this reconstruction. Signal
IDs, ordered item IDs, facts, limitations and endpoints remain unchanged, as do
all unique reference/consumer occurrence spans, decisions, candidates, companions,
consequence summaries and full draft memberships. Already-fitting responses keep
their direct evidence arrays. Canonical retained records are captured before this
projection and remain unchanged; compact manifests use their separate contract.
A missing ID, unequal descriptor/order, incomplete inventory/analysis or a still
oversized projection does not qualify: the existing conservative partial-output
path remains. Completeness is preserved only when the actual structured/text
encoding, escaping, omission metadata and framing reserve fit the effective cap.

When output fitting withholds full drafts, `draft_summaries[]` retains each draft's
ID, source snapshot, group kind/destination path, complete `item_ids`, `overlap_ids`,
non-additive `size_interpretation` where present, uncapped `consequence_summary`,
and unresolved decision IDs.
If overlap records are withheld, `counts.omissions.overlaps` and
`overlap_draft_group_links` account for them; surviving links and labels do not
promise that omitted records or display inventory are locally available.
These are non-executable membership summaries, not complete drafts;
the omitted decision records need not resolve locally. Display inventory is trimmed
before these summaries or decision groups; root/snapshot/coverage/omissions survive.
Only the final tier may omit summaries, with exact summary/reference counts.

Empty/singleton/recovered or unsupported-layout files return inventory and an explicit no-draft reason. A work, discovery, membership, freshness or mandatory evidence/output limit gives incomplete advice and withholds **all complete drafts**. Counts distinguish observed inventory/descriptors/candidates from returned items; `counts.omissions` records suppressed arrays and membership/decision links. Dropping source/context display text or sharing the proven duplicate declaration displays described above preserves complete drafts when full coordinates and required evidence links fit. `span.text:null` is not a shortened `expected_text`: obtain the complete current original bytes before execution. Integrity is input-only (`input_checked`, `input_recovered` or `not_checked`), always `semantic:"not_performed"`.

### Retained advice and compact manifests

`retain_snapshot` is a strict boolean, default false. `response_mode` is `full|compact|references`,
default full; neither compact nor references implicitly retains. Legacy full retention-off calls
allocate no retained record and keep their previous wire shape (no `retention` field).
Explicit retention adds a sub-result independently of full-response presentation.

```json
{"repo_path":"/absolute/project","crate_root":"src/lib.rs","source_path":"src/worker.rs","paths":["src"],"retain_snapshot":true,"response_mode":"compact","limits":{"response_bytes":2097152,"diagnostic_count":0,"text_bytes":0}}
```

Only completed, rechecked canonical analysis may be retained. Capture precedes
`max_items` display trimming, diagnostic shaping and wire fitting. Canonical
records use original coordinates and immutable buffers, not display snippets;
retained text is stored once and reconstructed without formatting or reanalysis.
No parser trees, rust-analyzer databases, open files, watchers or executable plans
are retained. Full fitting keeps its existing conservative partial-output meanings;
a retained complete canonical record can therefore outlive a partial full display.

A successful `retention` has `state:"retained"`, opaque `analysis_handle` and
`analysis_id`, corpus `snapshot_id`, separate `scope_input_digest`, normalized
`normalized_request`, grammar/server/build/grouping `provenance`, informative
RFC3339 `expires_at`, effective `limits` and `accounted_bytes`. Metadata includes
all effective request options and whether diagnostic expansion was explicit.
At v0.5.0, normalized `include_balanced` records the effective boolean: false
for omission/false and true for explicit opt-in, alongside server-version
provenance. Full, compact and retained historical group memberships follow that
same option; retrieval does not add a balanced alternative or regroup the record.
Handles identify one analysis, not a corpus hash or authorization. Separate
analyses on identical bytes/options have different handles. With unavailability,
there is no usable handle and `state:"unavailable"` names one reason:
`capacity`, `record_too_large`, `analysis_incomplete`, `response_budget` or
`cancelled_before_publication`. Otherwise complete analysis remains complete.
Compact retention-off reports `state:"not_requested"`.

Retention defaults are **one active record, 134,217,728 aggregate accounted bytes
(128 MiB), and a fixed non-sliding 900-second lifetime from publication**. They are
not runtime request settings. The byte cap is a maximum budget, not an upfront
allocation, repository-size limit, response-byte limit or total-process RSS ceiling.
Accounting conservatively includes owned container/string capacities,
descriptor/node reserves, buffers, input manifests and identity metadata,
separately from transient analysis/output and process peak RSS. Capacity is
reserved before additional retained-record materialization using a conservative
non-materializing bound; pending preparations occupy the record slot and byte
capacity, refunding or transferring their charges on failure or publication.
The bound may refuse retention even when final stored charges alone would fit.
This does not eliminate existing analysis/output buffers or bound total process memory.
Reducing compact JSON does not reduce the canonical retention reservation: full
uncapped decisions and test observations remain owned even when absent from the
manifest. Raising `response_bytes` can resolve `ADVICE_MANIFEST_TOO_LARGE`, but
cannot resolve `record_too_large`. Live retention may report `response_budget`
when the mandatory compact manifest exceeds `response_bytes`, even if the cache
also cannot admit the canonical record. Increasing `response_bytes` addresses
wire fitting only, not live-cache capacity or `record_too_large`; persisted spill
files remain independently available. Do not narrow away required consumer evidence
or assume historical inventory/drafts are retrievable merely because a manifest fits.

For read-only allocation measurement against a caller repository, run the ignored
unit probe (from the server checkout):

```sh
SPLIT_MEASURE_REPO=/absolute/project cargo test measure_coupled_manifest_and_retention --lib -- --ignored --nocapture
```

The probe reports JSON bytes by manifest field, duplicated MCP wire bytes, the
preparation reservation, final stored charge, canonical allocation by field, and
captured-evidence allocation. Its reference-table sizes are **counterfactual only**,
not a supported response format or a retention optimization. It uses the twelve
large source paths listed in the probe; other repositories must adjust those paths.
Test-only injectable limits and a cap-neutral allocation probe remain available
for lifecycle and measurement checks; they add no runtime knobs or MCP flags.
Registry removal does not subtract records or bytes still owned by an in-flight
reader. Expiry is monotonic and fixed, not extended by navigation; expired
payloads are reclaimed lazily. Release is explicit. No silent eviction,
background refresh, jobs or restart-survival promise exists for live handles.

#### Durable analysis files and reference presentation

Explicit `retain_snapshot:true` also spills the entire pre-wire-fitting canonical
envelope to server-owned state, independently of the bounded live-handle cache.
`retention.spill` reports `state:"persisted"`, absolute `directory` and `index`
paths, and a `files` map from canonical topic names to absolute paths. A successful
spill remains usable even when live retention reports `record_too_large` or
`capacity`. `get_split_detail` and `export_move_request` still require live handles;
they do not load spilled files. Their limits, expiry and applicability are unchanged.

The reserved tree is `$XDG_DATA_HOME/rust-sitter-mcp/analyses`, or
`~/.local/share/rust-sitter-mcp/analyses` when XDG_DATA_HOME is unset. XDG_DATA_HOME
must be absolute. The server rejects a spill location inside the caller repository,
including symlink-resolved paths. Each unique `analysis-<process>-<time>-<sequence>`
directory contains `index.json` and UTF-8 pretty JSON files such as
`test_observations.json`, `decisions.json`, `advice_decisions.json`,
`ownership_candidates.json`, `drafts.json`, `inspection_partitions.json` and
`inventory.json`. All envelope fields have matching topic files; `request.json`
records options, `source_buffers.json` preserves captured source bytes as JSON
strings, and `scope_evidence.json` preserves captured scope/input metadata. The
index discloses canonical omissions and whether evidence was captured. These are
full **captured** records, not a promise that stopped analysis scanned everything.
No new observation cap is introduced. In `test_observations.json`,
`decisions.json`, `advice_decisions.json` and `ownership_candidates.json`, anchors
whose canonical `span.text` was omitted receive a disk-only `captured_text` field
alongside `path` and `span`, sliced from the immutable captured source buffer.
This makes anchored symbol spellings searchable even with `limits.text_bytes:0`.
`test_observations.json` also includes a disk-only `captured_sources` path-to-text
map of all captured buffers, so symbols outside observation anchors are searchable
in that topic too. This map duplicates `source_buffers.json` on disk, not on the wire.
Canonical `span.text`, `text_omitted` and omission counts remain unchanged, as do
all wire presentations. No text is read from the current caller filesystem;
unavailable buffers or invalid byte/UTF-8 ranges are never guessed. Other topics
remain canonical; `source_buffers.json` provides the captured originals. Grep
decision reason values (for example `lexical_context_unproved`), not Rust type
names. For example, on the server host:

```sh
grep -n 'observation spelling' /absolute/analysis-directory/test_observations.json
rust-sitter-mcp flush
```

Files survive expiry, release and restart until explicitly flushed. `flush` removes
only the reserved analysis subtree and reports entry count/location; absent state
is a successful no-op. Source buffers can contain sensitive caller content; protect
the server account and flush when no longer needed. Analysis directories are private
on Unix. Paths refer to the server host, not necessarily the MCP client's filesystem.

`response_mode:"references"` plus `retain_snapshot:true` explicitly opts out of
duplicated inline manifest summaries. It returns the schema-1 manifest with
`response_mode:"references"` and a `references` map: `candidate_summaries` and
`consequence_summaries` point to canonical ownership candidates; `draft_memberships`
and `coverage.local_forecasts` point to canonical drafts; `decision_groups` and
`inspection_partitions` point to their matching files. Corresponding inline arrays
are empty, their counts are disclosed in `omissions`, and local forecasts are not
inlined. These references identify richer canonical records, not identical display
projections. Totals, analysis coverage, uncertainty and input-only integrity remain
inline. Full and ordinary compact modes preserve their previous inline meanings;
spill metadata is additive. Without requested retention, references falls back to
ordinary compact presentation.

Writes stream each topic through a cancellation-aware writer into a temporary
private directory, flush/sync files, then rename the completed directory. No network
is used. A write/path/cancellation failure reports `retention.spill.state:"unavailable"`
and a `reason`, publishes no file pointers and preserves the ordinary inline contract
(including fallback to `response_mode:"compact"` for references). Spill failure does
not by itself change analysis applicability or semantic labeling. Concurrent flush
can invalidate previously returned pointers; re-check file existence before use.


`scope_input_digest` binds the corpus snapshot, normalized scope and effective
in-root `.gitignore` inputs consulted in discovery, including absence and exact
bytes at observed directories, plus observed directory/source/ignore-file device/inode
identities and permission modes. Replacement by equal bytes is not silently relabeled
as the same observation. It does not change `snapshot_id` or search-cursor
semantics. Hard-excluded, symlink and nested-repository boundaries remain intact.
Policy observation is bounded to 100,000 entries, 1 MiB per input and 16 MiB
aggregate path/byte/entry accounting (also covering at most 200,000 filesystem
identity/permission entries) within the advice descriptor/time guards;
unreadable, unsafe or over-limit relevant input fails closed. The retained source
manifest records admitted paths, modes, lengths and content digests. This is a
freshness seam used by export, **not** a claim that historical detail checks
current source, ignore policy or modes.

Compact returns `schema_version:1`, `envelope_kind:"split_manifest"`,
`response_mode:"compact"`, `analysis_status`, `manifest_complete`, root/snapshot,
`totals`, `coverage`, `candidate_summaries`, `draft_memberships`,
`consequence_summaries`, `decision_groups`, `detail_availability`, `retention`,
`omissions`, `error` and input-only `integrity`. It has no legacy `inventory:[]`
masquerading as complete. Candidate core/alternative/companion membership and
consequences remain explicit; draft groups retain exact item/overlap membership
and consequences. `coverage.local_forecasts` preserves each draft/group's local
lower-bound forecast, test coupling and unchanged `assessment_scope` in group
order. Scope/boundary/test coverage, uncertainty, work guards, omissions and
partition outcome remain mandatory. Compact ignores display-only `max_items`
trimming; it does not bypass analysis guards.

Each detail-availability entry states collection total, returned records (zero),
omitted-from-manifest count, retrievability, permanently unavailable count and
retrieval tool. Availability is conditional on the fixed handle lifetime; it is
not persistence. Analysis completion, complete mandatory manifest, complete
returned detail page and retrievable evidence are separate claims. If mandatory
membership/consequence/coverage cannot fit the duplicated wire budget, return
`ADVICE_MANIFEST_TOO_LARGE` with `manifest_complete:false`, bounded totals/status
and explicit omissions/unavailability; publish no record. Never silently shorten
a complete manifest. Full schema-2 advice remains available.

### Historical detail and release (`get_split_detail`)

```json
{"analysis_handle":"opaque-analysis-token","snapshot_id":"sha1:captured-corpus-hash","selector":{"kind":"page","collection":"decisions","page_size":100},"limits":{"response_bytes":2097152}}
```

Required identity is `analysis_handle` plus expected `snapshot_id`; optional
`analysis_id` and `scope_input_digest` must match when supplied. Requests are
strict objects. Select exactly one tagged selector:

- `page`: collection is `inventory`, `groups`, `decisions`, `advice_decisions`,
  `companions`, `boundary_observations`, `test_coupling`, `overlaps` or
  `chain_diagnostics`. Page size is 1–1000 (default 100); repeat returned
  `next_page_token` as `selector.page_token` with all options unchanged.
- `records`: `{kind:"records",collection,ids:[...]}` retrieves complete selected
  collection records in supplied order. Derived groups have IDs
  `draft/N/group/M` (zero-based group index); companions add their `candidate_id`.
- `units`: `{kind:"units",item_ids:[...]}` returns complete original
  `{item_id,unit_ref:{analysis_id,item_id},item:{path,range,expected_text},enclosing_impl}`
  anchors, eligibility and exclusions. `enclosing_impl` is the exact header ending
  immediately before `{`, or null. Unit references are qualified selectors, not
  execution authority. Whole-impl/member overlap is not silently pruned.
- `release`: `{kind:"release"}` removes only process-local evidence, returning
  `released:true`; nothing is changed in the caller repository.

Explicit record/unit ID lists are nonempty, unique and at most 1000 IDs. IDs are
bounded to 512 bytes. Optional page `filter` accepts only `id`, `reason` and
`candidate_id`: `id` must exist in that collection; `reason` must be an observed
typed `reason` or member of `reasons` there (unsupported/unknown reasons refuse);
`candidate_id` must identify a retained candidate. Candidate filtering follows
core/companion/alternative item links, group/decision/overlap item membership,
boundary candidate links and test candidate couplings; companions match their
candidate directly. Combined supported filters may legitimately yield an empty
page. There is no arbitrary query language. Identity/token strings are at most
200 bytes; tokens must be ASCII. `limits` accepts only `response_bytes` (64 KiB–
16 MiB, default 2 MiB) and `time_budget_ms` (1–300000, default 60000), not scan or
`text_bytes` options. Decoded requests still have the common 8-MiB cap.

Schema-1 detail is `envelope_kind:"split_detail"` with analysis/snapshot/input
digest, `historical:true`, `live_freshness:"not_checked"`, collection/filter,
exact filtered `total`, records, `returned_page_complete`, `collection_exhausted`
and `next_page_token`, plus error and `integrity.semantic:"not_performed"`.
Pages use frozen original record order; all embedded source descriptors/anchors
are reconstructed from immutable buffers. Repeat a valid page to receive identical
evidence and token bytes while alive, even after live source/ignore edits.
No filesystem reads, fingerprinting, parsing, grouping or current timestamps occur
in detail. No patch, request scaffold or applicability judgment is returned.

Tokens bind process/handle, snapshot/input digest, collection/filter, ordering,
position, page size and effective detail limits. A page stops only on complete
record boundaries; exact totals/continuation remain explicit. An indivisible full
record/unit or complete explicit subset that cannot fit refuses with
`DETAIL_TOO_LARGE`, withholding every record/anchor rather than returning snippets.
A successful page never makes no progress when records remain. All detail/release
calls share the original single admission permit and cancellation/shutdown model;
`BUSY` starts no work and queues nothing. Store locks cover only registry lookup,
publication/pruning/removal, not I/O, serialization or response fitting.

| Detail error | Meaning |
|---|---|
| `ADVICE_SNAPSHOT_UNKNOWN` | Unknown, released, corrupted or previous-process handle |
| `ADVICE_SNAPSHOT_EXPIRED` | Valid current-process handle past fixed expiry, including after payload reclamation |
| `ADVICE_SNAPSHOT_MISMATCH` | Supplied snapshot/analysis/input digest differs; `error.field` names it |
| `INVALID_ADVICE_PAGE` | Invalid token/page size or changed bound options; `error.field` is `selector.page_token` |
| `UNKNOWN_ADVICE_ID` | Unknown or cross-collection selected/filter ID or candidate |
| `DETAIL_TOO_LARGE` | Complete indivisible evidence or explicit subset cannot fit |
| `INVALID_PARAMS` | Malformed selector/filter/limits, duplicate/empty/over-cap ID list or unknown typed reason filter |

Historical full bytes are **not verified current execution anchors**. Before
constructing/submitting an ordinary explicit `move_item` request, obtain/recheck
current original bytes and header provenance; that tool still independently
performs every audit. Historical retrieval itself never exports or submits a move.
The separately invoked export below can perform a current observational recheck.

### Inactive exact-anchor export (`export_move_request`)

```json
{"analysis_handle":"opaque-analysis-token","snapshot_id":"sha1:captured-corpus-hash","selection":[{"unit_ref":{"analysis_id":"opaque-analysis-identity","item_id":"i/src/worker.rs/0/14"},"destination":{"kind":"new_sibling","parent_path":"src/lib.rs","path":"src/helpers.rs"}}],"limits":{"response_bytes":2097152}}
```

This read-only tool accepts a **nonempty explicit selection**, not a
candidate/group ID or implicit companion closure. Required identity is the retained
`analysis_handle` and expected `snapshot_id`; optional top-level `analysis_id` and
`scope_input_digest` must match when supplied. Every entry requires an
`analysis_id`-qualified `unit_ref` and a complete caller-chosen ordinary
`Destination`: `existing`, `new_sibling`, `new_child` or `existing_impl`. For
`new_child`, `parent_path` must equal the selected unit's source and the path must
match its ordinary direct-child geometry. This syntax check does not run the
planner or prove prospective directory admission. IDs are bounded to 512 bytes; analysis identity strings to 200 bytes.
Duplicate IDs, unknown IDs, cross-analysis qualifiers and overlapping original
ranges (including whole-impl/member alternatives) refuse instead of pruning.
Supported whole-impl alternatives are distinct from the advisory instruction to
partition their members; member exclusions and syntax recovery still refuse.

All objects are strict: unknown fields, missing destinations and unsupported
variants fail. Destination validation checks normalized root-relative Rust paths,
literal sibling or explicit child/name/parent geometry (including child source/parent equality), member-only `existing_impl`, and internally
consistent complete caller `implementation`/`before_item` anchors. It does **not**
prove destination admission/absence, declaration identity, bindings, privacy,
trivia ownership or batch applicability. Those remain ordinary `move_item` audits.

Optional `move_options` permits **only** explicitly supplied `context`, ordinary
move `limits`, and `max_moves`. Their normal bounds/defaults apply, including move
`diagnostic_count` 0–100000 and `max_moves` default 500, range 1–5000. Selection
length must fit that cap; export never increases it automatically. Export `limits`
separately accepts only `response_bytes` (64 KiB–16 MiB, default 2 MiB) and
`time_budget_ms` (1–300000, default 60000). Advice display/work settings are not
silently copied into the move request. An omitted move diagnostic count remains
omitted, preserving ordinary blocked-preview defaults; explicit expansion stays
explicit. Assumptions, semantic configuration or
resolution, acknowledgments, draft provenance and rewrite/trivia choices are
**absent** from its JSON, taking strict ordinary defaults on submission. Edit
supported ordinary fields separately and explicitly if needed.

Schema-1 output is `envelope_kind:"move_request_scaffold"`, `tool:"export_move_request"`,
`scaffold:true`, `submitted:false`, `applicability:"not_assessed"`. On success,
`source_freshness:"checked_at_export"` and `request` contains exactly the supplied
selection/destinations in caller order, canonical Git root, original crate-root
and normalized discovery paths/globs. Item `{path,range,expected_text}` anchors
contain **full original frozen bytes**, never display snippets. Members additionally
have full exact header-only `enclosing_impl` anchors ending immediately before
`{`, preserving generic/where clauses and line endings rather than regenerating
Rust or returning an entire enclosing impl extent.

Outside the strict request, `provenance` supplies analysis handle/ID, snapshot,
input digest, normalized analysis request and grammar/server/build/grouping
provenance. `review` lists selected item IDs, relevant `companions_not_selected`
and collection-qualified `unresolved_decision_refs` from canonical evidence,
including source-wide uncertainty, with `destination_and_batch_applicability:"not_assessed"`.
These are review obligations, not selected moves or policy overrides.
`integrity.semantic` and export syntax validation remain `not_performed`: no new
parsing, grouping, compilation, equivalence or move applicability claim is made.

Before success, export rediscovery re-fingerprints the captured admitted corpus
and compare scope-input manifests: bytes, modes, normalized root/scope, observed
filesystem identities and effective in-root `.gitignore` presence/absence/bytes.
Missing/added admitted files, unsafe symlink/nested boundaries, policy changes
even with unchanged admission, and observed equal-byte replacements refuse.
Only captured evidence boundaries are promised; ignored/hard-excluded paths are
not promoted into scope. This is **observational**, not an atomic application-time
guarantee. `get_split_detail` remains historical after any edit. Export never
submits/calls `move_item`; callers separately submit the full ordinary request,
and later edits still fail its independent stale-anchor/audit checks.

The complete generated request must fit the same **8-MiB decoded-argument** JSON
accounting as MCP, and the whole wrapper must fit duplicated structured/text wire
accounting, escaping and framing reserve included. Oversize refuses the entire
request: no shortened anchors, partial executable JSON or automatic batch splitting.
`counts.selected_units` reports input count; `request_bytes` reports serialized
complete-request size once constructed, and `response_bytes` conservatively
reports the full attempted duplicated result when measured. Zero byte counts mean
that construction/measurement was not reached, not that the request was empty.

| Export error | Meaning; every failure has `request:null` |
|---|---|
| `ADVICE_SNAPSHOT_UNKNOWN` / `ADVICE_SNAPSHOT_EXPIRED` / `ADVICE_SNAPSHOT_MISMATCH` | Same fixed lifecycle and identity semantics as detail; no silent reanalysis |
| `UNKNOWN_ADVICE_ID` | Selected inventory ID is absent from this retained record |
| `INVALID_SCAFFOLD_SELECTION` | Empty/over-cap/invalid/duplicate/cross-analysis or overlapping explicit selection |
| `UNSUPPORTED_SCAFFOLD_SELECTION` | Excluded context/unit, recovery, or missing complete original byte/header provenance |
| `INVALID_DESTINATION` | Invalid destination syntax, path geometry or required caller anchor |
| `SOURCE_CHANGED` | Captured source/scope/policy/identity/modes changed or cannot be completely reobserved |
| `SCAFFOLD_TOO_LARGE` | Complete decoded request or duplicated wire response cannot fit; no partial request |
| `INVALID_PARAMS` | Malformed strict object, missing fields or unsupported options/bounds |
| `CANCELLED` / `planning_deadline` / `BUSY` | Existing cooperative stoppage or occupied slot; no request is published |

All exports use the original worker-owned single admission permit through
cancellation/shutdown, with no queue. Store locks cover only lookup/pruning;
freshness I/O and serialization run unlocked on an owned immutable record.

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
  Scoped path patterns are non-binding positions, but their constructor/constant
  dependencies are still analyzed separately. Bare identifiers in tuple, slice,
  tuple-struct and explicit struct-field pattern arguments prove local bindings
  only without competing evidence in the pattern's enclosing scopes:
  same-named written const/static, unit constructor/visible enum unit variant,
  named import leaf or an unresolved glob import. In `move_item`, a glob can
  instead be proved noncompeting for that spelling through a fully admitted,
  accessible ordinary file/inline module route (including grouped globs and
  evidenced named module aliases) to direct written exports. Every relevant
  glob must pass. Same-name const/static and unit/tuple constructors remain
  conservative competitors, including inaccessible declarations; ordinary
  functions and type-only declarations do not automatically compete. Target
  imports/re-exports, enum globs, external/unadmitted or ambiguous routes,
  conditional/provider/derive attributes, macros, recovered syntax, cycles and
  exhausted budgets cannot establish absence. This uses stricter evidence than
  consumer-glob reachability/exclusion and does not repair glob consumers.
  Every proved written binder in a moved item, even unread or unassisted by
  globs, carries private source provenance and is rechecked against complete
  assembled final bytes before artifact publication. Identifier, whole pattern
  and affected references require unique byte-exact copied origins. The final
  CST must derive that same pattern and the recorded scope kind, with no error
  or missing nodes in either; unrelated scope-byte repairs need not be copied.
  Rewrites inside the witnessed pattern, including constructor paths and guards
  in `match_pattern`, still refuse. Captured or validated synthesized module
  edges, final ancestors, globs and changed exporters remain checked; missing
  evidence or destination/companion competition retains original-anchored
  `lexical_context_unproved` and withholds all artifacts. This is not a general
  final lexical audit: retained items in receiving files are not enrolled, so a
  companion arrival can still change their binder interpretation. Validate the
  applied batch with the caller's compiler; type checking and inferred binding
  modes are not certified by written identity.
  No new flag, public proof class or semantic verification is introduced.
  Potentially item-producing macro
  invocations in the containing module or intervening block/declaration scopes
  also retain uncertainty, regardless of their position before or after the
  pattern; written syntax cannot rule out a macro-generated constant or variant.
  An uncertain pattern argument blocks even when its name is unused in the arm
  body. This is not macro expansion, constant resolution or a capitalization
  heuristic. Imports/items do not inherit into child modules; parent-module or
  sibling-scope invocations alone do not compete.
  Successful let-else bindings are out of scope in the initializer and failure
  block; match-arm bindings apply only to the guard and arm value. Non-binding
  literal/scoped-path or-pattern alternatives are admitted. Well-formed composite
  alternatives can also prove disjointness when none of their binding-capable
  positions has the queried spelling (including a consumer path prefix or alias).
  Constructor paths and explicit field labels are not binders; shorthand fields,
  captures, explicit binders and ambiguous arguments are checked, including
  nested alternatives. A matching possible binder in any alternative remains
  unsupported: this negative proof does not establish a shared binder identity.
  Macros, unknown shapes, recovery and exhausted budgets never prove disjointness.
  Macro/attribute vetoes still apply.
  A same-spelled value binding does not prove a type reference independent;
  that namespace uncertainty remains blocked rather than silently discarding
  the type dependency. The main decision retains the original occurrence
  anchor. For a standalone macro in the reference's block or an enclosing block,
  additive `witness_relation:"statement_macro_before_reference"` identifies an
  at-or-before written-token-tree witness; `"block_macro_may_introduce_items"`
  identifies a later invocation that may introduce hoisted block items. Both
  locate `pattern` within `scope`, not an anchor-containing binding pattern;
  expression-statement wrappers do not establish expression-only expansion.
  Neither relation proves expansion or hygiene. The former `hoist_possibility`
  relation is no longer emitted. Other witnesses omit
  `witness_relation`. These witness
  coordinates locate evidence; they are not replay targets or new request fields.
  Legacy `category:"binding_collision"`/`BINDING_COLLISION` can still accompany
  `reason:"lexical_context_unproved"`: that means uncertainty, not a proven
  collision. Witnesses survive `text_bytes:0`; if mandatory evidence cannot fit,
  the result is incomplete and decisions/artifacts are withheld with omissions.
- For retained `move_item` needs, refusal basis is disclosed per need in additive
  `refusal_basis:[{class,anchor:{path,range?},name?}]` records. Classes include
  `chain_macro_statement`, `derive_veto`, `shadow`, `macro_shadow`,
  `conditional_context`, `prelude_disabled`, `module_attribute`,
  `unparseable_attribute`, `glob_import`, `unresolved_chain`, `syntax_recovery`, `lexical_uncertainty`
  and the occurrence-level fallback `written_binding_unproved`. Canonical re-export
  route refusals use `inaccessible_route:<segment>` for inaccessible or unproved
  intermediate modules. `name` identifies a competing spelling or that route segment;
  written witnesses use original half-open byte coordinates,
  while unresolved files and synthesized bindings omit `range` rather than invent
  offsets. Lexical detail remains in `lexical_uncertainty`. Semantic refusals name
  the failed proof stage (`semantic_configuration_unproved`,
  `semantic_overlay_unavailable`, `semantic_mapping_unproved`,
  `semantic_source_fact_unproved`, `semantic_final_fact_unproved` or
  `semantic_identity_unproved`), anchored at the original occurrence; these stages
  do not claim a finer rust-analyzer internal cause. Inspect these records when
  an opt-in emits zero proofs. Disclosure does not relax vetoes or introduce
  overrides; capped decision exemplars retain the existing omission accounting.
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
  non-null `edits`, `created_files` and `patch`, plus non-null `directory_preconditions` for schema 3, before treating a move as applicable.

Both tools also publish `decision_groups[]` (inside `plan` for moves). Each group
contains `category`, `reason`, `route`, `blocks_applicability`, `decision_ids` and
`count`. These sorted cause summaries count decision records, not guessed
references. Both move and advice groups encode exact
contiguous runs, e.g. `decision_ids:[{"first_id":"d/0","count":2},
{"first_id":"d/3","count":1}]` names only `d/0`, `d/1`, `d/3`, not `d/2`.
A run increments the numeric suffix of the first ID; run counts sum to group count.
This run encoding applies to **every** `move_item` response (blocked and
applicable), full `suggest_split` responses and compact manifest decision groups.
Legacy-only moves and full advice publish `schema_version:2` (v1 listed group
decision IDs as plain strings); typed moves containing `new_child` publish schema 3
for the whole batch, including failures. Manifest/detail/scaffold envelopes
separately publish schema 1. Search tools
remain on schema version 1. Draft/risk decision IDs remain individual strings and can be joined to
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

1. Inspect the inventory, decisions and any structural draft (plus a distinct balanced alternative only if explicitly requested); edit/choose/ignore them **outside the server**. No-credible or incomplete advice is not a reason to force a partition.
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

Other bounds are positive. Partial option objects use defaults for omitted settings; unknown fields are rejected. Raw query size is at most 64 KiB, 64 patterns/64 capture names. Sugar size is at most 64 KiB, 4,096 significant IR nodes and 64 metavariable occurrences. Decoded arguments are at most 8 MiB. Per-file query execution permits at most 100,000 candidate matches and 128 MiB of capture/range descriptors. Unfinished file matches are discarded. One engine call is admitted at a time; blocking work uses at most four worker-local parsers/cursors. `BUSY` means the analysis slot is occupied: the rejected call starts no analysis and is not queued. Wait for the active call to finish or cancellation to settle, then retry serially. The rejection discloses no other request's identity and promises no retry deadline. Cancellation/deadlines are cooperative; OS/Git I/O can outlast them, and the active worker retains the admission permit until its work settles.

Structured results also include a JSON text fallback. Wire accounting includes both representations, JSON escaping and a framing reserve. The schema-version-1 envelope carries `tool`, canonical `root` (null if unresolved), `snapshot_id`, `status`, `coverage`, `counts`, effective `limits`, `truncation_reasons`, fixed-key `skipped`, bounded `diagnostics` and omissions, typed `error`, `matches`, `next_cursor` and `has_more`. Invalid/domain calls set MCP `isError:true` and return a failed envelope; successful empty results are not errors. Input deserialization failures use `INVALID_PARAMS`, not an opaque router response. When a string is supplied where an object is expected, or the literal string `"null"` causes a type error, the message adds a client-stringification hint: omit optional object parameters instead of passing null. Values are still rejected, never parsed again or coerced; required object parameters must be actual JSON objects.

`scan_exhausted` concerns traversal; `eligible_scan_complete` means a complete fingerprintable source manifest was built; `scope_exhaustive` additionally requires no eligible-file failures or unfinished matching. Hard/ignore/filter exclusions define scope, while policy boundary counts disclose symlink/nested exclusions. Skip records contain exact/saturating counts, at most three examples per reason, and `examples_omitted`. Non-UTF-8 path examples have null actionable path and bounded `path_bytes_hex`, never lossy path strings. Diagnostics/skip examples share a display budget.

Each match has `id`, `path`, `span`, capture arrays, syntax flags, context and relevant keep-in-place trivia ambiguity records. `span`/captures/context entries contain the **full original half-open byte range**, start/end positions, `text`, `text_bytes` and `text_omitted`. Lines are one-based and byte columns zero-based; LF advances the line, CR/tabs/Unicode remain original bytes. Context is exact complete source lines with ranges and clipped/omitted line counts. Oversized field text is null; a wire-oversized match advances pagination as a full-range descriptor with explicit occurrence/context/trivia omission counts. Display limits do not change matching. IDs are snapshot-local, not stable syntax identities.

Recovery diagnostics scan every node, including anonymous zero-width missing tokens. Syntax flags distinguish file, subtree and enclosing non-file recovery. Such matches do not certify valid Rust or compiler correctness.

## Continuation and counts

Repeat the same effective tool/pattern-or-query/root/scope/context/limits/page-size request with its returned `next_cursor`. Ordering is path UTF-8 bytes, start/end bytes, pattern index and sorted capture signature; completed files are sorted/deduplicated before paging. A lookahead determines whether a count-limited page really has remaining matches. Retry tokens are reusable, deterministic and process-local. Each series has one fixed 15-minute lifetime; any number of pages share it. At most 32 active series are retained; capacity is an explicit error, not silent eviction.

A cursor contains series ID, file/match position and a process-keyed corruption checksum. On continuation, re-discover/read/fingerprint the entire scope using read-only `git hash-object --stdin --no-filters` (never `-w`) before resuming matching. Invalid/unknown, expired, scope-mismatched and stale cursors produce distinct errors. Restart invalidates all tokens. `snapshot_id` is an object-format/corpus hash, not a commit, persisted Git object, authorization token or atomic snapshot.

Discovery/snapshot stoppage yields partial results and **null cursor** with a narrowing reason. Matching stoppage after a complete snapshot can resume at the unfinished file; a repeatedly non-progressing file needs a narrower query. `has_more:null` means work could not establish exhaustion. Counts distinguish discovered/scanned/eligible files, files whose matching completed, matches observed and matches emitted. `total_matches` is series-wide and exact only when the whole original matching scope was exhausted in this call. A terminal continued page that did not count its prefix reports null/`total_is_exact:false`; no extra scan/cache is introduced just to manufacture totals.

Both `search` and `search_query` already apply `page_size` and wire limits to returned matches, not scope-match admission. Their shared cursor pipeline does not reject a page because the scope has more matches; it reports a continuation and honest counts for the work actually completed. Unlike replacement's admitted explicit selection, a paged search need not count the whole original scope in one call. The common rule is unchanged: response limits bound payload, and exact totals are claimed only after complete scope counting.
