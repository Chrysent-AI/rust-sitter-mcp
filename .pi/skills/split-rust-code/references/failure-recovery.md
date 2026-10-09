# Failure Recovery Reference

Common failures and typed recovery routing for rust-sitter-mcp. Unknown or undiagnosed failures should stop for diagnosis — do not invent overrides.

## Triage rule

- **Failed call** (`status: "failed"`): inspect `error.code` and `error.field` — the fix targets the named request field.
- **Blocked plan** (`status: "complete"`, `plan.state: "blocked"`): inspect `plan.blockers[]` and `plan.decisions[]`; artifacts are withheld.
- **Export envelope:** `export_move_request` has its own envelope, not an ordinary `status`/`plan`. Inspect `error` and `request` first; any export failure has `request:null`, so there is no usable request to pass on.
- **Incomplete advice:** for full schema-2 responses, inspect `status`, `coverage`, `truncation_reasons`, and `counts.omissions`. For compact `envelope_kind:"split_manifest"` schema 1, inspect `analysis_status`, `manifest_complete`, `coverage`, `omissions`, `totals` and `detail_availability`; manifest omissions can be retrievable detail rather than incomplete analysis.

For a retained move need, inspect `plan.decisions[].refusal_basis[]`: each entry is `{class, anchor:{path,range?}, name?}`. Prelude/derive vetoes identify `chain_macro_statement`, `derive_veto`, `shadow` / `macro_shadow` (with the competing name), `conditional_context`, `prelude_disabled`, `module_attribute`, `unparseable_attribute`, `glob_import`, `unresolved_chain` or `syntax_recovery`; lexical refusals identify `lexical_uncertainty` or a `chain_macro_statement` in the reference's block or an enclosing block and retain the full `lexical_uncertainty` witness. Written re-export route failures with no accessible admitted fallback use `inaccessible_route:<segment>` with `name` identifying the inaccessible or unprovable intermediate module and its original declaration anchor when available. Other written failures use `written_binding_unproved` alongside the existing decision reason. Semantic entries identify the failed configuration, overlay, mapping, source-fact, final-fact or identity proof stage (`semantic_*_unproved`, except `semantic_overlay_unavailable`), not a finer internal resolver cause. Written witnesses use original byte ranges; unresolved paths and synthesized bindings omit the range. Zero proofs with `assume_standard_prelude` enabled are therefore diagnosable, not permission to add guessed imports or weaken a veto. If details are capped, request a larger `diagnostic_count` and response budget as needed.

`semantic_configuration` also feeds written conditional audits without `resolve_semantic:true`. Positive atoms discharge only known cfg context, not unknown predicates or independent blockers. Written-only calls do not emit `resolution_coverage`; inspect module assumptions, prelude proof bases and retained conditional consequences for configuration consultation, keeping `integrity.semantic:"not_performed"`. Selected items, required bindings and module edges must remain active; known-OFF retained context keeps conservative shadows.

With explicit `resolve_semantic`, also inspect `plan.resolution_coverage.context_evaluations[]` for the reached original/final attribute and cfg checks, exact anchors, true/false values, inactive payloads and skipped reasons. Listed cfg/features are ON; unlisted atoms remain unknown, including when another `all`/`any` operand determines the Boolean result. Supported `all`/`any`/`not` and bare compiler-built-in derives may remove their own RA context veto. Selected top-level item attributes are checked at their exact original/final attribute anchors, including attached bytes outside the item syntax range; `features:["name"]` and `cfg:[{key:"feature",value:"name"}]` supply equivalent ON evidence. Known-OFF selected items and required bindings retain `inactive_written_binding`. Declaring every manifest feature does not declare unrelated keys such as `docsrs` or `debug_assertions`, nor clear independent lexical/prelude, inline-module or macro vetoes. Inspect `fact_class` and `basis`: well-formed custom/qualified derives on a target or sibling can be admitted for nominal paths only when locally written declarations or explicit named import routes establish binding stability at both overlays. Generated explicit imports can override glob-resolved names without a compile error; additive output alone is insufficient. Inspect anchored `kind:"binding"` checks for `stable_written_identity` or `stable_written_identity_unproved`; an unproved route retains the source/final-fact refusal. Aliases/grouped leaves qualify only with non-glob module/re-export prefixes and active binding attributes; known-OFF imports are not evidence (`inactive_written_binding`). Configured extern-prelude dependency roots retain the strict gate (`configured_dependency_root_with_conservative_context`), not custom-derive admission. They still veto `generated_items` facts (including methods/field inference/function resolution); both evaluations may exist for the same anchor. An undeclared `cfg_attr` condition still vetoes either class. `classification:"context_attribute"` is attribute-context evidence for its stated fact class, not dependent pattern/body resolution or generated impl presence; access repairs and independent chain/macro/trivia/API blockers remain. `classification:"variant_path"` instead proves the terminal variant and parent enum identity/access at both overlays. Skipped checks are not successful evidence.

For bounded declarative identity, inspect `class:"declarative_macro_identity"`, `classification:"declaration_identity"`, null receivers, and both `declaration.declarative_macro:{invocation,definition}` pairs plus argument anchors. The opt-in admits only in-crate written `macro_rules!` with one ident/arm/expansion step, 4096 significant definition/output tokens and nesting 32. `kind:"declarative_macro"` and `"declarative_macro_definition"` disclose admission or `declarative_*` shape/cap/conditional failures; unsupported expansions keep the semantic source/final-fact blocker. Conditional definitions/invocations/enclosing modules veto even cfg ON. Generated type attributes permit only existing inert lint/doc metadata and unshadowed bare built-in derives. By default, helpers (including `serde(transparent)` beside `Serialize`/`Deserialize`) and custom/qualified derives refuse with `declarative_attribute_provider_uncertain`; helper registration is not inferred from spelling. The skipped basis names the attribute, anchored to its whole written definition for generated attributes or its exact written-owner range. The reached invocation is also skipped. Do not detach helpers to recover earlier unsupported proofs. With `assume_declared_helpers:true` plus configured `resolve_semantic`, the caller instead asserts direct written or generated struct/enum attributes are registered derive helpers, not replacing providers; registration is not engine-classified. Helper-bearing types can admit well-formed companion derive paths syntactically. Inspect distinct `assumed_declared_identity` proofs, their caller-assumption basis, `coverage.assumed_declared_identity` and `assumed_declared_helpers` context reasons. Nominal facts relying on this assumed namespace are conditional too; none inflate `coverage.ra_resolved`. Both-overlay anchors/access and all independent vetoes remain. Control attributes, non-type written owner attributes and helper-free generated custom derives still refuse. Proc macros never qualify or execute, and generated impl/method/field/constructor/variant facts remain blocked. A provisional existing public facade import says `pending both-overlay declaration identity`; accepting it does not bypass either identity check. No generated terminal or macro-created item is guessed. See `docs/tools.md` for the exact admission and refusal vocabulary.

Accepting unrelated rewrites does not clear independent blockers; unsupported decisions cannot be cleared by accepting unrelated rewrites.

Decision groups may encode `decision_ids` as `{first_id, count}` runs rather than one entry per decision. Read group counts and omissions; `diagnostic_count` controls returned full exemplars, not the underlying decision count.

Candidate/group `consequence_summary` links exact runs qualified by `decisions`
or `advice_decisions`; never merge their `d/N` and `ad/N` namespaces. Counts are
uncapped distinct records per class, non-additive across classes. `unmapped` keeps
unknown reasons visible and needs inspection, not an invented override. Omitted
full-response advice detail may need diagnostic/response expansion. If the caller
explicitly retained completed advice and its handle is still available, omitted
evidence may instead be retrieved historically; compact availability is not
implicit, so inspect `detail_availability` and `retention` before relying on it.
Selection-completeness and outside/test observations remain prospective review,
not proof the edited batch is incomplete or unrepairable. Local lower bounds and
unassessed destination/batch applicability remain unchanged.

### Compact manifests and retained historical detail

For `envelope_kind:"split_manifest"` schema 1, branch on `analysis_status` and
`manifest_complete`, then inspect `coverage`, `omissions`, `totals` and each
`detail_availability`. A compact manifest has no legacy `inventory[]`. Analysis
completion, mandatory-manifest completeness, detail availability and each
returned page's `returned_page_complete` / `collection_exhausted` state are
separate. Retrieve only when `retention.state:"retained"` and the collection is
marked `retrievable:true`; unavailable records are permanently unavailable from
that result. Do not silently substitute a fresh analysis for historical evidence.
A new `suggest_split` call is a separately requested analysis with a new identity.

| Condition or error | Meaning | Recovery |
|---|---|---|
| Retention `unavailable` with `capacity` or `record_too_large` | No usable historical handle was published | Review the ordinary full response if complete; release a retained record you no longer need if it occupies capacity, or retry without retention. Larger output limits do not remove the record/capacity limit. |
| Retention `unavailable` with `analysis_incomplete` | The completed-analysis retention precondition was not met | Recover the incomplete advice cause first; do not treat a partial result as retained evidence. |
| Retention `unavailable` with `response_budget` | Retention metadata or response did not fit | Raise `response_bytes` within bounds or use the full response mode; no retrievable handle is promised. |
| Retention `unavailable` with `cancelled_before_publication` | Cancellation prevented publication | Retry only if the analysis is still requested. |
| `ADVICE_MANIFEST_TOO_LARGE` | Mandatory compact membership/consequence/coverage could not fit; manifest is incomplete and no handle is published | Use full response mode or a larger allowed `response_bytes`; inspect remaining omissions and do not treat the failed compact manifest as complete. |
| `ADVICE_SNAPSHOT_UNKNOWN` | Handle is unknown, released, corrupted or from a previous process | Confirm the exact returned handle is still available in this process; do not guess or silently reanalyze. |
| `ADVICE_SNAPSHOT_EXPIRED` | Valid handle passed its fixed expiry, including after payload reclamation | Historical detail is no longer available; make a new analysis only as a new, explicitly requested snapshot. |
| `ADVICE_SNAPSHOT_MISMATCH` | Supplied snapshot, analysis or input digest differs; `error.field` names the mismatched field | Copy the identity values from the same retention result; do not combine records across analyses. |
| `INVALID_ADVICE_PAGE` | Page token, page size or bound options/position are invalid or changed | Reuse the returned token with the exact same handle, snapshot, collection, filter, page size and detail limits; otherwise restart that collection at its first page if the handle remains valid. |
| `UNKNOWN_ADVICE_ID` | Selected/filter ID or candidate is unknown or belongs to another collection | Use exact IDs from the retained record and the matching collection. |
| `DETAIL_TOO_LARGE` | Complete indivisible record/unit or explicit subset cannot fit | Increase detail `response_bytes` up to 16 MiB or request smaller pages/subsets. The tool withholds every record/anchor rather than returning snippets. |
| `INVALID_PARAMS` | Malformed selector/filter/limits, empty/duplicate/oversized ID list or unobserved typed reason filter | Correct the named request field; filters support only observed typed `id`, `reason` and `candidate_id` values. |
| `BUSY` | Shared analysis slot is occupied; detail/release/export started no work and was not queued | Wait for active work or cancellation to settle, then retry serially. |

Page tokens bind the analysis/handle, corpus snapshot, scope-input digest,
collection, filter, ordering, position, page size and effective detail limits.
Repeat every option unchanged with `selector.page_token`. A `reason` filter must
be an observed typed reason in that collection; unknown reasons refuse rather
than returning a misleading empty result. `scope_input_digest` is distinct from
`snapshot_id`: it also binds normalized scope, effective in-root ignore inputs,
and observed filesystem device/inode identities and full modes. The bounded manifest
permits at most 200,000 identity entries and 16 MiB aggregate accounting. Detail still
performs no live source, mode or ignore freshness check; export reobserves only
its captured evidence boundary and does not claim ignored/excluded-path coverage
or atomic application-time freshness.
Retention defaults are one active record, 134,217,728 aggregate accounted bytes
(128 MiB), and a fixed non-sliding 900-second lifetime from publication.
Preparations and in-flight readers occupy capacity; conservative reservation may
refuse before final stored charges would fit. The cap is not allocated upfront or
a repository-size, response-byte or process-RSS limit. Navigation does not extend
expiry, there is no silent eviction or disk persistence, restart loses records,
and explicit release removes only the process-local record.

`units` returns complete frozen original item and header anchors, but they are
historical (`historical:true`, `live_freshness:"not_checked"`), not checked-current
execution authority. For manual request construction, re-read and verify current
item/header bytes; `move_item` independently repeats its mandatory current-byte
and safety audits. A separate `export_move_request` can reobserve the captured
inputs and produce a complete inactive scaffold, but historical retrieval itself
still does not check freshness, select or submit units, refresh analysis, or
export a request.

### Inactive exact-anchor export recovery

Export is a separate call from historical detail. A usable success has
`schema_version:1`, `scaffold:true`, `submitted:false`,
`applicability:"not_assessed"`, `source_freshness:"checked_at_export"`,
`integrity.semantic:"not_performed"`, and a non-null ordinary strict `request`.
`provenance` and `review` are outside that request. Every export failure envelope
has `request:null`; never salvage a partial or preview request from `error`.

| Export error | Meaning and recovery |
|---|---|
| `ADVICE_SNAPSHOT_UNKNOWN` / `ADVICE_SNAPSHOT_EXPIRED` | Handle is unknown/released/from another process, or its fixed lifetime expired. Do not silently rerun; explicitly request a new retained analysis if still needed. |
| `ADVICE_SNAPSHOT_MISMATCH` | Supplied optional top-level `analysis_id` or `scope_input_digest`, or required `snapshot_id`, does not match the retained record. Copy identity values from one result; never combine analyses. |
| `UNKNOWN_ADVICE_ID` | The selected `item_id` is absent from this retained inventory. Use an exact inventory ID from the same record. |
| `INVALID_SCAFFOLD_SELECTION` | Empty/over-cap selection, invalid or duplicate unit reference, cross-analysis `unit_ref.analysis_id`, or overlapping whole-impl/member ranges. Correct the explicit selection; nothing is pruned automatically. |
| `UNSUPPORTED_SCAFFOLD_SELECTION` | Excluded/recovered/unsupported unit or missing complete frozen item/header provenance. Choose a supported inventory unit; snippets are not anchors. |
| `INVALID_DESTINATION` | Caller destination syntax, root-relative geometry, or required destination anchor is invalid. Correct it explicitly. `new_child` requires `{kind:"new_child",parent_path,path}`, with `parent_path` equal to the selected unit's source and `path` matching that parent's ordinary direct-child layout; export's check is syntax-only, not destination admission or applicability proof. |
| `SOURCE_CHANGED` | Captured source/corpus, normalized scope, effective ignore input, mode or observed filesystem identity changed or could not be completely reobserved. Request a fresh retained `suggest_split` analysis explicitly, then review and export again; `get_split_detail` remains historical. |
| `SCAFFOLD_TOO_LARGE` | The complete decoded request exceeds 8 MiB or the complete duplicated structured/text response cannot fit. Export never shortens anchors, emits partial JSON/snippets, or automatically splits the batch. The export `response_bytes` limit is 64 KiB–16 MiB; choose limits before retrying. |
| `INVALID_PARAMS` | Strict request object, required field, option or bound is invalid. Correct the named field; unknown fields are not accepted. |
| `CANCELLED` / `planning_deadline` | Cooperative work stopped before a request was published. Retry only if still requested. |
| `BUSY` | The shared analysis slot is occupied; this export started no work and was not queued. Wait for active work/cancellation to settle, then retry serially. |

The export request is explicit: each `selection` entry has an analysis-qualified
`unit_ref:{analysis_id,item_id}` and a caller-chosen destination. Optional top-level
identity/digest fields must match. No group/candidate ID is an execution selector,
and no companion, destination, assumption, acknowledgment, semantic option or
rewrite/trivia choice is added. `move_options` permits only `context`, ordinary
move `limits`, and `max_moves` (default 500, 1–5000); an omitted diagnostic count
stays omitted so ordinary preview defaults are preserved. Review and edit the
returned `request` separately, then submit it to ordinary `move_item`, which
performs every audit and can reject later stale anchors. `checked_at_export` is
an observational comparison at captured boundaries, not atomic application-time
freshness; ignored/hard-excluded paths are outside that evidence boundary. Export
uses the worker-owned shared admission permit through cancellation/shutdown: `BUSY`
starts no work and queues nothing, and cancellation does not free the slot until
admitted work settles.

## Private-child plans and schema-3 recovery

A caller explicitly selects `new_child`; export does not choose it, admit its
destination, run the planner or establish safety. The selected units must all
come from the caller's `parent_path`, which is the selected source file. Only the
evidenced ordinary direct-child path is supported: `foo.rs` → `foo/child.rs`,
`foo/mod.rs` → `foo/child.rs`, or a child beside an evidenced crate root. At most
one absent conventional directory may be required for the non-root flat-file
layout; higher ancestors must already exist and pass scope checks. Existing safe
directories are allowed. No arbitrary directory chain, new `mod.rs`, inline
parent, layout conversion or unrelated-source aggregation is supported. Existing
`new_sibling` admission remains strict.

Private-child plans synthesize a private ordinary `mod child;` declaration or
reuse one unique compatible private declaration under existing admitted-context
rules. Public/restricted, inline, remapped, ambiguous, unknown-cfg or unproved
declarations refuse. Do not widen the child for consumers outside its parent;
keep public facades in place and do not synthesize a facade/re-export.
Associated-item wrappers preserve supported full headers and all existing
proof/veto rules. Child geometry does not relax public-path, macro, trait, field,
constructor, concrete-type, receiver, context, scope, binding or visibility
refusals.

Branch on `schema_version` before interpreting any artifacts. Legacy-only typed
batches use schema 2 and the three artifact members `plan.edits`,
`plan.created_files` and `plan.patch`. Every successfully typed batch containing
a child, including mixed batches and planning/admission failures, uses schema 3,
selected before planning. Schema 3 requires all four artifact members: `edits`,
`created_files`, `directory_preconditions` and `patch`. Reject unknown/unsupported
versions before consuming artifacts; early untyped deserialization failures
retain the existing transport contract. Any unsuccessful schema-3 result sets
all four members to null together.

`directory_preconditions` disclose `id`, root-relative `path` (`.` for the
repository root), `observed_state` (`absent` or `existing_directory`),
`required_state` (`absent_then_directory` or `existing_directory`),
`basis:{kind,parent_path,crate_root,parent_module_segments}`, and
`dependent_created_file_ids`. The basis kind names the ordinary layout
(`flat_file_child`, `mod_rs_child`, `crate_root_child`, or `new_sibling` in a
mixed batch). Each created file's `directory_precondition_ids` links back; check
both directions. There is no directory mode, content, source range or
empty-directory Git artifact. Directory permissions belong to the caller.
`directory_diagnostics` contain `id`, `reason`, `path`, `parent_path`,
`expected_state` and `observed_state`, not a creation instruction or fabricated
byte anchor.

Use the exact directory refusal/race vocabulary in those records and linked blockers:

- `directory_context_unproved` marks a directory admission/context that could not be proved under the existing typed destination/scope rules.
- `directory_creation_unsupported_layout` identifies a missing higher ancestor; this layout is unsupported, not an invitation to plan an arbitrary directory chain.
- `CREATION_RACE` reports captured creation evidence that changed before publication. Its diagnostic `reason` distinguishes `directory_creation_race` (directory state/identity), `file_creation_race` (destination file or case alias appeared), and `competing_file_layout` (the competing ordinary module layout appeared). Read `path`, `parent_path`, `expected_state` and `observed_state`; do not invent source ranges or a directory-creation action.

A formerly absent directory appearing invalidates the plan even if it is now
empty and otherwise safe. JSON consumers must recheck source/parent bytes and
modes, file absence, the disclosed directory state and ordinary layout, ignore
inputs and competing layouts, then create only disclosed `absent_then_directory`
paths with caller-chosen permissions. Directory records do not expose filesystem
identities; the server rechecks its captured identities before publication. Git
patches contain the nested created file, not an empty-directory artifact;
`git apply --check` creates nothing and is not a freshness or atomicity guarantee.
On any race/change, stabilize, obtain fresh anchors and replan; never salvage an
artifact subset. The server never creates directories.

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
| `BUSY` | The shared analysis slot is occupied; the rejected call started no work and was not queued | Wait for the active call to finish or cancellation to settle, then retry serially. This also applies to export. No retry deadline or other request's identity is disclosed. |
| Cancellation | Work stopped | Only retry if still requested. |
| Limit/partial states | Membership/evidence/scan/output not complete | Inspect truncation+omissions; raise supported limits or reduce optional text/context — never exclude needed chain/binding evidence to force applicability. |

For `export_move_request`, `SOURCE_CHANGED` is an export error: the captured source, scope, ignore inputs, modes or observed filesystem identities changed or could not be completely reobserved; obtain a new retained analysis explicitly. In move planning, source/parent byte or mode changes and ignore-input changes remain `SOURCE_CHANGED`. `CREATION_RACE` and other move-planning freshness issues can appear as incompleteness reasons rather than error codes; `CRATE_IDENTITY_UNCERTAIN` typically surfaces as a module-context blocker. Stabilize the base, reread, and replan rather than treating old detail as fresh.

## Decision routing

Each decision carries `action.route`, `action.field`, `action.purpose`, and often `action.choices`. Follow them exactly:

- `resolve_decision` → replay the full published target via the named override (`rewrite_overrides` or `trivia_overrides`).
- `review_default` → a nonblocking default (usually trivia); adjust only with supported dispositions.
- `submit_for_analysis` → change `crate_root`, `paths`, or `moves` and rerun; applicability is not promised.
- `selection_change_required` → the selected items/destinations themselves must change.
- `unsupported_in_engine` → no override can resolve it; for blocking decisions, remove the affected items or handle separately. Nonblocking `post_move_import_review` advisories instead retain imports and require external compiler-backed cleanup after applying an otherwise applicable move. Never change selection or invent an override solely for these advisories.

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

Consumer-side named, unconditioned or declared-active `pub use`/`pub(crate) use` routes can be followed automatically to an admitted terminal declaration. Inspect `rewrites[].evidence` for `written_reexport`, every hop and terminal declaration in `anchors`, and the chosen canonical/public path in `after_text` (renames preserve the consumer binding). This is written syntax, not semantic proof. The chosen path's intermediate modules must be visible from the final destination: `pub`/`pub(crate)` permit crate access; private/`pub(self)` permit the declaring parent and its descendants; `pub(super)` permits that parent's parent and descendants; `pub(in ...)` must resolve through written `crate`/`self`/`super` to an ancestor containing the destination. An inaccessible or unprovable edge withholds the rewrite with `visibility_scope_unproved` and `inaccessible_route:<segment>` refusal basis (no invented range for a missing/synthesized edge). The engine first prefers an accessible canonical path, then searches admitted named public re-export routes to the same terminal. Fallback choice is fewest re-export hops, then lexicographic absolute path, disclosed in `rationale` as `written_reexport_route_fallback` with canonical rejection, chosen path, candidate count and hop count. Anchors retain original/chosen hops, terminal, rejected module and chosen module edges; hop targets must also be written-accessible from their exporting modules. Terminal attribute vetoes still apply, and no canonical module widening is offered. The anchored refusal remains only when no accessible admitted route exists. Inspect the named segment and anchor, then follow the decision route; do not guess an import or broaden visibility to force applicability. Globs, unproved/inactive attributed leaves (unknown `cfg`/`cfg_attr` included), competing leaves, cycles, routes exceeding eight hops and missing terminal declarations remain blocked by the written tier; the opt-in may provisionally preserve an existing accessible named public facade import only while retaining a both-overlay semantic identity need. Their written-binding/conditional-context reason is not an API override opportunity. Admit missing evidence only when the returned route permits scope correction, otherwise change selection or investigate separately. Moving the exposed declaration itself still requires the separate `public_path_change` API decision; no facade is synthesized.

`test_consumer_acknowledged` is a nonblocking disclosed decision, not a blocker or test result; inspect its anchors and any retained lexical witness, then run caller tests. A `statement_macro_before_reference` or `block_macro_may_introduce_items` acknowledgment accepts expansion risk without proving binding identity or repairing the consumer; other lexical ambiguity, written conflicts, stale anchors, and structural failures remain blocking. `standard_prelude`, `standard_prelude_constructor`, `standard_builtin_derive`, `ra_resolved`, `declarative_macro_identity`, and conditional `assumed_declared_identity` are proof classes, not refusal reasons; bare value-position `Some`/`None` constructor proofs assume identity with unshadowed `Option`, never qualified paths or patterns; their caller-assumption basis explicitly says derive-generated imports are not modeled; confirm their anchors and separate coverage counts. Resolution labeling is not compilation.

### Advice-completeness states (suggest_split)

`partition_outcome:"no_credible_written_partition"` is a completed negative result,
not a failure or a reason to force byte balancing. Review inventory/weak evidence
and choose a boundary externally if needed. `incomplete_analysis` cannot establish
that negative; inspect guards, omissions and coverage before retrying. Candidates
may still exist when ordinary layout cannot produce complete drafts. Companions and
impl/member alternatives are never automatic selections. Companion
`review_obligation:"association_unproved"` means supported written evidence cannot
establish the candidate-to-companion association; inspect its signal links rather
than guessing from consumer `classification`. Uncertain consumer attribution or
move exclusions alone do not erase a supported association or its obligation.
Candidate `candidate/N` and companion `companion/N` IDs are response-local only; execution still requires complete current anchors. Read the separate
`boundary_observations.coverage`; zero observed consumers is not proof of no others,
and uncertain routes cannot waive execution blockers or test acknowledgment rules.

**Complete duplicate-display fitting (full response).** When `status:"complete"` and
`counts.omissions.duplicate_declaration_display_spans` is nonzero, that count means
identical declaration display copies were suppressed, not that analysis is incomplete.
Only `item_size` and `name_prefix` signals qualify: if their `evidence` is empty,
resolve every existing `signal.item_ids` entry to `inventory[].span` in original ID
order. This tier is guarded by complete analysis and full same-response inventory,
with exact ordered ID-to-full-descriptor equality; missing IDs, unequal descriptors
or an incomplete inventory do not qualify. The empty array is not zero observed evidence,
and the reconstructed spans/IDs are not execution anchors. No retained handle or
follow-up call is needed. Do not generalize this to unique reference/consumer
occurrences, invent missing IDs/spans, or use it to label a genuine partial response
complete. Genuine `partial`/`response_bytes` recovery remains the response-budget
recovery described above.

| `draft_eligibility.state` / reason | Meaning | Recovery |
|---|---|---|
| `incomplete` + `response_bytes` | Drafts/decisions withheld at the response cap | Retry with `limits: {"response_bytes": 8388608}` (≤16 MiB). Membership may still be complete — the inventory is usable. |
| `no_draft` + `no_admitted_nonconflicting_name…` | Already modularized / too few movable units | Legitimate "nothing to do"; report the design, don't force a split. |
| `no_draft` + chain reasons | Root/scope problem | Fix `crate_root`/`paths` per chain diagnostics. |

`root: null` in the advice envelope means the same response-budget truncation — recover the budget before constructing anchors.

## Module-chain failures

Chain reasons appear under `chain_diagnostics`, including `macro_generated_module_tree` and `root_attribute_chain_uncertainty`; named refusals explain the cause but do not offer an override. A valid root can still hit unadmitted (`chain_file_unadmitted`), missing (`chain_file_missing`), conditional (`conditional_declaration`), path-attribute, competing (`competing_declarations`/`competing_file_layout`), or inline (`inline_module_layout`) hops. Follow the diagnostic's named failing hop; `relation` distinguishes `direct` observations from mere `possible_ancestor` obstructions. Workspace members need their Git-root-relative source root and all admitted hops, not the workspace manifest. Bare built-in `allow`/`warn`/`deny`/`forbid`/`doc` metadata and documentation-only `cfg_attr` payloads do not cause chain uncertainty, at crate roots or non-root module files. `doc = include_str!(...)` does not read or execute the expression. `no_std`, `no_core`, `macro_use`, `recursion_limit`, other feature gates and unknown/qualified providers retain exact attribute anchors and the inherited veto unless declared cfg proves their wrapper inactive. A matching `semantic_configuration` on `move_item`, without requiring semantic resolution, can admit ON cfg-gated edges from one matching root's explicitly listed positive atoms, but unknown atoms and known-OFF edges still refuse. `suggest_split` has no cfg opt-in; metadata chain admission never clears independent semantic/prelude/binding context vetoes.

## Rewrite overrides

Add to the **original request** (extra members, not a stand-alone call):

```json
{"target": { ...published target object, copied verbatim... }, "action": "accept_default"}
```

- `accept_default`: accept the proposed repair (no replacement text)
- `retain`: keep original bytes; blocks the batch if the repair was required
- `replace`: substitute supported same-target alternative text (≤64 KiB) — simple paths, private non-glob imports/aliases, private/`pub(self)`/`pub(super)`/`pub(in ...)`/`pub(crate)` visibility (ancestor and all-caller access rechecked), safe separators, unchanged ordinary module declaration

**Copy the published target object verbatim** — a source target contains a full anchor; a synthesis target contains `path`, `slot`, contributor `items`, `binding`, etc. Never reconstruct from display indices.

For a visibility choice, inspect `rewrites[].visibility`: original consumer anchors,
access reasons, original/final module regions, `preserved_access_region`,
`preserved_original_access_regions`, and `narrowest_covering_region`. Regions are
crate-relative segment arrays (`[]` means root). Empty consumers with **no new
observed caller** means original member access is being preserved, not a missing
caller to invent. The minimum remains unchanged by a broader accepted override;
`after_text` reports the chosen bytes. This evidence does not authorize repair of
unsupported fields, constructors, concrete types or uncertain receivers. Their
anchored refusals must follow the existing decision route.

## Trivia overrides

```json
{
  "trivia": {"path": "src/big.rs", "range": {...}, "expected_text": "// banner"},
  "disposition": "carry_with_item",
  "target_item": { ...full selected item anchor... }
}
```

Only ordinary ambiguous trivia (banners, blank-separated comments between units) can be re-dispositioned; `keep_in_place` forbids `target_item`. Protected/owned/internal trivia cannot be detached, retargeted, or discarded.
