# Failure Recovery Reference

Common failures and typed recovery routing for rust-sitter-mcp. Unknown or undiagnosed failures should stop for diagnosis — do not invent overrides.

## Triage rule

- **Failed call** (`status: "failed"`): inspect `error.code` and `error.field` — the fix targets the named request field.
- **Blocked plan** (`status: "complete"`, `plan.state: "blocked"`): inspect `plan.blockers[]` and `plan.decisions[]`; artifacts are withheld.
- **Incomplete advice** (`status` not complete, or omissions/truncation present): inspect `truncation_reasons`, `coverage`, `counts.omissions`.

For a retained move need, inspect `plan.decisions[].refusal_basis[]`: each entry is `{class, anchor:{path,range?}, name?}`. Prelude/derive vetoes identify `chain_macro_statement`, `derive_veto`, `shadow` / `macro_shadow` (with the competing name), `conditional_context`, `prelude_disabled`, `module_attribute`, `unparseable_attribute`, `glob_import`, `unresolved_chain` or `syntax_recovery`; lexical refusals identify `lexical_uncertainty` or a `chain_macro_statement` in the reference's block or an enclosing block and retain the full `lexical_uncertainty` witness. Written re-export route failures with no accessible admitted fallback use `inaccessible_route:<segment>` with `name` identifying the inaccessible or unprovable intermediate module and its original declaration anchor when available. Other written failures use `written_binding_unproved` alongside the existing decision reason. Semantic entries identify the failed configuration, overlay, mapping, source-fact, final-fact or identity proof stage (`semantic_*_unproved`, except `semantic_overlay_unavailable`), not a finer internal resolver cause. Written witnesses use original byte ranges; unresolved paths and synthesized bindings omit the range. Zero proofs with `assume_standard_prelude` enabled are therefore diagnosable, not permission to add guessed imports or weaken a veto. If details are capped, request a larger `diagnostic_count` and response budget as needed.

With explicit `resolve_semantic`, also inspect `plan.resolution_coverage.context_evaluations[]` for the reached original/final attribute and cfg checks, exact anchors, true/false values, inactive payloads and skipped reasons. Listed cfg/features are ON; unlisted atoms remain unknown, including when another `all`/`any` operand determines the Boolean result. Supported `all`/`any`/`not` and bare compiler-built-in derives may remove their own RA context veto. Inspect `fact_class` and `basis`: well-formed custom/qualified derives on a target or sibling can be admitted for nominal paths only when locally written declarations or explicit named import routes establish binding stability at both overlays. Generated explicit imports can override glob-resolved names without a compile error; additive output alone is insufficient. Inspect anchored `kind:"binding"` checks for `stable_written_identity` or `stable_written_identity_unproved`; an unproved route retains the source/final-fact refusal. Aliases/grouped leaves qualify only with non-glob module/re-export prefixes and active binding attributes; known-OFF imports are not evidence (`inactive_written_binding`). Configured extern-prelude dependency roots retain the strict gate (`configured_dependency_root_with_conservative_context`), not custom-derive admission. They still veto `generated_items` facts (including methods/field inference/function resolution); both evaluations may exist for the same anchor. An undeclared `cfg_attr` condition still vetoes either class. `classification:"context_attribute"` is attribute-context evidence for its stated fact class, not dependent pattern/body resolution or generated impl presence; access repairs and independent chain/macro/trivia/API blockers remain. `classification:"variant_path"` instead proves the terminal variant and parent enum identity/access at both overlays. Skipped checks are not successful evidence.

For bounded declarative identity, inspect `class:"declarative_macro_identity"`, `classification:"declaration_identity"`, null receivers, and both `declaration.declarative_macro:{invocation,definition}` pairs plus argument anchors. The opt-in admits only in-crate written `macro_rules!` with one ident/arm/expansion step, 4096 significant definition/output tokens and nesting 32. `kind:"declarative_macro"` and `"declarative_macro_definition"` disclose admission or `declarative_*` shape/cap/conditional failures; unsupported expansions keep the semantic source/final-fact blocker. Conditional definitions/invocations/enclosing modules veto even cfg ON. Generated type attributes permit only existing inert lint/doc metadata and unshadowed bare built-in derives. By default, helpers (including `serde(transparent)` beside `Serialize`/`Deserialize`) and custom/qualified derives refuse with `declarative_attribute_provider_uncertain`; helper registration is not inferred from spelling. The skipped basis names the attribute, anchored to its whole written definition for generated attributes or its exact written-owner range. The reached invocation is also skipped. Do not detach helpers to recover earlier unsupported proofs. With `assume_declared_helpers:true` plus configured `resolve_semantic`, the caller instead asserts direct written or generated struct/enum attributes are registered derive helpers, not replacing providers; registration is not engine-classified. Helper-bearing types can admit well-formed companion derive paths syntactically. Inspect distinct `assumed_declared_identity` proofs, their caller-assumption basis, `coverage.assumed_declared_identity` and `assumed_declared_helpers` context reasons. Nominal facts relying on this assumed namespace are conditional too; none inflate `coverage.ra_resolved`. Both-overlay anchors/access and all independent vetoes remain. Control attributes, non-type written owner attributes and helper-free generated custom derives still refuse. Proc macros never qualify or execute, and generated impl/method/field/constructor/variant facts remain blocked. A provisional existing public facade import says `pending both-overlay declaration identity`; accepting it does not bypass either identity check. No generated terminal or macro-created item is guessed. See `docs/tools.md` for the exact admission and refusal vocabulary.

Accepting unrelated rewrites does not clear independent blockers; unsupported decisions cannot be cleared by accepting unrelated rewrites.

Decision groups may encode `decision_ids` as `{first_id, count}` runs rather than one entry per decision. Read group counts and omissions; `diagnostic_count` controls returned full exemplars, not the underlying decision count.

## Common error codes

| Error code | Meaning | Recovery |
|---|---|---|
| `STALE_SELECTION` | Source changed since anchors were built, or range/text mismatch | Re-read current file, rebuild anchors from fresh inventory. Check `error.field` for the failing move. |
| `INVALID_ITEM_SELECTION` | Anchored bytes aren't one whole direct top-level item | Use the inventory's `span.range`; items inside inline `mod` bodies aren't selectable. |
| `DUPLICATE_MOVE` | Same unit assigned twice | Deduplicate by path+range. |
| `INVALID_DESTINATION` / `STALE_DESTINATION` | Bad destination kind/layout, or stale `before_item` | Correct layout/scope; refresh the `before_item` anchor; omit it for EOF append. |
| `INVALID_NEW_FILE_NAME` / `INVALID_DECLARATION_PARENT` | Illegal basename, or parent-to-child layout mismatch | Choose a legal non-keyword ASCII basename and the actual ordinary declaring parent (check chain diagnostics). |
| `DESTINATION_ALREADY_EXISTS` / `MODULE_DECLARATION_CONFLICT` | Occupied/case-colliding path or competing declaration | Choose an absent name, or append to the existing file with `kind: "existing"`. Never delete conflicts automatically. |
| `INVALID_PARAMS` / `INVALID_GLOB` | Malformed request field or glob | Validate the named field; globs are positive, root-anchored, no braces/negation. If the error shows `must be object` or a quoted-object/`"null"` value, your client stringified an object-typed parameter — omit the parameter entirely instead. |
| `PATH_NOT_FOUND` / `NOT_GIT_WORKTREE` / `GIT_READ_FAILED` | `repo_path`/paths don't resolve | Use an absolute path inside a real Git worktree; root-relative file paths. |
| `PREEXISTING_SYNTAX_ERROR` / `NEW_SYNTAX_ERROR` | Parse errors in base or resulting plan | Repair the base through separate authorized work; revise the batch. |
| `ATTRIBUTE_ATTACHMENT_CHANGED` / `OVERLAPPING_EDITS` | Unsafe attribute attachment or conflicting intervals | Remove the unsafe choice; revise the batch. |
| `INVALID_REWRITE_OVERRIDE` / `STALE_REWRITE_OVERRIDE` | Unsupported/duplicate override or changed contributor anchor | Rerun the same batch without stale choices; copy fresh full published targets verbatim. |
| `INVALID_MOVE_TRIVIA_OVERRIDE` / `UNSUPPORTED_TRIVIA_DISPOSITION` | Stale/unrelated trivia anchor, unselected target, protected attachment | Use exact current ordinary-ambiguous anchors only; leave protected/owned trivia alone. |
| `BUSY` | Another engine call is running | Serialize; retry after completion. |
| Cancellation | Work stopped | Only retry if still requested. |
| Limit/partial states | Membership/evidence/scan/output not complete | Inspect truncation+omissions; raise supported limits or reduce optional text/context — never exclude needed chain/binding evidence to force applicability. |

`SOURCE_CHANGED`/`CREATION_RACE` can appear as incompleteness reasons rather than error codes; `CRATE_IDENTITY_UNCERTAIN` typically surfaces as a module-context blocker. Same recovery either way: stabilize the base, reread, replan.

## Decision routing

Each decision carries `action.route`, `action.field`, `action.purpose`, and often `action.choices`. Follow them exactly:

- `resolve_decision` → replay the full published target via the named override (`rewrite_overrides` or `trivia_overrides`).
- `review_default` → a nonblocking default (usually trivia); adjust only with supported dispositions.
- `submit_for_analysis` → change `crate_root`, `paths`, or `moves` and rerun; applicability is not promised.
- `selection_change_required` → the selected items/destinations themselves must change.
- `unsupported_in_engine` → no override can resolve it; remove the affected items or handle separately.

Note: `request_field` route means "a request field or choice is actionable per `action.purpose`" — it is not synonymous with "add an override". Chain diagnostics, for example, route to `crate_root`/`paths` corrections.

### Common blocking reasons

| Reason | Meaning | Agent action |
|---|---|---|
| `external_or_missing_binding` | External/unresolved dependency; default blocker. Standard-prelude/builtin-derive proofs `ra_resolved`, `declarative_macro_identity`, or caller-assumed `assumed_declared_identity` may discharge only eligible occurrences when their opt-in conditions are met; otherwise no override. |
| `member_or_constructor_unproved` | Field/method access needs type knowledge — including method calls (`x.trim()`) and prelude types (`Option`, `String`, `usize`) inside the moved item's own body | Default blocker for unresolved method/field/constructor access. `resolve_semantic` can prove eligible true-inherent/field/constructor access only with explicit configuration and positive access at both ends; trait/generic/macro uncertainty remains blocked. |
| `lexical_context_unproved` | Binding/pattern/namespace context unknown | Inspect `lexical_uncertainty` as diagnostic evidence, not proof. `witness_relation: "statement_macro_before_reference"` marks an at-or-before written-token-tree witness; `"block_macro_may_introduce_items"` marks a later standalone macro in the reference's block or an enclosing block that may introduce hoisted items. Either witness need not contain the anchor; expression-statement syntax does not prove harmless output. This is not expansion or hygiene proof. `acknowledge_test_consumers` can acknowledge either risk on an otherwise eligible namespace-compatible same-file inline test reference with no competing written local or other lexical uncertainty. Otherwise it remains `unsupported_in_engine`; no binding-evidence field resolves it. Change selection or investigate separately. |
| `macro_context_unexamined` | Required macro expansion context | Unsupported for moved macro-dependent bodies/items; the narrow opt-in declaration-identity tier does not clear this independent blocker. |
| `glob_binding_unproved` | Wildcard import provenance unclear | Unsupported; no glob synthesis. |
| `glob_consumer_unrepaired` | Bare selected-name consumer in another admitted file may reach the moved item through visible globs, including chained `super`/module-alias routes | `selection_change_required`; inspect the first call-site identifier anchor and subsequent complete glob-statement anchors (`glob_import` refusal witnesses). Keep the item in place or revise the selected move. No third-file glob repair, semantic discharge or test acknowledgment; never narrow away the consumer to force applicability. |
| `public_path_change` | Moving the declaration itself would change a public/reexported path | No API shim; preserve the exposed path or design compatibility separately. Consumer imports may instead follow a unique written re-export route, but that does not discharge this moved-item API decision. |
| `required_rewrite_retained` | A required repair was rejected | Re-submit with `accept_default` or a valid `replace`. |
| `unsupported_unit_kind` / `unsupported_construct` | Context-sensitive or unsupported form | Omit from moves. |

Consumer-side named, unconditioned `pub use`/`pub(crate) use` routes can be followed automatically to an admitted terminal declaration. Inspect `rewrites[].evidence` for `written_reexport`, every hop and terminal declaration in `anchors`, and the chosen canonical/public path in `after_text` (renames preserve the consumer binding). This is written syntax, not semantic proof. The chosen path's intermediate modules must be visible from the final destination: `pub`/`pub(crate)` permit crate access; private/`pub(self)` permit the declaring parent and its descendants; `pub(super)` permits that parent's parent and descendants; `pub(in ...)` must resolve through written `crate`/`self`/`super` to an ancestor containing the destination. An inaccessible or unprovable edge withholds the rewrite with `visibility_scope_unproved` and `inaccessible_route:<segment>` refusal basis (no invented range for a missing/synthesized edge). The engine first prefers an accessible canonical path, then searches admitted named public re-export routes to the same terminal. Fallback choice is fewest re-export hops, then lexicographic absolute path, disclosed in `rationale` as `written_reexport_route_fallback` with canonical rejection, chosen path, candidate count and hop count. Anchors retain original/chosen hops, terminal, rejected module and chosen module edges; hop targets must also be written-accessible from their exporting modules. Terminal attribute vetoes still apply, and no canonical module widening is offered. The anchored refusal remains only when no accessible admitted route exists. Inspect the named segment and anchor, then follow the decision route; do not guess an import or broaden visibility to force applicability. Globs, attributed leaves (`cfg`/`cfg_attr` included), competing leaves, cycles, routes exceeding eight hops and missing terminal declarations remain blocked by the written tier; the opt-in may provisionally preserve an existing accessible named public facade import only while retaining a both-overlay semantic identity need. Their written-binding/conditional-context reason is not an API override opportunity. Admit missing evidence only when the returned route permits scope correction, otherwise change selection or investigate separately. Moving the exposed declaration itself still requires the separate `public_path_change` API decision; no facade is synthesized.

`test_consumer_acknowledged` is a nonblocking disclosed decision, not a blocker or test result; inspect its anchors and any retained lexical witness, then run caller tests. A `statement_macro_before_reference` or `block_macro_may_introduce_items` acknowledgment accepts expansion risk without proving binding identity or repairing the consumer; other lexical ambiguity, written conflicts, stale anchors, and structural failures remain blocking. `standard_prelude`, `standard_prelude_constructor`, `standard_builtin_derive`, `ra_resolved`, `declarative_macro_identity`, and conditional `assumed_declared_identity` are proof classes, not refusal reasons; bare value-position `Some`/`None` constructor proofs assume identity with unshadowed `Option`, never qualified paths or patterns; their caller-assumption basis explicitly says derive-generated imports are not modeled; confirm their anchors and separate coverage counts. Resolution labeling is not compilation.

### Advice-completeness states (suggest_split)

| `draft_eligibility.state` / reason | Meaning | Recovery |
|---|---|---|
| `incomplete` + `response_bytes` | Drafts/decisions withheld at the response cap | Retry with `limits: {"response_bytes": 8388608}` (≤16 MiB). Membership may still be complete — the inventory is usable. |
| `no_draft` + `no_admitted_nonconflicting_name…` | Already modularized / too few movable units | Legitimate "nothing to do"; report the design, don't force a split. |
| `no_draft` + chain reasons | Root/scope problem | Fix `crate_root`/`paths` per chain diagnostics. |

`root: null` in the advice envelope means the same response-budget truncation — recover the budget before constructing anchors.

## Module-chain failures

Chain reasons appear under `chain_diagnostics`, including `macro_generated_module_tree` and `root_attribute_chain_uncertainty`; named refusals explain the cause but do not offer an override. A valid root can still hit unadmitted (`chain_file_unadmitted`), missing (`chain_file_missing`), conditional (`conditional_declaration`), path-attribute, competing (`competing_declarations`/`competing_file_layout`), or inline (`inline_module_layout`) hops. Follow the diagnostic's named failing hop; `relation` distinguishes `direct` observations from mere `possible_ancestor` obstructions. Workspace members need their Git-root-relative source root and all admitted hops, not the workspace manifest. Built-in lint/doc metadata and documentation-only `cfg_attr` payloads no longer cause chain uncertainty; doc expressions are not read or executed. Configured `resolve_semantic` on `move_item` can admit ON cfg-gated edges from one matching root's explicitly listed positive atoms, but unknown atoms and known-OFF edges still refuse. `suggest_split` has no cfg opt-in; metadata chain admission never clears independent semantic/prelude/binding context vetoes.

## Rewrite overrides

Add to the **original request** (extra members, not a stand-alone call):

```json
{"target": { ...published target object, copied verbatim... }, "action": "accept_default"}
```

- `accept_default`: accept the proposed repair (no replacement text)
- `retain`: keep original bytes; blocks the batch if the repair was required
- `replace`: substitute supported same-target alternative text (≤64 KiB) — simple paths, private non-glob imports/aliases, private/`pub(crate)` visibility, safe separators, unchanged ordinary module declaration

**Copy the published target object verbatim** — a source target contains a full anchor; a synthesis target contains `path`, `slot`, contributor `items`, `binding`, etc. Never reconstruct from display indices.

## Trivia overrides

```json
{
  "trivia": {"path": "src/big.rs", "range": {...}, "expected_text": "// banner"},
  "disposition": "carry_with_item",
  "target_item": { ...full selected item anchor... }
}
```

Only ordinary ambiguous trivia (banners, blank-separated comments between units) can be re-dispositioned; `keep_in_place` forbids `target_item`. Protected/owned/internal trivia cannot be detached, retargeted, or discarded.
