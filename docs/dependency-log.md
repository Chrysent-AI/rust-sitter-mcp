# Dependency decisions

## 2026-10-06 — ra_ap_hir 0.0.357

HIR is the chosen structured method/type/visibility boundary; the shared comparison and exact coordinated pin rationale follow below.

## 2026-10-06 — ra_ap_base_db 0.0.357

Explicit crate graph and immutable source-root/text input APIs, directly used instead of a Cargo loader. Same maintenance, license, MSRV, feature and coordinated-upgrade decision as HIR below.

## 2026-10-06 — ra_ap_syntax 0.0.357

Byte-addressed ASTs for admitted occurrences and declaration anchors. Using the aligned parser avoids a cross-version AST contract; same vetted release and default-off features as HIR below.

## 2026-10-06 — ra_ap_ide_db 0.0.357

Structured, in-process resolution over application-admitted immutable texts and an explicit crate graph. Each directly used crate is pinned to `=0.0.357`, with `default-features = false` and no `in-rust-tree`. `cargo +1.98.1 info` verified each package's MIT OR Apache-2.0 license and Rust 1.98 minimum before manifest edits; the tested toolchain is 1.98.1. Same-day registry research records the October 5 release, active weekly July–October cadence, and no currently yanked releases in the examined RA histories (syntax: 3.44M total / 623K recent downloads; IDE: 1.57M / 284K). This reuses the completed registry research, not an independent metadata refresh for each subcrate.

Candidates: [pure HIR](https://crates.io/crates/ra_ap_hir) plus [RootDatabase](https://crates.io/crates/ra_ap_ide_db) wins for structured receiver, declaration and visibility evidence without caller-code execution. [IDE](https://crates.io/crates/ra_ap_ide) plus [load-cargo](https://crates.io/crates/ra_ap_load-cargo) adds unnecessary UI layers, project/tool execution and watcher/proc-macro machinery. [External LSP](https://rust-analyzer.github.io/book/contributing/architecture.html) has a stable protocol but no verified structured access/receiver proof contract; compilation executes caller scripts/macros and is not an identity proof. No loader, project auto-discovery, VFS watcher or executable proc-macro expander is introduced. FileSet/VfsPath are in-memory identifiers, not filesystem watchers. Base-db owns graph/text inputs; syntax owns byte-addressed ASTs; HIR resolves facts; ide_db supplies the concrete database. Adoption tests exercise all four API boundaries.

The published ide_db locked non-dev Darwin graph measured 133 package-version nodes (22 RA packages), not the application's incremental weight; the merged locked non-dev Darwin graph measures 193 package-version nodes and is policy-checked at adoption. Resolution adds 111 packages and unifies unicode-ident to 1.0.24 because the pinned RA lexer requires that exact version; no unrelated manifest requirements are changed. Root MIT/Apache licenses fit the application; the human-approved transitive `rustc_apfloat` license requires the explicit `Apache-2.0 WITH LLVM-exception` allowance. No CC0 loader dependency is needed. Exact coordinated 0.x pins follow the Tree-sitter deliberate-upgrade precedent: RA AST, Salsa, graph and inference APIs are unstable together. Upgrades require API/source verification, semantic/refusal replay, and all dependency gates; no floating RA upgrades or blanket unused-dependency exemptions.

## 2026-10-04 — similar 3.2.0

Generation-only line diffs: `similar = { version = "3.2", default-features = false, features = ["text"] }`, caret stable-major requirement with Cargo.lock and deliberate upgrades. `cargo info similar@3.2.0` reverified Apache-2.0, MSRV 1.85 (fits 1.97.1), and features before the manifest edit. Same-day registry comparison records Aug 17 update; 3.1.0–3.2.0 April–August cadence, 209.9M total/54.6M recent downloads, chosen version not yanked. Fresh crates.io API fetch returned 403; maintenance/non-yanked evidence is the completed same-day registry research, not an invented refresh. Candidates: [similar](https://crates.io/crates/similar), [diffy](https://crates.io/crates/diffy), [flickzeug](https://crates.io/crates/flickzeug), [imara-diff](https://crates.io/crates/imara-diff). The first wins for explicit generation/header/context APIs; diffy/flickzeug's parsing/application features are unnecessary, and imara-diff requires more low-level framing. Installed 3.2.0 `src/udiff.rs` verifies three-line context, verbatim labels and missing-newline hints; application code owns Git path quoting/framing. `cargo tree -e no-dev -p similar` shows no transitive dependencies with these features. License/advisory/source checks run through the introducing commit's gates. Git compatibility is verified in disposable fixtures, never by runtime application.

The search dependencies below were verified before manifest edits with `cargo info` for each exact release on 2026-10-04. Maintenance, cadence, downloads and non-yanked status come from the same-day crates.io metadata research; a full dependency comparison is retained in the project planning store. All declared MSRVs fit Rust 1.97.1. Cargo.lock pins compatible-major/0.x resolutions; upgrades are deliberate reviews. `cargo tree -e no-dev` was inspected on introduction: rmcp brings its schema/macros/futures/Tokio/UUID graph, Tree-sitter brings regex and native cc builds, ignore brings walkdir/globset/crossbeam. No HTTP/download transport, additional grammar pack, schema dependency, or dev dependency is added. License/source/advisory verification is enforced by the introducing commit's cargo-deny gate.

## 2026-10-04 — rmcp 3.5.0
Official MCP stdio SDK, Apache-2.0, MSRV 1.88. `3.5` caret, defaults off, only server/macros/transport-io. Candidates: [rmcp](https://crates.io/crates/rmcp), [rust-mcp-sdk](https://crates.io/crates/rust-mcp-sdk) (community contract rather than official SDK), [mcp_rs](https://crates.io/crates/mcp_rs) (stale since 2024). Registry: Sep 28 update, three September releases, 31.8M total/17M recent downloads. API verified against installed 3.5.0 source; structured success/error constructors include JSON text fallback, schema via SDK re-export. No Schemars direct dependency.

## 2026-10-04 — tree-sitter 0.27.0
[Core parser/query engine](https://crates.io/crates/tree-sitter), MIT, MSRV 1.90, default std only. Exact pin and deliberate upgrades because 0.x minors change query/API/ABI behavior. Only genuine candidate for Tree-sitter queries; local unpublished 0.28 is not a registry alternative. Aug 30 release, active July–August cadence, 42.7M/16M downloads. ABI 13–15 accepts pinned Rust grammar's 15. Native C11 compiler required; no bindgen/wasm.

## 2026-10-04 — tree-sitter-rust 0.24.2
[Rust grammar](https://crates.io/crates/tree-sitter-rust), MIT, exact reviewed grammar pin; no features. March 27 update, two March releases, 20.7M/7.3M downloads. Candidates: direct grammar wins over [language-pack](https://crates.io/crates/tree-sitter-language-pack) (hundreds of unused grammars/download machinery); [tree-sitter-language](https://crates.io/crates/tree-sitter-language) is only an ABI interface, not a grammar. Declared MSRV unavailable; native combined build is verified at introduction. Generated upstream source is unchanged.

## 2026-10-04 — ignore 0.4.33
[Gitignore walker](https://crates.io/crates/ignore), MIT option, MSRV 1.88, `0.4` caret/locked patch with deliberate 0.x updates, no extra features. Aug 4 release, frequent July–August releases, 183.2M/40.7M downloads. Alternatives [walkdir](https://crates.io/crates/walkdir) plus custom ignore matching or std traversal lack integrated scoped Git ignore precedence/negation; ignore supplies it and exposes separate inclusion filtering. Scope policy remains ours.

## 2026-10-04 — tokio 1.53.2
[Async runtime](https://crates.io/crates/tokio), MIT, MSRV 1.71, caret 1 with macros/rt-multi-thread/sync only; rmcp enables stdio I/O. Oct 3 release, active May–October cadence, 1.026B/244M downloads. Alternatives [async-std](https://crates.io/crates/async-std) and [smol](https://crates.io/crates/smol) do not remove rmcp's Tokio prerequisite. Admission uses explicit sync; blocking engine work does not run on executor threads.

## 2026-10-04 — serde 1.0.229
[Typed serialization](https://crates.io/crates/serde), MIT/Apache-2.0, MSRV 1.56, caret 1 plus derive. July 18 update, 1.478B/339M downloads. Alternatives [miniserde](https://crates.io/crates/miniserde) and hand-written serialization lack rmcp/Schemars' native typed integration and unknown-field rejection. Reuses SDK graph.

## 2026-10-04 — serde_json 1.0.151
[JSON wire format](https://crates.io/crates/serde_json), MIT/Apache-2.0, MSRV 1.71, caret 1/default std. July 20 update, 1.380B/340M downloads. Alternatives [simd-json](https://crates.io/crates/simd-json) and [json](https://crates.io/crates/json) add a second representation without a demonstrated throughput need; rmcp already uses serde_json. No preserve_order/arbitrary_precision/unbounded_depth.

## 2026-10-04 — thiserror 2.0.21
[Typed domain errors](https://crates.io/crates/thiserror), MIT/Apache-2.0, MSRV 1.77, caret 2/default std. Sep 23 update, 1.559B/398M downloads. Alternatives manual std Error and [snafu](https://crates.io/crates/snafu): derive is conventional and avoids a context-framework abstraction; library retains stable domain codes.

## 2026-10-04 — anyhow 1.0.104
[Binary boundary context](https://crates.io/crates/anyhow), MIT/Apache-2.0, MSRV 1.68, caret 1/default std. July 18 update, 1.013B/229M downloads. Alternatives boxed std Error and [eyre](https://crates.io/crates/eyre): anyhow supplies small standard startup/service context without richer report machinery. Not used for core contracts.

## 2026-10-04 — tracing 0.1.44
[Structured local logs](https://crates.io/crates/tracing), MIT, MSRV 1.65, caret 0.1/locked reviewed release, default std/attributes. Dec 18 2025 update, 898M/207M downloads; under the 12-month maintenance threshold. Alternatives [log](https://crates.io/crates/log) and stderr text do not supply typed span fields; rmcp already uses tracing. No telemetry.

## 2026-10-04 — tracing-subscriber 0.3.23
[Stderr log subscriber](https://crates.io/crates/tracing-subscriber), MIT, MSRV 1.65, caret 0.3/locked reviewed updates; defaults off, fmt/std/env-filter only. March 13 update, 642M/152M downloads. Alternatives [env_logger](https://crates.io/crates/env_logger) and custom subscriber do not natively retain tracing span fields or save meaningful machinery. No ANSI, JSON telemetry or exporter. Source/capture contents are not logged.

Dependency additions and re-pins are researched before changing the manifest and recorded in the same commit. See [AGENTS.md](../AGENTS.md) for the binding policy. Cargo.lock pins the binary's resolved graph; normal gates use `--locked`.

## 2026-10-04 — clap 4.6.7

- **Slot:** CLI help and build-version reporting; manifest requirement `4.6` (caret), resolved version **4.6.7** in Cargo.lock. Stable-major compatible updates are deliberate lockfile changes, not automatic upgrades.
- **Candidates:** [clap](https://crates.io/crates/clap), a hand-written std-only parser (the symbol-index tooling precedent), and [lexopt](https://crates.io/crates/lexopt). A manual parser saves dependencies but duplicates help/error/version behavior as the CLI grows; lexopt supplies low-level argument iteration, not the declarative help/version contract (`cargo info lexopt` verified 0.3.2, MIT, undeclared MSRV). Clap provides one maintained CLI implementation from the skeleton onward.
- **Maintenance:** `cargo info clap@4.6.7` verified MIT OR Apache-2.0 and Rust 1.85. The same-day dependency research's crates.io metadata records latest 4.6.7, updated 2026-09-14; releases 4.6.2–4.6.7 from July–September, approximately 1.191 billion total / 245.6 million recent downloads, with the chosen release not yanked. A fresh direct crates.io API fetch returned HTTP 403; metadata evidence is the already completed same-day registry research, not an invented refresh.
- **Features:** defaults disabled; only `derive`, `std`, `help`, `usage`, `error-context`. No color, suggestions, environment parsing, or runtime metadata dependency. Derive/build-version attributes were checked against installed clap 4.6.7 source and [the derive tutorial](https://docs.rs/clap/4.6.7/clap/_derive/_tutorial/index.html).
- **Compatibility/weight:** Rust 1.85 fits the tested 1.97.1 toolchain; either upstream license fits the MIT application. The introducing `cargo tree -e no-dev` audit resolves ten packages beyond this application: clap 4.6.7, clap_builder 4.6.7, clap_derive 4.6.7, anstyle 1.0.14, clap_lex 1.1.1, heck 0.5.0, proc-macro2 1.0.107, quote 1.0.47, syn 3.0.6, and unicode-ident 1.0.26. This is the builder/derive chain and its parser/proc-macro helpers; default color/suggestions are not enabled. No other direct application dependency or dev-dependency is introduced.

## 2026-10-04 — cargo-deny 0.20.2 (developer tool)

Required advisory/license/source gate, installed with `cargo install --locked cargo-deny --version 0.20.2`; not a Cargo dependency of this application. Same-day `cargo info`/registry research records Rust 1.88, MIT OR Apache-2.0, last release 2026-07-09, active May–July releases and approximately 6.13 million total / 1.79 million recent downloads. Installed version reverified. [cargo-deny](https://crates.io/crates/cargo-deny) wins over [cargo-audit](https://crates.io/crates/cargo-audit) plus separate license checks because one reviewed policy checks advisories, licenses and allowed sources. Exact tool pin keeps gate semantics stable; upgrades require review. Its transitive graph is a developer-tool cost, not part of the shipped binary.

## 2026-10-04 — cargo-machete 0.9.2 (developer tool)

Required unused-direct-dependency gate, installed with `cargo install --locked cargo-machete --version 0.9.2`; not an application dependency. Same-day registry research records MIT, release 2026-04-15 following 0.9.0/0.9.1 in August 2025, approximately 3.00 million total / 550 thousand recent downloads. Declared MSRV is unavailable; the installed binary works with this development environment. [cargo-machete](https://crates.io/crates/cargo-machete) wins over [cargo-udeps](https://crates.io/crates/cargo-udeps) because it works without a nightly compiler. Exact pin and no blanket unused-dependency exemptions. Build the locked tool separately if reinstalling; tool transitive weight is not shipped in the application.

## 2026-10-04 — gitleaks 8.30.1 (developer tool)

User-directed required staged secret-scan gate for open-source leak prevention; no application Cargo dependency. [Gitleaks](https://github.com/gitleaks/gitleaks) was chosen over [TruffleHog](https://github.com/trufflesecurity/trufflehog) (broader credential verification is unnecessary for a local staged gate) and [detect-secrets](https://github.com/Yelp/detect-secrets) (Python/baseline workflow adds setup) for native staged-scan ergonomics, Homebrew availability and its MIT license (verified in the installed distribution). The [8.30.1 release](https://github.com/gitleaks/gitleaks/releases/tag/v8.30.1) and installed `gitleaks version` were verified; the upstream release page marks it latest and shows ongoing post-release development. Installed `gitleaks protect --help` verifies the retained v8 staged command, redaction, banner/log controls and default findings exit code 1. We set that exit code explicitly and reject every nonzero scanner status, including execution/configuration failures.

Accept stable 8.x versions (optional `v` prefix), tested 8.30.1; no floating major compatibility. Homebrew supplies its packaged release; the Go fallback pins v8.30.1 and stamps 8.30.1 using upstream's `version.Version` linker variable. [Tagged go.mod](https://github.com/gitleaks/gitleaks/blob/v8.30.1/go.mod) still declares `github.com/zricethezav/gitleaks/v8` and Go 1.24.11; [version source](https://github.com/gitleaks/gitleaks/blob/v8.30.1/version/version.go) and [upstream Makefile](https://github.com/gitleaks/gitleaks/blob/v8.30.1/Makefile) confirm a plain Go install has no usable release version. The GitHub organization path is therefore not the Go install path. On Ubuntu without either installer, use the architecture-matched upstream release binary and published checksum. Upgrade deliberately after rechecking CLI compatibility, detection behavior and fixture acceptance; a new major requires explicit review. The separate scanner's Go dependency graph is developer-tool cost, not linked or vendored into the Rust binary. No custom config, baseline, allowlist, additional service or runtime library is introduced.
