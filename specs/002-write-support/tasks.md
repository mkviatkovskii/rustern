---

description: "Task list for feature 002: write support"
---

# Tasks: Write Support

**Input**: Design documents from `/specs/002-write-support/`

**Prerequisites**: [plan.md](plan.md), [spec.md](spec.md), [research.md](research.md),
[data-model.md](data-model.md), [contracts/cli.md](contracts/cli.md),
[contracts/filesystem.md](contracts/filesystem.md), [quickstart.md](quickstart.md)

**Tests**: REQUIRED. Constitution Principle VII and the spec's Testing Requirements: tests
are written, run and seen to fail before the implementation. Every user story phase lists
its test tasks first. Every bug fix task is preceded by a test that reproduces the bug.

**Organization**: Tasks are grouped by user story so each story can be implemented and
tested as an increment. The Reed-Solomon port (research W1, W2) was done ahead of the
task list in commit `0007da2`; its tasks are listed as done so the record is complete.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: Can run in parallel (different files, no dependencies on incomplete tasks)
- **[Story]**: US1–US4 from spec.md
- Every task names the exact file(s) it touches

## Conventions used by every task

- Everything in [001 tasks.md](../001-fuse-client-poc/tasks.md) "Conventions used by every
  task" still applies: crate dependency direction, test locations, cluster tests marked
  `#[ignore = "needs cluster: run via scripts/gate.sh"]`, the `cluster-serial` group for
  tests that stop or corrupt cluster processes (restored in a `Drop` guard), workspace
  lints, SPDX headers and `NOTICE` entries for derived files, and "run the merge gates"
  meaning `scripts/gate.sh` (full) unless `--quick` is stated.
- `rustern-rs` sits beside `rustern-core`: `rustern-rs` → `rustern-core` → `rustern-client`.
  `rustern-rs` depends on no other workspace crate.
- Every mutating client API takes `&WriteAccess` (W17). Only `rustern-client` can construct
  one, and only for a `ReadWrite` mount.
- Cluster write tests use the policy directories made by `scripts/cluster.sh policies`
  (T005): `/mirror` (Parity(1,1)), `/rs` (Parity(4,2)) and `/inline` (inline-only span
  policy), each test in its own fresh subdirectory named after the test. The reference
  mount is `$RUSTERN_TEST_DATA_DIR/fuse_loc1/mnt` as in 001.
- Upstream facts are quoted from `ternfs/` at commit `5550891`.
- Cookies, certificates, proofs and secret keys never appear in logs, metrics, test
  output or `Debug` (Principle IV); tests that touch them assert this.

---

## Phase 1: Setup (Shared Infrastructure)

**Purpose**: Dependencies, patched crates, cluster policy directories and the goldens every
later phase checks against.

- [X] T001 Create `crates/rustern-rs` as a workspace member (`license = "(MIT OR Apache-2.0) AND Apache-2.0 WITH LLVM-exception"`, `#![forbid(unsafe_code)]`) and add `bytemuck` (`default-features = false`, `avx512_simd`) and `fearless_simd` 1.2 (`default-features = false`, `std`) to `[workspace.dependencies]` in `Cargo.toml` (commit `0007da2`)
- [X] T002 Vendor fearless_simd 1.2.0 into `third_party/fearless_simd` with `third_party/fearless_simd.patch` applied (kernel-only tokens `Alderlake` and `SkylakeAvx512`, `Level::as_alderlake`, `Level::as_skylake_avx512`, token tests), point `[patch.crates-io]` in `Cargo.toml` at it, and document it in `third_party/README.md` (commit `0007da2`, CT-9)
- [X] T003 Add `NOTICE` entries for every `crates/rustern-rs/src/*.rs` file and for fearless_simd; `cargo deny check` passes (commit `0007da2`)
- [ ] T004 Add `FUSE_TMPFILE` (opcode 51, payload `fuse_create_in` plus a name, as CREATE) decoding and a `Filesystem::tmpfile` method to a fork of fuser 0.18.0, with a test in the fork; pin it in `Cargo.toml` with `[patch.crates-io] fuser = { git = "<fork URL>", rev = "<commit>" }` and allow exactly that URL in `deny.toml` `[sources] allow-git` (research W15, CT-8). Creating the public fork and opening the upstream fuser pull request are outward-facing: prepare them, then ask the user before publishing either
- [ ] T005 [P] Add a `policies` subcommand to `scripts/cluster.sh`: idempotently create `/mirror` with span policy Parity(1,1), `/rs` with Parity(4,2) and `/inline` with an inline-only span policy via `terncli` (`set-dir-info` on the ternfuse mount's root), print the resulting policies, and fail naming the directory if any step fails
- [ ] T006 [P] Extend `tools/goldens/main.go` with deterministic samples, written with upstream `Pack`, for every request and response this feature sends: shard `ConstructFile`, `AddSpanInitiate`, `AddSpanCertify`, `AddInlineSpan`, `LinkFile`, `MoveSpan`, `SameDirectoryRename`, `SoftUnlinkFile`, `SameShardHardFileUnlink`, `SetTime`; CDC `MakeDirectory`, `RenameFile`, `RenameDirectory`, `SoftUnlinkDirectory`, `CrossShardHardUnlinkFile`; registry `LocalCdc`; blocks `WriteBlock`; plus CDC request and response frames in `testdata/goldens/frames.hex`. Regenerate `testdata/goldens/` and commit it
- [ ] T007 [P] Create `tools/goldens/spanwrite.go`: for sizes 1, 255, 256, 4095, 4096, 4097, 1 MiB, 10 MiB − 1 and 10 MiB, data from the deterministic generator in `tools/goldens/fill.go`, and layouts mirrored D=1 P=1, D=1 P=2 and RS (4,2), (8,3), (10,4) with stripe policies giving S = 1, 2 and 15, write the `AddSpanInitiateReq` body produced by upstream `prepareSpanInitiateReq` (`go/client/span.go`) to `testdata/goldens/span_write.hex` as `<size> <D> <P> <target_stripe_size> <seed> <hex body> # description`; also gap spans (Size larger than the data written)
- [ ] T008 Make `tools/rsgen/main.go` classify every shard and CDC request kind as `ReadOnlyRequest` (the existing list) or a new marker trait `MutatingRequest` (all others); regenerate `crates/rustern-msgs/src/generated.rs`; no codec change (the drift check in `scripts/gate.sh` passes)
- [ ] T009 Run `scripts/gate.sh --quick`

**Checkpoint**: dependencies and goldens in place; `rustern-msgs` can tell mutating requests apart.

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: Reed-Solomon, the read-only gate, the CDC client, name validation, the errno
table, directory policies, flags and the FUSE mount mode. Every story depends on these.

**⚠️ CRITICAL**: No user story work can begin until this phase is complete.

### Reed-Solomon (`rustern-rs`, done in `0007da2`)

- [X] T010 [P] Write `tools/goldens/rs.go` against upstream `go/core/rs` and generate `testdata/goldens/rs.hex`: parity and a random recovery for every TernFS layout (D 2..10, P 1..6), block sizes 1 to 4103 bytes around the vector widths, and layouts up to D=15, P=15 (research W1)
- [X] T011 [P] Write `crates/rustern-rs/tests/goldens.rs` (every line of `rs.hex`, every level the CPU supports) and `crates/rustern-rs/tests/props.rs` (all kernels give identical parity; any D of D+P blocks recover the data; parity 0 is the XOR of the data)
- [X] T012 Implement `crates/rustern-rs/src/{lib,gf,matrix,scalar}.rs`: GF(2^8) over 0x11B, upstream's normalised Cauchy matrix, encoder and decoder, errors instead of aborts for bad arguments, recovery from exactly D blocks (research W1)
- [X] T013 Implement the SIMD kernels `crates/rustern-rs/src/{avx2,avx2_gfni,avx512,avx512_gfni}.rs` through `fearless_simd::kernel!` with tokens `Avx2`, `Alderlake`, `SkylakeAvx512` and `Avx512`, and `Level::best()` preferring GFNI and then 512-bit (research W2)
- [X] T014 Add `crates/rustern-rs/benches/rs.rs` and `tools/rsbench` (upstream C via cgo per CPU level); numbers recorded in research W2. The AVX-512 hardware run is T115

### Read-only gate and mutating requests (`rustern-client`, W17)

- [ ] T015 [P] Write failing tests in `crates/rustern-client/src/lib.rs`: `MountMode::{ReadOnly, ReadWrite}`; `Client::writer()` returns `None` for `ReadOnly` and `Some(WriteAccess)` for `ReadWrite`; a `compile_fail` doctest shows `WriteAccess` cannot be constructed outside the crate; `WriteOptions { overwrite: bool, accept_matching_attrs: bool, publish_wait: Duration }` defaults to false, false, 1 s
- [ ] T016 Implement `MountMode`, `WriteAccess` (zero-sized, private constructor) and `WriteOptions` in `crates/rustern-client/src/lib.rs` and `crates/rustern-client/src/config.rs` to pass T015
- [ ] T017 [P] Write failing tests in `crates/rustern-client/tests/shard_request.rs` for `shard_mutation(&WriteAccess, shard, &Req) -> Result<(Resp, Attempt), ClientError>` where `Req: MutatingRequest`: same retry schedule, address alternation and overall deadline as `shard_request`; the returned `Attempt` is the index of the attempt that answered (0 for the first) so callers can reconcile (W11); `compile_fail` doctests show `shard_request` rejects a `MutatingRequest` type and `shard_mutation` needs a `&WriteAccess`
- [ ] T018 Implement `shard_mutation` in `crates/rustern-client/src/shard.rs` sharing the retry loop with `shard_request`, to pass T017
- [ ] T019 [P] Write failing tests in `crates/rustern-client/tests/cdc_request.rs` with a fake CDC and a fake registry: CDC addresses come from `LocalCdcReq` and refresh with the shard table; frames match `testdata/goldens/frames.hex`; timeouts 1 s / 10 s / 60 s from `ClientConfig::cdc`; a `TIMEOUT` error response is retried, any other error is returned at once; `Attempt` is reported as in T017; every call takes `&WriteAccess`
- [ ] T020 Implement `crates/rustern-client/src/cdc.rs` (`cdc_request(&WriteAccess, &Req)` over the shared UDP socket) and the `LocalCdc` fetch in `crates/rustern-client/src/registry.rs` and `crates/rustern-client/src/addresses.rs`, to pass T019

### Names, errors and errno (`rustern-core`, `rustern-client`, `rustern-fuse`)

- [ ] T021 [P] Write failing tests in a new `crates/rustern-core/src/name.rs`: `Name::new(&[u8])` accepts any bytes up to 255 including non-UTF-8; more than 255 bytes → `NameError::TooLong`; empty, containing `/` or NUL, `.` or `..` → `NameError::Invalid`; proptest: every accepted name round-trips through `as_bytes()` (FR-017, W11)
- [ ] T022 Implement `crates/rustern-core/src/name.rs` and export it from `crates/rustern-core/src/lib.rs`, to pass T021
- [ ] T023 [P] Write failing tests in `crates/rustern-fuse/src/errno.rs` for every row of the data-model.md "Errno mapping for mutations" table, including: `LOOP_IN_DIRECTORY_RENAME` → EINVAL; `COULD_NOT_PICK_BLOCK_SERVICES` on a first attempt → ENOSPC and after blacklisting → EIO; `BAD_COOKIE`, `NOT_AUTHORISED` → EIO; `LAST_SPAN_STATE_NOT_CLEAN`, `MTIME_IS_TOO_RECENT`, `FILE_IS_TRANSIENT` → EBUSY; `CANNOT_OVERRIDE_NAME`, `NAME_IS_LOCKED`, `MORE_RECENT_CURRENT_EDGE` → EEXIST; `NameError::TooLong` → ENAMETOOLONG; new `ClientError` variants `NoSpace`, `WriteBehindEnd` (EINVAL), `Shrink` (EOPNOTSUPP), `ReadOfWriter` (EOPNOTSUPP), `NotPermitted` (EPERM), `CrossDirectory` (EXDEV), `Busy` (EBUSY), `Failed(errno)` (that errno). The match still has no wildcard arm. Every error reply goes through one helper that logs at debug with op, inode and cause and increments `rustern_fuse_errors_total{op,errno}` (FR-022)
- [ ] T024 Add the new variants to `crates/rustern-client/src/error.rs` and implement the mapping and the reply helper in `crates/rustern-fuse/src/errno.rs`, to pass T023

### Directory policies (`rustern-client`)

- [ ] T025 [P] Write failing tests in a new `crates/rustern-client/src/policy.rs` with fake `StatDirectory` responses: each of `SPAN_POLICY`, `BLOCK_POLICY`, `STRIPE_POLICY` and `SNAPSHOT_POLICY` resolves from the nearest ancestor that sets it; validation exactly as data-model.md: span "non-empty, ascending `max_size` ≤ `MAXIMUM_SPAN_SIZE` (100 MiB), `Parity` valid (D ≥ 1, D+P ≤ 16)", block "storage class not EMPTY or INLINE", stripe "> 0; stripes are clamped to 1..15"; any violation is `ClientError::Malformed` with an `alert = true` error event (captured with `tracing::subscriber::with_default`), never a panic; a second resolution within the cache lifetime sends no request; expiry follows 001's directory info cache
- [ ] T026 Implement `DirectoryPolicies` and its `moka` cache in `crates/rustern-client/src/policy.rs`, to pass T025

### Flags, configuration and metrics

- [ ] T027 [P] Write failing tests in `crates/rustern/src/cli.rs` for contracts/cli.md: `-o ro` → `ReadOnly`, `-o rw` and no `-o` → `ReadWrite`, `nosuid,nodev` always applied, `allow_other` adds `default_permissions`; `--overwrite` and `--accept-matching-attrs` default false; `--write-memory-limit` default 1GiB, below 256MiB exits with code 2; `--file-write-spans` default 2, outside 1–8 exits with code 2; `--publish-wait` default 1s, `0` disables waiting; `-close-tracker-object` still accepted with a warning (FR-023)
- [ ] T028 Implement the flags in `crates/rustern/src/cli.rs` and the fields `write_memory_limit`, `file_write_spans`, `publish_wait` with validation in `crates/rustern-client/src/config.rs`, to pass T027
- [ ] T029 [P] Write failing tests in `crates/rustern-client/tests/metrics.rs` for the contracts/cli.md metrics: `rustern_write_bytes_total`, `rustern_span_store_seconds{storage}`, `rustern_span_store_attempts_total{outcome}`, `rustern_publish_seconds`, `rustern_publish_failures_total{reason}`, `rustern_write_buffer_bytes`, `rustern_write_backpressure_seconds`, `rustern_files_being_written`; label values come from exported constant lists (`storage`: inline, mirrored, rs; `outcome`: ok, retry, failed; `reason`: exists, more_recent, cookie, timeout, other)
- [ ] T030 Add the metric names, label constants and recording helpers to `crates/rustern-client/src/metrics.rs`, to pass T029 (stories call them)

### FUSE mount mode (`rustern-fuse`)

- [ ] T031 [P] Write failing tests in `crates/rustern-fuse/tests/bridge_mount.rs`: a `ReadOnly` mount answers every mutation with EROFS exactly as in 001; a `ReadWrite` mount negotiates `FUSE_ATOMIC_O_TRUNC`, `max_write` 1 MiB with `max_pages`, no writeback cache, and `statfs` without `ST_RDONLY`; mutations not yet wired by a story still answer EROFS
- [ ] T032 Implement the mount mode in `crates/rustern-fuse/src/fs.rs` and pass the `WriteAccess` from `Client::writer()` into the bridge state in `crates/rustern-fuse/src/bridge.rs`, to pass T031; `crates/rustern/tests/refuses_mutations.rs` runs with `-o ro` and still passes

### Codec tests and fuzzing (`rustern-msgs`)

- [ ] T033 [P] Add `cdc_resp(&[u8])` to `crates/rustern-msgs/src/fuzz_support.rs` and the target `crates/rustern-msgs/fuzz/fuzz_targets/cdc_resp.rs`; seed `crates/rustern-msgs/fuzz/corpus/{shard_resp,cdc_resp,blocks_resp}/` with the new response frames from T006; `crates/rustern-msgs/tests/corpus_replay.rs` replays the `cdc_resp` corpus
- [ ] T034 Add the T006 types to the dispatch in `crates/rustern-msgs/tests/goldens.rs` and make it pass for every new golden line
- [ ] T035 Run the merge gates

**Checkpoint**: a read-write mount exists and still refuses every mutation; the client can send
mutating shard and CDC requests behind `WriteAccess`.

---

## Phase 3: User Story 1 - Write a new file and read it back anywhere (Priority: P1) 🎯 MVP

**Goal**: create, sequential write, flush, fsync, extend-only truncate and release-time
publish of new files, bit-identical with the reference client, plus the `--overwrite`
option.

**Independent Test**: write files of varied sizes (empty, under a page, inline size, one
span, many spans, several GiB) into `/mirror`, `/rs` and `/inline` through Rustern, and
compare checksums read through ternfuse and a fresh Rustern mount.

### Tests for User Story 1 (REQUIRED, written first) ⚠️

> **NOTE: Write these tests FIRST, run them, and confirm they FAIL before implementation**

- [ ] T036 [P] [US1] Write failing unit and property tests for the write-side layout in `crates/rustern-core/src/span.rs` (data-model.md SpanLayout): parity from "first span-policy entry with `max_size ≥ size`"; S "clamped to `[1, 15]`"; `cell_size` "`ceil(ceil(size/D)/S)` rounded up to 4096"; `block_size = cell_size * S`; `data_len = S * D * cell_size`; cell CRCs row-major `Crcs[B*s + b]`, mirrored "one CRC per stripe repeated"; `span_crc` is the CRC of the written bytes zero-extended to `size`. Properties: `span_crc` equals the CRC of the bytes 001's read-path layout (`crates/rustern-core/src/layout.rs`) returns for the span; the span CRC equals the cell CRCs appended and zero-extended; parity-0 cell CRCs equal the XOR combination of the data cell CRCs (shard `_checkSpanBody`)
- [ ] T037 [P] [US1] Write failing golden tests in `crates/rustern-core/tests/span_write_golden.rs`: for every line of `testdata/goldens/span_write.hex` (T007), the layout, CRC arrays and parity (via `rustern_rs`) produce the upstream `AddSpanInitiateReq` body byte for byte
- [ ] T038 [P] [US1] Write failing property tests for a CRC XOR helper in `crates/rustern-core/src/crc.rs`: for equal-length `a` and `b`, `crc(a ⊕ b) == crc(a) ⊕ crc(b) ⊕ crc(0ⁿ)`
- [ ] T039 [P] [US1] Write failing tests in a new `crates/rustern-client/src/write/buffers.rs` (data-model.md SpanBuffer, W7): a byte-counting `tokio::sync::Semaphore` of `write_memory_limit`; a file holds at most `file_write_spans` buffers; a writer at the limit waits and resumes when a permit is dropped; dropping a waiting or holding future releases its permits (cancel-safe); recycled buffers are re-zeroed up to `filled` only and read back as zeros; parity buffers are taken from the same semaphore when a span is sealed; no allocation beyond the permit (dhat, as `crates/rustern-client/tests/read_alloc.rs`)
- [ ] T040 [P] [US1] Write failing tests in a new `crates/rustern-client/src/write/state.rs` for every row of the data-model.md "Events and transitions" table and every rejected event: write at `size`, past `size` (gap) and before `size` (EINVAL, still writable); truncate up (zeros) and down (EOPNOTSUPP); flush and fsync wait for `certified == size` and set `flushed_idle` only after a flush; release with `handles > 1`, with a name (Publishing) and without (Abandoned); link into the owner directory and into another (EXDEV); `SpanFailed(e)` → `Failed(e)` replying `e` to every later command; `LinkErr` → Failed with a publish-failure action; `Unlinked` → Abandoned
- [ ] T041 [P] [US1] Write the failing exhaustive model test `crates/rustern-client/tests/write_model.rs` (W13): enumerate every ordering of up to 3 writes, 2 flushes, 1 fsync, 1 truncate, 1 release, store completions and failures, and 1 interrupt (dropped reply receiver) through `state.rs`, asserting the seven data-model.md invariants after each step; a `proptest` strategy samples longer sequences
- [ ] T042 [P] [US1] Write failing tests in a new `crates/rustern-client/src/write/pending.rs` (W8): an entry is inserted at create for named files only; `Open` not flushed-idle answers not-found at once; `Publishing` or flushed-idle waits up to `publish_wait` and returns the inode on `Done(Ok)`, not-found on timeout or `Done(Err)`; `publish_wait = 0` never waits; the entry is removed at `Done` and at `Abandoned`; two writers for the same `(DirId, Name)` leave the later one in the registry
- [ ] T043 [P] [US1] Write failing tests in a new `crates/rustern-client/tests/block_write.rs` with a fake block service: `WriteBlockReq{BlockId, Crc, Size, Certificate}` framing matches the goldens; the block's cells are sent as one vectored write from the span and parity buffers; `WriteBlockResp{Proof}` is returned; an error code, a closed connection and a timeout each fail the block with a typed error; the certificate and proof never appear in logs (captured subscriber)
- [ ] T044 [P] [US1] Write failing cluster tests in a new `crates/rustern-client/tests/span_store.rs` against `/inline`, `/mirror` and `/rs`: an inline span (< 256 bytes), a mirrored and an RS span are stored and read back identically through the 001 read path; a gap span (`Size` larger than its data blocks, W4) reads back zeros; an all-zero span written from the static page reads back zeros; a short inline body with a larger `size` is checked (W4 to-verify; record the outcome in research W4); in `mod cluster_serial`, a SIGSTOPped block service makes the span succeed on another service via blacklist and retry, with the dirty attempt moved by `ConstructFile{bad_add_span_attempt}` and `MoveSpan`; all block services of the storage class stopped → EIO within the 60 s deadline; `COULD_NOT_PICK_BLOCK_SERVICES` on the first attempt → `ClientError::NoSpace`
- [ ] T045 [P] [US1] Write failing mount tests in a new `crates/rustern/tests/write_lifecycle.rs` for US1 scenarios 1–9 with sizes 0, 1, 255, 4095, 4096, the inline limit, one span, many spans and 3 GiB in `/mirror`, `/rs` and `/inline`; FR-012 `stat` size while writing; FR-010 `read` of a file being written → EOPNOTSUPP; `fsync` stores but does not publish; `cmd > newfile` through a shell publishes everything; and the deviation tests `write_publish_point` (DEV-007), `failed_file_sticks` (DEV-008), `open_published_for_write` (DEV-009: EPERM at `open`, also with `O_APPEND`) and `overwrite_option` (DEV-010: with `--overwrite`, `O_TRUNC` replaces at release, an earlier reader keeps the old content, a failed replacement leaves the old file)
- [ ] T046 [P] [US1] Write the failing differential test `crates/rustern/tests/write_differential.rs` (SC-001): a write corpus of sizes from 0 bytes to 4 GiB in `/mirror`, `/rs` and `/inline` written through Rustern is read back through ternfuse and, when loaded, the kernel module with equal sha256; the same corpus written through ternfuse reads back identically through Rustern
- [ ] T047 [P] [US1] Write failing fault tests in `mod cluster_serial` of a new `crates/rustern/tests/write_faults.rs` (SC-002), reusing the fault mechanisms of `crates/rustern/tests/faults.rs`: packet loss, duplication and reordering on shard and block traffic; block service SIGKILL and SIGSTOP during span stores; a slow shard during publish; writer SIGKILL mid-write; daemon SIGKILL mid-write followed by a restart that serves the mount; a lost `LinkFile` response reconciled as success. After each run: no published file differs from what the writer got success for, and no failed file has a visible name
- [ ] T048 [P] [US1] Write failing stress tests in a new `crates/rustern/tests/write_concurrency.rs`: 16 writers in parallel; concurrent `fsync`, `close` of a duplicated descriptor and `stat` on one file being written; two processes creating the same new name in one mount (both outcomes reported, no data of either lost); a write larger than the per-file buffer waits instead of failing
- [ ] T049 [US1] Run T036–T048 and record that they fail

### Implementation for User Story 1

- [ ] T050 [P] [US1] Implement the write-side layout in `crates/rustern-core/src/span.rs`, ported from `ComputeSpanParameters` and `prepareSpanInitiateReqWithParams` in `go/client/span.go` (upstream copyright and SPDX header; add `crates/rustern-core/src/span.rs <- go/client/span.go` to `NOTICE`); zeros are CRC'd by zero-extend, never hashed byte by byte, to pass T036 and T037
- [ ] T051 [P] [US1] Implement the CRC XOR helper in `crates/rustern-core/src/crc.rs`, to pass T038
- [ ] T052 [P] [US1] Implement `crates/rustern-client/src/write/buffers.rs` (pooled `Vec<u8>` sized to the file's largest span, `OwnedSemaphorePermit` per buffer, the static 4 KiB zero page), to pass T039
- [ ] T053 [P] [US1] Implement the sans-IO `WriteState` and `FileWriter::step(event) -> Vec<Action>` in `crates/rustern-client/src/write/state.rs` with exhaustive matches, to pass T040 and T041
- [ ] T054 [P] [US1] Implement the pending-publish registry (`DashMap<(DirId, Name), watch::Sender<PendingState>>`) in `crates/rustern-client/src/write/pending.rs`, to pass T042
- [ ] T055 [P] [US1] Implement `WriteBlock` in `crates/rustern-client/src/blocks.rs` over the existing per-block-service `deadpool` pools, with block service addresses from the 001 cache (an unknown id triggers a registry refresh), to pass T043
- [ ] T056 [US1] Implement `SpanStore` in a new `crates/rustern-client/src/write/store.rs` (depends on T050, T052, T055): `AddSpanInitiate` with the blacklist, parallel `WriteBlock`s, `AddSpanCertify`; `AddInlineSpan` below 256 bytes; blacklist-and-retry following `writeSpanBlocks` in `go/client/span.go`, up to 5 attempts within the 60 s deadline, moving dirty attempts with `ConstructFile{bad_add_span_attempt}` and `MoveSpan`; CRC and parity on `spawn_blocking` using `rustern_rs::Level::best()` chosen once per client; record `rustern_span_store_seconds` and `rustern_span_store_attempts_total`, to pass T044
- [ ] T057 [US1] Implement the per-file actor in a new `crates/rustern-client/src/write/actor.rs` (depends on T053, T056): bounded `mpsc` mailbox of `Write`, `Flush`, `Fsync`, `Truncate`, `Release`, `Link`, `Stat`, `Unlinked`, each with a `oneshot` reply; store completions re-enter as events; fill span k+1 while span k stores, initiate k+1 only after certify k; `LinkFile` at release; publish failures logged at warn with file id and name and counted in `rustern_publish_failures_total{reason}`; dropped reply receivers never leave a half-applied state
- [ ] T058 [US1] Implement the public write API in a new `crates/rustern-client/src/write/mod.rs` and export it from `crates/rustern-client/src/lib.rs`: `Client::create_file(&WriteAccess, dir, Name, &WriteOptions) -> Result<WriterHandle, ClientError>` (`ConstructFile{FILE, Note: name}` on the parent's shard, policies resolved, pending entry inserted), `WriterHandle::{write, flush, fsync, truncate, stat, add_handle, release}`, `Client::replace_file` for `--overwrite` (W9: `CANNOT_OVERRIDE_NAME` at open → EISDIR, `MORE_RECENT_CURRENT_EDGE` at publish → publish failure), and `Client::pending_lookup`; `rustern_files_being_written` and `rustern_write_bytes_total` recorded; T044 passes and the client-level parts of T045 pass
- [ ] T059 [US1] Wire the bridge in `crates/rustern-fuse/src/bridge.rs` per contracts/filesystem.md: `create` (`O_EXCL` → EEXIST; existing name otherwise as `open`), `open` (new handle on a writer; published file → EPERM; `O_TRUNC` with `--overwrite` → replacement writer), `write` (copy once into the span buffer lease), `flush`, `fsync`/`fdatasync`, `release`, `setattr` size, `read` on a writer → EOPNOTSUPP, `lookup` and `readdir` waiting on pending names, `mknod` of `S_IFREG` (create then release) and EPERM for other types; writer handles use `FOPEN_DIRECT_IO`
- [ ] T060 [US1] Give writer attributes zero validity in `crates/rustern-fuse/src/attr.rs` and invalidate `(parent, name)` and the inode through fuser's `Notifier` on publish in `crates/rustern-fuse/src/fs.rs`, before the pending entry resolves; T045 passes
- [ ] T061 [US1] Implement shutdown in `crates/rustern/src/main.rs` (FR-021): on SIGINT, SIGTERM or SIGHUP new mutations get EIO (logged once), in-flight span stores, publishes and namespace requests get the 5 s grace, files still being written are not published and their count is logged at warn (`shutdown: N files being written were not published`) and counted; add a case to `crates/rustern/tests/write_lifecycle.rs`
- [ ] T062 [US1] Add `differential-write` and `faults-write` subcommands to `scripts/gate.sh` running T046 and T047, and include them in the full run; T046, T047 and T048 pass
- [ ] T063 [US1] Add DEV-007, DEV-008, DEV-009 and DEV-010 to `docs/deviations.md` with their test paths from T045, and narrow DEV-001 (read-only mount) to `-o ro`
- [ ] T064 [US1] Run the merge gates

**Checkpoint**: new files can be written and published; the MVP is usable for jobs that write
output files into existing directories.

---

## Phase 4: User Story 2 - Organise the namespace (Priority: P2)

**Goal**: mkdir, rmdir, unlink, rename (three kinds) and symlink as ternfuse does them, with
idempotent reconciliation after lost responses.

**Independent Test**: run a scripted sequence of mkdir, renames, unlink, rmdir and symlink
through Rustern and compare the tree, with types, sizes and symlink targets, against the
reference client.

### Tests for User Story 2 (REQUIRED, written first) ⚠️

- [ ] T065 [P] [US2] Write failing cluster tests in a new `crates/rustern-client/tests/mutate.rs` for US2 scenarios 1–6 through the client API: `mkdir` inherits the parent's policies and gives EEXIST on an existing name; `rmdir` of a non-empty directory → ENOTEMPTY; `unlink` removes the name and, where the snapshot policy allows immediate deletion, the file is hard-unlinked; same-directory, cross-directory file and directory renames; a directory renamed into its own subtree → EINVAL; rename onto an existing file replaces it and the old file is a snapshot edge; `symlink` targets of 1 byte, the inline limit and longer (one block span) read back through `readlink`
- [ ] T066 [P] [US2] Write failing tests in a new `crates/rustern-client/tests/reconcile.rs` using `crates/rustern-client/tests/support/relay.rs` to drop the first response of `MakeDirectory`, `SoftUnlinkDirectory`, `SoftUnlinkFile`, `SameDirectoryRename`, `RenameFile`, `RenameDirectory` and `LinkFile`: each call reports success, not EEXIST or ENOENT (US2-7, FR-015); a genuine EEXIST on the first attempt is still EEXIST
- [ ] T067 [P] [US2] Write a failing unit test in a new `crates/rustern-client/src/mutate.rs` comparing the immediate-deletion predicate taken from the cluster's snapshot-policy evaluation (`go/cleanup`) with ternfuse's `canDeleteFileImmediately` over a table of snapshot policies (time-only with non-zero retention, versions-only, both, neither); the test documents where they differ (W11)
- [ ] T068 [P] [US2] Write the failing differential test `crates/rustern/tests/namespace_differential.rs`: one scripted sequence of mkdir, rename (all three kinds), unlink, rmdir and symlink on a Rustern mount and on the ternfuse mount, then compare both trees (types, sizes, symlink targets); include `errno_mapping` (DEV-013: rename into own subtree EINVAL, a cookie fault EIO, first-attempt no-space ENOSPC)
- [ ] T069 [P] [US2] Add the failing test `unlink_while_writing` (DEV-012) to `crates/rustern/tests/write_lifecycle.rs`: `unlink` of a name being written succeeds and the file is never published while its descriptor stays writable; `rename` of a flushed-idle name waits for the publish, otherwise EBUSY
- [ ] T070 [US2] Run T065–T069 and record that they fail

### Implementation for User Story 2

- [ ] T071 [P] [US2] Implement `crates/rustern-client/src/reconcile.rs` (W11): `checkDeletedEdge` (FullReadDir backwards, deletion edge plus snapshot edge), `checkNewEdgeAfterRename` (60 s creation-time fuzz, always applied) and the mkdir reconciliation (EEXIST on attempt > 0 and a directory created within the attempt window → success), ported from `go/client/shardreq.go`, `cdcreq.go` and `metadatareq.go` with SPDX headers and `NOTICE` entries, to pass T066
- [ ] T072 [US2] Implement namespace mutations in `crates/rustern-client/src/mutate.rs` (depends on T071): `mkdir` (CDC `MakeDirectory`), `rmdir` (`Lookup`, CDC `SoftUnlinkDirectory`), `unlink` (`Lookup`, `SoftUnlinkFile`, then `SameShardHardFileUnlink` or CDC `CrossShardHardUnlinkFile` when the predicate of T067 allows, errors of the hard unlink ignored), `rename` (`SameDirectoryRename`, CDC `RenameFile`, CDC `RenameDirectory`), `rename_noreplace` (lookup first, best effort), `symlink` (`ConstructFile{SYMLINK}`, `AddInlineSpan` or one block span, `LinkFile`), all behind `&WriteAccess` with names validated first, to pass T065 and T067. If T067 shows the rules differ, use the cluster's rule and record DEV-014
- [ ] T073 [US2] Wire `mkdir`, `rmdir`, `unlink`, `rename` and `rename2` (`RENAME_NOREPLACE`; `RENAME_EXCHANGE` and `RENAME_WHITEOUT` → EINVAL) and `symlink` in `crates/rustern-fuse/src/bridge.rs`: `unlink` of a pending name abandons the writer and removes the entry, `rename` of or onto a pending name waits as `lookup` when flushed-idle and is EBUSY otherwise, and every mutation invalidates the affected entries and inodes before replying, to pass T068 and T069
- [ ] T074 [US2] Add DEV-012 and DEV-013 (and DEV-014 if T072 recorded it) to `docs/deviations.md` with their test paths
- [ ] T075 [US2] Run the merge gates

**Checkpoint**: Rustern is a complete read-write namespace for files written through it.

---

## Phase 5: User Story 3 - Work with ordinary tools (Priority: P3)

**Goal**: `cp`, `cp -r`, redirection, `tar -x`, `rsync -r`, `touch -d` and `mv` work; times
can be set; `O_TMPFILE` with `linkat`; unsupported behaviour fails with a documented errno.

**Independent Test**: run the fixed tool script against Rustern and the reference client;
every tool gives identical results or fails on Rustern with its documented errno, and the
trees match.

### Tests for User Story 3 (REQUIRED, written first) ⚠️

- [ ] T076 [P] [US3] Create `scripts/tools.sh <rustern-mount> <reference-mount>` (SC-003): build a source tree with nested directories, symlinks, empty files and a multi-GiB file; run `cp`, `cp -r`, `cmd > file`, `tar -x`, `rsync -r`, `touch -d`, `mv` and a write-to-temporary-then-rename tool into empty target directories on both mounts; compare trees byte for byte; check that `cp -p`, `rsync -a` and `tar -x` report EOPNOTSUPP for the mode and owner change by default and succeed with `--accept-matching-attrs`; check that an in-place edit (`dd conv=notrunc seek=…`) and a random write fail with their documented errno; exit non-zero naming the first mismatch
- [ ] T077 [P] [US3] Write the failing test `crates/rustern/tests/tmpfile.rs` (US3-4, FR-008): `O_TMPFILE` then `linkat` into the same directory publishes the file; into another directory → EXDEV; closing without `linkat` publishes nothing; `linkat` of a published file → EPERM. A client-API variant of the same cases runs without the fuser patch, and the mount variant expects EOPNOTSUPP while CT-8 is open
- [ ] T078 [P] [US3] Add failing tests to `crates/rustern/tests/write_lifecycle.rs`: `utimensat` and `touch -d` set atime and mtime visible through both clients (US3-2, FR-016), `UTIME_OMIT` leaves a time unchanged, `UTIME_NOW` uses the daemon clock, and times set on a file being written are applied after publish; `chmod_chown_policy` (DEV-011): EOPNOTSUPP by default; with `--accept-matching-attrs`, values equal to what the mount presents succeed with no request, others → EPERM
- [ ] T079 [P] [US3] Create `scripts/posix.sh` with `build` (fetch and build the Rust `pjdfstest` outside the workspace) and `run <mount>` (the create, open, write, rename, unlink, mkdir, rmdir, symlink and utimensat subset), reading exclusions with their TernFS reason from `docs/coverage.md`
- [ ] T080 [P] [US3] Add a `terntests` subcommand to `scripts/gate.sh` that builds upstream `ternfs/go/terntests` and runs its filesystem tests against a Rustern mount, skipping the exclusions listed in `docs/coverage.md` (SC-006, W16); include it in the full run
- [ ] T081 [US3] Run T076–T080 and record that they fail

### Implementation for User Story 3

- [ ] T082 [US3] Implement `SetTime` in `crates/rustern-client/src/mutate.rs` (bit 63 marks each time to set) and a deferred-times field in the writer applied after publish in `crates/rustern-client/src/write/actor.rs`; wire `setattr` atime/mtime in `crates/rustern-fuse/src/bridge.rs` with inode invalidation, to pass the time cases of T078
- [ ] T083 [US3] Implement the mode, uid and gid `setattr` policy in `crates/rustern-fuse/src/bridge.rs` (EOPNOTSUPP by default; with `accept_matching_attrs`, compare with the presented attributes from `crates/rustern-fuse/src/attr.rs`), to pass `chmod_chown_policy`
- [ ] T084 [US3] Implement `tmpfile` and `link` in `crates/rustern-fuse/src/bridge.rs` and an unnamed writer with a `Link` event in `crates/rustern-client/src/write/mod.rs` (no pending entry; `linkat` replies after `LinkFile`), using the fuser patch of T004, to pass T077
- [ ] T085 [US3] Answer `setxattr`, `removexattr`, `fallocate` and `copy_file_range` with EOPNOTSUPP in `crates/rustern-fuse/src/bridge.rs` (contracts/filesystem.md)
- [ ] T086 [US3] List the `terntests` and POSIX exclusions with their TernFS reason in `docs/coverage.md` and add DEV-011 to `docs/deviations.md`; T076, T079 and T080 pass
- [ ] T087 [US3] Run the merge gates

**Checkpoint**: everyday tools work on the mount, and the remaining gaps are documented.

---

## Phase 6: User Story 4 - Writes keep up with reads (Priority: P4)

**Goal**: write throughput at least ternfuse's with less CPU, and readers stay responsive
while writers run.

**Independent Test**: run the write benchmarks on both clients on the same cluster and
compare.

### Benchmarks for User Story 4 (written first)

- [ ] T088 [P] [US4] Add the criterion benchmark `crates/rustern-core/benches/span_write.rs`: layout, cell CRCs and parity per span for 1 MiB and 10 MiB spans at (1,1), (4,2) and (10,4)
- [ ] T089 [P] [US4] Add a `write` suite to `scripts/bench.sh` running each workload of the spec's Performance Targets on a given mount: `seq-write` (1 and 4 GiB files, 1 MiB writes, 1/2/4/8/16 writers, `/rs`), `seq-write-mirror`, `small-files` (16 writers, 4 KiB), `meta-write-latency` (p50 and p99 of 1 MiB `close`, `mkdir`, `rename`, `unlink`) and `mixed-rw` (16 readers with 16 writers against the read-only p99); `fio` psync with `--create_on_open=1 --fallocate=none` as in research W14; client CPU from `/proc/<pid>/stat`; peak RSS; results written to `specs/002-write-support/benchmarks.md`; a `--compare` mode exits non-zero when a target is missed by more than 5%
- [ ] T090 [US4] Run the suite on ternfuse for the numbers research W14 lists as missing (`seq-write-mirror`, 2 and 8 writers, `meta-write-latency`, `mixed-rw`) and record them in `specs/002-write-support/benchmarks.md` and the Performance Goals table of `specs/002-write-support/plan.md`

### Implementation for User Story 4

- [ ] T091 [US4] Run the suite on Rustern; for every missed target profile it (flame graph from an unprivileged `perf` where `perf_event_paranoid` allows, otherwise `tracing` timings) and fix only what the profile shows, within the memory budget (about 2 GiB per client); retune `--file-write-spans` and `--write-memory-limit` defaults if the numbers justify it, updating contracts/cli.md
- [ ] T092 [US4] Make the write benchmarks a merge gate in `scripts/gate.sh` (`scripts/bench.sh write --compare`), as the spec's Performance Targets require
- [ ] T093 [US4] Run the merge gates

**Checkpoint**: every Performance Target met and gated.

---

## Phase 7: Performance, Interop & Polish

**Purpose**: Release-relevant checks that span the stories, and the open merge conditions
from research and Complexity Tracking.

- [ ] T094 [P] Add `degraded_read_of_rustern_rs_files` to `mod cluster_serial` in `crates/rustern/tests/write_differential.rs`: write RS files through Rustern, stop up to P block services holding their blocks, and read them back with equal sha256 through ternfuse and, when loaded, the kernel module (spec Testing Requirements, Interoperability)
- [ ] T095 [P] Add `--write` to `scripts/soak.sh`: 30 minutes of mixed reads and writes under the periodic faults of 001's soak; pass when the mount is alive, no byte is wrong and peak RSS stays within `--memory-limit + --write-memory-limit` plus a fixed margin (SC-005)
- [ ] T096 [P] Document the write flags, `-o ro`, the opt-in options and memory sizing (read plus write limits) in `docs/deployment.md`
- [ ] T097 [P] Add to `docs/future-features.md`: retargeting a pending name on rename (W12), and the short inline body if T044 did not verify it
- [ ] T098 Confirm every deviation DEV-007 to DEV-013 (and DEV-014 if recorded) is in `docs/deviations.md` with a test that exists and passes
- [ ] T099 Run the AVX-512 kernels on real hardware before the merge (research W2): on a Skylake-SP machine run `cargo nextest run -p rustern-rs` and `cargo test --manifest-path third_party/fearless_simd/Cargo.toml`, and confirm `Level::best()` reports `Avx512`; on an Ice Lake or Zen 4 machine, or under Intel SDE (`sde64 -icx -- cargo nextest run -p rustern-rs`), confirm `Avx512Gfni` passes the goldens; run `cargo bench -p rustern-rs` on each machine and add the numbers to research W2
- [ ] T100 Resolve CT-8 in `specs/002-write-support/plan.md`: if the fuser change (T004) is in a release, drop the git patch from `Cargo.toml` and `deny.toml`; if it is not accepted by the merge, make `tmpfile` answer EOPNOTSUPP, add the deviation row to `docs/deviations.md` and drop the patch
- [ ] T101 Record the status of CT-9 in `specs/002-write-support/plan.md`. Filing the fearless_simd upstream issue (draft prepared) is outward-facing and happens only with the user's approval
- [ ] T102 Run every row of `specs/002-write-support/quickstart.md` and fix any mismatch
- [ ] T103 Re-check the Constitution Check and Complexity Tracking rows of `specs/002-write-support/plan.md` against the result (CT-2, CT-3, CT-6, CT-8, CT-9) and update their status
- [ ] T104 Run the merge gates

---

## Dependencies & Execution Order

### Phase Dependencies

- **Setup (Phase 1)**: T001–T003 done. T004 is needed only by T084 (tmpfile) and can run
  alongside everything else.
- **Foundational (Phase 2)**: depends on T006 and T008. Blocks every story.
- **US1 (Phase 3)**: after Phase 2. The MVP.
- **US2 (Phase 4)**: after Phase 2. `unlink_while_writing` and pending-name handling (T069,
  T073) need US1's pending registry (T054); the rest is independent of US1.
- **US3 (Phase 5)**: needs US1 (files to copy) and US2 (directories, renames, symlinks) for
  the tool script; the time and attribute tasks (T078, T082, T083) need only Phase 2 and T058.
- **US4 (Phase 6)**: needs US1 for the write workloads and US2 for `meta-write-latency`.
- **Polish (Phase 7)**: after the stories it checks. T099 needs only the done RS tasks and can
  run at any time.

### Key task dependencies

- T007 → T037 → T050; T006 → T033, T034; T008 → T017
- T050, T052, T055 → T056 → T057 → T058 → T059 → T060
- T053 → T041 (the model test drives the state machine directly)
- T054 → T059, T073
- T071 → T072 → T073
- T004 → T084; T100 depends on T004's outcome
- T089 → T090, T091 → T092

### Within each story

- Tests are written and fail before the implementation
- Core and client before the FUSE bridge
- Merge gates pass before the story is complete

---

## Parallel Opportunities

- Phase 1: T005, T006 and T007 in parallel; T004 is independent of the rest
- Phase 2: the test tasks T015, T017, T019, T021, T023, T025, T027, T029, T031 and T033 touch
  different files and run in parallel; each implementation follows its own test
- US1: tests T036–T048 in parallel; implementations T050–T055 in parallel, then T056–T060 in
  order
- US2: T065–T069 in parallel; T071 alongside the US1 bridge work
- US3: T076–T080 in parallel
- Polish: T094–T097 and T099 in parallel

## Parallel Example: User Story 1

```bash
# Tests first, together:
Task: "Write-side layout tests in crates/rustern-core/src/span.rs"
Task: "Span write goldens in crates/rustern-core/tests/span_write_golden.rs"
Task: "Writer state tests in crates/rustern-client/src/write/state.rs"
Task: "Exhaustive model test in crates/rustern-client/tests/write_model.rs"
Task: "Fake block service tests in crates/rustern-client/tests/block_write.rs"

# Then the independent building blocks, together:
Task: "Write-side layout in crates/rustern-core/src/span.rs"
Task: "Span buffers in crates/rustern-client/src/write/buffers.rs"
Task: "Pending registry in crates/rustern-client/src/write/pending.rs"
Task: "WriteBlock in crates/rustern-client/src/blocks.rs"
```

---

## Implementation Strategy

### MVP first (User Story 1)

1. Phase 1 (T004 may still be open)
2. Phase 2
3. Phase 3: write and publish new files, with `--overwrite`
4. **Stop and validate**: quickstart rows US1-1, US1-2, US1-3, US1-7, US1-8 and the write
   corpus differential

### Incremental delivery

1. Setup + Foundational: a read-write mount that still refuses mutations
2. US1: new files → MVP
3. US2: the namespace
4. US3: tools, times, `O_TMPFILE`
5. US4: performance gated
6. Polish: interop with degraded reads, soak, hardware runs, CT resolution

---

## Notes

- [P] tasks touch different files and depend on no incomplete task
- Commit after each task or logical group; commits and pushes only when the user asks
- Outward-facing steps (the fuser fork and pull request, the fearless_simd issue) wait for
  the user's approval
