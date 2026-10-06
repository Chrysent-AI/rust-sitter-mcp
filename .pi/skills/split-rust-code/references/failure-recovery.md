# Failure Recovery Reference

Common failures and typed recovery routing for rust-sitter-mcp. Unknown or undiagnosed failures should stop for diagnosis — do not invent overrides.

## Triage rule

- **Failed call** (`status: "failed"`): inspect `error.code` and `error.field` — the fix targets the named request field.
- **Blocked plan** (`status: "complete"`, `plan.state: "blocked"`): inspect `plan.blockers[]` and `plan.decisions[]`; artifacts are withheld.
- **Incomplete advice** (`status` not complete, or omissions/truncation present): inspect `truncation_reasons`, `coverage`, `counts.omissions`.

For a retained move need, inspect `plan.decisions[].refusal_basis[]`: each entry is `{class, anchor:{path,range?}, name?}`. Prelude/derive vetoes identify `chain_macro_statement`, `derive_veto`, `shadow` (with the competing name), `conditional_context`, `glob_import`, `unresolved_chain` or `syntax_recovery`; lexical refusals identify `lexical_uncertainty` or a positional `chain_macro_statement` and retain the full `lexical_uncertainty` witness. Other written failures use `written_binding_unproved` alongside the existing decision reason. Semantic entries identify the failed configuration, overlay, mapping, source-fact, final-fact or identity proof stage (`semantic_*_unproved`, except `semantic_overlay_unavailable`), not a finer internal resolver cause. Written witnesses use original byte ranges; unresolved paths and synthesized bindings omit the range. Zero proofs with `assume_standard_prelude` enabled are therefore diagnosable, not permission to add guessed imports or weaken a veto. If details are capped, request a larger `diagnostic_count` and response budget as needed.

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
| `external_or_missing_binding` | External/unresolved dependency; default blocker. Standard-prelude/builtin-derive proofs or `ra_resolved` may discharge only eligible occurrences when their opt-in conditions are met; otherwise no override. |
| `member_or_constructor_unproved` | Field/method access needs type knowledge — including method calls (`x.trim()`) and prelude types (`Option`, `String`, `usize`) inside the moved item's own body | Default blocker for unresolved method/field/constructor access. `resolve_semantic` can prove eligible true-inherent/field/constructor access only with explicit configuration and positive access at both ends; trait/generic/macro uncertainty remains blocked. |
| `lexical_context_unproved` | Binding/pattern/namespace context unknown | Inspect `lexical_uncertainty` as diagnostic evidence, not proof. `witness_relation: "hoist_possibility"` marks a positional same-block macro witness that need not contain the anchor. `acknowledge_test_consumers` can acknowledge only that risk on an otherwise eligible same-file inline test reference with no competing written local or other lexical uncertainty. Otherwise it remains `unsupported_in_engine`; no binding-evidence field resolves it. Change selection or investigate separately. |
| `macro_context_unexamined` | Required macro expansion context | Unsupported; the engine doesn't expand macros. |
| `glob_binding_unproved` | Wildcard import provenance unclear | Unsupported; no glob synthesis. |
| `public_path_change` | Move would change a public/reexported path | No API shim; preserve the exposed path or design compatibility separately. |
| `required_rewrite_retained` | A required repair was rejected | Re-submit with `accept_default` or a valid `replace`. |
| `unsupported_unit_kind` / `unsupported_construct` | Context-sensitive or unsupported form | Omit from moves. |

`test_consumer_acknowledged` is a nonblocking disclosed decision, not a blocker or test result; inspect its anchors and any retained lexical witness, then run caller tests. A same-block `hoist_possibility` acknowledgment accepts expansion risk without proving binding identity or repairing the consumer; other lexical ambiguity, written conflicts, stale anchors, and structural failures remain blocking. `standard_prelude`, `standard_builtin_derive`, and `ra_resolved` are proof classes, not refusal reasons; confirm their anchors and separate coverage counts. Resolution labeling is not compilation.

### Advice-completeness states (suggest_split)

| `draft_eligibility.state` / reason | Meaning | Recovery |
|---|---|---|
| `incomplete` + `response_bytes` | Drafts/decisions withheld at the response cap | Retry with `limits: {"response_bytes": 8388608}` (≤16 MiB). Membership may still be complete — the inventory is usable. |
| `no_draft` + `no_admitted_nonconflicting_name…` | Already modularized / too few movable units | Legitimate "nothing to do"; report the design, don't force a split. |
| `no_draft` + chain reasons | Root/scope problem | Fix `crate_root`/`paths` per chain diagnostics. |

`root: null` in the advice envelope means the same response-budget truncation — recover the budget before constructing anchors.

## Module-chain failures

Chain reasons appear under `chain_diagnostics`, including `macro_generated_module_tree` and `root_attribute_chain_uncertainty`; named refusals explain the cause but do not offer an override. A valid root can still hit unadmitted (`chain_file_unadmitted`), missing (`chain_file_missing`), conditional (`conditional_declaration`), path-attribute, competing (`competing_declarations`/`competing_file_layout`), or inline (`inline_module_layout`) hops. Follow the diagnostic's named failing hop; `relation` distinguishes `direct` observations from mere `possible_ancestor` obstructions.

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
