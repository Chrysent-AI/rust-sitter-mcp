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
- Visibility repairs support private/`pub(crate)`/`pub(super)`/`pub(in path)` minimal-region
  computation for cross-module inherent calls; other widening shapes still escalate to
  `pub(crate)` review.

## Measured state (oss-benchmark run 6, 2026-10-08)

0 of 114 batch-probed items applicable across 6 external repos; remaining refusals fully
classified (workspace chains fixed, declared-cfg written-tier wiring landed, root-attr
admission landed; residual walls: sysroot-class member calls, deeper written-tier
conditionals on unadmitted dependency modules, macro bodies, pub-API conservatism). The
configured-target workflow (declared configuration + assumption flag) is proven end-to-end
on the internal target including 19-method impl partitioning with 517 scheduler tests green.
