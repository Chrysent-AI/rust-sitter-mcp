# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project uses [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed

* Correct the install and clone URLs in `README.md` and `CONTRIBUTING.md`, which
  pointed at the wrong repository host. Add a "configure the request for this
  repository" section to the bundled skill covering crate-graph derivation from
  `Cargo.toml`, source-tier selection and local preconditions, scope/budget
  reconnaissance, and the eligibility rule for macro- or glob-dense modules.

* Clarify the documented capability of the `sysroot_sources` evidence tier: it
  proves written inherent methods only. Written field accesses and direct
  constructions of sysroot types remain refused by the audited installation
  profile, and `docs/known-limitations.md` now records the measured walls. No
  engine behavior changed.

### Added

* Bumped version.

### Fixed

* Repair qualified consumer paths with `resolve_semantic:true` when one admitted
  immediate-parent glob carries a unique written module declaration or
  non-renamed module-import route. Preserve exact route anchors and path bytes
  outside the splice; ambiguous, conditional, recursive, competing and
  provider-uncertain routes still refuse. This is a written repair, not a new
  semantic proof or `ra_resolved` count; default-off and bare-name glob behavior
  are unchanged.

* Avoid `glob_consumer_unrepaired` for a moved free function when the consumer's
  only lexical uncertainty is a same-named pattern binding versus constant.
  Non-function selections, genuine glob-carried calls and other lexical
  uncertainties retain their blockers.

* Resolve cross-file qualified consumer evidence against the enclosing written
  path instead of its terminal identifier. Original-overlay facts can now be
  reached; missing final-overlay repairs still refuse, and semantic resolution
  remains opt-in.

* Admit tuple expressions/patterns and string patterns in exact caller-asserted
  standard `matches!` invocations. Classify field receivers in stationary
  consumers without confusing field names with moved free items; unasserted
  and unknown macro token trees retain conservative name-candidate refusals,
  including dot-following names.

* Discharge unshadowed standard-prelude types and `Some`/`None` independently of
  registered helper attributes (`serde`, `schemars`) attributable to a
  same-declaration derive on unrelated declarations. Fail closed on every other
  companion attribute — qualified, unknown or custom, written directly or inside
  `cfg_attr` — with an exact written-attribute witness, because a proc-attribute
  macro can emit module-scope items. Direct unknown/custom/path-qualified derives
  also veto unrelated prelude type/constructor proofs with unnamed
  `conditional_context` evidence at the exact outer `#[derive(...)]` attribute,
  retaining their own derive-name `derive_veto` and suppressing mixed-list builtin
  proofs. Admit bare `Serialize`/`Deserialize`/`JsonSchema` registrar spellings
  under the same disclosed spelling-based trust as their helpers, without
  discharging their own derive-identity needs. Preserve shadowed/unresolved
  derive-provider, same-spelling shadow and unproved-context refusals; recognize
  bare compiler `doc` metadata as context-independent.

## [0.6.0] - 2026-10-12

### Added

* Add default-off, move-only `sysroot_sources` evidence from the audited installed
  stable 1.97.1 aarch64-apple-darwin `rust-src` profile. Publish real both-overlay
  written `String::new`/`push_str` proofs with bounded immutable capture, explicit
  sysroot-only configuration and trusted-local-installation disclosure; missing,
  unsupported or stale inputs withhold proofs and artifacts. No installation,
  source writes, compilation, trait/generic or generated-member support is added.

* Add partial-only `suggest_split.analysis_budget` disclosure for descriptor
  guard stops, with typed collector stage, aggregate work cap and additive
  per-stage byte charges in full advice and compact manifests. Distinguish
  analysis work from retention and wire budgets; preserve incomplete labels,
  conservative draft withholding, limits and unaffected response shapes.

* Add an advisory `suggest_split` parent-directory scope hint when destination
  naming fails solely on `paths`/`globs` exclusions. Preserve existing reasons,
  ignore/collision disclosures and unaffected response shapes; admission policy
  remains unchanged.

* Add default-off `external_path_dependencies` source consultation for direct
  out-of-tree path libraries, with offline locked Cargo discovery in an ephemeral
  server-owned Cargo home, immutable bounded capture, consulted-root/digest
  disclosure and real both-overlay inherent-method proofs in `move_item`.
  Advice remains non-semantic; unsupported or stale inputs withhold plans.

* Persist explicitly retained split advice as server-owned, greppable per-topic
  JSON with an index and additive `retention.spill` pointers; add opt-in
  `response_mode:"references"` to avoid duplicate summaries and `rust-sitter-mcp flush`
  to clear durable analyses without modifying caller repositories.

* Add `move_item`'s `plan.blocked_summary` over the full blocking decision set,
  with typed reasons, every distinct refusal class, occurrence anchors and exact
  totals. Budget omissions are explicit and never change applicability or the
  existing individual-blocker disclosure.

* Disclose group-level `selection_completeness`, explicit reviewed supported
  bundles and visible exclusions in `suggest_split`. Keep incomplete selections
  and dominated files without supported names in destination-free
  `inspection_partitions`; report dominant indivisible items and original-core
  extraction payoff without changing honest negative outcomes or auto-selecting
  companions. Full advice schema 2 and compact manifest schema 1 remain additive.

* Add `read_skill_guide` for binary-only callers: serve the complete bundled
  Markdown guide and references, or one document, with exact byte counts and
  Git blob SHA-1 digests. Preserve the document index when response limits
  explicitly omit content; perform no runtime skill or repository reads.

* Add explicit `parent_facade` for private-child moves, preserving old named
  paths through region-bounded parent re-exports while keeping the child private.
  Only proven unchanged named/direct-glob routes discharge their path needs;
  ambiguous, conditional, conflicting and unsupported privacy cases still refuse.
  Derive, macro, receiver, test acknowledgment and final binder checks remain
  independent, with no compilation or semantic claim.

* Add exact `derived_attribute_relocations` caller assertions for unchanged
  written derive invocations and registered helpers with explicit provider import
  routes at both ends. Label admission as `assumed_derive_relocation` with separate
  coverage, not resolved evidence or generated-member proof; default and
  independent cfg, namespace, access, body and attachment vetoes remain strict.

* Add invocation-specific `standard_matches_assertions` for bounded standard
  `matches!` consumers, with separate `assumed_standard_matches` proofs and
  coverage. Preserve ordinary written-binding checks, exact variant suffixes,
  known-shadow refusals and conservative defaults; no expansion or semantic
  verification is claimed.

### Changed

* Disclose observed `paths`/`globs` versus ignore/policy rejection counts in
  `suggest_split` naming warnings; preserve collision counts and retain naming
  exhaustion warnings in optional `draft_eligibility.warnings`. Admission policy,
  reason IDs and the parent-directory recovery hint remain unchanged.

* Extend opt-in external evidence to accessible written named-field accesses,
  struct literals and tuple-struct calls, preserving both-overlay declaration
  and receiver identity in read-only `@external/` coordinates. Private, generic,
  trait-associated, generated and context-unproved probes still refuse; ordinary
  external type paths and enum variant paths remain unsupported.

* Extend external library evidence to the complete bounded normal-dependency
  transitive source closure, including mixed path, cached registry and vendor
  origins. Preserve package-ID edges and distinct namespaces across both overlays;
  disclose shared capture/comparison charges and per-overlay source totals in
  `coverage.external_dependencies.capture_usage`. Missing roots, cycles, stale
  inputs, cancellation and exhausted budgets fail closed without source writes.

* Extend `external_path_dependencies` to exact already-present registry and
  Cargo directory/vendor libraries without downloads or caller-cache writes.
  Resolve with bounded independent copies in ephemeral server state, preserve
  renamed edges and distinct package versions, disclose original source identity
  and `provenance`, and refuse missing or ambiguous local packages. External
  evidence remains read-only across both overlays; supported proofs are bounded
  inherent methods, written named fields and struct/tuple-struct constructors.

* Clarify that compact response-byte budgets and canonical retained-allocation
  budgets are independent; raising `response_bytes` does not recover
  `record_too_large` historical evidence. Document the read-only measurement probe.

* Prove spelling-specific pattern binders through fully admitted noncompeting
  direct globs. Unknown routes, competing names and existing independent
  blockers still refuse.
* Prove supported ordered if/while let-chain local scopes, preserving outer
  bindings and shadowing without relaxing independent move blockers.

### Fixed

* Retain structured external root identities, active capture limits and partial
  usage on discovery/capture refusals from `move_item` and `suggest_split`, even
  under response-byte pressure. Mark failed capture incomplete without a corpus
  digest; never publish prefix-corpus proofs or artifacts.

* Withhold already-computed resolution proofs and clear their counts when the
  external corpus fails its final freshness or cancellation check, alongside
  withholding every move artifact.

* Validate raw SHA-256 registry lock/index/archive and vendor file checksums
  before external evidence admission. Require fresh private archive unpacking to
  match the captured checkout's complete file membership and bytes; mismatched,
  missing or unsupported integrity inputs withhold all proofs and move artifacts.

* Preflight the bounded manifest-input closure before path-dependency discovery
  creates server state: check transitive path declarations and literal workspace
  members, including unconfigured siblings. Refuse overlapping roots, path cycles,
  unsafe/incomplete manifests, exhausted budgets, member globs/ambiguous lists and
  explicit workspace pointers before Cargo. Disclose conservative over-refusal;
  source admission separately follows supported resolved normal-library edges.

* Make spilled split-advice observation and decision anchors searchable for
  captured source spellings even when inline `text_bytes` is zero, without
  changing canonical omission flags or wire budgets.

* Distinguish `suggest_split` destination scope rejection, actual naming collisions
  and absent supported concept names. Preserve the ordinary prefix/file-fallback
  ladder, label file-based names as placeholders, and never interpret naming
  exhaustion as evidence that a file needs no reorganization.

* Verify version bumps with workspace tests and require active tracked hooks;
  reject direct current-package-version literals in test comparisons before
  they can drift. Leave failed bump diffs visible without committing or tagging.

* Reject unusable supplied semantic configurations with `INVALID_PARAMS`, an
  indexed input field, offending-value preview and actionable reason, even when
  no resolution needs are collected. Explain Rust crate names versus hyphenated
  Cargo package names; keep skipped resolution labeled `not_performed` and the
  written-only configuration path unchanged.
* Allow consumer path-prefix repair through well-formed composite pattern
  alternatives that provably cannot bind the queried spelling. Matching possible
  binders, macros, recovery and unknown syntax retain anchored uncertainty;
  final moved-binder identity and independent blockers remain unchanged.
* Withhold `move_item` artifacts when any proved written binder in a moved item
  loses final identity or encounters destination/companion competition, even
  without glob evidence or reads. Copy identifier, whole pattern and references
  exactly; derive valid final scopes so type-path repairs outside patterns stay
  applicable. Rewrites inside patterns still refuse. Retained code is not
  enrolled; externally compile/check the full batch.

## [0.5.0] - 2026-10-10

### Added

* Add explicit private `new_child` extraction plans through the existing planner
  for ordinary flat-file, `mod.rs` and crate-root parents, including one absent
  conventional flat-parent directory. Child-containing batches use schema 3 with
  linked directory preconditions and complete all-or-nothing artifacts; preserve
  legacy schema 2, strict proof boundaries and read-only behavior. Inactive request
  export accepts caller-selected child geometry without executing the planner.

* Add read-only `export_move_request` for one complete inactive ordinary request
  from explicit retained unit/destination entries, with exact item/header bytes,
  separate review/provenance and current observational corpus/scope/ignore/mode/
  filesystem-identity checks; refuse stale, excluded, overlapping or oversized
  selections without partial requests, implicit companions or move submission.

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

* **Balanced alternatives are now opt-in:** `suggest_split` omitting
  `include_balanced` or setting it false excludes byte balancing at the v0.5.0
  boundary. Explicit true requests the existing distinct low-confidence
  original-order alternative only when structural advice and layout are valid;
  identical alternatives are deduplicated, with no two-proposal promise. No
  balanced primary substitutes for `no_credible_written_partition`, and true
  does not override incomplete analysis, exclusions, membership or proof rules.
* Stop forming credible advice cores from weak naming/section/attribute/adjacency
  signals; completed analysis with no supported core reports
  `no_credible_written_partition` rather than a balanced primary. Balancing is
  available only as an explicitly requested alternative when structural advice exists.
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
