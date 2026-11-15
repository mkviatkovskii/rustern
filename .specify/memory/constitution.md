# Rustern Constitution

Rustern is a FUSE client for TernFS written in Rust. It exists to be more correct,
more robust, and faster than the upstream Go client (`ternfuse`) while remaining fully
interoperable with an unmodified TernFS cluster.

When principles conflict, the order of precedence is: correctness, then robustness and
security, then performance, then convenience. A faster client that can return wrong
bytes or take down a mount is a regression, whatever the benchmark says.

## Core Principles

### I. Correctness Above All (NON-NEGOTIABLE)

- Every byte returned to the kernel MUST have been verified by CRC32C. Block data is
  verified against the per-page CRCs the block service stores (anchored to the metadata
  block CRC, which the block service checks when the block is written); inline data,
  any read covering a whole span, and bytes produced by erasure-code reconstruction are
  verified against the CRC32C recorded in TernFS metadata.
- A write MUST NOT be reported as successful until its spans are stored and certified.
  Errors discovered at flush, close, or link time MUST reach the caller as an errno.
- No silent degradation. Every failure maps to a defined errno; no `Result` is
  discarded on a data or metadata path; placeholder values (for example a zero size or
  mtime in place of a failed stat) MUST NOT be served or cached.
- Anything placed in a kernel cache (attributes, directory entries, pages) MUST have a
  written validity rule. Immutability of linked files is the only justification for
  long-lived caching, and the spec must say where it applies.
- The file write lifecycle is an explicit state machine (transient, linked, failed)
  expressed as an exhaustively matched type, not as flags.

Rationale: users treat a mounted filesystem as ground truth. Corruption and silent data
loss are the only failures that cannot be fixed after the fact.

### II. Protocol Fidelity with Upstream

- Wire formats MUST be bit-exact with upstream TernFS at the pinned commit. Message
  types and their codecs are generated from the upstream definitions; they are never
  transcribed by hand.
- Span, stripe and cell layout, the Reed-Solomon encoding, and CRC32C arithmetic
  (including append, xor and zero-extend combination) MUST be bit-identical to
  upstream. This is proven by golden vectors generated from the upstream C and Go
  code, not assumed.
- Interoperability is a release gate: files written by Rustern MUST be readable by
  `ternfuse` and the kernel module and vice versa, including degraded reads with
  missing or corrupt blocks.
- `ternfuse` at the pinned commit is the behavioural reference for the operation
  surface and errno mapping. Parity is with intended behaviour: defects are not
  ported. Every deliberate difference is recorded in `docs/deviations.md` with its
  rationale and a test.
- Moving the upstream pin is an explicit, reviewed change that reruns the full
  interop suite.

Rationale: the cluster, not this client, defines the truth. A client that is subtly
incompatible is worse than one that is slow.

### III. The Mount Must Not Die

- Request paths MUST NOT panic. `unwrap`, `expect`, `panic!`, unchecked indexing and
  unchecked arithmetic are denied by lint outside tests and process start-up. An
  unexpected value from the kernel or the network is an error, never a crash.
- All input is untrusted: FUSE requests and every network response have their lengths,
  counts, offsets and enum tags validated before use.
- Everything is bounded: memory per handle and in total, queue depths, concurrency,
  retries. No allocation size may be derived from unvalidated caller or server input
  (sparse writes and extending truncates stream zeros; they do not allocate them).
- Every network operation has a timeout, jittered exponential backoff and an overall
  deadline. Mutations that are retried after a lost response MUST go through
  idempotency reconciliation, as the upstream client does for rename, unlink and link.
- Partial failure is the normal case: a dead or slow block service, a shard leader
  change, or an unreachable registry MUST be handled by failover, reconstruction or
  cached addresses before an error is surfaced.
- Cancellation is safe: an interrupted request or dropped future leaves no leaked
  transient file, buffer or lock. Async functions on the write path document their
  cancel-safety.
- If the process must exit, it unmounts first. A dangling "transport endpoint is not
  connected" mount is a release blocker.

Rationale: a FUSE daemon is a single point of failure for every process using the
mount. One bad packet must cost one request, not the mount.

### IV. Secure by Default

- `unsafe` is forbidden (`#![forbid(unsafe_code)]`) in every crate except explicitly
  designated low-level crates (SIMD kernels, syscall shims). Each `unsafe` block there
  carries a `SAFETY:` justification, is exercised under Miri or sanitizers, and needs
  two reviewers.
- Least privilege: the daemon runs unprivileged wherever the kernel allows. Features
  that need elevated privileges (eBPF close tracking, direct mount, `allow_other`) are
  opt-in, isolated, and drop capabilities as soon as set-up completes.
- Mounts default to `nosuid,nodev`. The TernFS protocol is permissionless; Rustern
  MUST NOT imply access control it cannot enforce, and the uid, gid and mode it
  presents are explicit, documented configuration.
- Cookies, certificates and block proofs never appear in logs or metrics.
- Supply chain: `Cargo.lock` is committed; `cargo-deny` (advisories, licences, bans,
  sources) gates CI; a new dependency needs a written justification and minimal
  feature flags.
- Every decoder that touches network or kernel bytes has a fuzz target.

Rationale: the client parses attacker-reachable input and often runs on shared
machines. The existing client was passed over for security and robustness reasons;
this project must not repeat that.

### V. Performance Is a Measured Requirement

- Every feature spec states numeric targets: throughput, p50 and p99 latency, CPU per
  GiB transferred, and memory ceiling. The baseline is `ternfuse` at the pinned commit
  on the same hardware and cluster. Rustern MUST meet or beat the baseline on every
  tracked benchmark before 1.0 and SHOULD aim to approach the kernel module.
- Benchmarks live in the repository (micro-benchmarks for codecs, CRC and
  Reed-Solomon; `fio`-style macro-benchmarks against a local cluster) and run in CI.
  A regression of more than 5% on a tracked benchmark blocks the merge unless
  explicitly approved and recorded.
- Hot-path rules: no steady-state heap allocation per request (pooled buffers); no
  copy of file data beyond the one into the kernel (zero-copy decoding, vectored I/O);
  no lock held across an `.await`; syscalls batched where the kernel offers it.
- SIMD: a build MAY fix a baseline target CPU (`-C target-cpu`) for the whole binary,
  only if `docs/deployment.md` states it as a hard requirement; raising it is a
  breaking deployment change and is called out in the release notes. Instructions
  above the baseline are selected by runtime CPU detection, never by per-crate or
  per-function `-march`/`target_feature` without detection. Every SIMD kernel has a
  tested scalar fallback, used on targets without the baseline (other architectures,
  generic x86-64 builds).
- Immutability is exploited: content of linked files may be cached and read ahead
  aggressively, within the validity rules required by Principle I.
- No optimisation without a profile or benchmark that motivates it, and none that
  weakens Principles I to IV.

Rationale: "as fast as possible" is only meaningful as numbers that CI can defend.

### VI. Parallel by Design

- FUSE requests are dispatched on multiple threads. There is no global lock on any
  request path; shared state is sharded or lock-free, with per-inode or per-handle
  granularity.
- I/O is concurrent by default: the blocks of a stripe are fetched or written in
  parallel; independent spans are pipelined; metadata fan-out (for example the stats
  behind readdirplus) is concurrent. All fan-out is bounded and applies backpressure.
- Duplicate work is coalesced: concurrent requests for the same span or attribute
  share one in-flight fetch.
- CPU-bound work (Reed-Solomon, CRC over large buffers) MUST NOT stall I/O reactor
  threads.
- Concurrency is verified, not hoped for: lock ordering is documented; every custom
  synchronisation primitive and state machine has model tests (loom or shuttle);
  stress tests and ThreadSanitizer run in CI.
- Scalability is tracked: benchmarks run at 1, 2, 4 ... N threads, and a scalability
  regression counts as a performance regression.

Rationale: the workload is many processes reading large files at once. Throughput
that does not scale with cores is left on the table.

### VII. Test-First, Verified Against Reality

- Tests are written and seen to fail before the implementation, following the
  spec-kit flow.
- Required layers: unit tests; property tests for codecs, span layout arithmetic,
  Reed-Solomon and CRC algebra; fuzzing for every decoder with the corpus replayed in
  CI; golden vectors from upstream; differential tests against the Go client;
  integration tests against a real local cluster (upstream `ternrun`), including the
  upstream `terntests` filesystem tests run against a Rustern mount; a POSIX
  conformance suite with documented exclusions.
- Fault injection is mandatory for the read and write paths: packet loss, duplication
  and reordering; block service death; corrupted blocks; slow shards; daemon
  `SIGKILL` mid-write.
- Every bug fix lands with a test that reproduces the bug.

Rationale: a storage client is trusted only as far as its failure modes have been
exercised on purpose.

### VIII. Observable and Operable

- Structured logging throughout; per-kind counters and latency histograms for FUSE,
  shard, CDC and block requests, exportable and dumpable on signal as the Go client
  does. Conditions that need a human raise an alert-level event.
- Nothing is logged at info level or above on a hot path, and disabled log levels
  cost nothing.
- Every non-trivial errno returned to a caller is traceable to a log line or metric.
- The command line stays compatible with `ternfuse` flags where the semantics match,
  so Rustern can be a drop-in replacement.

Rationale: a filesystem that cannot explain its own latency or errors cannot be run
in production.

## Technology Baseline

Dependencies are chosen deliberately. A crate qualifies when it is actively maintained
(or credibly finished), widely used in production, licence-compatible, and does not
duplicate something already in the tree. The baseline below is the default; replacing
an entry needs a short decision record with benchmark or risk evidence. Versions are
pinned in `Cargo.lock`, not here. Baseline last reviewed 2026-11-14.

| Concern | Baseline | Notes |
|---|---|---|
| Toolchain | Stable Rust, edition 2024, pinned via `rust-toolchain.toml` | Linux only; x86_64 is the primary target |
| FUSE | `fuser` | Multiple event loops per session, device-fd cloning, kernel cache notifications. Kept behind an internal trait so `fuse3` or FUSE-over-io_uring can be evaluated by benchmark |
| Async runtime | `tokio` (multi-thread) | io_uring only through the maintained `io-uring` crate, behind the same I/O trait, and only if benchmarks justify it |
| Buffers and wire | `bytes`, `zerocopy`, in-house buffer pool | Zero-copy decoding of generated message types |
| CRC32C | `crc32c` or `crc-fast` for the accelerated core (choose by benchmark) | Combination algebra (append, xor, zero-extend) from the chosen crate where it provides it, otherwise in-house; either way proven bit-identical by golden vectors from upstream |
| Reed-Solomon | In-house port of the upstream `rs` implementation | GF(2^8) with the Rijndael polynomial, systematic, first parity equals XOR. General-purpose RS crates are not assumed compatible; use one only if golden vectors prove bit-identity |
| Concurrency | `parking_lot`, `arc-swap`, `crossbeam`; `papaya` or `dashmap` for concurrent maps; `quick_cache` or `moka` for caches | Final picks by benchmark under contention |
| Errors | `thiserror` in libraries; `anyhow` only in `main` | Typed errors map to errno in exactly one place |
| CLI, logs, metrics | `clap`, `tracing`, `metrics` with a Prometheus exporter, `hdrhistogram` | |
| Syscalls | `rustix` first, then `socket2`, `libc` only when unavoidable | Batched UDP send and receive for metadata traffic |
| eBPF (if close tracking is kept) | `aya` or `libbpf-rs` | Optional, privileged, isolated per Principle IV |
| Allocator | `mimalloc` or `tikv-jemallocator` | Selected by benchmark; must be swappable |
| Verification | `proptest`, `cargo-fuzz`, `loom`, `shuttle`, `turmoil`, `criterion`, `cargo-nextest`, Miri, `cargo-deny` | |

Architecture constraints:

- The workspace is library-first. Protocol types, core algorithms (CRC, Reed-Solomon,
  parity), the async TernFS client, and the FUSE adapter are separate crates. The
  client library MUST be usable and testable without a mount.
- The FUSE adapter contains no protocol logic; the client library contains no FUSE
  types.

## Reference Implementation and Scope

- Upstream pin: `XTXMarkets/ternfs` at commit `5550891` (2026-11-11). Behavioural
  reference: `go/ternfuse`. Protocol and algorithm reference: `go/msgs`, `go/client`,
  `go/core`.
- Version 1 scope is functional parity with `ternfuse`: lookup, getattr, setattr
  (times; extend-only truncate while a file is being written), create, `O_TMPFILE`
  with link, append-only write, flush and close leading to link, open and read with
  erasure-coded recovery, mkdir, rmdir, unlink (honouring snapshot policy for
  immediate deletion), rename (same directory, cross-directory file, cross-directory
  directory), symlink and readlink, and directory reading with seek and readdirplus.
  Extended attributes are out of scope.
- TernFS semantics are constraints, not defects: files are immutable once linked,
  writes are sequential, and the protocol carries no permissions.
- Where `ternfuse` behaviour is ambiguous, depends on privileged machinery (notably
  detecting a real `close()` before linking a file), or is defective, the feature spec
  MUST make an explicit decision and record it in `docs/deviations.md`. Nothing is
  inherited by default.
- Licensing: Rustern is licensed `MIT OR Apache-2.0` (`LICENSE-MIT`,
  `LICENSE-APACHE`). Code written for this project is offered under both.
- Upstream licences differ by directory: `go/ternfuse` is GPL-2.0-or-later, while
  `go/msgs`, `go/client`, `go/core` and `go/bincodegen` are Apache-2.0 WITH
  LLVM-exception. `ternfuse` is a behavioural reference only: specs are derived from
  it, code is not, and its code MUST NOT be translated or copied.
- Code MAY be derived from the Apache-licensed upstream directories (translated or
  adapted code, copied comments, codecs generated from upstream definitions, a
  generator adapted from `bincodegen`). Every derived file carries the upstream
  copyright line and `SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception`; a
  `NOTICE` file lists the upstream sources; a crate containing derived files declares
  `(MIT OR Apache-2.0) AND Apache-2.0 WITH LLVM-exception`. Protocol facts (wire
  layouts, message ids, algorithms) and golden vectors produced by running upstream
  code are not derived code.

## Development Workflow and Quality Gates

- Work follows the spec-kit sequence: constitution, specify, clarify, plan, tasks,
  implement. Every plan contains a Constitution Check that addresses Principles I to
  VIII individually; a violation is either removed or justified in Complexity
  Tracking with a simpler alternative that was rejected and why.
- Merge gates, all automated: `cargo fmt --check`; `clippy` with warnings denied and
  the panic-related lints from Principle III enabled; unit, property and integration
  tests; fuzz corpus replay; Miri for crates that contain `unsafe`; `cargo-deny`;
  tracked benchmarks within budget.
- Review: one reviewer for ordinary changes; two for `unsafe`, wire-format code, and
  anything on the write path.
- Release gates: full interop matrix against the pinned upstream (Go client and
  kernel module); fault-injection suite green; a soak test under mixed load with no
  leak, hang or corruption; no open correctness or data-loss bug.
- Scope discipline: anything beyond parity with the reference needs its own spec.
  Complexity is added only when a measured requirement demands it.

## Governance

This constitution supersedes other project conventions. Where a guideline, template
or habit conflicts with it, the constitution wins or is amended.

- Amendments are made by pull request stating the change, the rationale, and the
  effect on existing specs, plans and templates. Dependent templates are updated in
  the same change.
- Versioning is semantic: MAJOR for removing or redefining a principle, MINOR for
  adding a principle or materially expanding guidance, PATCH for clarifications.
- Compliance is checked at every plan (Constitution Check) and every pull request.
  Exceptions are time-boxed, recorded in Complexity Tracking, and reviewed at the
  next release.
- Day-to-day development guidance for humans and coding agents lives in the
  repository's agent guidance file and MUST stay consistent with this document.

**Version**: 1.1.0 | **Ratified**: 2026-11-14 | **Last Amended**: 2026-11-15
