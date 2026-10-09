# Rust Modularization Guidance

Idiomatic patterns for deciding how to split Rust code, grounded in the Rust Reference, API Guidelines, and observed practice in major Rust projects. Read this before choosing destination names or deciding what to group.

## When to split

**Split on a describable responsibility**, not a line count. A cohesive file with 1000 lines is fine; two files with artificial boundaries at 300 lines each is worse.

Signals it's time to split:
- You can name the new module's responsibility in one sentence
- The file contains 2–3 distinct concerns that could be tested independently
- Nobody can find things because the file is a dumping ground

Signals NOT to split:
- Items are tightly coupled (shared types, mutual calls)
- You'd need to widen visibility just to enable the split
- The only motivation is a line-count target

**No universal file-size threshold exists in Rust convention.** The 256-line group aim in the MCP's split advice is its own heuristic, not a Rust rule. Advice sizes are original descriptor values/sums, not final module sizes. Whole impls overlap their member descriptors, including across groups: follow `enclosing_impl_id`, `overlap_ids` and `overlaps[].draft_groups`, and heed the `non_additive` size interpretation. Exact-once inventory-ID membership is not a disjoint source partition; choose a whole impl or its members, never both.

## Naming modules

- **`snake_case`** (Rust API Guidelines requirement)
- Name after the **actual concept/capability**: `parser`, `imports`, `rewrites`, `sync`, `resolver`
- Use existing domain vocabulary from the codebase
- Avoid: `part_2`, `misc`, `utils` (unless the project already has a bounded one that works), `helpers`, `common`
- Names should predict membership

## Visibility strategy

Start with the most restrictive visibility that works:

| Visibility | Use when |
|---|---|
| (private) | Default. Visible in the defining module and descendants. |
| `pub(crate)` | Crate-wide internal access; not public API. |
| `pub(super)` / `pub(in path)` | Deliberately narrow internal boundary. |
| `pub` | Intended external API. Only what callers need. |

**The MCP engine selects the narrowest sufficient ancestor region**: `pub(super)` for a non-root immediate parent, `pub(in crate::path)` for a more distant non-root ancestor, and `pub(crate)` only when crate-root access is required. It merges all proven final caller chains into one repair, preserving already-written access and moved private members' original defining region. Review visibility rewrites; a private or restricted override must still cover that combined requirement. This is admitted-scope evidence, not exhaustive caller discovery or compilation.

Visibility rewrites expose `visibility.consumers` with exact original anchors,
access reasons and original/final consumer module regions, plus declaration regions,
preserved access requirements and `narrowest_covering_region`. Regions are arrays
relative to `crate` (`[]` is root). Preservation-only member repairs report empty
consumers and no new observed caller; do not infer unseen callers from that requirement.
The minimum describes the computed default, not a broader caller-selected `after_text`.
Unsupported private-field, constructor, concrete-type and receiver access still blocks.

## Import organization

- **Prefer explicit imports** during a split — each new module imports exactly what it uses
- **`crate::` paths** are clearest for internal absolute references; `self::`/`super::` express local relationships
- **Don't reorder or merge import groups** during a move — the Rust Style Guide forbids merging/reordering groups
- **Don't introduce new wildcard imports** as a split shortcut — they hide binding provenance and the MCP engine refuses uncertain globs. Existing globs are not blanket vetoes, but bare consumers in other admitted files reached through candidate globs block with `glob_consumer_unrepaired`; review their exact call-site and glob-statement anchors rather than assuming the old route survives.

## File layout conventions

**Modern (2018 edition) layout** — preferred for new code:

```
src/
├── lib.rs          # mod foo; mod bar;
├── foo.rs          # mod nested; → loads foo/nested.rs
└── foo/
    └── nested.rs
```

Rules:
- Can't have both `foo.rs` and `foo/mod.rs` — pick one
- `new_sibling` creates a literal `.rs` sibling in an existing directory. The
  explicitly selected private `new_child` layout can use one conventional child
  directory; neither destination creates a new `mod.rs` file or supports arbitrary
  directory chains.
- `mod foo;` loads a module; `use` imports names but doesn't load files
- Legacy `mod.rs` trees are valid — the engine works with them but never creates new ones
- Root files (`lib.rs`, `main.rs`) can contain logic, not just declarations

### Explicit private-child destinations

The caller may select `{kind:"new_child",parent_path,path}` for a private
ordinary direct child. Every unit assigned to that destination must originate in
`parent_path`, which is the selected source file. The child path must match the
evidenced Rust layout: `foo.rs` → `foo/child.rs`, `foo/mod.rs` →
`foo/child.rs`, or a child beside an evidenced crate root. At most one absent
conventional directory is supported for a non-root flat-file parent, and its
higher ancestors must already exist and pass scope checks; existing safe
directories are also supported. An absent child directory/file is not admitted
by `paths`; admit an existing higher directory, and ensure any positive glob
includes the future file. `new_sibling` remains limited to an absent file in an
existing directory.

The planner synthesizes one private ordinary `mod child;` declaration or reuses
one unique compatible private declaration under existing admitted-context rules.
Public/restricted, inline, remapped, ambiguous, unknown-cfg or unproved
declarations refuse; the child is not widened for consumers outside its parent.
The caller chooses units and destination; no core selection, public facade or
re-export is inferred or synthesized. Preserve public facades explicitly. Parent
imports are not inherited, and child placement adds no proof or visibility
exemption. Supported associated-item wrappers retain their full audited headers.
No arbitrary `mkdir`, new `mod.rs`, inline parent, layout conversion or
unrelated-source aggregation is supported.

Typed batches containing a child use schema 3, including mixed batches and
failures; legacy-only typed batches remain schema 2. Reject unknown versions
before interpreting artifacts. Schema 3 requires `plan.edits`,
`plan.created_files`, `plan.directory_preconditions` and `plan.patch` together;
unsuccessful results withhold all four. Directory records disclose root-relative
paths, observed and required states, ordinary-layout basis, and dependent
created-file IDs, with matching `created_files[].directory_precondition_ids`;
they do not specify a directory mode or empty-directory Git artifact. Directory
permissions are caller policy. The server creates nothing. Before external JSON
reconstruction, callers recheck source/parent bytes and modes, file absence, the
disclosed directory state and layout, ignores and competing layouts, then create
only disclosed absent directories with caller-chosen permissions. Directory
records do not expose filesystem identities; the server rechecks its captured
identities before publication. A previously absent directory appearing—even
empty—invalidates the plan. Git patches contain nested file entries but no
separate empty-directory artifact; `git apply --check` creates nothing and
provides no atomic freshness guarantee.

## Reading structural ownership advice

Name prefixes, banners, shared attributes and adjacency are displayed weak facts,
not core-forming edges. In a complete full response fitted under budget pressure,
`item_size` and `name_prefix` signals may have `evidence:[]` with a nonzero
`counts.omissions.duplicate_declaration_display_spans`; for those empty arrays only,
resolve every existing `signal.item_ids` entry, in original order, to its
`inventory[].span` in the complete same-response inventory.
This restores display evidence only: it does not make weak facts core-forming or
establish ownership or move safety. Unique reference/consumer occurrences remain
direct, companions are never auto-selected, and signal IDs/spans are not execution
anchors. Inspect the ranked written core and its separately disclosed
impl/payload/helper/constant companions. Candidate `candidate/N` and companion
`companion/N` IDs are response-local, not move anchors. Read each companion's
`review_obligation` from candidate-to-companion association evidence:
`selection_completeness` calls for review of a supported type/impl association's
selection shape, retaining any exclusions; `boundary_dependency` allows review
of retaining a written dependency in the parent; `association_unproved` lacks
supported written evidence for that association. Consumer `classification`,
traversal stops and move eligibility are separate: uncertain consumers do not erase
a supported association, and an exclusive payload/helper reference does not
mandate co-location or prove access repairs.
Shared orchestrators/helpers are boundaries, not joining hubs. Compare whole-impl versus explicit member alternatives, respecting
exclusions and overlap links. Boundary observations show admitted written consumers,
not symbol resolution, move repairability or architectural ownership. An observed-
exclusive classification is restricted to the completed admitted observation scope.

Use each candidate/group's `consequence_summary` to route inspection, not to
score move safety. Counts are uncapped distinct decision records per class with
exact collection-qualified links; classes overlap and unknown reasons remain in
`unmapped`. Selection-completeness asks for explicit impl/member-shape review,
not automatic co-location. Broader boundary/test observations are advice-only;
zero counts do not assess destination/batch applicability or prove repairability.

When advice was explicitly retained, `analysis_id`-qualified `unit_ref` values
select inventory units in the retained analysis; they are not move commands or
execution authority. `get_split_detail` remains historical: pages and complete
unit/header anchors preserve frozen bytes but do not check live freshness, layout,
grouping quality or `move_item` applicability. A separately invoked
`export_move_request` accepts explicit per-unit references and caller-chosen
`existing`, `new_sibling`, `new_child` or `existing_impl` destinations,
reobserves the captured corpus/scope, effective ignore inputs, modes and observed
filesystem identities, and can return full current-observed anchors in an inactive
schema-1 scaffold. For `new_child`, the caller supplies
`{kind:"new_child",parent_path,path}`; the selected source must equal
`parent_path` and the path must match its ordinary direct-child geometry. Export
validates this syntax and reobserves captured inputs only: it does not admit the
future destination or run the planner/applicability audits.
This freshness result is observational only—not grouping approval, applicability,
semantic proof or an atomic application-time guarantee—and ignored/excluded paths
are outside its evidence boundary. The caller reviews/edits and separately submits
the ordinary request to `move_item`, which independently performs every audit and
can reject later changes. Manual construction from current bytes remains supported.
Associated-unit headers are exact header-only anchors ending immediately before
`{`, preserving generic/where text and CRLF bytes. Keep whole-impl versus member
choices explicit, retain exclusions and overlapping alternatives, and never
auto-select companions or destinations.

## Inspecting test coupling

Read the separate `test_observations` projection before deciding test placement.
Conditional out-of-line routes include declaration/attribute anchors and both
conventional file candidates. Missing/unadmitted/competing/remapped routes and
unlinked marked files mean unscanned or unproved evidence, never tests unaffected.
CST field/path shapes survive line wrapping; private-state links require supported
written nominal/lexical evidence. Arbitrary receivers and unrelated types remain
uncertain. Macro-input shapes use `macro_token_candidate`, not expansion or assertion
equivalence. Non-exclusive facade/implementation/private-state/mixed/unresolved
labels identify relationships to inspect; possible companions are never mandatory
relocations. Preserve assertions and test placement unless the caller separately
chooses a supported change. Broader advice does not broaden the exact same-file
inline acknowledgement class or waive third-file/out-of-line glob blockers.

## What to keep together

- A type and its inherent `impl` blocks
- A trait and its impls (unless the project separates them)
- Tightly coupled helpers with a single consumer
- Unit tests alongside tested code (`#[cfg(test)] mod tests`)

**Co-location is grouping advice, not a capability guarantee.**
> The declared `semantic_configuration` also feeds written cfg context audits without semantic resolution, using the same positive-only predicate evaluator. Unknown atoms and inactive selected/required bindings still block, and independent macro/prelude/access/repair obligations remain.
 Inline test modules are context-sensitive; `#[cfg]`, `#[derive]`, proc-macro attributes, and relevant macro contexts can block moves. Do not extract their bodies or detach attributes to bypass a blocker. Opt-in standard-prelude proofs cover only the documented unshadowed names; bare value-position `Some`/`None` additionally assume constructor identity with unshadowed `Option`, without modeling derive-generated imports. Explicit `resolve_semantic` covers only bounded, positively resolved eligible occurrences, including variant paths with parent-enum identity at both overlays. Its context gate admits custom derives for nominal paths only with stable locally written declarations or explicit named import routes at both overlays; aliases/grouped leaves need non-glob module/re-export prefixes and active binding attributes (known-OFF imports are not evidence). Generated explicit imports can override glob-resolved names, so additive output alone does not establish path identity. Configured extern-prelude roots and generated-item-dependent facts keep the non-builtin derive veto (inspect anchored `context_evaluations`, `fact_class` and `basis`; unlisted cfg stays unknown and undeclared `cfg_attr` still vetoes either class); `acknowledge_test_consumers` discloses exact same-file inline test consumers without validating them. The narrow declarative identity tier admits type-position names from bounded in-crate written `macro_rules!` pairs, with exact original/final invocation/definition/argument anchors; inspect `declarative_macro_identity` proofs and their identity-only basis. By default this tier refuses helper-shaped attributes, including `serde(transparent)` despite a `Serialize`/`Deserialize` spelling, plus custom/qualified derives; only existing inert lint/doc metadata and unshadowed bare built-in derives are exempt on generated types. Inspect `declarative_attribute_provider_uncertain` and the named attribute in its anchored basis; helper registration is not inferred. A separate `assume_declared_helpers` opt-in with configured `resolve_semantic` lets the caller assert direct written or generated struct/enum attributes belong to registered derive helpers; companion derive paths are checked syntactically only. Inspect `assumed_declared_identity`, its caller-assumption basis and separate `coverage.assumed_declared_identity`, never real-proof counts. Identity is assumed with the type, not engine-classified; dependent nominal namespace facts are conditional too. Written helpers qualify for nominal identity only; both-overlay anchors, cfg/caps, non-type written-owner and generated-item vetoes remain. It never executes proc macros or admits generated members, constructors or variants. Conditional or over-budget expansion remains uncertainty. None resolves general generic/trait/macro uncertainty or proves compilation/equivalence. The engine generates narrow ancestor visibility repairs as described above; unproved access and moved relative/re-scoped restrictions still block.

## Anti-patterns

### God-module relocation
Moving a catch-all to `helpers.rs` changes the address, not the responsibility. If the new module's name doesn't describe its contents, rethink the split.

### Artificial cycles
Cross-module cycles within a crate aren't illegal Rust, but they make understanding/testing require the whole cycle. Keep strongly interdependent units together; don't create a `common`/`traits` module just to break a cycle diagram.

### Over-fragmentation
No one-function-per-file mandate. Major Rust projects colocate related items; a file with one struct + impl + helpers is fine.

### Leaky abstractions
Don't widen visibility just to enable a split. Review the minimum access each consumer needs — including the engine's ancestor-scoped visibility repairs.

## Lessons from major projects

Cargo, rustfmt, Tokio, and Serde all split by *responsibility*, use *descriptive concept names*, keep *tight implementations private*, expose *minimal public APIs* via selective reexports, and colocate related items (including tests at the root). None uses a line-count threshold; several keep root files with real logic; `mod.rs` remains legitimate practice (Tokio). Public facades (`pub use`) can decouple API paths from physical layout, but the engine does not synthesize API shims — preserve exposed paths yourself.
