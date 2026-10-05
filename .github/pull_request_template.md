## What changed and why

<!-- One paragraph: what this PR does, and the motivation. Link issues as "Fixes #N" where applicable. -->

## Checklist

Fill this in honestly — test counts and commands, not adjectives. PRs that skip or dismiss the checklist rules may be rejected without further review.

- [ ] Git hooks installed (`make install-hooks`) and every commit passed the local gates (secret scan, fmt, clippy `-D warnings`, tests, cargo-deny, cargo-machete, tooling fixtures)
- [ ] Tests: `cargo test --workspace --all-features --locked` — **N passed, 0 failed** (state N; note any intentionally ignored tests)
- [ ] Commits follow the conventional format (`feat:`/`fix:`/…, subject ≤72 chars)
- [ ] No unrelated changes or reformatting churn in this PR
- [ ] If dependencies changed: research done and `docs/dependency-log.md` entry added in the same commit
- [ ] If tool contracts changed: `docs/tools.md` updated
- [ ] Platform-sensitive changes: state what you validated and where (CI covers Linux amd64+arm64; macOS is validated locally by maintainers — do not add macOS CI)

## AI assistance disclosure

Per the [AI contribution policy](CONTRIBUTING.md#ai-contribution-policy):

- [ ] No AI assistance was used in this contribution, **or** it is disclosed below
- <!-- If AI tools or coding agents contributed (code, docs, tests, commit/PR text), describe their role in one sentence, e.g. "code drafted by <tool> under human direction". A human author remains accountable for the change. -->

## Reviewer notes

<!-- Anything the reviewer should look at extra closely; optional. -->
