# Known limitations and gaps

Honest boundaries of the evidence tiers and split planning as of v0.5.0.
Historical applicability measurements below are from v0.4.1 (2026-10-08),
not new balanced-default evidence. Measured on the oss-benchmark (see `docs/benchmark-unlocks.md` and the orchestration
artifacts for run data: 6 popular open-source repos + two additional targets,
24 files, 114 selected items across the combined cohort). Each limitation is ticketed or documented; none is a silent guess —
everything the engine cannot prove is refused with anchored, classified disclosure.

## Proof-tier boundaries

- **Let-chain proof is lexical only.** Supported if/while `&&` chains expose
  admitted pattern bindings to later operands and the success body, not their
  own initializers, earlier operands, else branches or following code. Disjoint
  patterns preserve outer lookup, and later binders shadow earlier ones. Nested
  closures may capture bindings; nested items cannot. Parenthesized Boolean
  operands are supported, not parenthesized let conditions. Let-chain match
  guards, unsupported/recovered patterns, macro/conditional context, unresolved
  glob constant competition and constructor identity remain independent refusals.

- **Sysroot and external-crate member calls block.** Method calls on `std`/external types
  (`.iter()`, `.push()`, `format!`-adjacent chains) cannot be proven: the admitted graph
  contains only explicitly configured crates, and no sysroot is loaded. Crate-local
  inherent calls with concrete receivers discharge under `resolve_semantic`.
- **Trait methods and generic functions block.** `fact_attached` refuses trait-associated
  and generic declarations; only bounded concrete inherent facts are provable.
- **Macro expansion is not modeled.** Any macro invocation in a moved body vetoes
  (`macro_context_unexamined`); declarative (`macro_rules!`) expansion is admitted ONLY for
  declaration-identity facts via the bounded tier (`declarative.rs`), with containment for
  provider-uncertain helper attributes and a labeled caller-assumption flag
  (`assume_declared_helpers`). Location-observable macros are a known residual risk class
  and must be reviewed by callers.
- **External dependency sources are not admitted.** `semantic_configuration.dependencies`
  exists, but the oss-benchmark ran with `dependencies: []`; external-crate bindings block
  (measured: the dominant refusal class on external repos after workspace/feature fixes).
  Admitting dependency sources is a designed-but-unbuilt provenance expansion.
- **Pub-API moves refuse by design.** `public_path_change` blocks moves that would alter a
  public path; no automatic re-export shims are synthesized.

## Advice-tier boundaries

- Under full-response budget pressure, `item_size`/`name_prefix` declaration
  displays may use `evidence:[]` with existing ordered `item_ids` pointing to
  same-response inventory spans. Clients consuming those direct arrays must
  dereference IDs in order when
  `counts.omissions.duplicate_declaration_display_spans` is present. The count
  describes proven identical duplicate displays, not missing analysis; no retained
  handle is needed. Full-descriptor equality and complete inventory/analysis are
  required. Unique reference/consumer occurrences, candidates, companions,
  consequence summaries and draft records remain direct and unchanged. This is a
  late fitting tier only; already-fitting output and retained canonical detail are
  untouched. Genuine mandatory overflow still yields partial advice and withholds
  complete drafts, at the unchanged default 2-MiB duplicated-response cap.
- Historical detail requires explicit `retain_snapshot:true` on a completed
  analysis. Legacy full calls allocate no record; `response_mode:"compact"` alone
  does not retain. Compact is a separate manifest, not a complete shortened
  inventory; mandatory membership/consequence/coverage overflow refuses rather
  than silently dropping evidence. Unavailable retention does not by itself make
  completed advice incomplete.
- Retention is bounded, process-local and immutable: no silent eviction, persistence,
  jobs, background refresh or restart survival. Defaults are one active record,
  134,217,728 aggregate accounted bytes (128 MiB), and a fixed non-sliding
  900-second lifetime from publication. Preparations and in-flight readers occupy
  capacity; conservative reservation may refuse before final stored charges would
  fit. The byte cap is not allocated upfront or a repository-size, response-byte
  or process-RSS limit; transient analysis/output are separate. Release is explicit.
  Fixed monotonic expiry does not slide on detail reads, and release/expiry retain
  record/byte accounting for still-owned in-flight readers.
- `get_split_detail` returns frozen original evidence with `historical:true` and
  `live_freshness:"not_checked"`, never live reanalysis. Full unit/header anchors
  are not current execution authority. Unknown/expired/mismatched identities,
  changed page options, unknown IDs and oversized complete records/units refuse
  explicitly; no snippets are exported as full anchors and no move request is
  submitted or scaffolded. Ordinary `move_item` still requires full current anchors
  and every existing audit.
- `export_move_request` is a separately invoked inactive scaffold, not an applicable
  plan. It requires explicit analysis-qualified unit/destination entries, preserves
  exact full frozen item/header bytes and strict execution defaults, and never adds
  companions or submits moves. Freshness compares the captured corpus, scope, ignore
  inputs, modes and observed filesystem identities, without parsing/grouping; it is
  observational, not atomic application-time freshness. Unsupported/overlapping/stale
  or oversized selections refuse with `request:null`. Both the 8-MiB decoded request
  and duplicated wire response must fit; no snippets or automatic batch splitting.
  Caller destination anchors receive syntax checks only. Ordinary `move_item` still
  independently checks current anchors and every safety/applicability audit.
- A separate `scope_input_digest` captures normalized scope/corpus and effective
  observed in-root `.gitignore` presence/absence/exact bytes without changing corpus
  `snapshot_id` semantics. Policy observations are finite (100,000 entries, 1 MiB
  per input, 16 MiB aggregate accounting, plus at most 200,000 observed filesystem
  identity/permission records under the same aggregate cap); unsafe/unreadable/over-limit inputs fail
  closed. This freshness seam is not a claim that detail checks current source,
  modes or ignore policy. Export uses this seam for current observational rechecks;
  ignored/hard-excluded paths remain outside the captured evidence boundary.
  See `docs/tools.md` for selectors, export refusals and limits.

- Candidate/group `consequence_summary` counts uncapped decision records per class,
  with exact links and explicit unmapped reasons. Classes overlap; counts are not
  additive blocker/unit totals. Separate `advice_decisions` link inspection evidence,
  never establish outside-consumer repairability or change local forecasts. Zero
  counts still leave destination/batch applicability unassessed. Display omissions
  can withhold linked detail; expand budgets rather than treating it as absent evidence.

- `test_observations` separately scans admitted written inline/ordinary out-of-line
  test routes, including conditional exact `cfg(test)` declarations in both layouts.
  Missing/excluded/competing/remapped routes and unlinked test roots remain explicit
  limitations. No Cargo target or library alias is inferred. Completion/zero records
  never means tests unaffected. CST access shapes tolerate line wrapping; only narrow
  written nominal/lexical receiver evidence links fields. Arbitrary receiver chains,
  inferred types and method identity remain unproved; unrelated same-spelled fields
  stay unlinked. Macro-input access shapes are `macro_token_candidate`, excluding
  comments/literals, not expansion or assertion equivalence. Non-exclusive coupling
  labels and possible companions never select tests, rewrite assertions or expose
  state. Legacy same-file signals and exact inline acknowledgement eligibility are
  unchanged; out-of-line/third-file glob consumers still block in the move planner.

- Ownership cores use written SCCs, unique nominal/impl associations and bounded
  same-owner member/signature-payload seeds, not names or inferred architectural
  ownership. Companions and impl/member alternatives are inspection-only and never
  expand selection. Companion `review_obligation` distinguishes supported type/impl
  selection-completeness review, written boundary dependencies and unproved
  associations independently of observed-consumer classification. Exclusive
  payload/helper consumers do not prove mandatory co-location or access repairs.
  Shared/public/import/unsupported/uncertain nodes stop traversal.
- `observed_exclusive` means completed admitted written observations only. Macro,
  inline-module, forwarded named-route, receiver and unadmitted/generated contexts
  remain unproved. Boundary observations disclose separate coverage and candidates;
  they neither change local blocker forecasts nor assess destination/batch repairability.
- Since v0.5.0, omitted/false `include_balanced` excludes byte-balanced
  alternatives. Explicit true requests the existing distinct low-confidence,
  original-order alternative only when structural advice and layout are valid;
  identical alternatives are deduplicated, never a two-proposal promise. Before
  this boundary a distinct balanced alternative was included by default when
  structural advice existed. Completed analysis without a supported core still
  reports `no_credible_written_partition` with inventory/evidence and no balanced
  primary, even with true. Incomplete analysis cannot establish that negative;
  unsupported layout remains separate. No ownership, exclusion, membership or
  execution-proof policy is relaxed.
- Completing the fixed release workload does not imply all repository-scale
  advice is complete. Work-guard stops and mandatory-output overflow remain
  partial; display-only diagnostic/text caps or proven duplicate-display fitting
  are not missing analysis. Read each result's status, coverage and omissions.

- Inventory includes overlapping whole impl and member descriptors, with explicit
  containment and cross-group links. Non-additive size labels preserve original
  descriptor byte/line values and sums; no unique-byte or generated-module-size
  metric is provided. Exact-once IDs do not establish a disjoint source partition.
- Associated-item inventory covers **root-level impls only**; impl members inside nested
  inline modules appear as opaque `mod_item`s (follow-up tracked in the ticket store).
- Drafts exclude context-sensitive members from top-level-only batches; selecting them
  requires the associated-item anchor shape.
- The `unsupported_or_uncertain_ordinary_layout` no-draft cause remains for genuinely
  uncertain layouts; member-aware drafts (v0.4.1) removed the blanket refusal on ordinary
  files.

## Private-child planning boundaries

- Explicit `new_child:{parent_path,path}` supports only direct ordinary children
  of the admitted selected source: flat `foo.rs`, existing `foo/mod.rs`, or an
  evidenced crate root. One absent conventional directory is supported only for
  the non-root flat layout, with all higher ancestors safe and existing. No
  arbitrary directory chains, new `mod.rs`, layout conversion, inline-parent
  extraction or mixed-source aggregation is planned. The server creates nothing.
- Private declarations are synthesized or uniquely reused under the existing
  admitted attribute/configuration rules; public/restricted, inline, remapped,
  ambiguous or unproved declarations refuse. Parent imports are not inherited,
  and the child declaration is not widened for outside consumers. Keep public
  facades explicitly; public-path, macro/trait/generic, private-field/constructor,
  concrete-type, receiver and every other original/final proof veto stays intact.
- Every typed child-containing batch uses schema 3, including mixed batches and
  failures. Older file-only clients must reject that version, not ignore directory
  obligations or apply a subset. Legacy-only requests keep schema 2. Schema-3
  directory preconditions link complete created files, captured absent/existing
  states and ordinary layout basis; directory permissions are caller policy,
  not Git modes. All four artifacts are withheld on any failure/incompleteness.
- Source/parent bytes and modes, effective ignore inputs, ancestor/directory
  identities, absence and competing layouts are observationally rechecked before
  publication. A previously absent directory appearing invalidates the plan.
  Publication is not an atomic or application-time guarantee: callers must repeat
  prechecks. JSON reconstruction creates only disclosed required directories;
  Git patches encode nested files, never empty directories. Expressible layout
  does not imply all historical extractions become applicable.

## Post-move polish boundaries

- Dead-import detection is advisory (nonblocking decisions), text-scan based; renamed and
  multi-binding use lines are only flagged conservatively.
- Gap collapse applies to interior batch-removal gaps only; BOF/EOF-touching gaps default
  to byte-identical retention; single-item plans keep legacy serialized shapes.
- Supported visibility repairs compute the narrowest ancestor region covering
  preserved access and proven final consumers, not a blanket `pub(crate)` escalation.
  Their `visibility` evidence identifies exact written access anchors/reasons,
  original/final module regions and the computed covering region. Preservation-only
  moved-member repairs explicitly report no new observed caller. This is not exhaustive
  caller discovery or a new privacy capability: unsupported private fields/constructors,
  concrete types and uncertain receivers retain anchored refusals.

## Measured state (oss-benchmark run 6, 2026-10-08)

0 of 114 batch-probed items applicable across the combined six public repositories
and two additional targets; remaining refusals fully classified (workspace chains fixed, declared-cfg written-tier wiring landed, root-attr
admission landed; residual walls: sysroot-class member calls, deeper written-tier
conditionals on unadmitted dependency modules, macro bodies, pub-API conservatism). The
configured-target workflow (declared configuration + assumption flag) is proven end-to-end
on the additional application target including 19-method impl partitioning with
517 scheduler tests green.

The pre-cutover frozen-input replay on Linux arm64 preserved all original roots,
scopes, flags/configuration and 114 selections: 24 rows across eight repositories,
22 complete blocked move batches, zero applicable batches or driver errors, and
21 available historical plans exactly equal. This is Linux arm64 verification,
not the original Darwin binary/platform. All 24 advice calls were partial under
work/output limits, recorded independently of move applicability; no narrowed
subset is presented as the complete cohort. The final cutover changes advice
selection and package version only, not move/root/freshness/proof contracts.
