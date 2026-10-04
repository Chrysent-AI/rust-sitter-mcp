# Move planning workload measurements

## Native macOS arm64, 2026-10-04

Measured on macOS 26.6.2, Apple M2 Pro, 10 logical cores, 32 GiB RAM,
internal Apple Fabric SSD (`Solid State: Yes`). Toolchain: rustc 1.97.1,
Cargo 1.97.1, Git 2.44.0. This meets the local ≥8-core/16-GiB resource
qualification. It is **not** Linux or amd64 evidence.

Reproduction:

```sh
cargo test --locked --offline --release --test fixture_smoke release_move_workload_ten_runs -- --ignored --nocapture
```

The ignored release-only test generates a disposable Git fixture and an
admitted `corpus` containing exactly 100 regular Rust files and 10,485,760
source bytes. `corpus/lib.rs` declares 99 ordinary modules. One module
contains a large function and 100 small functions with local string-literal
payloads; remaining bytes are deterministic retained block-comment padding.
These are intentionally dependency-free, low-reference-density cases, not a
representative dense resolver workload. No import/path/visibility repairs
are performed or implied.

Every repetition warms those files by reading them, launches and initializes
a **fresh real stdio server process**, then measures the complete move call
through JSON response reception/decoding and structured/text equivalence.
There is no retained parse state. Server startup/initialization, external Git
application and read-only observations are outside the timer. Each call uses
`limits.text_bytes:0` and otherwise unchanged defaults, including the 2-MiB
duplicated response bound. Every repetition returns complete applicable
artifacts and preserves source/Git bytes, entries, modes and mtimes. The first
repetition of each workload additionally passes external `git apply --check`,
application and independent JSON file-set/byte/mode reconstruction on copies.
The server does not apply patches or execute Cargo.

| Workload | Selected bytes | New files | Relevant candidates | Audited rewrites | Structured bytes | Duplicated wire bytes¹ |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| One item | 49,185 | 1 | 0 | 1 | 392,195 | 789,563 |
| 100-item batch | 16,400 | 2 | 0 | 101 | 711,619 | 1,476,892 |

¹ Includes the same conservative 4,096-byte framing reserve used by the
planner. Path lengths can change these sizes on another host. The rewrites
are private module declarations and minimal boundary separators, not
semantic repairs. Zero reference candidates is an observed fixture property,
not a claim of compiler-resolved independence in arbitrary code.

Ten repetitions, milliseconds:

- One item: **162, 162, 163, 164, 162, 163, 159, 159, 160, 163**.
- 100-item batch: **227, 226, 226, 225, 224, 227, 222, 226, 224, 229**.

Both workloads satisfy their respective ≤5,000-ms / ≤10,000-ms targets in
**10 of 10** native macOS runs. The batch's 16,400 selected bytes are below
the specified 200-KiB upper bound; this does not establish throughput or
complete-output fitting at that upper bound or at higher reference density.

## Output-bound observation and outstanding platforms

An earlier 100-item fixture used 80,400 selected bytes. Its complete audit,
creation/edit and patch response exceeded the unchanged default wire cap.
The server correctly returned an incomplete `response_bytes` result with
**all three artifacts null**, not a subset. It is not passing performance
or applicability evidence. The final fixture uses smaller literal payloads
so it meets the workload's required complete-artifact/default-bound condition;
no response cap or product obligation was raised or weakened.

Ubuntu arm64 conformance and ten-run timings were **not run** in this session:
the installed Docker client could not connect to its local daemon socket.
No container or native-Linux qualification is claimed. Ubuntu amd64 and the
inherited qualified Linux search benchmark obligations remain outstanding.
Split-advice timings are outside this move-only measurement and remain
unmeasured. Native macOS results do not satisfy those other obligations.
