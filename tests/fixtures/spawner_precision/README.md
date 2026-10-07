# Spawner lexical-precision probes

These are syntax-complete text extracts adapted from the spawner implementation of the target application, inspected at revision `f27ec0d5e336536d8da994cc148cdbc9d670abd5` (2026-10-05). Tests load the checked-in text with `include_str!`; they do not read the source project's checkout, run its executable, or add its dependencies.

`raw_line.rs` retains the primitive raw-line constant, its ordinary line-length consumer, and the monitoring function's lexical shape: preceding tuple channel declaration, async/move task, reader loop, match, while-let, let-else, and the exact `stdout_carry.len() > TRANSCRIPT_MAX_RAW_LINE` branch. Unrelated setup, monitor branches, and task handling are elided. The first line-length consumer's return type/body is shortened to a primitive result. Names referring to omitted setup are deliberately unresolved: this is a syntax probe, not a compilable replica of the source project. The relevant constant occurrences and their ancestor/sibling binding contexts are retained.

`sanitizers.rs` retains the complete `compact_transcript_message` body and the three `sanitized_transcript_*` bodies. Imports are narrowed to the helpers/protocol constant they use, and only required primitive constants are retained (their unrelated documentation is omitted). The display helper declarations are minimal stand-ins, not copied implementations; their `#[must_use]` attributes deliberately preserve the engine's unexamined-attribute blocker. `serde_json::Value` paths and member calls are unchanged. No serde_json or Tokio dependency is added to a fixture manifest, and these extracts are not claimed to compile.

`tests/support/spawner_precision.rs` supplies an ordinary `lib.rs → subagent/mod.rs → subagent/spawner.rs` tree, a wrong-root `main.rs`, and small display/protocol declarations. All paths and coordinates used in assertions come from the checked-in replica bytes.

Patch checks and any Rust compilation happen only on disposable applied copies. Caller repositories are observed before/after every engine invocation, including Git data, bytes, modes, symlinks, and modification times.
