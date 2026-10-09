use crate::{
    engine::Engine,
    move_plan::{MoveEnvelope, MoveRequest},
    plan::{PlanEnvelope, ReplaceRequest},
    result::{DomainError, Limits, PatternRequest, SearchEnvelope, SearchRequest},
    split::{SuggestSplitEnvelope, SuggestSplitRequest},
};
use rmcp::{
    RoleServer, ServerHandler,
    handler::server::{router::tool::ToolRouter, tool::ToolCallContext, wrapper::Parameters},
    model::{
        CallToolRequestParams, CallToolResponse, CallToolResult, Implementation, ListToolsResult,
        PaginatedRequestParams, ServerCapabilities, ServerConfig, Tool,
    },
    service::RequestContext,
    tool, tool_router,
};
use std::sync::{
    Arc, Mutex, Weak,
    atomic::{AtomicBool, Ordering},
};

#[derive(Clone)]
pub struct Server {
    router: ToolRouter<Self>,
    engine: Arc<Engine>,
    admission: Arc<tokio::sync::Semaphore>,
    active: Arc<Mutex<ActiveRequests>>,
}
#[derive(Default)]
struct ActiveRequests {
    cancelled: bool,
    flags: Vec<Weak<AtomicBool>>,
}
struct CancelOnDrop(Arc<AtomicBool>);
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}
fn busy_error() -> DomainError {
    DomainError::new(
        "BUSY",
        concat!(
            "the analysis slot is occupied; this rejected call started no analysis and was not queued; ",
            "wait for the active call to finish or cancellation to settle, then retry serially"
        ),
    )
}
fn wire(result: SearchEnvelope) -> CallToolResult {
    let failed = result.error.is_some();
    let value = serde_json::to_value(result).expect("serializable envelope");
    if failed {
        CallToolResult::structured_error(value)
    } else {
        CallToolResult::structured(value)
    }
}
impl Server {
    pub fn new() -> Result<Self, DomainError> {
        let launch = std::env::current_dir()
            .map_err(|_| DomainError::new("INTERNAL", "cannot capture launch directory"))?;
        Ok(Self {
            router: Self::tool_router(),
            engine: Arc::new(Engine::new(launch)?),
            admission: Arc::new(tokio::sync::Semaphore::new(1)),
            active: Arc::new(Mutex::new(ActiveRequests::default())),
        })
    }
    pub fn cancel_requests(&self) {
        let mut active = self.active.lock().expect("active lock");
        // Also cancel handlers already dispatched but not yet registered at EOF.
        active.cancelled = true;
        for flag in active.flags.iter().filter_map(Weak::upgrade) {
            flag.store(true, Ordering::Relaxed);
        }
    }
    async fn run_search(
        &self,
        request: SearchRequest,
        sugar: bool,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        let tool = if sugar { "search" } else { "search_query" };
        let failure = |limits, error| {
            let mut result = SearchEnvelope::failed(limits, None, error);
            result.tool = tool.into();
            wire(result)
        };
        let Ok(permit) = self.admission.clone().try_acquire_owned() else {
            return failure(request.limits, busy_error());
        };
        let flag = Arc::new(AtomicBool::new(false));
        let _guard = CancelOnDrop(flag.clone());
        {
            let mut active = self.active.lock().expect("active lock");
            flag.store(active.cancelled, Ordering::Relaxed);
            active.flags.retain(|weak| weak.strong_count() > 0);
            active.flags.push(Arc::downgrade(&flag));
        }
        let request_id: String = format!("{:?}", context.id).chars().take(128).collect();
        let span = tracing::info_span!("search_call", tool, request = %request_id, paths_count = request.paths.as_ref().map_or(0, Vec::len), globs_count = request.globs.as_ref().map_or(0, Vec::len));
        let engine = self.engine.clone();
        let limits = request.limits.clone();
        let worker_flag = flag.clone();
        let mut job = tokio::task::spawn_blocking(move || {
            let _entered = span.enter();
            let _permit = permit;
            engine.search_interpreted(request, sugar, &worker_flag)
        });
        let result = tokio::select! {
            result = &mut job => result,
            _ = context.ct.cancelled() => { flag.store(true,Ordering::Relaxed); job.await }
        };
        match result {
            Ok(result) => wire(result),
            Err(_) => failure(
                limits,
                DomainError::new("INTERNAL", "blocking engine task failed"),
            ),
        }
    }
    async fn run_replace(
        &self,
        request: ReplaceRequest,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        let failure = |limits, error| plan_wire(PlanEnvelope::failed(limits, error));
        let Ok(permit) = self.admission.clone().try_acquire_owned() else {
            return failure(request.limits, busy_error());
        };
        let flag = Arc::new(AtomicBool::new(false));
        let _guard = CancelOnDrop(flag.clone());
        {
            let mut active = self.active.lock().expect("active lock");
            flag.store(active.cancelled, Ordering::Relaxed);
            active.flags.retain(|weak| weak.strong_count() > 0);
            active.flags.push(Arc::downgrade(&flag));
        }
        let request_id: String = format!("{:?}", context.id).chars().take(128).collect();
        let span = tracing::info_span!("replace_call", tool="replace", request=%request_id, paths_count=request.paths.as_ref().map_or(0, Vec::len), globs_count=request.globs.as_ref().map_or(0, Vec::len));
        let engine = self.engine.clone();
        let limits = request.limits.clone();
        let worker_flag = flag.clone();
        let mut job = tokio::task::spawn_blocking(move || {
            let _entered = span.enter();
            let _permit = permit;
            engine.replace(request, &worker_flag)
        });
        let result = tokio::select! {
            result = &mut job => result,
            _ = context.ct.cancelled() => { flag.store(true, Ordering::Relaxed); job.await }
        };
        match result {
            Ok(result) => plan_wire(result),
            Err(_) => failure(
                limits,
                DomainError::new("INTERNAL", "blocking engine task failed"),
            ),
        }
    }
    async fn run_move(
        &self,
        request: MoveRequest,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        let failure = |limits, error| move_wire(MoveEnvelope::failed(limits, error));
        let Ok(permit) = self.admission.clone().try_acquire_owned() else {
            return failure(request.limits.into(), busy_error());
        };
        let flag = Arc::new(AtomicBool::new(false));
        let _guard = CancelOnDrop(flag.clone());
        {
            let mut active = self.active.lock().expect("active lock");
            flag.store(active.cancelled, Ordering::Relaxed);
            active.flags.retain(|weak| weak.strong_count() > 0);
            active.flags.push(Arc::downgrade(&flag));
        }
        let engine = self.engine.clone();
        let limits = request.limits.clone().into();
        let worker_flag = flag.clone();
        let mut job = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            engine.move_item(request, &worker_flag)
        });
        let result = tokio::select! {
            result = &mut job => result,
            _ = context.ct.cancelled() => { flag.store(true, Ordering::Relaxed); job.await }
        };
        match result {
            Ok(result) => move_wire(result),
            Err(_) => failure(
                limits,
                DomainError::new("INTERNAL", "blocking engine task failed"),
            ),
        }
    }
    async fn run_suggest(
        &self,
        request: SuggestSplitRequest,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        let failure = |limits: crate::split::AdviceLimits, error| {
            suggest_wire(SuggestSplitEnvelope::failed(limits.into(), error))
        };
        let Ok(permit) = self.admission.clone().try_acquire_owned() else {
            return failure(request.limits, busy_error());
        };
        let flag = Arc::new(AtomicBool::new(false));
        let _guard = CancelOnDrop(flag.clone());
        {
            let mut active = self.active.lock().expect("active lock");
            flag.store(active.cancelled, Ordering::Relaxed);
            active.flags.retain(|weak| weak.strong_count() > 0);
            active.flags.push(Arc::downgrade(&flag));
        }
        let engine = self.engine.clone();
        let limits = request.limits.clone();
        let worker_flag = flag.clone();
        let mut job = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            engine.suggest_split(request, &worker_flag)
        });
        let result = tokio::select! {
            result = &mut job => result,
            _ = context.ct.cancelled() => { flag.store(true, Ordering::Relaxed); job.await }
        };
        match result {
            Ok(result) => suggest_wire(result),
            Err(_) => failure(
                limits,
                DomainError::new("INTERNAL", "blocking engine task failed"),
            ),
        }
    }
    pub async fn shutdown(&self) {
        self.cancel_requests();
        // The owned permit lives until all blocking workers and Git children have joined.
        let _permit = self.admission.acquire().await;
    }
}
fn plan_wire(result: PlanEnvelope) -> CallToolResult {
    let failed = result.error.is_some();
    let value = serde_json::to_value(result).expect("serializable plan");
    if failed {
        CallToolResult::structured_error(value)
    } else {
        CallToolResult::structured(value)
    }
}
fn move_wire(result: MoveEnvelope) -> CallToolResult {
    let failed = result.error.is_some();
    let value = serde_json::to_value(result).expect("serializable move");
    if failed {
        CallToolResult::structured_error(value)
    } else {
        CallToolResult::structured(value)
    }
}
fn suggest_wire(result: SuggestSplitEnvelope) -> CallToolResult {
    let failed = result.error.is_some();
    let value = serde_json::to_value(result).expect("serializable advice");
    if failed {
        CallToolResult::structured_error(value)
    } else {
        CallToolResult::structured(value)
    }
}
#[tool_router]
impl Server {
    #[tool(name = "suggest_split", description = r#"Suggest ways to organize one large Rust source file into smaller module files, without changing it.

Use for:
- Inventory functions, types, whole impl blocks and their direct written associated units in a large file; supported inherent methods/consts can enter draft sibling groups.
- Compare up to two draft partitions, with reasons, original-range sizes, explicit impl/member containment and cross-group overlap links. Affected sizes are labeled non_additive; exact-once item IDs are not a disjoint byte partition or generated module size estimate.
- Inspect ranked structural ownership_candidates separately from inspection companions and whole-impl/member alternatives; weak naming/section/attribute facts never join cores.
- Observe admitted-file incoming/outgoing written routes through separately scoped boundary_observations; not symbol resolution, repairability or a change to local forecasts.
- Disclose separate test_observations for admitted inline/out-of-line conditional test routes, CST field/path shapes and macro_token_candidate access shapes. Read route/attribute anchors, missing/unadmitted/competing/remapped limitations, unlinked roots, uncertainty and non-exclusive coupling labels. No Cargo alias inference, expansion, assertion equivalence or mandatory test relocation; zero records never means tests unaffected. Exact same-file inline acknowledgement eligibility and third-file glob blockers are unchanged.
- Choose items to submit later as an explicit move_item batch. Completed analysis with no supported core reports partition_outcome no_credible_written_partition, not a balanced primary; incomplete analysis cannot establish that negative.
Does NOT:
- Execute a split, extract inline-module bodies, or return an applicable move plan.
- Resolve a call graph or trait methods, perform semantic rename, expand macros, or evaluate cfg/types.
- Prove API safety or assign comment-banner ownership from adjacency.

Example arguments (repo_path '.' assumes the server was launched in the Git repository):
{"repo_path":".","crate_root":"src/lib.rs","source_path":"src/lib.rs","paths":["src"]}
repo_path: the Git repository directory; an absolute path is also accepted, and relative paths use the server's launch directory. An existing file/directory inside the worktree also locates its Git root.
crate_root: a Rust source file path like 'src/lib.rs' or 'src/main.rs' that establishes which module tree the tool analyzes; not a Cargo.toml. Use a Git-root-relative existing .rs file, not an inferred Cargo target.
source_path: the Git-root-relative existing .rs file to analyze, such as 'src/lib.rs' or 'src/large.rs'.
paths: optional scope narrowing to existing Git-root-relative files/directories; omit for repository-wide discovery. Filters also narrow module/dependency evidence, so include crate_root and necessary module-chain files. Workspace-member roots such as 'member/src/lib.rs' are supported when the required written module-chain files are admitted; workspace manifests are not loaded. paths/globs combine with discovery; empty arrays are invalid.

Workflow: Review, edit or ignore the draft outside the server. Join group item_ids to inventory entries, obtain complete current item bytes, then submit chosen items as move_item anchors and destinations in one batch. Each item anchor uses the item's path, span.range and full original text as expected_text; IDs and omitted display text are not execution anchors. For an associated unit, also supply moves[].enclosing_impl using its exact enclosing_impl.anchor header bytes ending immediately before '{'; do not select an enclosing impl together with its overlapping members. Advice does not guarantee that the chosen moves will be applicable. Advice remains syntactic-only; move_item supports a narrow written class plus explicit bounded resolution and caller-assumption tiers, never general idiomatic relocation or compilation/equivalence verification.

Safety: Read-only advice: no patch, edit, creation content, execution handle, stored plan or autonomous application. Syntax checking is input-only; semantic checking is not_performed. Inspect completeness, omissions and decisions before relying on a draft.

Risk signals: Every group has expected_to_block lower-bound observed decision counts for member calls, macros, external bindings, conditional/derive context and cfg(test) consumers (plus other_local), with decision IDs. Counts are local risk coverage, not a guaranteed final blocker count; move_item adds consumer/destination/batch/module-chain/trivia checks and may repair or deduplicate dependencies. assessment_scope is local_only with those other concerns explicitly not_assessed, even at zero counts: zero local risks never means a safe move. cfg_test_consumer signals link observed same-file test consumers via super:: paths/imports to affected inventory items; test_coupled groups warn that those consumers will block relocation. Advice itself adds no cfg evaluation or acknowledgment, and no planner runs per draft. An explicit move_item batch may opt into acknowledge_test_consumers; review that tool's disclosure contract.

Response detail: Candidate IDs use candidate/N; each companion has a unique response-local companion/N ID and a review_obligation (selection_completeness, boundary_dependency or association_unproved), separate from observed-consumer classification. These are review distinctions, not automatic selection or access/repairability proof. Schema version 2 encodes decision_groups[].decision_ids as exact {first_id,count} runs, with one routing summary and consequence per cause/route/consequence group. Default decisions contain one full anchored exemplar per group, bounded by limits.diagnostic_count (default 64); counts and draft risk links cover omitted details through these runs. Explicit limits.diagnostic_count (0–100000) returns the first N full decisions with unchanged fields; raise response_bytes too when expanding large files. Omissions are counted; capping decision detail alone does not withhold full drafts or mark analysis incomplete.

Advanced details: Inventories every written top-level unit, including anonymous impls and context-sensitive constructs, plus direct written impl members. Associated entries remain context_sensitive: only members with enclosing_impl present and empty reasons can enter sibling groups; excluded members and their overlapping enclosing impls stay retained. counts.eligible_items includes these non-excluded members, so filtering only supported_unit misses draftable units. Draft eligibility is advice, not move applicability. Source-linked heuristic prefixes, reference candidates, sections and sizes explain partitions, or an honest no-draft/incomplete result. Banner adjacency never assigns ownership. max_items defaults to 500 (1–5000), bounds displayed membership rather than an execution selection, and exceeded limits withhold complete drafts. Optional context/limits objects use defaults for omitted settings; there is no cursor. docs/tools.md is the authoritative detailed contract for inventory/member labels, decision action routes, chain diagnostics and edited-batch execution."#, output_schema = rmcp::handler::server::tool::schema_for_output::<SuggestSplitEnvelope>(), annotations(read_only_hint = true, destructive_hint = false, open_world_hint = false))]
    async fn suggest_split(
        &self,
        Parameters(request): Parameters<SuggestSplitRequest>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        self.run_suggest(request, context).await
    }
    #[tool(name = "move_item", description = r#"Plan one batch of whole written top-level Rust items or supported whole inherent methods/consts between files, without changing the repository.

Use for:
- Move selected functions, types, whole impls or supported whole inherent methods/consts into module files. Associated moves require an exact header-only enclosing_impl anchor; existing_impl destinations require a whole destination implementation anchor and identical same-type headers. Unchanged generic/where impl headers are copied verbatim; no trait, unsafe, conditional, generic-member or generated-member admission.
- Plan an absent new sibling .rs module file for selected items.
- Turn reviewed suggest_split groups into explicit moves with import/path repairs.
Does NOT:
- Extract an inline 'mod name { ... }' body into a file or create an inline-module destination.
- Move partial items/members, create directories, or reorder items within one file.
- Perform semantic rename, trait-method resolution, arbitrary macro expansion, or inferred Cargo target discovery; the bounded declarative identity exception is explicitly scoped below.
- Repair changed public API exposure or synthesize a facade, compatibility shim or new public re-export; retained public_path_change needs still block the batch.

Example arguments (repo_path '.' assumes launch in the Git repository; src/lib.rs starts with exactly 'fn helper() {}\n', and src/helpers.rs is absent):
{"repo_path":".","crate_root":"src/lib.rs","paths":["src"],"moves":[{"item":{"path":"src/lib.rs","range":{"start_byte":0,"end_byte":14},"expected_text":"fn helper() {}"},"destination":{"kind":"new_sibling","path":"src/helpers.rs","parent_path":"src/lib.rs"}}]}
repo_path: the Git repository directory; absolute paths are accepted, relative paths use the server's launch directory, and an existing file/directory inside the worktree also locates its Git root.
crate_root: a Rust source file path like 'src/lib.rs' or 'src/main.rs' that establishes which module tree the tool analyzes; not a Cargo.toml. Use a Git-root-relative existing .rs file. The server does not infer a root from Cargo. Workspace-member roots such as 'member/src/lib.rs' are supported with admitted written chains; workspace manifests are not loaded.
paths: optional scope narrowing to existing Git-root-relative files/directories; omit for repository-wide discovery. Include the root, sources, destination/parent and module-chain files needed as evidence. paths/globs restrict the analysis corpus; empty arrays are invalid. For new files, include their existing directory, not the absent filename.

Workflow: Review a suggest_split draft or find complete top-level items with search_query, e.g. '(source_file (function_item) @match)'. Obtain full current bytes, then supply each item's path, original [start_byte,end_byte) range and exact expected_text in moves[].item, plus an explicit destination. For a supported inherent method/const, also supply moves[].enclosing_impl as the exact header-only anchor ending immediately before '{'; inventory entries expose enclosing_impl.anchor. Do not select overlapping whole impls and members. Do not use expression anchors, truncated text or split-group IDs. Submit all intended moves together. Review every rewrite and decision, including pub(super), pub(in crate::path) and necessary pub(crate) access repairs chosen for the narrowest evidenced ancestor covering preserved access and all proven final callers; recheck unchanged base bytes and creation-path absence before external application.

Safety: Read-only simultaneous plan; required unsupported or uncertain dependencies block the entire batch. Blocked previews are not applicable artifacts. Edits, creations and Git patch are published together or withheld; require plan.applicable:true. Default semantic checking is not_performed; opt-in bounded resolution is separately labeled, never compilation/equivalence. The server never writes, formats, compiles or applies changes. The default syntactic stage delivers design advice plus proven-complete patches for a narrow class; general idiomatic Rust relocation is not guaranteed.

Prelude option: assume_standard_prelude defaults to false (unchanged written-only analysis). Opt in per batch to assume unshadowed type-position Option, Result, Box, Vec and String from the edition-independent standard prelude, plus the same five names in qualified type position (std::option::Option, std::result::Result, std::boxed::Box, std::vec::Vec, std::string::String — the qualified form bypasses terminal-name import shadowing but still audits the std root and original/final context), plus bare value-position Some/None constructors (identity assumed with unshadowed Option; the constructor spelling must also be unshadowed; qualified constructors, constructor patterns, associated calls, methods and arbitrary macro expansion are not covered), plus bare compiler-built-in derives Debug/Clone/Copy/PartialEq/Eq/PartialOrd/Ord/Hash/Default. Each discharged type occurrence has plan.binding_proofs class standard_prelude; bare value-position constructors use class standard_prelude_constructor; each discharged derive name has class standard_builtin_derive. Both carry full source anchor, item_ids, destination_path, standard_path and caller-assumption basis; coverage.standard_prelude and coverage.standard_builtin_derive separately count occurrences, including omitted records. No import is synthesized. Source and destination/batch visible chains must be complete; competing same-spelling macro_rules/items/use-leaves/aliases or visible globs refuse derives. Path-form derives never qualify; mixed lists discharge only unshadowed built-ins and keep every other name's veto (Debug, Args still blocks). Serde/expect and other unexamined attributes, prelude-disabling attributes, conditional/recovered or macro context still veto. Scoped child/sibling declarations do not shadow parent references. Proven written bindings still require completed existing repairs. Constructors, associated calls, methods and arbitrary macro expansion are not covered; other blockers still withhold the entire batch. No Cargo.toml read or semantic verification is performed.

Test-consumer option: acknowledge_test_consumers defaults to false (flag-off response bytes unchanged). Opt in to disclose residual consumers inside the moved items' own file's directly inventoried exact #[cfg(test)] inline module as nonblocking test_consumer_acknowledged decisions. Super-path/import/glob consumers include macro-argument candidates such as assert!/assert_eq!; no expansion, rewrite or test validation is performed. Every acknowledged decision retains its full anchor and item links, coverage.test_consumers_acknowledged counts them, and plan.test_consumer_disclosure names the caller's test run as the validator. Acknowledged risks remain visible even in capped blocked previews (response-byte fitting still counts omissions and withholds artifacts). This acknowledgment alone does not discharge other conditional contexts (including cfg(feature)/cfg(unix)); separately supplied declared configuration may admit supported written cfg contexts. Unresolved nested-module, outside-test macro, binding, member/constructor, selected-item and module/root-chain needs still block. This flag does not change integrity.semantic or promise compilation/equivalence.

Written configuration option: semantic_configuration also supplies positive cfg evidence to written module-chain, inline-module, import, required-binding, selected-item, lexical and standard-prelude context audits with resolve_semantic:false. Exactly one valid configured entry must match crate_root. Listed features or exact cfg atoms are ON; unlisted atoms are unknown, not OFF. all/any/not evaluate every operand, including unknown atoms behind a determining operand; supported true/false and nested cfg_attr follow the same evaluator. Selected items, required bindings and module edges must remain active. Known-OFF retained contexts do not erase written shadow names. Unknown/malformed predicates or arbitrary providers retain anchored refusals. This is written evidence, not RA resolution or compilation: integrity.semantic remains not_performed, with no synthetic RA proofs or resolution_coverage. Module assumptions and refusal consequences disclose consulted configuration; independent macro/access/API/repair vetoes remain. Advice has no configuration opt-in.
Resolution option: resolve_semantic defaults to false (no RA work on the flag-off path; a separately supplied semantic_configuration can still affect written audits). Include semantic_configuration:{crates:[{name,root_file,edition,features:[],cfg:[],dependencies:[]}]} to query pinned pure rust-analyzer databases over admitted immutable texts and the assembler's actual final virtual batch. Roots must be admitted, unique, and include crate_root; dependency edges are {name,crate_name} naming another explicit root (max 32 crates, acyclic). cfg atoms are {key,value:null|string}; features set feature cfg values. Selected top-level attached attributes reach both-overlay context_attribute checks at exact attribute anchors; declared feature atoms and all/any/not predicates can clear only that context need. Unknown atoms and inactive selected items/required bindings retain anchored refusals; independent prelude/lexical, macro, inline-module, access, API and repair vetoes remain. No Cargo/project discovery, implicit sysroot, caller build scripts, generated environment, VFS watchers or proc-macro execution. Concrete known original/adjusted receivers, true inherent resolved declarations, accessible fields/constructors and external written bindings qualify only when stable anchored identity and positive access survive at both ends. Generic/trait-object/trait-impl/unknown/generated-member context retains existing typed blockers, never candidate-callback proof. plan.binding_proofs class ra_resolved records receivers, original/final declaration anchors and crate origins, inherent/field/type classification, both-end access and explicit config/snapshot/analyzer/overlay digests. coverage.ra_resolved separately counts occurrences. Queries use integrity.semantic:resolution_performed and plan.resolution_coverage.statement: resolution performed for N decisions under one explicit configuration; compilation/equivalence not performed. Missing config/evidence refuses. Existing chain, macro, cfg(test), API and trivia vetoes remain; ordinary proc-macro implementation bodies are not refused by crate kind. Cancellation observes the existing flag/deadline and retains BUSY ownership until database work/controller join. No hard cancellation latency or compilation/equivalence promise.

Declarative identity option: the same resolve_semantic flag admits type-position paths from in-crate written macro_rules definitions and invocations in admitted files, using pinned RA token substitution with no code execution. Budget: one ident argument, one arm ($name:ident), one expansion step, 4096 significant definition/output tokens, nesting 32. Conditional definitions/invocations/enclosing modules veto even cfg ON; repetitions, recursive/nested calls, other fragments, external/generated definitions and builtins refuse with anchored declarative_* context reasons. Proc macros never qualify or execute. Struct/enum names must map exactly to the written argument; no fallback/literal-name identity. Output cannot inject imports/modules/other namespace-producing items; generated impls/methods/fields/constructors/variants remain unproved and unselectable. Default generated type attributes admit only inert lint/doc metadata and unshadowed bare compiler-built-in derives; helper-shaped attributes and custom/qualified derives refuse as provider-uncertain. Proof class declarative_macro_identity, classification declaration_identity, null receivers, bounded written-token basis, and declaration.declarative_macro:{invocation,definition} anchors disclose original/final pairs; name and both whole-token anchors must origin-normalize to identical bytes. Unconditional counts stay in coverage.ra_resolved. Opt-in assume_declared_helpers defaults to false and requires resolve_semantic plus semantic_configuration: the caller asserts each provider-uncertain written or generated struct/enum attribute belongs to a registered derive helper, rather than a replacing attribute provider. Companion derive path lists are checked syntactically only. Conditional identities use distinct class assumed_declared_identity and coverage.assumed_declared_identity, never coverage.ra_resolved; identity is assumed with the type, not engine-classified, with no procedural expansion, hygiene or compilation claims. Nominal facts depending on that conditional namespace admission are also labeled assumed. Both-overlay anchors/access, cfg/cfg_attr, token/nesting/shape caps, proc-macro and generated-item vetoes remain; unknown attributes on non-type written owners and helper-free generated custom/qualified derives still refuse. Written helper admission is nominal only. Existing accessible written public facade imports can be carried provisionally for bare signature types; their semantic needs remain until both-overlay identity passes and no generated canonical terminal is guessed. Independent chain/API/trivia/repair blockers remain.

Response/refusal details: Schema version 2 decision_groups encode exact {first_id,count} ID runs, with complete group routing and unresolved_consequence summaries. Blocked previews default to one decision exemplar per group (up to 64) and four moves/rewrites/origins each; explicit limits.diagnostic_count (0–100000) expands each array, subject to response_bytes. Mandatory acknowledged test risks remain separately visible. include_unselected_trivia defaults to false and affects blocked-preview display only. Applicable plans retain the complete audit. Inspect counts.omissions, chain_diagnostics, decisions[].reason, action.route and refusal_basis:[{class,anchor:{path,range?},name?}]. Retained needs disclose written_binding_unproved, conditional_context, glob_import, inaccessible_route:<segment> or the reached semantic proof-stage refusal; unresolved/synthesized witnesses omit range rather than invent coordinates. glob_consumer_unrepaired is a decision reason for reachable or unproved third-file wildcard consumers, with glob_import witnesses and selection_change_required routing, not a proof of glob binding identity. Neither test-consumer acknowledgment nor semantic resolution waives these consumers. Keep the item in place or change the selected move; never narrow away admitted consumers merely to obtain applicability. Disclosure and zero proofs do not relax a veto or authorize an unsupported override.
Advanced details: Unique ordinary module and written-binding evidence supports itemized import/path repairs and narrow ancestor visibility repairs, including necessary pub(crate) access repairs only when the root is the smallest covering region; bounded opt-in semantic discharge supplies proofs only, never another assembler. Consumer-side named pub use/pub(crate) use routes can reach directly inventoried written declarations through at most eight unique, active re-export hops, rejecting cycles/globs/ambiguity. Canonical paths are preferred when every module edge is accessible; otherwise an admitted accessible public route is selected by fewest hops then lexicographic absolute path, without widening hidden canonical modules. Ordinary rewrite kinds retain written_reexport evidence and original hop/terminal anchors; fallback rationale uses written_reexport_route_fallback, not a new semantic binding-proof class. Terminal attribute/constructor/access and moved-item API vetoes remain. Destinations are existing files or absent literal sibling .rs files in existing directories; all item/destination/parent paths are Git-root-relative. Existing destinations append by default, or accept a full before_item anchor. Parent module declarations start as private 'mod name;' declarations or are reused; any necessary ancestor-region visibility repair is separately audited. New declarations follow the last sibling file-module declaration, else precede the first cfg(test) item's attached trivia, else append at EOF. Imports follow the last attached whole use (or precede the first attached item), preserving existing use-group adjacency. Created files get an audited trailing newline. Owned trivia travels; file/module prologues and ambiguous banners stay by default. Oversized pure interior removal gaps bounded by retained syntax on both sides collapse to one blank line by default in multi-item batches, through anchored removal_gap rewrites; BOF/EOF gaps default to keep_in_place with explicit collapse/retain still available. Single-item serialized plans remain byte-identical to their pre-batch-collapse shape, including decision strings and explicit-collapse audit evidence/rationale. Nonblocking removal_gap decisions expose exact residual before_text, candidate after_text and keep_in_place/collapse default and selected dispositions. Replay action.target in rewrite_overrides with action retain to keep original gap bytes, or action replace and replacement_text equal to removal_gap.after_text to collapse; accept_default/omission follow the disclosed default. Single/double newlines are unchanged. Only pure removal-boundary blank-line runs are offered; overlapping repairs, shared payload-insertion boundaries, retained trivia and unrelated gaps are not normalized. A separate repair may start at a gap's end; start-boundary edits stay excluded. Replay supported alternatives using the published full target in rewrite_overrides or exact trivia anchors in trivia_overrides, never display IDs. max_moves defaults to 500 (1–5000); optional context/limits use defaults for omitted settings. No formatting or source-byte normalization occurs. Nonblocking post_move_import_review decisions flag zero written-name references in surviving private source imports and disclose unenumerated globs; imports are retained, no cleanup override is offered, and token absence does not prove semantic unusedness or a warning-free build. Review imports with the caller's compiler after external application. A new sibling directly under tests/ may create a Cargo integration-test crate; review target discovery externally. docs/tools.md is the authoritative detailed contract for request flags, proof/refusal classes, written routes, access choices, response omissions and replay."#, output_schema = rmcp::handler::server::tool::schema_for_output::<MoveEnvelope>(), annotations(read_only_hint = true, destructive_hint = false, open_world_hint = false))]
    async fn move_item(
        &self,
        Parameters(request): Parameters<MoveRequest>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        self.run_move(request, context).await
    }
    #[tool(name = "replace", description = r#"Produce a dry-run patch that replaces selected Rust expressions, never changing files in place.

Use for:
- Plan a rewrite from '$receiver.unwrap()' to '$receiver.expect("reason")'.
- Change a written one-argument call shape from '$receiver.old($arg)' to '$receiver.new($arg)'.
- Replace only the matches you chose from search results.
Does NOT:
- Perform semantic rename, rename declarations, or distinguish receiver types and trait methods.
- Author generic calls, blocks/control flow, closures, structs, casts or macros as patterns/templates; search_query finds broader syntax but does not extend replacement grammar.
- Format code, add parentheses automatically, expand macros, or verify compilation/semantic equivalence.

Example arguments (repo_path '.' assumes launch in the Git repository; omitted selection selects all scope matches):
{"repo_path":".","pattern":"$receiver.unwrap()","replacement":"$receiver.expect(\"reason\")","paths":["src"]}
repo_path: the Git repository directory; absolute paths are accepted, relative paths use the server's launch directory, and an existing file/directory inside the worktree also locates its Git root.
paths: optional scope narrowing to existing Git-root-relative files/directories; omit for repository-wide discovery. paths/globs combine with discovery; empty arrays are invalid. No crate_root is needed.

Workflow: First call search to find matches, then supply selected matches as replace's selection parameter. For each selected match, copy {path: match.path, range: match.span.range, expected_text: match.span.text}; obtain full original bytes first if text was omitted. Omitted or null selection means all matches, [] means none; explicit anchors must be unique and byte-exact. A search_query result is usable only if this pattern grammar can express the same expression. Require plan.applicable:true, review all artifacts/decisions, and verify unchanged base bytes before applying with another tool.

Safety: Read-only, all-or-nothing planning. Unretained trivia, conflicts, syntax recovery, source changes and exceeded limits withhold ALL artifacts; blocked/incomplete previews must not be applied. Applicable plans include a complete Git patch (three context lines) and original-byte JSON edits; the server never writes, formats, compiles or applies them. Integrity is Tree-sitter syntax only; semantic checking is not_performed.

Advanced details: Same expression-pattern grammar as search: one expression without a semicolon, concrete method/field/path names and exact argument arity. Bound $name copies exact original capture bytes; no automatic parentheses, dedent or formatting. Dollars in literals/comments are literal, not interpolation; no $$ or ${name}. Trivia stays in place by default; review anchored trivia_overrides when needed. max_matches defaults to 500 (1–5000) and caps selected matches; it does not count unselected scope matches. The former pre-selection scope-match gate is removed: an explicit selection within the cap is admitted regardless of scope-match totals, subject to existing work limits and safety checks. Omitted selection selects all scope matches and is still capped. Admitted selections report exact scope totals after complete bounded matching; early scan stops report observed counts with unknown totals. Optional context/limits use defaults for omitted settings. No cursor; narrow paths/globs to reduce work. docs/tools.md is the authoritative detailed contract for syntax, scope, limits, omissions and caller review."#, output_schema = rmcp::handler::server::tool::schema_for_output::<PlanEnvelope>(), annotations(read_only_hint = true, destructive_hint = false, open_world_hint = false))]
    async fn replace(
        &self,
        Parameters(request): Parameters<ReplaceRequest>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        self.run_replace(request, context).await
    }
    #[tool(name = "search_query", description = r#"Find Rust syntax that simple search expression patterns cannot describe, including whole items, generic calls and control flow.

Use for:
- Find written function items with '(function_item) @match'.
- Find generic calls such as 'collect::<Vec<_>>()' or control-flow nodes using a Rust Tree-sitter query.
- Obtain whole top-level item ranges for move_item, e.g. '(source_file (function_item) @match)', or direct written impl methods/consts with the additional enclosing_impl anchor required by move_item.
Does NOT:
- Resolve types or trait-method identity, perform semantic rename, expand macros, or evaluate cfg.
- Accept search-style '$name' expression patterns; use search for simple expressions like '$receiver.unwrap()'.
- Turn arbitrary query matches into valid replace templates or move_item anchors.

Example arguments (repo_path '.' assumes the server was launched in the Git repository):
{"repo_path":".","query":"(function_item) @match","paths":["src"],"page_size":100}
repo_path: the Git repository directory; an absolute path is also accepted, and relative paths use the server's launch directory. An existing file/directory inside the worktree also locates its Git root.
paths: optional scope narrowing to existing Git-root-relative files/directories; omit for repository-wide discovery. paths/globs combine with discovery; empty arrays are invalid. No crate_root is needed.

Workflow: Inspect matches, coverage, skipped files and omissions. To continue a page, send next_cursor as cursor with every other argument unchanged; cursors expire after 15 minutes in this server process. For replacement, the pattern must still fit replace's narrower expression grammar. For a move, obtain full current bytes and anchor a complete top-level item, or a supported whole inherent method/const with its exact header-only enclosing_impl anchor ending immediately before '{'; suggest_split inventory exposes that context. A query match alone does not establish eligibility. docs/tools.md is the authoritative detailed contract for query syntax and move handoffs.

Safety: Read-only written-syntax search, with no compilation or semantic checking. Limits may omit field text or return partial pages. An empty search does not prove absence in generated code.

Advanced details: query is a Rust Tree-sitter query (at most 64 KiB, 64 patterns and 64 capture names). Each top-level pattern must be rooted with exactly one @match capture; capture the complete node to return. Other captures are arrays; reusing a name does not require equal source text. Supported text predicates: #eq?, #not-eq?, #any-eq?, #any-not-eq?, #match?, #not-match?, #any-match?, #any-not-match?, #any-of?, #not-any-of?. The only custom predicate is #rust-arity? @arguments "N": capture the arguments node once, with a quoted decimal u32 count. No property predicates or directives. Original half-open byte ranges, 1-based lines and 0-based UTF-8 byte columns; context is exact complete source lines. page_size defaults to 100 (1–1000); optional context/limits use defaults for omitted settings."#, output_schema = rmcp::handler::server::tool::schema_for_output::<SearchEnvelope>(), annotations(read_only_hint = true, destructive_hint = false, open_world_hint = false))]
    async fn search_query(
        &self,
        Parameters(request): Parameters<SearchRequest>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        self.run_search(request, false, context).await
    }
    #[tool(name = "search", description = r#"Find Rust expressions by their written call or expression shape, with named placeholders for parts you want to capture.

Use for:
- Find zero-argument '.unwrap()' calls with '$receiver.unwrap()'.
- Find one-argument '.expect(...)' calls with '$receiver.expect($reason)'.
- Find repeated byte-identical arguments with 'pair($value,$value)'.
Does NOT:
- Resolve types or trait methods, perform semantic rename, expand macros, or evaluate cfg.
- Author generic calls like 'collect::<Vec<_>>()', blocks/control flow, closures, structs, casts or macros in a pattern; use search_query instead.
- Capture a method/field/path name or a variable-length argument list; names are concrete and argument arity is exact.

Example arguments (repo_path '.' assumes the server was launched in the Git repository):
{"repo_path":".","pattern":"$receiver.unwrap()","paths":["src"],"page_size":100}
repo_path: the Git repository directory; an absolute path is also accepted, and relative paths use the server's launch directory. An existing file/directory inside the worktree also locates its Git root.
paths: optional scope narrowing to existing Git-root-relative files/directories; omit for repository-wide discovery. paths/globs combine with discovery; empty arrays are invalid. No crate_root is needed.

Workflow: Inspect matches, coverage, skipped files and omitted text before claiming completeness. While next_cursor is present, send it as cursor with every other argument unchanged; cursors expire after 15 minutes in this server process. First call search to find matches, then supply selected matches as replace's selection parameter using each match's path, span.range and complete span.text as expected_text. Obtain full original bytes first if text was omitted. Search does not edit files.

Safety: Primary read-only written-syntax search; no compilation or semantic checking. Limits can omit field text or return partial pages. Empty results do not prove absence in generated code.

Advanced details: pattern is one expression without a semicolon; no sequences, $$ escape or @capture annotation. $name binds one expression node, even a whole source block/closure/generic call/macro, never a list. Placeholders fit expression operands, callee/receiver and each argument; field/method/path names stay concrete. Names are ASCII identifiers excluding __ssr_. Repeated names require byte-identical source; distinct names bind independently. Concrete leaves/punctuation and exact arity must match; comment extras and whitespace gaps are ignored. Dollars inside literals/comments are literal. Supported pattern forms: $name, identifiers/self/scoped paths, literals (including byte/C prefixes), calls/fields/tuple fields, parentheses/unit/tuple/arrays/repeat arrays, unary/reference/try/await/index, binary/assignment/compound-assignment/range. Original half-open byte ranges, 1-based lines and 0-based UTF-8 byte columns. Context is exact complete source lines; limits/cursors share search_query behavior. page_size defaults to 100 (1–1000); optional context/limits use defaults for omitted settings. docs/tools.md is the authoritative detailed contract for grammar, scope, limits, omissions and caller review."#, output_schema = rmcp::handler::server::tool::schema_for_output::<SearchEnvelope>(), annotations(read_only_hint = true, destructive_hint = false, open_world_hint = false))]
    async fn search(
        &self,
        Parameters(request): Parameters<PatternRequest>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        self.run_search(request.into(), true, context).await
    }
}
impl ServerHandler for Server {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("rust-sitter-mcp", env!("VERSION_FULL")))
    }
    async fn list_tools(
        &self,
        _: Option<PaginatedRequestParams>,
        _: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, rmcp::ErrorData> {
        Ok(ListToolsResult::with_all_items(self.router.list_all()))
    }
    fn get_tool(&self, name: &str) -> Option<Tool> {
        self.router
            .list_all()
            .into_iter()
            .find(|tool| tool.name == name)
    }
    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, rmcp::ErrorData> {
        if request.name != "search_query"
            && request.name != "search"
            && request.name != "replace"
            && request.name != "move_item"
            && request.name != "suggest_split"
        {
            return Err(rmcp::ErrorData::method_not_found::<
                rmcp::model::CallToolRequestMethod,
            >());
        }
        let args = serde_json::Value::Object(request.arguments.clone().unwrap_or_default());
        let decoded = if serde_json::to_vec(&args).expect("JSON args").len() > 8 * 1024 * 1024 {
            Err("decoded arguments exceed 8 MiB".into())
        } else {
            if request.name == "suggest_split" {
                serde_json::from_value::<SuggestSplitRequest>(args)
                    .map(|_| ())
                    .map_err(|e| e.to_string())
            } else if request.name == "move_item" {
                serde_json::from_value::<MoveRequest>(args)
                    .map(|_| ())
                    .map_err(|e| e.to_string())
            } else if request.name == "replace" {
                serde_json::from_value::<ReplaceRequest>(args)
                    .map(|_| ())
                    .map_err(|e| e.to_string())
            } else if request.name == "search" {
                serde_json::from_value::<PatternRequest>(args)
                    .map(|_| ())
                    .map_err(|e| e.to_string())
            } else {
                serde_json::from_value::<SearchRequest>(args)
                    .map(|_| ())
                    .map_err(|e| e.to_string())
            }
        };
        if let Err(mut message) = decoded {
            if message.starts_with("invalid type: string ")
                && (message.contains("expected struct ")
                    || message.contains("expected a map")
                    || message.contains("expected internally tagged enum ")
                    || message.starts_with("invalid type: string \"null\","))
            {
                // Leave room for the hint inside DomainError's bounded message.
                message = message.chars().take(800).collect();
                message.push_str(" (a client may have stringified an object parameter; omit optional object parameters instead of passing null)");
            }
            if request.name == "suggest_split" {
                return Ok(suggest_wire(SuggestSplitEnvelope::failed(
                    Limits::default(),
                    DomainError::new("INVALID_PARAMS", message),
                ))
                .into());
            }
            if request.name == "move_item" {
                return Ok(move_wire(MoveEnvelope::failed(
                    Limits::default(),
                    DomainError::new("INVALID_PARAMS", message),
                ))
                .into());
            }
            if request.name == "replace" {
                return Ok(plan_wire(PlanEnvelope::failed(
                    Limits::default(),
                    DomainError::new("INVALID_PARAMS", message),
                ))
                .into());
            }
            let mut failed = SearchEnvelope::failed(
                Limits::default(),
                None,
                DomainError::new("INVALID_PARAMS", message),
            );
            failed.tool = request.name.to_string();
            return Ok(wire(failed).into());
        }
        self.router
            .call(ToolCallContext::new(self, request, context))
            .await
    }
}
