# Rust Modularization Guidance

Idiomatic patterns for deciding how to split Rust code, grounded in the Rust Reference, API Guidelines, and observed practice in major Rust projects. Read this before choosing destination names or deciding what to group.

## When to split

**Split on a describable responsibility**, not a line count. A cohesive file with 1000 lines is fine; two files with artificial boundaries at 300 lines each is worse.

Signals it's time to split:
- You can name the new module's responsibility in one sentence
- The file contains 2–3 distinct concerns that could be tested independently
- Nobody can find things because the file is a dumping ground

Signals NOT to split:
- Items are tightly coupled (shared types, mutual calls)
- You'd need to widen visibility just to enable the split
- The only motivation is a line-count target

**No universal file-size threshold exists in Rust convention.** The 256-line group aim in the MCP's split advice is its own heuristic, not a Rust rule.

## Naming modules

- **`snake_case`** (Rust API Guidelines requirement)
- Name after the **actual concept/capability**: `parser`, `imports`, `rewrites`, `sync`, `resolver`
- Use existing domain vocabulary from the codebase
- Avoid: `part_2`, `misc`, `utils` (unless the project already has a bounded one that works), `helpers`, `common`
- Names should predict membership

## Visibility strategy

Start with the most restrictive visibility that works:

| Visibility | Use when |
|---|---|
| (private) | Default. Visible in the defining module and descendants. |
| `pub(crate)` | Crate-wide internal access; not public API. |
| `pub(super)` / `pub(in path)` | Deliberately narrow internal boundary. |
| `pub` | Intended external API. Only what callers need. |

**The MCP engine supports only private and `pub(crate)` repairs** — it won't generate `pub(super)`/`pub(in ...)` alternatives. If the ideal visibility is narrower than `pub(crate)`, the engine may widen more than strictly necessary; review visibility rewrites in the plan.

## Import organization

- **Prefer explicit imports** during a split — each new module imports exactly what it uses
- **`crate::` paths** are clearest for internal absolute references; `self::`/`super::` express local relationships
- **Don't reorder or merge import groups** during a move — the Rust Style Guide forbids merging/reordering groups
- **Don't introduce new wildcard imports** as a split shortcut — they hide binding provenance and the MCP engine refuses uncertain globs. Existing globs are not blanket vetoes, but bare consumers in other admitted files reached through candidate globs block with `glob_consumer_unrepaired`; review their exact call-site and glob-statement anchors rather than assuming the old route survives.

## File layout conventions

**Modern (2018 edition) layout** — preferred for new code:

```
src/
├── lib.rs          # mod foo; mod bar;
├── foo.rs          # mod nested; → loads foo/nested.rs
└── foo/
    └── nested.rs
```

Rules:
- Can't have both `foo.rs` and `foo/mod.rs` — pick one
- The MCP creates new files as literal `.rs` siblings (`src/new_module.rs`), never directories or new `mod.rs` files
- `mod foo;` loads a module; `use` imports names but doesn't load files
- Legacy `mod.rs` trees are valid — the engine works with them but never creates new ones
- Root files (`lib.rs`, `main.rs`) can contain logic, not just declarations

## What to keep together

- A type and its inherent `impl` blocks
- A trait and its impls (unless the project separates them)
- Tightly coupled helpers with a single consumer
- Unit tests alongside tested code (`#[cfg(test)] mod tests`)

**Co-location is grouping advice, not a capability guarantee.** Inline test modules are context-sensitive; `#[cfg]`, `#[derive]`, proc-macro attributes, and relevant macro contexts can block moves. Do not extract their bodies or detach attributes to bypass a blocker. Opt-in standard-prelude proofs cover only the documented unshadowed names; bare value-position `Some`/`None` additionally assume constructor identity with unshadowed `Option`, without modeling derive-generated imports. Explicit `resolve_semantic` covers only bounded, positively resolved eligible occurrences, including variant paths with parent-enum identity at both overlays. Its context gate admits custom derives for nominal paths only with stable locally written declarations or explicit named import routes at both overlays; aliases/grouped leaves need non-glob module/re-export prefixes and active binding attributes (known-OFF imports are not evidence). Generated explicit imports can override glob-resolved names, so additive output alone does not establish path identity. Configured extern-prelude roots and generated-item-dependent facts keep the non-builtin derive veto (inspect anchored `context_evaluations`, `fact_class` and `basis`; unlisted cfg stays unknown and undeclared `cfg_attr` still vetoes either class); `acknowledge_test_consumers` discloses exact same-file inline test consumers without validating them. None resolves generic/trait/macro uncertainty or proves compilation/equivalence. The engine supports only private and `pub(crate)` repairs — it won't generate `pub(super)`/`pub(in ...)` alternatives; review any widening.

## Anti-patterns

### God-module relocation
Moving a catch-all to `helpers.rs` changes the address, not the responsibility. If the new module's name doesn't describe its contents, rethink the split.

### Artificial cycles
Cross-module cycles within a crate aren't illegal Rust, but they make understanding/testing require the whole cycle. Keep strongly interdependent units together; don't create a `common`/`traits` module just to break a cycle diagram.

### Over-fragmentation
No one-function-per-file mandate. Major Rust projects colocate related items; a file with one struct + impl + helpers is fine.

### Leaky abstractions
Don't widen visibility just to enable a split. Review the minimum access each consumer needs — including the engine's `pub(crate)` repairs.

## Lessons from major projects

Cargo, rustfmt, Tokio, and Serde all split by *responsibility*, use *descriptive concept names*, keep *tight implementations private*, expose *minimal public APIs* via selective reexports, and colocate related items (including tests at the root). None uses a line-count threshold; several keep root files with real logic; `mod.rs` remains legitimate practice (Tokio). Public facades (`pub use`) can decouple API paths from physical layout, but the engine does not synthesize API shims — preserve exposed paths yourself.
