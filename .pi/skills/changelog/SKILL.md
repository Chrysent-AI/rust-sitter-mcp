---
name: changelog
description: Keep CHANGELOG.md current under [Unreleased] as user-facing work lands in rust-sitter-mcp, and cut releases with make bump-version. Use when committing features, fixes, contract or behavior changes, when asked to update the changelog, or when preparing a release.
---

# Changelog discipline for rust-sitter-mcp

`CHANGELOG.md` follows [Keep a Changelog](https://keepachangelog.com/en/1.0.0/).
It has one `## [Unreleased]` section at the top and one immutable section per
release. The rule: **a user-facing change and its changelog bullet land in the
same commit.** Do not batch notes for later; later never comes.

## Writing entries

- Add bullets under `## [Unreleased]` using the standard subsections:
  `### Added`, `### Changed`, `### Fixed`, `### Removed`. Create the
  subsection if it does not exist yet; keep subsections in that order.
- One bullet per user-visible change, present tense, backticked identifiers:
  `* Add `written_reexport` route evidence for consumer import repairs through
  named public re-export leaves`.
- Belongs in the changelog: MCP contract changes — new/changed request flags or
  fields on `search`, `search_query`, `replace`, `move_item`, `suggest_split`;
  new proof or evidence classes (`standard_prelude`, `variant_path`,
  `written_reexport`, `ra_resolved`, …); new or reworded refusal/blocked-plan
  behavior, decision routes, or refusal-basis classes; capability changes to
  what the engine proves, plans, or refuses; visibility/repair behavior;
  dependency switches (ra_ap/tree-sitter pins); platform support; packaging
  (`make install`, hooks); and security fixes.
- Does not belong: pure internal refactors with no observable tool behavior
  change, test or fixture churn, benchmark bookkeeping, formatting, typo-level
  doc fixes. When unsure, one short line under `### Changed` beats silence.
- Every contract change additionally requires the bundled-skill lockstep review
  (`.pi/skills/split-rust-code/`) in the same commit per `AGENTS.md` — the
  changelog bullet documents it for users; lockstep keeps the agent surface
  truthful.

## Creating CHANGELOG.md entries from history

When backfilling (e.g. after a large landed batch), reconstruct entries from
`git log` commit subjects and bodies of the merged work, one bullet per
user-visible change, grouped under the correct subsection. Verify each bullet
against the diff (`git show --stat`) — commit subjects alone are leads, not
evidence. Keep bullets about what a tool user observes, not how it was built.

## Cutting a release

Never hand-edit version numbers yourself:

```bash
make bump-version BUMP=patch   # or minor|major, or VERSION=x.y.z
```

Unlike jett's script, this repo's `scripts/bump-version.sh` bumps only
`Cargo.toml` and refreshes the `Cargo.lock` package entry — it does **not**
touch `CHANGELOG.md`. The release flow is therefore:

1. Update the changelog yourself: rename `## [Unreleased]` to
   `## [x.y.z] - <today>`, insert a fresh empty `## [Unreleased]` above it,
   and fill an empty section with a default "Bumped version" bullet.
2. Review the full diff, commit (hooks run the quality gates), then tag with
   `git tag vX.Y.Z` to match the existing `v*` tag series.
3. Never rewrite or reorder historical release sections.

Version semantics follow Cargo default: new capabilities or contract surface =
minor; fixes and refinements = patch; breaking request/response contract
changes that remove or redefine existing fields = major while pre-1.0
conventions still allow.
