# Known limitations and gaps

Honest boundaries of the evidence tiers and split planning as of v0.4.1 (2026-10-08).
Measured on the oss-benchmark (see `docs/benchmark-unlocks.md` and the orchestration
artifacts for run data: 6 popular open-source repos + two internal targets, 24 files,
114+ probed items). Each limitation is ticketed or documented; none is a silent guess —
everything the engine cannot prove is refused with anchored, classified disclosure.

## Proof-tier boundaries

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
- Completed analysis without a supported core reports `no_credible_written_partition`
  with no balanced primary. Existing balanced alternatives remain available when
  structural advice exists until the separately announced default cutover.

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

0 of 114 batch-probed items applicable across 6 external repos; remaining refusals fully
classified (workspace chains fixed, declared-cfg written-tier wiring landed, root-attr
admission landed; residual walls: sysroot-class member calls, deeper written-tier
conditionals on unadmitted dependency modules, macro bodies, pub-API conservatism). The
configured-target workflow (declared configuration + assumption flag) is proven end-to-end
on the internal target including 19-method impl partitioning with 517 scheduler tests green.
