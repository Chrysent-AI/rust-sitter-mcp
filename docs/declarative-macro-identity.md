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
Generated type attributes must be inert lint/doc attributes or well-formed
additive derives; arbitrary attribute macros and conditional attributes refuse.
The exact `#[serde(transparent)]` helper on a type with a written `Serialize` or
`Deserialize` derive is tolerated as declaration metadata, not executed or used
as field/serialization evidence. Other helper shapes are not admitted.
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
expansion of written tokens. Receiver fields are null: generated field types,
including unadmitted external types, are irrelevant to declaration identity and
must not be smuggled into a receiver/type-layout proof. Context records disclose
original/final invocation and definition checks, cap/conditional failures and
the identity-only basis. The opt-in request schema is unchanged. Default-off
behavior, independent chain/trivia/API/repair gates and read-only byte-exact
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
this proves only the signature prerequisite, not an applicable associated-method
move batch. Formatting, lint, full tests and hook-backed quality gates remain
required before handoff.
