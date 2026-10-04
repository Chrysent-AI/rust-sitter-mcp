# Large-repository throughput benchmark

Measured on 2026-10-04 against source commit
`6697d4f6432e9369b5902ae8857d0ebafa7c960e`, release identity
`0.1.0 (6697d4f6432e)`. No product code, dependencies, limits policy, or
runtime behavior was changed for this benchmark.

## Results and qualification

The acceptance rule is **at least nine of ten complete runs** within **5 seconds
for discovery** and **30 seconds for full search**. Timings below are seconds,
rounded to six decimal places; the verdict uses the unrounded measurements.

| Platform | Discovery ≤5 s | Full search ≤30 s | Qualification |
| --- | --- | --- | --- |
| macOS arm64, native | **PASS: 10/10** | **PASS: 10/10** | Meets the specified local SSD, ≥8 logical cores, ≥16 GiB conditions |
| Ubuntu arm64, container | **PASS: 10/10 observed** | **PASS: 10/10 observed** | Supplemental container evidence; guest has only 5 vCPUs and approximately 7.72 GiB usable RAM |

**No timing target missed in either set.** The Linux leg is not evidence of a
native Linux run or of a Linux execution environment satisfying the specified
minimum CPU/RAM conditions. That qualification gap remains open; the successful
container observations do not waive the hardware requirement. Ubuntu amd64 was
not benchmarked in this task.

| Run | macOS discovery | macOS full search | Ubuntu container discovery | Ubuntu container full search |
| ---: | ---: | ---: | ---: | ---: |
| 1 | 1.193464 | 16.247916 | 1.449500 | 23.606121 |
| 2 | 1.163448 | 17.071721 | 0.830925 | 15.700048 |
| 3 | 1.230093 | 17.423660 | 1.117950 | 14.368887 |
| 4 | 1.511980 | 17.027979 | 1.098184 | 15.942667 |
| 5 | 1.099291 | 20.189667 | 0.927886 | 12.357219 |
| 6 | 1.105364 | 16.406831 | 0.758158 | 13.102346 |
| 7 | 1.237584 | 15.872255 | 0.789361 | 12.897458 |
| 8 | 1.297859 | 20.630414 | 0.787427 | 12.868043 |
| 9 | 1.246354 | 16.441652 | 0.791502 | 12.693993 |
| 10 | 1.171307 | 16.907253 | 0.759213 | 12.858564 |

All 20 full-search calls returned `status: complete`, with all three coverage
flags true; discovered, scanned, eligible, and matched file counts were each
10,000. Each call returned exactly 50 matches with an exact total, no cursor,
`has_more: false`, no skips, no diagnostics, and no truncation reasons. Both
structured content and its JSON text fallback agreed. Full JSON-RPC responses
were 209,646 bytes on macOS and 209,616 bytes in the container, within the
2-MiB response limit. Root-path length accounts for the size difference.

## Hardware, operating systems, and builds

### Primary: native macOS arm64

- Apple M2 Pro; `hw.logicalcpu = 10`, `hw.physicalcpu = 10`.
- `hw.memsize = 34,359,738,368` bytes (32 GiB).
- macOS 26.6.2, build 25G83; Darwin arm64.
- Internal NVMe **Apple SSD AP0512Z**, 500,277,792,768-byte capacity,
  TRIM enabled. Fixture was under `/private/tmp` on the APFS data volume,
  not a RAM disk or network mount.
- Rust/Cargo 1.97.1; rustc target `aarch64-apple-darwin`, LLVM 22.1.6.
  Python 3.12.7; Git 2.44.0.
- `cargo build --release --locked` succeeded. No `RUSTFLAGS`, worker-count,
  source, or dependency changes were made for the benchmark.
- Measurement window: 12:52:27–12:55:44 UTC, discovery then full search
  interleaved for each run number.

### Supplemental: Ubuntu arm64 container

- Same physical Mac/SSD, but **not native Linux**: Podman client 6.0.0,
  server 6.0.2, AppleHV VM, Fedora CoreOS 44.20260707.3.1.
- VM configuration: **5 vCPUs, 8 GiB allocated RAM, 93 GiB virtual disk**.
  Linux reports 8,289,288,192 bytes usable RAM (approximately 7.72 GiB),
  with no swap. These virtual resources are below the benchmark's required
  eight logical cores and 16 GiB; physical-host capacity is not substituted
  for resources available to the measured Linux process.
- Ubuntu 24.04.5 LTS, arm64/aarch64; shared VM kernel
  `7.1.3-200.fc44.aarch64`.
- Retained Ubuntu 24.04 image ID
  `95d16dfcd4ab8b61154e2280ab9fe4afa2ef03e5eefa8a689e86571e8a1b32cf`,
  digest `sha256:534baea6a22c03a63003dbc8dbe78fe34bc0d7e595d9a9dc9834884ff530eb55`.
- Ordinary user `ubuntu`, UID/GID 1000. No additional container CPU/memory
  cap (`Memory = 0`, `NanoCpus = 0`); the VM resources still bound execution.
- Fixture created independently inside the container at
  `/tmp/rust-sitter-benchmark-wjsh0jpv/fixture`, on container overlay storage
  backed by the VM's XFS `/var` filesystem and the host SSD. No host bind mount
  or tmpfs fixture; filesystem/page-cache layers differ from native APFS.
- Rust/Cargo 1.97.1; target `aarch64-unknown-linux-gnu`, LLVM 22.1.6.
  Python 3.12.3; Git 2.43.0. `cargo build --release --locked` succeeded
  against the same clean source revision and produced the same version identity.
- Measurement window: 12:56:18–12:58:57 UTC, using the same fixed protocol.

Host specifications were obtained from targeted `sysctl`, `sw_vers`,
`system_profiler SPNVMeDataType`, Podman resource/image inspection, `nproc`,
`free -b`, and filesystem inspection. The existing container's Rust installation
was used with explicit `CARGO_HOME=/root/.cargo`, `RUSTUP_HOME=/root/.rustup`,
and its Cargo binary directory on `PATH`; no product dependency was added.

## Fixture and request

Each independently generated fixture is a disposable Git worktree containing:

- **Exactly 10,000 eligible UTF-8 `.rs` files**, totaling **209,715,200 bytes
  (exactly 200 MiB)**. There are 5,200 files of 20,972 bytes and 4,800 of
  20,971 bytes; all are below 1 MiB. Files occupy 100 source shard directories.
- Varied, deterministic Rust: functions, structs/impls, enums, traits,
  iteration, closures, arithmetic, match expressions, attributes/docs, Unicode,
  `Option`/`Result`, formatting, and string handling. A short final line comment
  sizes each file precisely; padding totals 3,045,052 bytes (approximately
  1.45%), not a comment-only or repeated-blank-text workload. This is synthetic
  written syntax, not an assembled or compiler-checked Cargo application.
- **Exactly 50 query sites**, one `value.unwrap()` every 200 files, spread
  through the sorted corpus. At most one site appears in an eligible file.
- **50,000 extra `.rs` fixture files in hard-excluded directories**: 20,000
  under root `target/noise`, 10,000 under `src/target/noise`, and 20,000 under
  `.git/benchmark-noise`. Each contains an unwrap call. Normal Git-created
  administrative files also exist; they are not counted as fixture noise.
- Both legs verified the same eligible corpus SHA-256:
  `abab93dcd1aae0836f7154a3284a01e6c23650a6c51f7f34e64f2b142337bb04`.
  Hash input is sorted root-relative UTF-8 path followed by full source bytes,
  each field prefixed by its eight-byte big-endian length. This fixture hash is
  separate from the server's root-sensitive Git corpus fingerprint.

Full search used the `search` tool with no path/glob restrictions or cursor:

```json
{
  "repo_path": "/absolute/disposable/fixture",
  "pattern": "$a.unwrap()",
  "context": {"before_lines": 2, "after_lines": 2},
  "page_size": 100,
  "limits": {
    "max_file_bytes": 1048576,
    "max_files": 20000,
    "max_source_bytes": 268435456,
    "time_budget_ms": 60000,
    "query_state_limit": 4096,
    "response_bytes": 2097152,
    "diagnostic_count": 64,
    "text_bytes": 8192
  }
}
```

The 1-MiB eligibility cap includes every eligible fixture file; all other
limits above are the normal defaults. The 60-second cooperative work budget
is not the 30-second acceptance target: a complete 31-second run would be
recorded as a miss, not retimed or narrowed. Fifty matches fit one page without
continuation, and completeness checks ensure all files were actually searched.

## Measurement protocol and caveats

1. Build the unchanged package in release mode with its locked graph. Validate
   fixture byte/count/encoding/site invariants independently on each platform.
2. Immediately before **each** measurement, read all 200 MiB of eligible
   content, stat its files, and list root/source/exclusion directories. This
   warms filesystem data/metadata, not a server or parser. No cold-cache claim
   is made, and cache residency was not established with a page-residency probe.
3. Run discovery and full search once each for run numbers 1–10, sequentially.
   No preliminary full searches, discarded observations, replacement runs,
   warm server reuse, code tuning, or rerun-until-lucky were used. The first
   recorded run on each platform is included. Host background activity was not
   disabled or controlled; the two platform legs were not run concurrently.
4. **Discovery alone:** a fresh process of a scratch executable per run. The
   executable compiles unchanged `src/scope.rs` via a source include and links the package's release
   result types and Cargo-selected release dependencies. There is no discovery
   MCP tool, so discovery is not falsely approximated by an empty search.
   Rust `Instant` starts before root resolution and stops after `Scope::new`
   and `scope::discover` return: it includes canonicalization/Git root checks,
   ignore/exclusion traversal, sorting, eligibility/UTF-8 reads, immutable
   snapshots, and corpus fingerprinting. It excludes parsing/matching,
   process startup, JSON summary formatting, and teardown. Returned counts,
   source-byte total, coverage, fingerprint, skips, and bounds are checked.
5. **Full search:** a fresh separate `target/release/rust-sitter-mcp` process
   for every run, not the installed gateway's server. Send newline-delimited
   JSON-RPC `initialize` (protocol `2025-06-18`), wait for initialization and
   verify build identity, then send `notifications/initialized` and `tools/call`
   for `search`. Python `perf_counter` measures from just before encoding and
   writing the tool call until the complete response line is received. This
   includes discovery, parsing, matching, rendering, bounded structured/text
   emission, and pipe transport; it excludes process startup/initialization,
   client response JSON decoding/assertions, and shutdown. The fresh server has
   no prior query/parse cache; the product also has no persistent parse cache.
6. Initialize has a 15-second client timeout; measurements have a 75-second
   outer process/response timeout, above the server's 60-second budget.
   Shutdown has a 10-second timeout; stuck process groups are killed and reaped.
   Invalid/incomplete/timed-out measurements would be recorded as failures,
   never accepted as fast searches. No such failure occurred.

These observations apply to this modest-output sugar query and synthetic corpus,
not arbitrary raw-query complexity, high match density, all hardware, cold disks,
or a native Linux platform matrix. Virtualization/overlay storage and uncontrolled
background activity prevent treating the two time distributions as an OS
performance comparison. Discovery uses the original source module in a separate
release harness because the protocol exposes only search/replacement tools;
its compilation/linkage is not byte-identical to the server binary, although its
source and dependency graph are unchanged.

## Evidence and reproduction

Raw full-precision results, every response and per-process stderr log, fixture
metadata, build logs, scripts, and targeted hardware evidence were retained in
the local session's external `artifacts/benchmark/wjsh0jpv/` directory. The
`macos/results.json` and `linux/results.json` files contain all 20 observations
per platform; neither the 200-MiB fixture nor local evidence is shipped as product
code. The appendices preserve the exact generator/driver and historical
full-precision timing observations without requiring those local artifacts.

For independent reproduction, save the four appendices below with the shown
filenames into a **new scratch directory**. Use source revision
`6697d4f6432e9369b5902ae8857d0ebafa7c960e` and the pinned toolchain. The scripts
use Python's standard library only; Cargo artifact selection avoids accidentally
linking stale release feature variants already present in `target/`.

```sh
PROJECT=/absolute/path/to/rust-sitter-mcp
SCRATCH=/absolute/path/to/new/scratch
cd "$PROJECT"
cargo build --release --locked
python3 "$SCRATCH/prepare.py" "$PROJECT" "$SCRATCH"
python3 "$SCRATCH/generate.py" "$SCRATCH/fixture"
python3 "$SCRATCH/measure.py" "$SCRATCH/fixture" \
  "$PROJECT/target/release/rust-sitter-mcp" "$SCRATCH/discover" "$SCRATCH/results"
```

`generate.py` refuses an existing fixture and `measure.py` refuses an existing
results directory. Run once, retain all observations, and report misses rather
than changing the workload or selecting a better batch. For Linux, execute the
same commands **inside** an Ubuntu arm64 environment as an ordinary user, not
around `podman exec` on the host; the measured pipe/time path must stay inside
the container. The recorded container was reused from prior platform validation,
not rebuilt or resized to improve these results.

## Appendix: full-precision observations

```json
{
  "macos": {
    "discovery": [
      1.193463708,
      1.1634484999999999,
      1.230092625,
      1.511979666,
      1.099290792,
      1.105364209,
      1.237584292,
      1.297858583,
      1.246354208,
      1.171306917
    ],
    "full": [
      16.24791574990377,
      17.071720583364367,
      17.423659875057638,
      17.02797900000587,
      20.189667208120227,
      16.406831082887948,
      15.872255124617368,
      20.630413583945483,
      16.441652290988714,
      16.90725287469104
    ]
  },
  "linux": {
    "discovery": [
      1.4494999499999999,
      0.830924598,
      1.117950108,
      1.098184086,
      0.92788632,
      0.758157515,
      0.789360664,
      0.787427275,
      0.791502221,
      0.759213231
    ],
    "full": [
      23.60612073802622,
      15.700047755963169,
      14.368886811018456,
      15.94266710197553,
      12.357219428988174,
      13.102345847990364,
      12.897457948012743,
      12.86804347904399,
      12.693993059976492,
      12.858563908957876
    ]
  }
}
```

## Appendix: reproducible harnesses

The following files are scratch benchmark support, not runtime additions.

### generate.py

SHA-256: `8685a2d16414bc431bb39024f0d883ae7a5640322b688bbdb40729f35f426b95`

```python
#!/usr/bin/env python3
"""Deterministic 200-MiB Rust corpus; never modifies an existing fixture."""
import hashlib
import json
import pathlib
import subprocess
import sys

root = pathlib.Path(sys.argv[1]).resolve()
root.mkdir()  # Fail closed if a previous fixture exists.
subprocess.run(["git", "init", "--quiet", str(root)], check=True, timeout=15)
total = 200 * 1024 * 1024
small, larger_count = divmod(total, 10_000)
templates = [
    '''/// Compute a stable score for a batch of records.
pub fn score_{n}(values: &[u64], seed: u64) -> u64 {{
    let mut score = seed.wrapping_add({salt});
    for (index, value) in values.iter().enumerate() {{
        score = score.wrapping_add(value.rotate_left((index % 32) as u32));
        if score & 1 == 0 {{ score ^= 0x9e3779b9; }}
    }}
    score
}}
''',
    '''#[derive(Debug, Clone, PartialEq)]
pub struct Record_{n} {{ pub label: String, pub weight: u64, pub active: bool }}
impl Record_{n} {{
    pub fn new(label: &str, weight: u64) -> Self {{
        Self {{ label: label.to_owned(), weight: weight + {salt}, active: true }}
    }}
    pub fn display(&self) -> String {{ format!("{{}}:{{}}", self.label, self.weight) }}
    pub fn deactivate(&mut self) {{ self.active = false; }}
}}
''',
    '''pub fn normalize_{n}(input: &str) -> Result<Vec<String>, &'static str> {{
    let tokens: Vec<_> = input.split(',').map(|s| s.trim().to_owned()).collect();
    if tokens.is_empty() {{ return Err("empty input"); }}
    Ok(tokens.into_iter().filter(|s| !s.is_empty()).collect())
}}
pub fn bucket_{n}(value: u32) -> &'static str {{
    match value % {modulus} {{ 0 => "zero", 1 | 2 => "small", 3..=7 => "medium", _ => "large" }}
}}
''',
    '''#[derive(Clone, Debug)]
pub enum State_{n} {{ Ready(u64), Pending {{ retry: u32 }}, Done }}
pub fn advance_{n}(state: State_{n}) -> State_{n} {{
    match state {{
        State_{n}::Ready(value) if value > {salt} => State_{n}::Done,
        State_{n}::Pending {{ retry }} => State_{n}::Ready(u64::from(retry)),
        other => other,
    }}
}}
''',
    '''pub trait Measure_{n} {{ fn measure(&self) -> usize; }}
impl<T> Measure_{n} for Vec<T> {{ fn measure(&self) -> usize {{ self.len() }} }}
pub fn window_{n}(values: &[i32]) -> Vec<i32> {{
    values.windows(2).map(|w| w[0].saturating_add(w[1])).collect()
}}
pub fn choose_{n}(value: Option<u64>) -> u64 {{
    if let Some(number) = value {{ number + {salt} }} else {{ 0 }}
}}
''',
]
manifest = hashlib.sha256()
padding_total = 0
for index in range(10_000):
    size = small + (index < larger_count)
    text = f"//! Synthetic Rust module {index}; café records and state transitions.\n"
    if index % 200 == 0:
        text += "pub fn required(value: Option<u64>) -> u64 { value.unwrap() }\n"
    content = text.encode("utf-8")
    number = 0
    while True:
        block = templates[(index + number) % len(templates)].format(
            n=number, salt=(index * 17 + number * 13) % 1009, modulus=11 + index % 7
        ).encode("utf-8")
        if len(content) + len(block) + 5 > size:
            break
        content += block
        number += 1
    remainder = size - len(content)
    content += b"// " + b"p" * (remainder - 4) + b"\n"
    padding_total += remainder
    assert len(content) == size and size <= 1024 * 1024 and b"\0" not in content
    content.decode("utf-8")
    assert content.count(b".unwrap()") == int(index % 200 == 0)
    relative = f"src/shard_{index // 100:03}/module_{index:05}.rs"
    path = root / relative
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(content)
    name = relative.encode("utf-8")
    for field in [name, content]:
        manifest.update(len(field).to_bytes(8, "big"))
        manifest.update(field)
for directory, count in [("target/noise", 20_000), ("src/target/noise", 10_000), (".git/benchmark-noise", 20_000)]:
    folder = root / directory
    folder.mkdir(parents=True)
    for index in range(count):
        (folder / f"excluded_{index:05}.rs").write_text(
            "fn excluded() { Some(1_u8).unwrap(); }\n", encoding="utf-8"
        )
stats = {
    "eligible_files": 10_000, "eligible_bytes": total, "min_file_bytes": small,
    "max_file_bytes": small + 1, "hard_excluded_rs_files": 50_000,
    "expected_matches": 50, "padding_bytes": padding_total,
    "eligible_sha256": manifest.hexdigest(),
}
(root.parent / "fixture.json").write_text(json.dumps(stats, indent=2) + "\n")
print(json.dumps(stats), flush=True)
```

### discover.rs.in

SHA-256: `689483608412073e1feed5e96f6245c249d1c3385198537e468b5765a45b758a`

```rust
// Compiles the original private discovery module without altering the checkout.
mod result {
    pub use rust_sitter_mcp::result::*;
}
#[path = "@SCOPE_PATH@"]
mod scope;

use result::{Context, Limits, SearchEnvelope, SearchRequest};
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

fn main() {
    let root = std::env::args().nth(1).expect("fixture path");
    let limits = Limits {
        max_file_bytes: 1024 * 1024,
        max_files: 20_000,
        max_source_bytes: 256 * 1024 * 1024,
        time_budget_ms: 60_000,
        ..Limits::default()
    };
    limits.validate().expect("valid benchmark limits");
    let request = SearchRequest {
        repo_path: root,
        query: String::new(),
        paths: None,
        globs: None,
        context: Context::default(),
        limits: limits.clone(),
        page_size: 100,
        cursor: None,
    };
    let mut envelope = SearchEnvelope::empty(limits);
    let cancelled = AtomicBool::new(false);
    let launch = std::env::current_dir().expect("launch path");
    let start = Instant::now();
    let deadline = start + Duration::from_secs(60);
    let root = scope::resolve(&request.repo_path, &launch).expect("resolve root");
    envelope.root = Some(root.to_str().expect("UTF-8 root").into());
    let scope = scope::Scope::new(root, &request).expect("scope");
    let (files, snapshot) = scope::discover(&scope, &mut envelope, deadline, &cancelled)
        .expect("discover");
    let elapsed = start.elapsed().as_secs_f64();
    envelope.snapshot_id = snapshot;
    let bytes: usize = files.iter().map(|file| file.source.len()).sum();
    println!("{}", serde_json::json!({
        "elapsed_seconds": elapsed, "source_bytes": bytes,
        "counts": envelope.counts, "coverage": envelope.coverage,
        "snapshot_id": envelope.snapshot_id, "skipped": envelope.skipped,
        "truncation_reasons": envelope.truncation_reasons,
    }));
}
```

### prepare.py

SHA-256: `f3f0b6d9cf33e89a646671233d1509001005aa4b7fa72ef316e3a4cafd40234d`

```python
#!/usr/bin/env python3
"""Build a scratch discovery executable against the unchanged release graph."""
import json
import pathlib
import subprocess
import sys

project, scratch = (pathlib.Path(path).resolve() for path in sys.argv[1:3])
source = (scratch / "discover.rs.in").read_text().replace("@SCOPE_PATH@", str(project / "src/scope.rs"))
(scratch / "discover.rs").write_text(source)
deps = project / "target/release/deps"
# Ask Cargo which artifacts belong to this build; old feature variants may coexist.
built = subprocess.run(["cargo", "build", "--release", "--locked", "--message-format=json"], cwd=project, check=True, capture_output=True, text=True, timeout=300)
artifacts = {}
for line in built.stdout.splitlines():
    event = json.loads(line)
    if event["reason"] == "compiler-artifact":
        libraries = [name for name in event["filenames"] if name.endswith(".rlib")]
        if libraries:
            assert len(libraries) == 1
            artifacts[event["target"]["name"]] = libraries[0]
command = ["rustc", "--edition=2024", "-O", str(scratch / "discover.rs"), "-o", str(scratch / "discover"), "-L", f"dependency={deps}"]
for crate in ["rust_sitter_mcp", "ignore", "serde_json"]:
    command += ["--extern", f"{crate}={artifacts[crate]}"]
print(" ".join(command), flush=True)
subprocess.run(command, cwd=project, check=True, timeout=120)
```

### measure.py

SHA-256: `8d8ca46994e89a67fa0367dad0e70849b59da484dea4357be289820c2ba84f5f`

```python
#!/usr/bin/env python3
"""One fixed ten-run protocol: fresh process and warmed source bytes every time."""
import datetime
import hashlib
import json
import os
import pathlib
import selectors
import signal
import subprocess
import sys
import time

root, binary, discovery, output = map(pathlib.Path, sys.argv[1:5])
root = root.resolve()
output.mkdir()  # Refuse overwrite/retry-until-lucky.
files = sorted(root.glob("src/shard_*/*.rs"))
assert len(files) == 10_000
manifest = hashlib.sha256()
source_bytes = 0
sites = 0
for path in files:
    content = path.read_bytes()
    content.decode("utf-8")
    assert b"\0" not in content and len(content) <= 1024 * 1024
    source_bytes += len(content)
    sites += content.count(b".unwrap()")
    for field in [path.relative_to(root).as_posix().encode(), content]:
        manifest.update(len(field).to_bytes(8, "big"))
        manifest.update(field)
excluded_count = sum(1 for folder in ["target/noise", "src/target/noise", ".git/benchmark-noise"] for _ in (root / folder).glob("*.rs"))
assert source_bytes == 209_715_200 and sites == 50 and excluded_count == 50_000
summary = {
    "eligible_files": len(files), "eligible_bytes": source_bytes,
    "expected_matches": sites, "hard_excluded_rs_files": excluded_count,
    "eligible_sha256": manifest.hexdigest(), "runs": [],
}
(output / "fixture-verified.json").write_text(json.dumps({k: v for k, v in summary.items() if k != "runs"}, indent=2) + "\n")
arguments = {
    "repo_path": str(root), "pattern": "$a.unwrap()",
    "context": {"before_lines": 2, "after_lines": 2}, "page_size": 100,
    "limits": {
        "max_file_bytes": 1048576, "max_files": 20000,
        "max_source_bytes": 268435456, "time_budget_ms": 60000,
        "query_state_limit": 4096, "response_bytes": 2097152,
        "diagnostic_count": 64, "text_bytes": 8192,
    },
}
(output / "arguments.json").write_text(json.dumps(arguments, indent=2) + "\n")

def warm():
    # Warm reads/metadata, never a server/parse/query warmup.
    for path in files:
        path.stat()
        path.read_bytes()
    for directory in [root, root / "src", root / "target", root / ".git"]:
        list(directory.iterdir())


def terminate(child):
    if child.poll() is None:
        os.killpg(child.pid, signal.SIGKILL)
    child.wait(timeout=10)


def check_common(value):
    assert value["coverage"] == {"scan_exhausted": True, "eligible_scan_complete": True, "scope_exhaustive": True}, value["coverage"]
    for key in ["discovered_files", "scanned_files", "eligible_files"]:
        assert value["counts"][key] == 10000, value["counts"]
    assert value["snapshot_id"] and not value["truncation_reasons"]
    assert all(record["count"] == 0 for record in value["skipped"].values()), value["skipped"]


class Rpc:
    def __init__(self, child):
        self.child = child
        self.buffer = bytearray()
        self.selector = selectors.DefaultSelector()
        self.selector.register(child.stdout, selectors.EVENT_READ)

    def send(self, message):
        self.child.stdin.write(json.dumps(message, separators=(",", ":")).encode() + b"\n")
        self.child.stdin.flush()

    def receive(self, timeout):
        deadline = time.monotonic() + timeout
        while b"\n" not in self.buffer:
            remaining = deadline - time.monotonic()
            if remaining <= 0 or not self.selector.select(remaining):
                raise TimeoutError("stdio response deadline")
            data = os.read(self.child.stdout.fileno(), 65536)
            if not data:
                raise EOFError("server closed stdout before response")
            self.buffer.extend(data)
            if len(self.buffer) > 16 * 1024 * 1024:
                raise ValueError("oversized protocol output")
        line, _, rest = self.buffer.partition(b"\n")
        self.buffer = bytearray(rest)
        return bytes(line)


def measure_discovery(run):
    stderr_path = output / f"discovery-{run:02}.stderr"
    with stderr_path.open("wb") as log:
        child = subprocess.Popen([str(discovery), str(root)], stdout=subprocess.PIPE, stderr=log, start_new_session=True)
        try:
            raw, _ = child.communicate(timeout=75)
            assert child.returncode == 0, f"discovery exit {child.returncode}"
        finally:
            terminate(child)
    (output / f"discovery-{run:02}.json").write_bytes(raw)
    value = json.loads(raw)
    check_common(value)
    assert value["source_bytes"] == 209715200
    return value["elapsed_seconds"], {"counts": value["counts"], "source_bytes": value["source_bytes"]}


def measure_full(run):
    stderr_path = output / f"full-{run:02}.stderr"
    with stderr_path.open("wb") as log:
        child = subprocess.Popen([str(binary)], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=log, start_new_session=True)
        rpc = Rpc(child)
        try:
            rpc.send({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
                "protocolVersion": "2025-06-18", "capabilities": {},
                "clientInfo": {"name": "throughput-benchmark", "version": "1"},
            }})
            initialized = json.loads(rpc.receive(15))
            assert initialized["id"] == 1 and "result" in initialized, initialized
            version = initialized["result"]["serverInfo"]["version"]
            assert version == "0.1.0 (6697d4f6432e)", version
            rpc.send({"jsonrpc": "2.0", "method": "notifications/initialized"})
            request = {"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {"name": "search", "arguments": arguments}}
            start = time.perf_counter()
            rpc.send(request)
            raw = rpc.receive(75)
            elapsed = time.perf_counter() - start
            (output / f"full-{run:02}.json").write_bytes(raw + b"\n")
            response = json.loads(raw)
            assert response["id"] == 2 and "result" in response
            assert not response["result"].get("isError", False), response
            value = response["result"]["structuredContent"]
            check_common(value)
            assert value["status"] == "complete" and value["error"] is None
            assert value["counts"]["matched_files"] == 10000, value["counts"]
            assert value["counts"]["total_matches"] == 50 and value["counts"]["total_is_exact"] is True
            assert value["counts"]["returned_matches"] == len(value["matches"]) == 50
            assert value["next_cursor"] is None and value["has_more"] is False
            assert not value["diagnostics"] and value["diagnostics_omitted"] == 0
            assert len(raw) <= arguments["limits"]["response_bytes"]
            assert json.loads(response["result"]["content"][0]["text"]) == value
            child.stdin.close()
            assert child.wait(timeout=10) == 0
            return elapsed, {"counts": value["counts"], "wire_bytes": len(raw), "version": version}
        finally:
            rpc.selector.close()
            terminate(child)


for run in range(1, 11):
    for name, measure, target in [("discovery", measure_discovery, 5), ("full", measure_full, 30)]:
        warm()
        record = {"run": run, "measurement": name, "started_at": datetime.datetime.now(datetime.timezone.utc).isoformat(), "target_seconds": target}
        start = time.perf_counter()
        try:
            elapsed, detail = measure(run)
            record.update(elapsed_seconds=elapsed, valid=True, target_met=elapsed <= target, detail=detail)
        except Exception as error:
            record.update(valid=False, target_met=False, elapsed_seconds=None, observed_wall_seconds=time.perf_counter() - start, error=f"{type(error).__name__}: {error}")
        summary["runs"].append(record)
        (output / "results.json").write_text(json.dumps(summary, indent=2) + "\n")
        print(json.dumps(record), flush=True)
summary["verdicts"] = {
    name: {"met": sum(r["target_met"] for r in summary["runs"] if r["measurement"] == name), "of": 10}
    for name in ["discovery", "full"]
}
(output / "results.json").write_text(json.dumps(summary, indent=2) + "\n")
print(json.dumps(summary["verdicts"]), flush=True)
```
