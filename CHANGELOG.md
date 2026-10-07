# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project uses [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

* Add `declarative_macro_identity` signature-type proofs through bounded in-crate
  written `macro_rules!` expansion, with matching invocation/definition anchors
  at both overlays and provisional preservation of accessible public facade
  imports; retain conditional/complex expansion and generated-member vetoes,
  and never execute procedural macros.
* Extend `assume_standard_prelude` type proofs to `std::option::Option`,
  `std::result::Result`, `std::boxed::Box`, `std::vec::Vec` and
  `std::string::String`, preserving original/final shadow and context audits
  and caller-assumption labeling without semantic configuration.

### Fixed

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
