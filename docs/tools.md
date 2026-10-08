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

This is **consumer-side import resolution only**. Moving a declaration that is
itself publicly exposed or re-exported still retains `public_path_change` and
requires a separate API decision; no facade, compatibility shim or re-export is
synthesized. An unsupported consumer route remains a written-binding or
conditional-context blocker, not permission to bypass it with an override.

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
merged. Unknown names never receive proof records. Counts survive response fitting;
`counts.omissions.binding_proofs` discloses omitted records, and oversized audits
withhold all artifacts. Empty derive proof/count fields are omitted.

No import is synthesized: the destination is assumed to have the same implicit
prelude. Except for the bare value-position `Some`/`None` assumption above,
constructors, associated calls, methods and arbitrary macro expansion remain
outside the bridge. All other collisions, missing context and blockers still apply to the
entire batch. Original bytes remain lossless, no `Cargo.toml` is read, and
`semantic:"not_performed"` is unchanged. This option does not make idiomatic Rust
relocation generally executable.

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
No Cargo.toml/lockfile, sysroot, filesystem watcher, dependency cache, build
script, executable override or proc-macro expander is loaded. Admit required
dependency sources in `paths`/`globs` and add their roots explicitly, or their
names stay unresolved. A dependency edge is
`{"name":"external","crate_name":"dependency_root_name"}`; `name` is its extern
prelude spelling and `crate_name` names another entry in `crates`. Crate names
and roots must be unique; graphs are acyclic and capped at 32 crates with 32
edges per crate. Editions are `2015`, `2018`, `2021` or `2024`.

Each crate declares its selected `features`, `cfg` atoms and dependencies, with
no implicit defaults. A cfg atom is `{"key":"unix","value":null}` or
`{"key":"target_os","value":"linux"}`; each selected feature also sets
`feature="name"`. These are caller-provided inputs, not discovered Cargo facts.
There is no build-script environment, target data layout or implicit standard
library source. Missing/invalid/incomplete graph evidence does not acknowledge
away a blocker: the original typed needs remain.

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
supported. Existing/new-sibling destinations synthesize audited `impl_wrapper`
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

For `move_item`, one uniquely configured `semantic_configuration` entry matching `crate_root` can also admit active `cfg`/`cfg_attr` chain attributes, without requiring semantic resolution, under its explicitly listed ON features/cfg atoms. All operands must be known; unlisted atoms remain unknown, not OFF, even in a Boolean expression with a determining operand. Known-OFF declarations do not supply edges. Invalid matching edition/cfg data or duplicate matching roots, unknown/unsupported payloads, path remapping, prelude controls, parser recovery and competing declarations/layouts retain their vetoes. The root's `ModuleEvidence.assumptions` discloses positive cfg use; this is not Cargo discovery or compilation. `suggest_split` has no configuration opt-in and still refuses condition-dependent edges.

A new destination must be an absent literal `.rs` sibling of every assigned source, in an existing directory. Its basename is an ASCII identifier, not `_`, `mod`, a raw identifier, or any Rust strict/reserved/contextual keyword (including `gen`, `raw`, `safe`, `union` and `macro_rules`). No directory creation or same-file reordering is supported. Symlinks, hard exclusions, nested repositories, ignores, caller filters, existing entries, case-folded aliases, competing `name/mod.rs` layouts and declaration conflicts fail closed. Use an admitted existing directory in `paths`, not an absent path.

Ordinary children of the root or an existing `mod.rs` reside in that file's directory. Children of non-root `foo.rs` reside in `foo/`. Thus moving `src/source.rs` → `src/moved.rs` usually needs parent `src/lib.rs`, **not** `src/source.rs`. Existing legacy layouts are supported with ordinary evidence, but `mod.rs` is never created or restructured. One matching declaration with admitted chain attributes is reused without a parent edit unless a necessary visibility repair is separately linked. Relative/re-scoped restrictions, unverified access and public exposure requiring an API decision remain blockers. Incompatible layouts cannot be acknowledged away.

### Known structural limitations

- **Macro-generated crate roots:** `crate_root!()`-style roots can emit module declarations from macro bodies; `include!(concat!(env!("OUT_DIR"), ...))` can supply build-generated declarations. The syntactic stage neither expands macros nor executes build scripts, so it cannot discover or prove these module trees. Such selections are advice-only, not applicable moves. Literal declaration-shaped tokens can name the macro cause; an opaque include without written declarations remains unproved rather than guessed.
- **Cargo autotest discovery:** creating a sibling directly under `tests/*.rs` can create a new integration-test crate under Cargo's default discovery rules. Ordinary module validity cannot see this build-graph change; review the target layout and Cargo test discovery externally before applying a patch.
- **Macro-invocation-as-item files:** when most of a file consists of macro invocations generating implementations, the inventory exposes those invocations, not their expanded items. This archetype is unsplittable by written-item moves; unlike ordinary inherent impl members, generated members are not selectable; use advice rather than treating generated methods/items as selectable written units.

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

Defaults use complete CST paths and use leaves, not equal-looking text in comments, strings or token trees. Explicit aliases survive; grouped imports use an unchanged prefix, a shared prefix edit, or comment-free leaf extraction with unrelated leaves/trivia preserved. Local and inline-module import extraction keeps the original binding scope. Only necessary imports are synthesized; equivalent destination bindings are reused and shared needs are deduplicated. Items moving together do not create artificial cross-module imports or visibility changes.

Complete `crate`/`self`/`super` paths and at most one uniquely evidenced explicit module alias can identify a written target. Moved code is interpreted in its old lexical module, then mapped into the final batch. Recognized local functions, parameters, simple local bindings and generics are independent bindings, not leftover callers. Ordinary tuple/slice/constructor/struct binding positions are compared against the queried spelling: disjoint written names do not create uncertainty for every spelling in that scope. Explicit `mut`/`ref` binders and ordinary struct shorthand can prove independent locals; a matching plain identifier in a composite or refutable pattern still requires binding-versus-constant evidence. For/match/if-let/while-let patterns are checked only where their bindings are in scope, not in initializers, other arms or an if-let's else branch. Unsupported patterns, let chains, relevant local imports, macros and recovered/conditional lexical contexts remain anchored uncertainty. No macro expansion or semantic resolution is performed. Prelude assumptions are limited to the explicit opt-in type-position and bare value-position Option-constructor bridge described above. External explicit imports retain their written spelling, not a symbol-resolution guarantee; missing names are not guessed to be prelude imports on the default path.

Visibility checks inspect the item and each ordinary module declaration from its declaring parent scope. Already sufficient visibility and proven unchanged ancestor restrictions survive. A proven insufficient access receives the deepest common ancestor region containing the declaration's preserved access region and all proven final callers: `pub(super)` for a non-root immediate parent, `pub(in crate::path)` for a more distant non-root ancestor, or `pub(crate)` only when the root is the smallest covering region. Multiple callers merge into one repair and can escalate a previously narrower requirement. Moved private inherent members also preserve their original defining region's access, without inventing callers outside the admitted corpus. Retained relative restrictions can be widened when their parent scope is unchanged; moved relative/re-scoped restrictions still block. Visibility alternatives must denote an evidenced ancestor and cover the merged requirement; insufficient choices block artifacts with `visibility_scope_unproved`, while public exposure, non-ancestor paths, comments and injected tokens are rejected. Private fields/tuple constructors, unproved associated/type-directed references, relevant globs/macros, unadmitted conditional imports/modules and unsupported dependency forms remain decisions rather than guessed repairs. An unrelated glob/macro outside the relevant binding context is not by itself a veto. Consumer-glob candidates are checked against written module routes: file identity plus inline-module anchors, scoped module aliases, grouped use prefixes, and exact default-layout parent declarations. A same-named module in another subtree is not the source module. Proven different modules and enum-variant globs are excluded; a caller-selected file root does not inherit aliases from every unrelated `crate` import. Unknown/recovered routes, competing inclusions, local alias shadows, and possible wildcard/re-export/same-name forwarding remain conservative candidates. This is admitted written-syntax evidence, not Cargo discovery, cfg evaluation, macro expansion, or semantic name resolution. Optional `coverage.glob_exclusions` counts excluded selected-item/glob pairs by reason (`different_written_module`, `enum_variants`, `no_written_route`); it is not a count of files skipped or semantic absence proofs. Test-consumer advice uses the same written-route classifier, including aliases that glob the immediate test parent; it infers no edge from an unknown route.

Bare selected-name references in other admitted files are checked against visible glob imports, including `use super::super::*`, grouped/module-alias routes and possible chained forwarding. A reachable or unproved route emits `glob_consumer_unrepaired` in category `glob_dependency`, with `action.route:"selection_change_required"`, `blocks_applicability:true`, and no acknowledgment or rewrite choice. Its first anchor is the original reference identifier in the consumer file; subsequent anchors contain the complete original candidate glob statements. `refusal_basis` retains their `glob_import` ranges and affected name, plus any lexical uncertainty witness. This is conservative written reachability, not proof of glob binding identity. Named bindings in the same scope and proven independent locals are not glob-only consumers; sibling/parent-module imports are not inherited. Existing explicit-import and same-file repair paths are unchanged. These third-file risks cannot be waived by `acknowledge_test_consumers` or semantic resolution; all artifacts remain null while any such blocker exists. Keep the affected item in place or change the selected move; never narrow away admitted consumers just to obtain an applicable plan.

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

`plan` contains `state`, `applicable`, `selected_count`, `moves`, `trivia_decisions`, `decisions`, `decision_groups`, `chain_diagnostics`, `rewrites`, `origins`, `base_files`, `blockers`, `edits`, `created_files`, `patch` and `integrity`. Default outcomes disclose `semantic:"not_performed"`; explicitly configured opt-in resolution uses the scoped evidence/label described above. Only an applicable plan has all three non-null artifacts; blocked/failed/incomplete results set **edits, creations and patch to null**, never a safe subset. `moves:[]` is an explicit checked no-op with empty edits/creations/patch and no virtual syntax claim; stray overrides are rejected.

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

The default syntactic stage delivers design advice plus proven-complete patches
for a narrow class of written Rust. Opt-in bounded resolution widens that class
only for proved occurrences; general idiomatic Rust relocation is not guaranteed.

Common limits/admission/cancellation apply. `max_moves` defaults to 500 (1–5,000) and counts the whole explicit list. Fixed work guards are 100,000 inventory descriptors, 100,000 relevant reference candidates and 128 MiB aggregate analysis descriptors, including conservatively accounted transient binding/evidence records. Effective guards, observed counts and reference coverage are reported; query-state limits do not apply to direct CST analysis. Mandatory anchors/artifacts/audit must fit the complete duplicated wire response. `text_bytes:0` omits descriptive slices, **not** freshness checks or artifact bytes. On overflow, all artifacts are withheld and preview-array omissions are explicit. Calls allocate no search cursor/series.

Definite bad requests use `INVALID_ITEM_SELECTION`, `DUPLICATE_MOVE`, `STALE_SELECTION`, `INVALID_DESTINATION`, `INVALID_NEW_FILE_NAME`, `INVALID_DECLARATION_PARENT`, `DESTINATION_ALREADY_EXISTS`, `MODULE_DECLARATION_CONFLICT`, `STALE_DESTINATION`, `INVALID_MOVE_TRIVIA_OVERRIDE`, `INVALID_REWRITE_OVERRIDE` or `STALE_REWRITE_OVERRIDE`, with actionable input fields. Uncertain identity uses `CRATE_IDENTITY_UNCERTAIN`, not a guessed cross-crate assertion. Dependency decisions distinguish binding, glob, macro, re-export, module, visibility, inherited scope and unsupported-form categories with evidence and narrowing actions. Pre-existing/new syntax recovery, unsafe attachment, overlap, cancellation, work/output limits, source changes or `CREATION_RACE` withhold the entire batch. Final byte/mode/ignore/absence rechecks are observational, not atomic or application-time guarantees.

## Advisory file splitting (`suggest_split`)

```json
{"repo_path":"/absolute/project","crate_root":"src/lib.rs","source_path":"src/rich.rs","paths":["src"],"limits":{"text_bytes":0}}
```

This call inspects **one existing admitted file**. `crate_root` supplies the same ordinary written module context as `move_item`; it is not Cargo target discovery. Optional `max_items` defaults to 500 (1–5000). `paths`, `globs`, `context` and `limits` have the usual meanings. There is no cursor, saved plan, execution handle, or implicit selection. Advice consumes no search-series capacity and shares the same admission/cancellation/shutdown lifecycle as other tools.

The result has `advisory:true`, `source`, `inventory[]`, `item_contexts[]`, `impl_contexts[]`, `scope_trivia[]`, `signals[]`, `decisions[]`, `chain_diagnostics[]`, `drafts[]` and `draft_eligibility`. It has **no patch, edits, creation content or move plan**, even when a draft is complete.

- Inventory is in original source order and includes every significant top-level written unit. Anonymous impl blocks have IDs and written type/optional trait spans in `impl_contexts`; they do not disappear because `name` is null. Outer attributes/docs are associated spans, not extra units. Each unit reports full original coordinates, kind/name, raw visibility, byte/line sizes, syntax flags, unit-kind eligibility/reasons and `signal_ids`.
- `supported_unit` means the top-level unit kind is supported, **not** that a move is dependency-free or safe. Modules, uses, extern/foreign and macro constructs remain inventoried with context-sensitive reasons and are retained in complete partitions. Direct written impl members remain `context_sensitive`: selection requires their exact header-only `enclosing_impl.anchor`. Members with empty `reasons` can enter draft sibling groups; members with nonempty exclusion reasons and their overlapping enclosing impls stay in the retain set. `counts.eligible_items` counts supported top-level units plus these non-excluded associated units, not proven safe moves. A `supported_unit`-only batch still excludes members; construct drafted member moves with the associated-item anchor shape described above.
- Unattached scope/prologue/ambiguous trivia has separate descriptors and a `keep_in_place` default. Banner links and `banner_section` signals mean adjacency only, never banner ownership. Internal scopes belong to their whole unit. Item contexts are original neighboring complete lines; the entire-source context has no lines outside the file.
- Signals expose snake_case/CamelCase word prefixes (≥3 characters, ≥2 names), counted directed bare/simple-path candidates with occurrence spans, banner sections, adjacent shared outer attributes/doc headings and item sizes. `get`, `set`, `new`, duplicate/common names, uncertain lexical bindings, unexpanded macros and type-directed accesses are not strong evidence. Same-spelled recognized locals/parameters/generics are not file-level edges. Inline-module bodies, macro tokens, strings/comments and unrelated qualified suffixes do not supply file-level reference proof. Candidates are **not a resolved call graph**.

### Reading and editing partitions

Each draft has a nonempty retain group and up to two sibling groups. Every inventoried ID appears **exactly once** in each complete draft. A sibling destination supplies a proposed absent path, an evidenced ordinary declaration parent/module chain, and an existing matching declaration descriptor when reused. The server probes the base name then `_2` through `_99`, checking scope admission, ignores, filesystem/case/layout occupancy and declaration/import collisions. It never guesses a parent from the source basename.

The primary proposal seeds clusters from prefixes/sections/outer-text adjacency, joins strongly connected candidate references, and ranks by descending internal reference occurrences, same-prefix members, then section/attribute/doc agreements; earliest original span breaks ties. Other units stay in source, retaining the earliest eligible unit if necessary. A distinct alternative retains excluded/context-only units (including enclosing impls) and the earliest draft-eligible unit, then chooses the original-order boundary closest to half the remaining bytes (earlier boundary wins ties). Weak cohesion uses that balanced arrangement with explicitly low confidence. Equivalent alternatives are not duplicated.

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
These signals project the existing conditional-consumer decision reason;
**default execution behavior is unchanged**: same-file cfg(test) consumers still
block relocation in `move_item`. The separate explicit `acknowledge_test_consumers`
option above discloses supported consumer risks instead; advice itself never
acknowledges them. No cfg evaluation or safe-relocation guarantee is introduced.

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
  Scoped path patterns are non-binding positions, but their constructor/constant
  dependencies are still analyzed separately. Bare identifiers in tuple, slice,
  tuple-struct and explicit struct-field pattern arguments prove local bindings
  only without competing evidence in the pattern's enclosing scopes:
  same-named written const/static, unit constructor/visible enum unit variant,
  named import leaf or any glob import. Potentially item-producing macro
  invocations in the containing module or intervening block/declaration scopes
  also retain uncertainty, regardless of their position before or after the
  pattern; written syntax cannot rule out a macro-generated constant or variant.
  An uncertain pattern argument blocks even when its name is unused in the arm
  body. This is not macro expansion, constant resolution or a capitalization
  heuristic. Imports/items do not inherit into child modules; parent-module or
  sibling-scope invocations alone do not compete.
  Successful let-else bindings are out of scope in the initializer and failure
  block; match-arm bindings apply only to the guard and arm value. Non-binding
  literal/scoped-path or-pattern alternatives are admitted; binding or unresolved
  bare alternatives remain unsupported. Macro/attribute vetoes still apply.
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
