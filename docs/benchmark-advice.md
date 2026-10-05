# Advisory split workload measurements

## Native macOS arm64, 2026-10-05

Measured on macOS 26.6.2, Apple Silicon (`Mac14,9`, Apple M2 Pro),
10 logical cores, 32 GiB RAM, internal SSD (`diskutil info /dev/disk0`
reports `Solid State: Yes`). Toolchain: rustc 1.97.1, Cargo 1.97.1,
Git 2.44.0. This meets the local ≥8-core/16-GiB/SSD qualification.
It is **not** Ubuntu, container, or amd64 evidence.

Reproduction:

```sh
cargo test --release --locked --offline --test suggest_split release_advice_workload_ten_runs -- --ignored --nocapture
```

The release-only test generates a disposable Git fixture with an admitted
`corpus` of exactly **100 regular Rust files / 10,485,760 source bytes**.
`corpus/lib.rs` contains 99 ordinary module declarations. The advice source,
`corpus/file_000.rs`, has **740,016 bytes / 400 top-level functions**:
200 `alpha_*` and 200 `beta_*` functions, each containing one written call to
its adjacent paired function and a 1,800-byte local string-literal payload.
Remaining corpus bytes are deterministic retained block-comment padding.
The measured source is below 1 MiB and 500 units; the graph has 400 candidate
occurrences, below the workload's 1,000-candidate ceiling. Advice produces no
rewrites or execution artifacts; no execution workload is inferred from it.

Each repetition reads all corpus files to warm the filesystem cache, launches
and initializes a **fresh real stdio server process**, then measures the
complete `suggest_split` call through response reception, JSON decoding and
structured/text equality checks. Server startup/initialization and the
subsequent read-only observations are outside the timer. There is no retained
parse state. Requests set only `limits.text_bytes:0`; all other source, work,
context and response settings retain their defaults. The test refuses debug
build timing evidence.

Every run asserts complete inventory, exact-once draft membership, same-response
signal/decision closure, no patch/edit/create/plan/handle fields, unchanged
source/Git bytes, directory entries, modes and observable mtimes, and identical
advice across all ten fresh processes. Complete membership/evidence fits within
the unchanged 2-MiB duplicated response cap. The corpus has strong prefix and
paired-reference cohesion; these figures do not promise throughput for arbitrary
dependency density, local-binding complexity, macro expansion or huge descriptors.

| Run | Milliseconds |
| --- | ---: |
| 1 | 265 |
| 2 | 262 |
| 3 | 275 |
| 4 | 259 |
| 5 | 250 |
| 6 | 283 |
| 7 | 254 |
| 8 | 310 |
| 9 | 258 |
| 10 | 245 |

The **≤5,000-ms target was met in 10/10 runs** (required ≥9/10).

All ten responses had these observed properties:

| Property | Value |
| --- | ---: |
| Eligible reference-corpus files | 100 |
| Advice-source bytes | 740,016 |
| Inventoried / returned units | 400 / 400 |
| Written reference candidate occurrences | 400 |
| Signals | 802 |
| Envelope decisions | 3 |
| Complete drafted partitions | 1 |
| Accounted analysis descriptor bytes | 2,407,946 |
| Structured JSON bytes | 846,762 |
| Duplicated wire bytes, including framing reserve | 1,800,284 |

Wire accounting includes both structured and JSON-text representations,
JSON escaping and the same conservative 4,096-byte framing reserve used by
the server. Path lengths can change these sizes on another host. One draft
is intentional: the primary cluster allocation and byte-balanced alternative
have identical membership on this symmetric corpus, so the duplicate is not
returned. The rich integration fixture independently verifies two distinct
proposals and an externally edited two-sibling batch.

Confidence is syntactic organization evidence, not a probability of correct
Rust compilation. Input parsing is reported as `input_checked`; semantics
remain `not_performed`. The server executes neither Cargo nor patch application.
This benchmark measures **advice**, not the earlier single/batch move timings.

## Outstanding platform evidence

Ubuntu arm64 conformance and the ten-run advice workload were **not run** here.
The installed Docker client could not connect to its local daemon socket:
`unix:///Users/said/.docker/run/docker.sock` was absent. No container/native-Linux
performance claim is made and no platform obligation is waived. Ubuntu amd64
and qualified inherited Linux search-benchmark evidence also remain outstanding.
Native macOS success does not satisfy those obligations.
