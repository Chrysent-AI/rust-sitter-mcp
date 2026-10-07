# Bounded declarative-macro declaration identity

## Trust boundary

A `macro_rules!` definition and its invocation are written tokens in admitted
sources. Declarative expansion is deterministic token substitution (including
fragment matching, repetition and recursion in the general language); it executes
no code. Procedural macros instead execute arbitrary code. They are never loaded,
executed or admitted as declaration-identity evidence, under any flag. Build
scripts and generated files outside the admitted snapshot remain outside the tier.

The permitted fact is only: a type-position path denotes the named declaration
produced by this written invocation and this written, in-crate definition. It is
not a fact about generated fields, constructors, variants, impls, methods, traits,
derive output or the layout/contents of the type. These facts retain their vetoes.
No compilation, equivalence or general macro-hygiene claim is made.

## Small admitted expansion

Reuse the pinned `ra_ap@0.0.357` request-owned resolver rather than implement a
second macro interpreter. Installed source exposes `Semantics::resolve_macro_call`,
`Macro::is_proc_macro`, `Macro::kind`, `Semantics::source`, `Semantics::expand` and
macro-file call-site mapping. These let the engine verify the written definition,
expand one vetted declarative invocation and compare the resolved ADT identity.
The databases have no proc-macro expander or Cargo/project loader.

Before admitting expansion evidence, require a real-file `macro_rules!`
definition in the invocation's configured crate and a real-file invocation.
The initial fragment budget is one `ident` argument, one arm with the matcher
`($name:ident)`, and substitution of that identifier only. No repetition,
recursive/nested macro calls, additional metavariables or other fragment kinds
are admitted. The expansion depth cap is one. Definition/output token budgets
are 4,096 significant tokens each; token-tree nesting is capped at 32. Unsupported
shapes or limits retain the semantic source/final-fact blocker with an anchored
context disclosure, not an approximation or a partially expanded proof.

Expansion may contain named structs/enums and impls, but no module-level imports,
modules, constants, functions, macro definitions or other namespace-producing
items. Generated impls are merely tolerated output: never used as facts.
By default, generated type attributes must be the existing inert lint/doc attributes
(`allow`, `warn`, `deny`, `forbid`, `doc`) or unshadowed bare compiler-built-in
derives. All other attributes, including `#[serde(transparent)]` beside a written
`Serialize`/`Deserialize` derive, refuse as provider-uncertain. Custom and qualified
derives also refuse in this tier. A familiar spelling cannot establish helper
registration: identical admitted tokens can name either an inert helper or an
attribute macro that replaces the declaration, depending on unmodeled providers.
Written declarative definitions cannot supply attribute/derive providers. No
provider evidence is guessed and no procedural expansion is observed.
Conditional attributes on a macro definition, invocation or their enclosing
items/modules veto even if their predicates are listed ON. Definitions in another
configured crate, builtin macros and procedural macros never qualify.

## Both-overlay identity and disclosure

Resolve and audit the original snapshot and assembler's exact final overlay
independently. Keep the existing stable explicit-import/non-glob prefix discipline.
The declaration identity anchors the invocation's identifier argument plus the
whole written invocation and definition. All three anchors must normalize through
the assembler's byte-origin map to the same original bytes and crate origin.
Changing the definition, invocation, route or generated declaration therefore
cannot pass merely because its final spelling is unchanged. For a bare signature
name imported through an existing written public named re-export, the planner may
carry that same accessible facade path into a sibling as a provisional explicit
import when resolution is enabled. It does not guess a deeper generated terminal,
widen visibility or clear the original need: both-overlay identity and all path
prefix access checks must still succeed. Missing, conditional, competing or
inaccessible facade evidence retains the blocker. This uses the existing import
repair machinery and does not introduce a request field.

Successful evidence uses additive `class:"declarative_macro_identity"` and basis
text stating that identity was established through bounded declarative-macro
expansion of written tokens, with resolved ADT identity and written argument
provenance checked at both overlays. Its basis states that provider-uncertain
attributes refuse and helper registration is not inferred from derive spelling;
RA equality is not a claim to observe unmodeled rustc replacements. Receiver fields
are null: generated field types, including unadmitted external types, are irrelevant
to declaration identity and
must not be smuggled into a receiver/type-layout proof. Context records disclose
original/final invocation and definition checks, cap/conditional failures and
the identity-only basis. `declarative_attribute_provider_uncertain` records the
attribute in its basis and anchors generated attributes to the whole written
macro definition; written owner attributes have exact original-source anchors.
The reached invocation also retains the refusal.

`assume_declared_helpers` adds a separate default-false caller assertion alongside
configured `resolve_semantic`: provider-uncertain written or generated struct/enum attributes
belong to registered derive helpers, not replacing providers. Well-formed companion
custom/qualified derive path lists on helper-bearing types are checked syntactically
only. The engine neither classifies registration nor executes providers. Conditional
proofs use `assumed_declared_identity` and basis wording that identity is assumed
with the type, not engine-classified, with no procedural expansion, hygiene or
compilation claims. Nominal written identities depending on that assumed namespace
are also labeled conditional. `coverage.assumed_declared_identity` counts these
separately, including omitted records, without inflating `coverage.ra_resolved`;
shared `resolution_coverage.decisions` counts their sum. Assumed context admissions
use `assumed_declared_helpers` and name the reached helper on the written definition.
All original/final identity anchors and access checks remain mandatory. The flag
cannot admit cfg/cfg_attr or prelude controls, attributes on non-type written owners, helper-free generated custom derives, over-cap definitions, proc-macro construction
or generated members. Omission/false preserves containment response bytes.
Independent chain/trivia/API/repair gates and
read-only byte-exact
planning are unchanged.

For nominal context only, a module-level macro can cease to be a blanket veto
when this same bounded audit proves its output cannot inject imports/modules or
other competing bindings. Generated-item-dependent context retains the blanket
module-macro veto. No macro-generated item becomes a selectable inventory unit.

## Verification

Use identifier-wrapper signature fixtures with named re-exports and unknown
external field types, original/final argument/definition anchors and source
immutability checks. Cover procedural/unresolved macros, conditional definitions,
repetition/recursion/token/nesting caps, external definitions, generated
methods/fields/constructors/variants, arbitrary attributes and generated imports.
Check identity mismatch and final-only context failure independently. Also replay
the frozen application signature shapes under their explicit edition/cfg graph;
helper-bearing declarations must remain blocked by default, and prove only as
labeled caller assumptions when `assume_declared_helpers` is enabled. Check mixed
real/assumed coverage, omission counters and unchanged default-off response bytes. Helper-free wrappers with inert metadata/bare built-in derives
still prove only the signature prerequisite, not an applicable associated-method
move batch. Verify replacement/re-export/alias and cfg-replacement counterexamples
with identical admitted inputs but differing external providers. Formatting, lint,
full tests and hook-backed quality gates remain required before handoff.
