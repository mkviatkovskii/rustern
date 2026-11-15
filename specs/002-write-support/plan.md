# Implementation Plan: Write Support

**Branch**: `002-write-support` (no git branch; tracked via `.specify/feature.json`) | **Date**: 2026-11-15 | **Spec**: [spec.md](spec.md)

**Input**: Feature specification from `/specs/002-write-support/spec.md`

<!--
  Rustern project override of the spec-kit plan template. Technical Context
  defaults come from the constitution's Technology Baseline; the Constitution
  Check is one row per principle, as Development Workflow and Quality Gates
  requires.
-->

## Summary

Rustern becomes a read-write TernFS mount with ternfuse's operation surface. Covered:

- create, sequential write, flush, fsync, extend-only truncate and `O_TMPFILE` with `linkat`;
- mkdir, rmdir, unlink, rename and symlink;
- `utimensat`.

Two opt-in options exist: overwrite with `O_TRUNC`, and matching-attribute chmod/chown.

**File lifecycle.** Each file being written is owned by one actor task that runs a sans-IO,
exhaustively matched state machine: Open, Publishing, Published, Failed or Abandoned. Spans
are filled in pooled buffers while the previous span is stored.

**Span storage.** Spans are laid out, CRC'd and erasure-coded bit-identically to
`go/client/span.go`, using an in-house port of upstream `go/core/rs` (`rustern-rs`) whose safe
AVX2, AVX-512 and GFNI kernels are chosen by CPU detection at run time.
Each span is stored with `AddSpanInitiate`, parallel `WriteBlock` and `AddSpanCertify`, with
upstream's blacklist-and-retry.

**Publish point.** Every flush stores and certifies, and release publishes with `LinkFile`. A
pending-publish registry makes same-mount lookups wait for a publish that is about to happen
(DEV-007). Namespace mutations use ternfuse's requests plus upstream's idempotency
reconciliation.

**Gating.** A `WriteAccess` token keeps `-o ro` mounts unable to send mutations. Write
throughput and CPU are gated against the ternfuse baseline measured on the local cluster:

- seq-write: 90, 218 and 295 MiB/s at 1, 4 and 16 writers;
- 2.5–2.7 CPU-s/GiB.

## Technical Context

**Language/Version**: Rust stable 1.100.0, edition 2024, pinned via `rust-toolchain.toml`.
x86-64 builds use `target-cpu=x86-64-v3` (`.cargo/config.toml`), the baseline documented in `docs/deployment.md` (Principle V).

**Primary Dependencies**: `fuser` 0.18, `tokio` 1 (mpsc, oneshot, watch, Semaphore,
`spawn_blocking`), `bytes`, `futures-util`, `dashmap` 6 (pending-publish registry), `deadpool`
0.13 (block-service TCP pools, now also for `WriteBlock`), `moka` 0.12 (directory policy
cache), `backon`, `fastrand`, `crc-fast` 1.10 (cell CRCs; combination from `rustern-core::crc`),
`rustix` 1, `thiserror` 2, `clap` 4, `bytesize`, `tracing`, `metrics` and
`metrics-exporter-prometheus` 0.18.

New: `bytemuck` (safe SIMD loads) and `fearless_simd` 1.2 (safe SIMD dispatch), vendored with a
patch in `third_party/` (CT-9); see Dependency Justification. Also a pinned `fuser` patch for
`FUSE_TMPFILE` (CT-8).

**Storage**: TernFS cluster at upstream pin `5550891`. Protocol reference: `go/msgs`,
`go/client` (`span.go`, `blocksreq.go`, `shardreq.go`, `cdcreq.go`, `metadatareq.go`) and
`go/core/rs`. Shard semantics come from `cpp/shard/ShardDB.cpp`: `_checkSpanBody`,
`_applyLinkFile` and `_createCurrentEdge`.

**Testing**: `cargo-nextest`; `proptest` (layout, CRC algebra, RS, write-sequence splitting);
goldens from `tools/goldens` (new: RS parity, `AddSpanInitiateReq` bodies); `cargo-fuzz` with
corpus replay for the new response decoders; an exhaustive interleaving model test of the
writer state machine (research W13); `criterion` (`rs`, `span_write` benches).

Cluster suites driven by `scripts/gate.sh`:

- write lifecycle;
- namespace differential;
- write-corpus differential in both directions, including the kernel module when loaded;
- write fault injection;
- concurrency stress;
- `terntests`;
- the `pjdfstest` subset (`scripts/posix.sh`);
- the tool script (`scripts/tools.sh`).

`fio` for macro benchmarks.

**Target Platform**: Linux, x86_64 primary (x86-64-v3); RS picks AVX2, AVX-512 or GFNI kernels
at run time, and other targets use the scalar kernel.

**Project Type**: Library-first Cargo workspace with a FUSE daemon binary.

**Crates Touched**:

- `rustern-rs` (new, done): RS port with its SIMD kernels.
- `rustern-core`: write-side span layout, CRC zero-extend use.
- `rustern-msgs`: fuzz targets for the write responses; the read-only list in `tools/rsgen`
  becomes a classification, with no codec change.
- `rustern-client`: policies, writer, span store, block writes, CDC client, mutations,
  pending registry, `WriteAccess`.
- `rustern-fuse`: new `AsyncFilesystem` methods, write handles, errno rows, invalidation.
- `rustern`: flags.

The FUSE adapter gains no protocol logic: it maps FUSE requests to client calls and replies.
The client gains no FUSE types: offsets, names, flags and errnos cross the boundary as plain
types, as in 001.

**Performance Goals** (spec Performance Targets; baseline from research W14):

| Metric | Target | ternfuse baseline | Benchmark |
|---|---|---|---|
| seq-write throughput, RS directory, 1/2/4/8/16 writers | ≥ baseline at each count; 1→4 ≥ 2x | 90 / – / 218 / – / 295 MiB/s (2 and 8 measured in the first bench task) | `seq-write` |
| seq-write throughput, mirrored directory | ≥ baseline | measured in the first bench task | `seq-write-mirror` |
| small files, 16 writers, 4 KiB | ≥ baseline | ≈ 11 files/s | `small-files` |
| close (publish) of 1 MiB, mkdir, rename, unlink: p50/p99 | p99 ≤ baseline | mkdir mean 636 ms; the rest measured in the first bench task | `meta-write-latency` |
| read p99 with 16 readers + 16 writers | ≤ 1.5x read-only p99 | 001 read-only p99 from `benchmarks.md` | `mixed-rw` |
| CPU per GiB written, 4 writers | ≤ 2.46 CPU-s/GiB | 2.46 | `seq-write` |
| Write memory, 16 writers | within `--write-memory-limit`; peak RSS recorded | — | `seq-write` |
| RS encode (micro) | ≥ 5 GiB/s of data per core at D10 P4 (`Avx2`); not slower than upstream at the same level | upstream C via `tools/rsbench`: D10 P4 parity 900 µs (AVX2), 565 µs (GFNI) per 10 MiB stripe; ours 732 / 425 µs (research W2) | `rs` |

These targets block merges once `scripts/bench.sh write` exists (spec; CT-4 is closed for
writes).

**Constraints**:

- No allocation per write request: span buffers are pooled and taken under a byte semaphore.
- Zeros are never materialised beyond one static page (W4).
- No lock held across `.await`: the actor owns the state.
- CRC and RS run on `spawn_blocking`.
- The overall block deadline is 60 s (DEV-006).
- 5 attempts per span.
- Shard and CDC deadlines as in 001.
- Write memory: 1 GiB by default, read memory 1 GiB (about 2 GiB per client).
- `max_write` 1 MiB.
- Unprivileged.

**Scale/Scope**:

- File sizes from 0 to 4 GiB in the corpus; streaming files of any size.
- Up to 16 concurrent writers benchmarked; up to 50 files streaming at full pipeline depth
  with the default memory limit and 10 MiB spans.
- Directories as in 001.

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-checked after Phase 1 design (below the table).*

| Principle | Status | Evidence in this plan | Complexity ref |
|-----------|--------|-----------------------|----------------|
| I. Correctness Above All | PASS | Success to `flush`, `fsync` and `linkat` only after certification; publish only via `LinkFile` of a fully certified writer (contracts/filesystem.md guarantees). Errors are sticky and reach `write`, `fsync`, `close` and `linkat`; publish failure at release is logged and counted (FR-005, the only case with no caller). The lifecycle is the exhaustive `WriteState` enum (data-model.md). Kernel cache rules for writers: zero attr validity, `DIRECT_IO`, invalidation on every mutation (W10). Read of a being-written file: EOPNOTSUPP | - |
| II. Protocol Fidelity | PASS | Messages are generated (`rustern-msgs`, already present). Layout, CRCs and RS are ports of `go/client/span.go` and `go/core/rs`, proven by goldens from upstream code and checked by the shard's `_checkSpanBody`. Interop gate: Rustern-written corpus read by ternfuse and the kernel module, including degraded reads; ternfuse-written corpus read by Rustern. Deviations DEV-007..013 go to `docs/deviations.md` with tests | - |
| III. The Mount Must Not Die | PASS with exceptions | Panic lints unchanged. Names, offsets, sizes and policies validated. Every bound explicit: write semaphore, spans per file, mailbox depth, attempts, deadlines. Gaps stream zeros (W4). Reconciliation for link, rename, unlink and rmdir as upstream, plus mkdir (W11). Block failover by blacklist and retry (W6). Cancel-safety: dropped callers drop only their `oneshot`, and permits and buffers are RAII (data-model.md). Unmount with a 5 s grace (contracts/cli.md). Still open from 001: no read-path reconstruction, so a degraded read of an RS file gets EIO (CT-2); interrupts declined (CT-3) | CT-2, CT-3 |
| IV. Secure by Default | PASS with exception | No `unsafe` in our crates: the SIMD kernels are safe Rust (`fearless_simd::kernel!` + `bytemuck`). The vendored fearless_simd patch adds detection-guarded `unsafe` token construction (CT-9). Unprivileged; no close tracker; `nosuid,nodev` kept; `allow_other` explicit. Cookies, certificates and proofs are excluded from Debug output (`tools/rsgen` `secretFields`) and from logs and metrics. New dependency justified below. Fuzz targets for every new response decoder. The `fuser` git patch is allowed in `cargo-deny` sources for one URL only | CT-8, CT-9 |
| V. Performance Is a Measured Requirement | PASS | Numeric targets with baseline numbers and named benchmarks (above). Pooled span buffers and one copy from the FUSE buffer; vectored block writes from the span buffer; no lock across await; RS and CRC benchmarked with criterion, RS also against upstream (`tools/rsbench`). The x86-64-v3 baseline is documented in `docs/deployment.md`; RS kernels above it (GFNI, AVX-512) are selected by runtime detection, each with a tested scalar fallback | — |
| VI. Parallel by Design | PASS | Per-file actors, no global lock (pending registry is a `DashMap`). Blocks of a span written in parallel; next span filled and coded while the previous one stores; files fully concurrent; fan-out bounded by the semaphore and spans per file. CRC and RS on `spawn_blocking`. Exhaustive interleaving model test of the writer state machine (W13); stress tests at 1–16 writers. TSan and the UDP-table model test still deferred from 001 | CT-6 |
| VII. Test-First, Verified Against Reality | PASS | Each story's tasks start with failing tests. All spec Testing Requirements layers: unit, property, fuzz + replay, goldens, differential both ways, cluster integration, `terntests`, POSIX subset, write fault injection (packet loss, duplication and reordering on metadata and block traffic; block service death and SIGSTOP; slow shards during publish; writer and daemon SIGKILL mid-write; lost responses to every mutation), model tests, interop. CT-5 from 001 is closed | - |
| VIII. Observable and Operable | PASS | New counters and histograms (contracts/cli.md) in the SIGUSR1 dump and the exporter. Each errno logged at debug with op, inode and cause, and counted by op and errno. Publish failures at warn; cluster policy misconfiguration as an alert. ternfuse flags unchanged; `-close-tracker-object` ignored with a warning; new flags are Rustern-only | - |
| Technology Baseline and Architecture constraints | PASS | RS is the baseline's in-house port, in its own crate `rustern-rs` (SIMD kernels kept apart from the protocol crates); `bytemuck` and `fearless_simd` are the new dependencies. The other crates keep their split; the client is testable without a mount (writer, model and span store tests use the client API against the cluster). The FUSE adapter stays protocol-free | - |
| Reference Implementation and Scope | PASS | Scope is v1 parity items: create, `O_TMPFILE`+link, append-only write, flush/close→link, setattr times and extend-only truncate, mkdir, rmdir, unlink with snapshot policy, renames, symlink. The opt-ins (DEV-010, DEV-011) are spec-decided. Derived code (RS port, span layout port) from Apache-2.0 WITH LLVM-exception directories carries SPDX lines and `NOTICE` entries; `rustern-rs` and `rustern-core` declare the combined licence. The vendored fearless_simd (Apache-2.0 OR MIT) has its own `NOTICE` entry. ternfuse is a behavioural reference only, not copied | - |

**Post-design re-check (after Phase 1)**: unchanged. The data model keeps the lifecycle as one
enum with a transition table. The contracts add no new privileged path. The two deviations
found during design are recorded in the spec's Deviations table:

- DEV-012: unlink and rename of a name being written.
- DEV-013: errno differences (ELOOP→EINVAL, first-attempt ENOSPC, BAD_COOKIE→EIO).

DEV-014 (the immediate-deletion predicate) is added only if a task shows the cluster rule
differs from ternfuse's.

### Dependency Justification

| Crate | Purpose | Why not in-tree or an existing dependency | Features enabled | `cargo-deny` clean |
|-------|---------|-------------------------------------------|------------------|--------------------|
| `bytemuck` 1 | Safe casts between byte arrays and `__m256i`/`__m512i` in the SIMD GF(2^8) kernels (research W2) | `std::arch` loads and stores take raw pointers, and `unsafe` is forbidden outside designated crates. `zerocopy` 0.8 (already in the lock file, transitively) has no `__m256i` impls. `bytemuck` is a top-50 crate with no dependencies | `default-features = false`, `avx512_simd` (`__m512i: Pod`) | yes (MIT OR Apache-2.0 OR Zlib) |
| `fearless_simd` 1.2 (vendored, patched) | Safe runtime SIMD dispatch: `kernel!` turns a detected CPU token into the `#[target_feature]` context of the RS kernels (research W2) | A hand-written dispatcher needs `unsafe` and a designated crate. The patch adds the `Alderlake` and `SkylakeAvx512` tokens upstream lacks (CT-9) | `default-features = false`, `std` | yes (Apache-2.0 OR MIT; path source) |
| `fuser` (patched fork, same version 0.18) | Decode `FUSE_TMPFILE` (opcode 51) into a `tmpfile` callback (research W15) | No released fuser decodes it, so without it `O_TMPFILE` gets EOPNOTSUPP | as today | git source allowed for one URL (CT-8) |
| dev/tools: `pjdfstest`, `terntests`, `fio` | POSIX subset, upstream suite, benchmarks | external binaries, built or installed by scripts, not Cargo dependencies | — | n/a |

## Project Structure

### Documentation (this feature)

```text
specs/002-write-support/
├── plan.md              # This file
├── research.md          # Phase 0: decisions W1–W17
├── data-model.md        # Phase 1: lifecycle, entities, errno table
├── quickstart.md        # Phase 1: validation guide
├── contracts/
│   ├── cli.md           # delta of 001 cli.md: -o rw/ro, new flags, metrics
│   └── filesystem.md    # delta of 001 filesystem.md: every mutating FUSE op
├── checklists/requirements.md
└── tasks.md             # Phase 2 (/speckit-tasks)
```

### Source Code (repository root)

```text
Cargo.toml                         # + bytemuck, fearless_simd; [patch.crates-io] fuser (CT-8),
                                   #   fearless_simd (CT-9)
deny.toml                          # allow the fuser fork git source
NOTICE                             # + go/core/rs (done), go/client/span.go, fearless_simd (done)
third_party/                       # done: fearless_simd 1.2.0 + fearless_simd.patch, README.md
crates/
├── rustern-msgs/
│   └── fuzz/fuzz_targets/         # + write responses (AddSpanInitiate, Certify, LinkFile,
│                                  #   renames, MakeDirectory, ConstructFile, WriteBlock)
├── rustern-rs/                    # done (0007da2): derived from go/core/rs (SPDX)
│   ├── src/{lib,gf,matrix,scalar}.rs
│   ├── src/{avx2,avx2_gfni,avx512,avx512_gfni}.rs   # SIMD kernels via fearless_simd::kernel!
│   ├── tests/{goldens,props}.rs
│   └── benches/rs.rs
├── rustern-core/
│   ├── src/span.rs                # + write-side layout (derived from go/client/span.go)
│   ├── src/crc.rs                 # zero-extend/append already present; + xor for checks
│   ├── tests/span_write_golden.rs
│   └── benches/span_write.rs
├── rustern-client/
│   ├── src/policy.rs              # directory policy resolution and cache
│   ├── src/cdc.rs                 # CDC requests (MakeDirectory, renames, SoftUnlinkDirectory,
│   │                              #   CrossShardHardUnlinkFile)
│   ├── src/reconcile.rs           # checkDeletedEdge, checkNewEdgeAfterRename, mkdir
│   ├── src/mutate.rs              # namespace mutations and SetTime, behind WriteAccess
│   ├── src/write/{mod,state,actor,store,buffers,pending}.rs
│   ├── src/blocks.rs              # + WriteBlock over the existing pools
│   ├── src/metrics.rs             # + write metrics
│   └── tests/{write_model,span_store,mutate,reconcile}.rs
├── rustern-fuse/
│   ├── src/bridge.rs              # AsyncFilesystem: create, tmpfile, write, flush, fsync,
│   │                              #   setattr, link, mkdir, rmdir, unlink, rename, symlink, mknod
│   ├── src/fs.rs                  # mount mode, handles, invalidation
│   ├── src/errno.rs               # data-model.md errno table
│   └── src/attr.rs                # writer attributes, zero TTL
└── rustern/
    ├── src/cli.rs                 # -o rw/ro, --overwrite, --accept-matching-attrs,
    │                              #   --write-memory-limit, --file-write-spans, --publish-wait
    └── tests/{write_lifecycle,write_differential,namespace_differential,tmpfile,
               write_faults,write_concurrency,refuses_mutations}.rs
tools/goldens/                     # + rs.go (parity and recovery vectors, done), spanwrite.go
                                   #   (initiate bodies)
tools/rsbench/                     # done: upstream RS benchmark per CPU level (cgo)
scripts/
├── cluster.sh                     # + policies subcommand
├── bench.sh                       # + write suite (seq-write, mirror, small-files, meta, mixed)
├── gate.sh                        # + write suites, terntests
├── posix.sh                       # new: pjdfstest subset
├── tools.sh                       # new: US3 tool script on two mounts
└── soak.sh                        # + --write
docs/
├── deviations.md                  # DEV-007..DEV-013 (DEV-001 narrowed to -o ro)
├── coverage.md                    # terntests and POSIX exclusions
├── deployment.md                  # write flags and memory sizing
└── future-features.md             # retargeting pending names
```

**Structure Decision**: the five crates of 001 plus `rustern-rs`. The write state machine and
span store live in `rustern-client::write` so they are testable without a mount. RS lives in
`rustern-rs`, which keeps the SIMD kernels, their dependencies and benchmarks apart and is reusable
by degraded reads (CT-2); layout lives in `rustern-core`. Neither needs `unsafe` (W2).

## Complexity Tracking

Exceptions are time-boxed and reviewed at the next release (Governance). CT-7 (the
x86-64-v3 build flag against Principle V) was closed by constitution 1.1.0, which allows a
baseline target CPU documented in `docs/deployment.md`.

| Violation | Why Needed | Simpler Alternative Rejected Because | Expires |
|-----------|------------|-------------------------------------|---------|
| CT-2 (from 001, unchanged): Principle III, no read-path reconstruction (DEV-004) | Writing needs only the encoder. The decoder is ported here but wiring degraded reads is its own feature | Adding it here enlarges a write feature with read-path work and its own fault suite | Feature adding degraded reads (before v1) |
| CT-3 (from 001, unchanged): Principle III, interrupts declined (DEV-005) | `fuser` declines FUSE_INTERRUPT. An interrupted `write` or `close` runs to its deadline (60 s at most) | As in 001 | Before v1 |
| CT-6 (from 001, narrowed): Principle VI, no TSan; UDP pending table not model-tested | Still no `unsafe` in any crate. The writer state machine now has its model test (W13) | As in 001 | TSan: first crate with `unsafe`. UDP table: before v1 |
| CT-8: Principle IV supply chain, `fuser` taken from a pinned git fork via `[patch.crates-io]` | `FUSE_TMPFILE` is not decoded by any fuser release, and FR-008 needs it | Shipping FR-008 as EOPNOTSUPP (a v1 scope gap). Replacing fuser is out of proportion | The first fuser release with tmpfile support. If the upstream change is not accepted by the merge, FR-008 ships as EOPNOTSUPP with a deviation and the patch is dropped |
| CT-9: Principle IV, `unsafe` we wrote in a vendored crate: `third_party/fearless_simd.patch` (research W2) | Upstream fearless_simd has no token for Skylake-SP class (x86-64-v4) or Alder Lake class (GFNI without AVX-512) CPUs. The patch builds each token after `is_x86_feature_detected!` of exactly its `#[target_feature]` set (about ten `unsafe` blocks, each with a safety comment). The work machines are Skylake-SP | AVX2 only on those CPUs (GFNI is 1.7–1.9× faster; AVX-512 unused), or a designated `unsafe` crate of our own with the same blocks plus the dispatch | Upstream ships equivalent tokens (issue to be filed), or the release after this feature |
