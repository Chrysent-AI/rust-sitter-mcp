# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project uses [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

* Add opt-in immutable completed-advice retention, a separate compact manifest and
  `get_split_detail` for deterministic historical pages, exact original unit/header
  anchors and explicit release; expose accounted allocation, fixed expiry and honest
  capacity/identity/page/oversize refusals without live reanalysis or move submission.
  Defaults are one active record, 134,217,728 aggregate accounted bytes (128 MiB),
  and a fixed non-sliding 900-second lifetime from publication; preparations and
  in-flight readers occupy capacity, and conservative admission may refuse before
  final stored charges would fit. The cap is not allocated upfront or a repository,
  response or process-RSS limit; no eviction, persistence or restart survival.
* Capture a separate effective-ignore `scope_input_digest`, including observed
  `.gitignore` absence and exact bytes, without changing corpus snapshot semantics.

* Add uncapped, linked `consequence_summary` counts to ownership candidates and
  draft groups, with separate advisory decisions, explicit unmapped reasons and
  non-additive class membership; preserve local forecasts and unassessed applicability.

* Disclose separate advisory `test_observations` for admitted inline/out-of-line
  test routes, multiline CST accesses and labeled macro-token candidates, with
  anchored discovery limitations, attribution uncertainty and non-exclusive
  coupling labels; leave test acknowledgement, placement and assertions unchanged.

* Add structural `ownership_candidates`, inspection-only companions and explicit
  whole-impl/member alternatives with deterministic integer ranking; disclose
  separately scoped admitted-file `boundary_observations` without move-safety claims.

* Link inventory members to enclosing impl IDs and expose original-range containment
  and cross-group overlap records in `suggest_split`; label affected sizes as
  non-additive without changing descriptor sizes or exact-once draft membership.
* Explain supported `move_item` visibility repairs with exact observed access
  anchors/reasons, original/final module regions, preserved access requirements
  and the narrowest covering region; explicitly disclose preservation-only repairs
  with no new observed caller, without changing visibility eligibility or refusals.
* Flag retained source imports with zero post-move written-name references through
  anchored nonblocking `post_move_import_review` advisories; exclude strings/comments
  and public re-exports, retain all import bytes, and disclose unenumerable globs
  without claiming semantic unused-import proof or warning-free compilation.
* Add whole written inherent method/const partitioning with header-only enclosing
  anchors, unchanged generic/where headers, audited verbatim impl wrappers and
  same-type existing-impl destinations; retain trait, cfg and generated-member vetoes.
* Extend `assume_declared_helpers` to direct written struct/enum helper attributes
  for caller-assumed nominal identity, keeping conditional coverage separate.
  Associated-item batches share reached-context evidence instead of repeating it
  in every proof.

* Add default-off `assume_declared_helpers` for caller-asserted registered helpers
  on bounded declarative generated types, with distinct `assumed_declared_identity`
  proofs and coverage, unchanged both-overlay anchors and independent vetoes;
  never classify helper registration or claim procedural expansion, hygiene or
  compilation.
* Add `declarative_macro_identity` signature-type proofs through bounded in-crate
  written `macro_rules!` expansion, with matching invocation/definition anchors
  at both overlays and provisional preservation of accessible public facade
  imports; retain conditional/complex expansion and generated-member vetoes,
  and never execute procedural macros.
* Extend `assume_standard_prelude` type proofs to `std::option::Option`,
  `std::result::Result`, `std::boxed::Box`, `std::vec::Vec` and
  `std::string::String`, preserving original/final shadow and context audits
  and caller-assumption labeling without semantic configuration.

### Changed

* Stop forming credible advice cores from weak naming/section/attribute/adjacency
  signals; completed analysis with no supported core reports
  `no_credible_written_partition` rather than a balanced primary. Existing balancing
  remains an alternative when structural advice exists, pending the announced cutover.
* Explain every `BUSY` rejection: the analysis slot is occupied, the rejected call
  starts no analysis and is not queued; wait for active work or cancellation to
  settle, then retry serially, without a retry deadline or another request's identity.
* Collapse oversized pure interior removal-boundary blank-line runs by default in
  multi-item `move_item` batches through anchored, audited rewrites; keep BOF/EOF
  gaps unchanged by default, preserve explicit `retain` replay, byte-identical
  single-item serialized plans, single/double newlines and unrelated whitespace.

### Fixed

* Keep large full advice complete at the default response cap by sharing only
  proven identical `item_size`/`name_prefix` declaration displays through existing
  ordered inventory IDs under budget pressure. Count duplicate display omissions
  as `duplicate_declaration_display_spans`; direct-array clients must dereference
  inventory spans. Preserve unique occurrences, candidates, companions,
  consequence summaries, drafts and canonical retained evidence; genuine overflow
  remains partial.
* Reserve retained-advice record and byte capacity before materializing a retained
  copy; refund failed or dropped candidates and preserve charges for in-flight readers.
* Distinguish positive `#[cfg(test)]` root markers from other cfg mentions of
  `test`; disclose the latter as uncertainty without implying test coupling.
* Distinguish inspection companions' selection-completeness, boundary-dependency
  and unproved-association review obligations from observed-consumer classification;
  emit `candidate/N` IDs and deterministic unique response-local `companion/N` IDs.
* Keep competing nominal imports and uncertain local written consumers from
  gaining credible ownership seeds or exclusive companion classifications.
* Emit narrow ancestor-scoped visibility repairs (`pub(super)` or
  `pub(in crate::path)`) instead of blanket `pub(crate)` widening; merge all proven
  caller regions and reject insufficient visibility overrides while preserving
  existing access boundaries.
* Include non-excluded written inherent methods/consts in `suggest_split` drafts
  while preserving `context_sensitive` eligibility, exact enclosing anchors,
  retained impls and per-group blocker forecasts; avoid scanning impl bodies
  twice when their members are separately inventoried.
* Consult declared features/cfg in written conditional context audits without
  requiring semantic resolution, sharing positive-only predicate evaluation
  across module chains, imports, required bindings, lexical and prelude scopes;
  retain anchored unknown-atom, inactive-binding and independent expansion vetoes.
* Route selected top-level `cfg`/`cfg_attr` predicates through configured
  original/final context checks, discharging active declared features with exact
  attribute proofs while retaining anchored unknown and inactive-item refusals.
  Keep direct derives on their binding audit path and active custom-derive
  payloads blocked; predicate context cannot erase a `derive_veto`.
* Classify all written impl members as `context_sensitive` in `suggest_split`,
  retaining their enclosing header anchors and excluding them from top-level-only
  batches without changing associated-item move validation.
* Prove ordinary workspace-member module chains through inert lint/doc metadata
  and configured positive cfg edges, without Cargo discovery or executing doc
  expressions; retain unknown, inactive, competing, unadmitted and recovered
  chain refusals with anchored evidence. Share lint/doc identity classification
  across context checks, keeping root and non-root controls and unknown attribute
  providers refused with exact anchors.
* Refuse provider-uncertain attributes in declarative declaration-identity proofs,
  including `serde(transparent)` beside familiar derive spellings; disclose the
  attribute and written owner anchors instead of treating unproved helper
  registration as inert metadata. Keep helper-free built-in-derive wrappers
  admitted, without executing procedural macros or claiming compilation.
* Disclose nested declarative expansion refusals at the outer written invocation
  and definition with `declarative_recursion_limit`, without admitting multi-step
  declaration identities.
* Audit only the effective binding of an extern-crate declaration: allow qualified
  standard-prelude proofs under `extern crate std as something;` while preserving
  the competing-root veto for `extern crate external as std;`.
* Discharge admitted qualified standard-prelude types despite same-name imports
  or declarations; audit competing `std` roots in the module/block scope chain,
  including written `mod std` declarations and `extern crate ... as std` aliases,
  while preserving context vetoes and nonsemantic caller-assumption labeling.

## [0.4.0] - 2026-10-07

### Added

* Add `refusal_basis` to retained `move_item` decisions, with original-source
  anchors for prelude, derive, shadow and lexical vetoes and failed semantic
  proof stages; omit unavailable coordinates instead of inventing ranges.
* Add `written_reexport` evidence for consumer import repairs through unique,
  unconditioned named public re-exports, preserving aliases and disclosing each
  hop and terminal declaration; bound chains to eight hops and reject uncertain
  routes without bypassing moved-item API decisions.
* Evaluate written `cfg` and `cfg_attr` under the caller's
  `semantic_configuration` and admit unshadowed bare compiler-built-in derive
  context; disclose reached original/final checks, inactive payloads and skipped
  reasons in `context_evaluations`, keeping undeclared atoms unknown.
* Add `ra_resolved` proofs classified as `variant_path` for written enum variants
  in value and pattern positions, with anchored variant and parent-enum identities
  and access checks at both original and final revisions; do not claim compilation
  or equivalence checking.
* Extend `assume_standard_prelude` to unshadowed bare value-position `Some` and
  `None`, with separate `standard_prelude_constructor` proofs and coverage;
  disclose that constructor identity is assumed, derive-generated imports are
  not modeled and macro hygiene is not proved.
* Add the repository `changelog` skill for maintaining user-facing release notes
  and cutting releases with explicit changelog rotation.

### Changed

* Replace the intermediate `hoist_possibility` lexical witness relation with
  `statement_macro_before_reference` for at-or-before invocations and
  `block_macro_may_introduce_items` for later invocations in enclosing blocks;
  disclose written witnesses without claiming expansion or hygiene.
* Scope custom-derive admission to nominal type and variant identity established
  through active written declarations or explicit non-glob import routes in both
  overlays; disclose `fact_class` and `basis`, retain conservative gates for
  generated-item-dependent facts and configured external roots, and withhold
  proofs when generated imports could override glob-resolved names.

### Fixed

* Allow `acknowledge_test_consumers` to acknowledge macro-witnessed bare references
  in the moved file's exact inline `#[cfg(test)]` module only when written super
  imports reach the source, type/value namespaces agree and a separate binding
  audit finds no other uncertainty; preserve witnesses on disclosed risks.
* Scope standard-prelude shadow audits to each reference's own/enclosing inline
  modules and final destination batch instead of inheriting filesystem ancestors'
  ordinary imports and declarations; keep `no_implicit_prelude` local and inherited
  downward, while retaining module-identity and unbounded expansion vetoes.
* Refuse lexical and standard-prelude proofs under potentially item-producing
  macros or unexamined outer attributes in enclosing blocks regardless of source
  order; keep body-only expansion sites from vetoing signatures outside those
  blocks and anchor definite shadows at their declarations.
* Distinguish scoped constructor/constant patterns from local binders and admit
  pattern arguments only without competing written constants, unit constructors,
  imports, globs or potentially item-producing macros; retain constructor path
  dependencies and unused ambiguous pattern needs, and respect match-arm and
  `let-else` binding scopes.
* Check written accessibility of every intermediate canonical re-export module
  from the final consumer; retain anchored `inaccessible_route:<segment>` refusal
  bases when no admitted accessible route exists, without widening hidden modules.
* Fall back to accessible written public re-export routes when a canonical path
  is hidden, including direct canonical imports; prefer accessible canonical paths,
  otherwise choose fewest hops then lexicographic absolute path and disclose
  `written_reexport_route_fallback` with route and visibility anchors.
* Block unrepaired bare consumers in other admitted files reached through visible
  relative, aliased, grouped or forwarded glob routes with
  `glob_consumer_unrepaired`, reference/import anchors and
  `selection_change_required`; withhold artifacts rather than waive these consumers
  through test acknowledgment or semantic resolution.
