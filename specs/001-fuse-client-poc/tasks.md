---

description: "Task list for feature 001: read-only FUSE client proof of concept"
---

# Tasks: Read-Only FUSE Client Proof of Concept

**Input**: Design documents from `/specs/001-fuse-client-poc/`

**Prerequisites**: [plan.md](plan.md), [spec.md](spec.md), [research.md](research.md),
[data-model.md](data-model.md), [contracts/cli.md](contracts/cli.md),
[contracts/filesystem.md](contracts/filesystem.md), [quickstart.md](quickstart.md)

**Tests**: REQUIRED. Constitution Principle VII and the spec's Testing Requirements: tests
are written, run and seen to fail before the implementation. Every user story phase lists
its test tasks first. Every bug fix task is preceded by a test that reproduces the bug.

**Organization**: Tasks are grouped by user story so each story can be implemented and
tested as an increment.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: Can run in parallel (different files, no dependencies on incomplete tasks)
- **[Story]**: US1, US2, US3 from spec.md
- Every task names the exact file(s) it touches

## Conventions used by every task

- Crates (plan.md): `rustern-msgs` → `rustern-core` → `rustern-client` → `rustern-fuse` →
  `rustern` (binary). Arrows are the only allowed dependency direction. `rustern-client`,
  `rustern-core` and `rustern-msgs` never depend on `fuser`.
- Unit tests live in `#[cfg(test)] mod tests` next to the code. Integration tests live in
  `crates/<crate>/tests/<name>.rs`. Property tests use `proptest`.
- **Cluster tests** (they need the local cluster) are marked
  `#[ignore = "needs cluster: run via scripts/gate.sh"]`. `scripts/gate.sh` runs them with
  `cargo nextest run --run-ignored all`; `scripts/gate.sh --quick` runs only the default
  set. Cluster tests read `RUSTERN_TEST_REGISTRY` (default `127.0.0.1:10001`) and
  `RUSTERN_TEST_DATA_DIR` (default `ternfs/mnt`, the `ternrun -data-dir`); the reference
  (ternfuse) mount is `$RUSTERN_TEST_DATA_DIR/fuse_loc1/mnt` and the corpus is its
  `corpus/` subdirectory. A cluster test that cannot reach the cluster fails with a
  message naming the variable; it never passes silently.
- Fault tests that stop or corrupt cluster processes run in the nextest test group
  `cluster-serial` (`max-threads = 1`; selected by living in `mod cluster_serial`, since
  nextest rejects `binary()` filters that match no binary yet) and restore what they changed, even on failure
  (restore in a `Drop` guard).
- Upstream facts are quoted from `ternfs/` at commit `5550891`; paths like
  `go/msgs/msgs.go:661` refer to that checkout.
- Lints from `[workspace.lints]` apply everywhere outside tests: no `unwrap`, `expect`,
  `panic!`, unchecked indexing or unchecked arithmetic on request paths.
- Files derived from upstream Apache code start with
  `// Copyright 2025 XTX Markets Technologies Limited` and
  `// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception`, and are listed in `NOTICE`.
- "Run the merge gates" means `scripts/gate.sh` (full, with cluster) unless the task says
  `--quick`. The task is done only when the gate passes.

---

## Phase 1: Setup (Shared Infrastructure)

**Purpose**: Workspace, tooling and gate script, so every later task has a place to land
and a way to be checked.

- [X] T001 Convert the root `Cargo.toml` into a virtual workspace: `[workspace] members = ["crates/*"]`, `resolver = "3"`; `[workspace.package]` with `version = "0.1.0"`, `edition = "2024"`, `rust-version = "1.100"`, `license = "MIT OR Apache-2.0"`, `repository` left empty for now; remove the root `[package]`; move `src/main.rs` to `crates/rustern/src/main.rs` and delete the root `src/`
- [X] T002 Add `[workspace.lints]` to `Cargo.toml`: `rust.unsafe_code = "forbid"`; `clippy.unwrap_used`, `clippy.expect_used`, `clippy.panic`, `clippy.indexing_slicing`, `clippy.arithmetic_side_effects` = `"deny"`; `clippy.all = "warn"`. Each crate's `Cargo.toml` gets `[lints] workspace = true`
- [X] T003 [P] Create `rust-toolchain.toml` (`channel = "1.100.0"`, `components = ["rustfmt", "clippy"]`) and `clippy.toml` with `allow-unwrap-in-tests = true`, `allow-expect-in-tests = true`, `allow-panic-in-tests = true`, `allow-indexing-slicing-in-tests = true`; test modules that need unchecked arithmetic add `#[allow(clippy::arithmetic_side_effects)]` at module level
- [X] T004 Create the five crate skeletons with `Cargo.toml` and an empty `src/lib.rs` (or `src/main.rs` for the binary): `crates/rustern-msgs` (its `license = "(MIT OR Apache-2.0) AND Apache-2.0 WITH LLVM-exception"`), `crates/rustern-core`, `crates/rustern-client`, `crates/rustern-fuse`, `crates/rustern`; path dependencies only in the allowed direction (Conventions)
- [X] T005 Add `[workspace.dependencies]` to `Cargo.toml` with the versions and features from plan.md's Dependency Justification table: `fuser = "0.18"` (default features), `tokio = { version = "1", features = ["rt-multi-thread", "net", "time", "sync", "signal", "macros", "io-util"] }`, `bytes = "1"`, `crossbeam-queue = "0.3"`, `futures-util = { version = "0.3", default-features = false, features = ["std"] }`, `dashmap = "6"`, `backon = { version = "1", default-features = false, features = ["std"] }`, `fastrand = "2"`, `moka = { version = "0.12", features = ["future"] }`, `crc32c = "0.6"`, `thiserror = "2"`, `clap = { version = "4", features = ["derive"] }`, `tracing = "0.1"`, `tracing-subscriber = { version = "0.3", features = ["fmt", "env-filter"] }`, `metrics = "0.24"`, `metrics-exporter-prometheus = { version = "0.18", default-features = false }`, `rustix = { version = "1", features = ["process"] }` (for `-signal-parent`, see T072); dev: `proptest = "1"`, `criterion = "0.7"`, `dhat = "0.3"`. Each crate references only what it uses (`dep.workspace = true`); commit `Cargo.lock`
- [x] T006 [P] (Done during `/speckit-tasks`.) Add the `rustix` row to the Dependency Justification table in `specs/001-fuse-client-poc/plan.md` (purpose: send SIGUSR1 to the parent for `-signal-parent`; baseline Syscalls entry; feature `process`), and correct the Structure Decision line to `replace github.com/XTXMarkets/ternfs/go => ../ternfs/go` in `tools/go.mod` (upstream module path is `github.com/XTXMarkets/ternfs/go`)
- [X] T007 [P] Create `deny.toml`: `[advisories]` deny vulnerabilities and unmaintained; `[licenses] allow = ["MIT", "Apache-2.0", "Apache-2.0 WITH LLVM-exception", "BSD-2-Clause", "BSD-3-Clause", "ISC", "Unicode-3.0", "Zlib"]`; `[bans] multiple-versions = "warn"`, `wildcards = "deny"`; `[sources] unknown-registry = "deny"`, `unknown-git = "deny"`, `allow-git = []`
- [X] T008 [P] Extend `.gitignore` with `/target`, `/ternfs/`, `crates/*/fuzz/target`, `crates/*/fuzz/artifacts` (fuzz `corpus/` directories are committed)
- [X] T009 [P] Create `.config/nextest.toml`: default profile with `slow-timeout = { period = "60s", terminate-after = 5 }`; `[test-groups] cluster-serial = { max-threads = 1 }`; an override assigning `test(/^cluster_serial::/)` to `cluster-serial`
- [X] T010 [P] Create `tools/go.mod` (module `rustern.local/tools`, `go 1.25`, `require github.com/XTXMarkets/ternfs/go v0.0.0`, `replace github.com/XTXMarkets/ternfs/go => ../ternfs/go`) and placeholder `tools/rsgen/main.go` and `tools/goldens/main.go` that compile; run `go mod tidy` in `tools/` and commit `tools/go.sum`
- [X] T011 [P] Create `NOTICE`: project name, then "This product includes software derived from TernFS (https://github.com/XTXMarkets/ternfs), Copyright 2025 XTX Markets Technologies Limited, licensed under Apache-2.0 WITH LLVM-exception", then a list of derived files (initially `tools/rsgen/main.go`, `crates/rustern-msgs/src/generated.rs`, `crates/rustern-msgs/src/spans.rs`), each with its upstream source path
- [X] T012 [P] Create `docs/deviations.md` with the DEV-001…DEV-006 table copied from spec.md's "Deviations from ternfuse" section (columns ID, behaviour, ternfuse, Rustern, rationale, test name); each later story task fills in the test file path
- [X] T013 Create `scripts/cluster.sh` with subcommands `start`, `stop`, `status`: `start` refuses a data directory on tmpfs (`stat -f -c %T` = `tmpfs`) and one with less than 20 GB free, runs `ternfs/build/alpine/ternrun -leader-only -data-dir "$DATA_DIR" -binaries-dir "$PWD/ternfs/build/alpine/"` in the background (`DATA_DIR` defaults to `$PWD/ternfs/mnt`), logs to `$DATA_DIR/../ternrun.log`, waits up to 120 s for the line containing `operational`, writes `$DATA_DIR/../ternrun.pid`; `stop` sends SIGTERM, waits 30 s, then runs `fusermount3 -u` on any leftover mount under `$DATA_DIR`; `status` exits 0 only if the registry port `127.0.0.1:10001` accepts TCP
- [X] T014 Create `scripts/gate.sh` (bash, `set -euo pipefail`): check required tools (`cargo`, `cargo-nextest`, `cargo-deny`, `go`, `fusermount3`; full mode adds `cargo-fuzz` and `fio` is not required) and fail naming the missing one; check `git -C ternfs rev-parse HEAD` starts with `5550891`; run `cargo fmt --all --check`; `cargo clippy --workspace --all-targets -- -D warnings`; `cargo nextest run --workspace` (default set, includes fuzz corpus replay tests); `cargo deny check`; codec drift check (`go run ./tools/rsgen` into a temp file, `diff` against `crates/rustern-msgs/src/generated.rs`, skip with a notice until T024 lands); dependency direction check (`cargo tree -e normal -i fuser --workspace --prefix none` lists only `rustern-fuse` and `rustern`); without `--quick`: start the cluster with `scripts/cluster.sh start` unless `status` succeeds, run `scripts/make-corpus.sh` if `corpus/` is missing, run `cargo nextest run --workspace --run-ignored all`, stop the cluster only if this script started it (trap on EXIT)
- [X] T015 Run `scripts/gate.sh --quick` on the empty workspace and fix until it passes

**Checkpoint**: empty workspace builds, lints and gates run.

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: Generated codecs, core types, transport, error mapping, the async FUSE bridge
and the cluster test harness. Every story depends on these.

**⚠️ CRITICAL**: No user story work can begin until this phase is complete.

### Wire primitives and generated codecs (`rustern-msgs`)

- [X] T016 [P] Write failing unit tests in `crates/rustern-msgs/src/wire.rs` for: little-endian `u8/u16/u32/u64` round trips; `string`/`[]byte` = u8 length + bytes (encoding more than 255 bytes is an `EncodeError`); `Blob` and lists = u16 length, lists tagged `len32` = u32 length; fixed byte arrays raw; truncated input at every byte position returns `DecodeError::Truncated` (never panics); a list whose count `n` gives `n × MIN_ENCODED_SIZE` greater than the remaining bytes returns `DecodeError::LengthExceedsInput` before any allocation; trailing bytes after a top-level message return `DecodeError::TrailingBytes`
- [X] T017 Implement `crates/rustern-msgs/src/wire.rs` (about 150 lines) on `bytes::{Buf, BufMut, Bytes}`: traits `Encode { fn encoded_len(&self) -> usize; fn encode(&self, out: &mut impl BufMut) }` and `Decode: Sized { const MIN_ENCODED_SIZE: usize; fn decode(buf: &mut Bytes) -> Result<Self, DecodeError> }`, a top-level `decode_exact`, impls for integers, `[u8; N]`, `Bytes` (u8 length), `Blob` (u16 length), `Vec<T>` (u16) and a `Len32<Vec<T>>` wrapper (u32); `DecodeError` and `EncodeError` via `thiserror`; byte strings are `Bytes`, never `String` (names may be non-UTF-8)
- [X] T018 [P] Implement `tools/goldens/main.go`: import `github.com/XTXMarkets/ternfs/go/msgs` and upstream `go/core/crc32c`; for each message the PoC uses (`LocalShardsReq/Resp`, `LookupReq/Resp`, `StatFileReq/Resp`, `StatDirectoryReq/Resp`, `ReadDirReq/Resp`, `LocalFileSpansReq/Resp` including inline and blocks `FetchedSpan` variants and mirrored (D=1) and RS (D=10, P=4) parities, `FetchBlockWithCrcReq/Resp`, and an ERROR response) build deterministic samples (empty lists, 255-byte names, non-UTF-8 names, maximum list lengths that fit 1472 bytes) and write `testdata/goldens/<TypeName>.hex`, one sample per line as `<hex> # <description>`, using upstream `Pack`; also write full request/response frames (`testdata/goldens/frames.hex`, using the headers in research R4) and CRC32C cases (`testdata/goldens/crc32c.hex`: empty, `123456789`, random buffers with their append, combine and zero-extend results from upstream `crc32c`); run it and commit `testdata/goldens/`
- [X] T019 [P] Write failing golden tests in `crates/rustern-msgs/tests/goldens.rs`: for every line of every `testdata/goldens/*.hex` (except `crc32c.hex`) decode with the matching type, re-encode, assert byte-identical output; assert that decoding every strict prefix returns an error and does not panic
- [X] T020 [P] Write a failing test in `crates/rustern-msgs/tests/redaction.rs`: `format!("{:?}", msg)` of every generated type that has a cookie, certificate, secret key or block proof field (rsgen's secret field list, T021) contains `<redacted>` and not the field bytes (FR-017)
- [X] T021 Implement `tools/rsgen/main.go` (derived file, SPDX header, listed in `NOTICE`), adapted from `ternfs/go/bincodegen/bincodegen.go`: copy its error list and shard, CDC, registry and blocks request/response lists from `main()` (`bincodegen.go:1580` onwards), reflect over the `msgs` types the same way, and emit one Rust file: protocol constants (`SHARD_REQ_PROTOCOL_VERSION = 0x414853`, `SHARD_RESP_PROTOCOL_VERSION = 0x1414853`, `CDC_*`, `REGISTRY_REQ_PROTOCOL_VERSION = 0x554853`, `REGISTRY_RESP_PROTOCOL_VERSION = 0x1554853`, `BLOCKS_REQ_PROTOCOL_VERSION = 0x4f4c42`, `BLOCKS_RESP_PROTOCOL_VERSION = 0x14f4c42`), `ErrorCode` (`#[repr(u16)]` with `TryFrom<u16>`), message-kind enums per service, one struct per message type, newtypes for Go named scalars (`InodeId`, `BlockId`, `Parity`, `StorageClass`, `TernTime`, …), `Encode`/`Decode` impls as straight-line code calling `wire.rs`, `MIN_ENCODED_SIZE` constants, a `Secret<T>` wrapper (Debug prints `<redacted>`) for fields named in a secret list (cookies, certificates, secret keys, proofs), and a marker trait `ReadOnlyRequest` implemented only for `LookupReq`, `StatFileReq`, `StatDirectoryReq`, `ReadDirReq`, `LocalFileSpansReq`, `FileSpansReq`, `LocalShardsReq` and `FetchBlockWithCrcReq` (read-only kinds, see `cpp/shard/ShardDB.cpp` `readOnlyShardReq`). Skip `FetchedSpan` and `FetchedFullSpan` (hand-written, as upstream does). Output is deterministic and piped through `rustfmt --edition 2024`
- [X] T022 Run `go run ./tools/rsgen > crates/rustern-msgs/src/generated.rs` from the repository root, add `mod generated; pub use generated::*;` in `crates/rustern-msgs/src/lib.rs`, and make the crate build cleanly under the workspace lints (generated code must not use indexing or unchecked arithmetic)
- [X] T023 Hand-write `crates/rustern-msgs/src/spans.rs` (derived file, SPDX header): `FetchedSpan { header: FetchedSpanHeader, body: FetchedSpanBody }` and `FetchedFullSpan` likewise, where the body is `Inline(FetchedInlineSpan)` when `header.storage_class == INLINE (1)` and `Blocks(FetchedBlocksSpan)` otherwise, with `Encode`/`Decode`, matching `go/msgs/msgs.go:1007-1075` and its hand-written Go codec
- [X] T024 Make T019 and T020 pass; enable the codec drift check in `scripts/gate.sh`
- [X] T025 [P] Write failing frame tests in `crates/rustern-msgs/src/frame.rs` against `testdata/goldens/frames.hex`: shard/CDC request `u32 protocol, u64 request_id, u8 kind, body` and response `u32 protocol, u64 request_id, u8 kind, body` where kind ERROR carries a `u16` code; registry request `u32 protocol, u32 length (1 + body), u8 kind, body`; blocks request `u32 protocol, u64 block_service_id, u8 kind, body` and response `u32 protocol, u8 kind (or ERROR + u16), body`; wrong protocol, unknown kind and short frames are errors
- [X] T026 Implement `crates/rustern-msgs/src/frame.rs` (frame encode/decode helpers returning the kind and a `Bytes` body) to pass T025
- [X] T027 [P] Add `crates/rustern-msgs/src/fuzz_support.rs` (`#[doc(hidden)] pub`) with `shard_resp(&[u8])`, `registry_resp(&[u8])` and `blocks_resp(&[u8])`: decode the frame, then the body for its kind; if decoding succeeds, re-encode and assert that decoding the re-encoded bytes gives an equal value
- [X] T028 [P] Create the fuzz crate `crates/rustern-msgs/fuzz/` (`cargo fuzz init` layout, excluded from the workspace) with targets `fuzz_targets/shard_resp.rs`, `registry_resp.rs`, `blocks_resp.rs` calling `fuzz_support`; seed `crates/rustern-msgs/fuzz/corpus/<target>/` from the golden response frames
- [X] T029 [P] Write `crates/rustern-msgs/tests/corpus_replay.rs`: run every file under `crates/rustern-msgs/fuzz/corpus/*/` through the matching `fuzz_support` function on stable Rust (no panics)

### Core types (`rustern-core`)

- [X] T030 [P] Write failing tests in `crates/rustern-core/src/inode.rs`: "Top 3 bits: type (`DIRECTORY` = 1, `FILE` = 2, `SYMLINK` = 3); low byte: shard id"; "type bits must be 1–3, otherwise the id is rejected as malformed (EIO)"; FUSE mapping "ino 1 ↔ `ROOT_DIR_INODE_ID` (`1 << 61`); every other id maps to itself"; FUSE ino 0 is rejected; proptest: for every valid id, `from_fuse(to_fuse(id)) == id`
- [X] T031 Implement `crates/rustern-core/src/inode.rs` (`InodeKind`, `TernInode::new(u64) -> Result<_, InvalidInode>`, `shard()`, `kind()`, `to_fuse()`, `from_fuse()`) to pass T030
- [X] T032 [P] Write failing property tests in `crates/rustern-core/src/crc.rs`: CRC32C of any buffer equals the `append` chain over any split of it; `combine(crc(a), crc(b), len(b)) == crc(a ‖ b)`; `zero_extend(crc(a), n) == crc(a ‖ 0ⁿ)`; every case in `testdata/goldens/crc32c.hex` matches
- [X] T033 Implement `crates/rustern-core/src/crc.rs` over the `crc32c` crate (`crc32c`, `crc32c_append`, `crc32c_combine`; zero-extend as combine with the CRC of `n` zero bytes, matching upstream `go/core/crc32c`) to pass T032

### Client foundation (`rustern-client`)

- [X] T034 [P] Write failing tests in `crates/rustern-client/src/error.rs`: `ClientError` variants `Tern { code: ErrorCode, kind }`, `Timeout { service, elapsed }`, `Decode(DecodeError)`, `CrcMismatch { block_id, page }`, `Connect { addr, source }`, `Io(std::io::Error)`, `Malformed { what }`, `InvalidInode`; `Display` names the service, address or block involved; `Debug` of any variant contains no secret bytes
- [X] T035 Implement `crates/rustern-client/src/error.rs` with `thiserror` to pass T034
- [X] T036 [P] Write failing tests in `crates/rustern-client/src/config.rs`: defaults "Shard timeouts 100 ms / 2 s / 10 s", "CDC timeouts 1 s / 10 s / 60 s", "Block timeouts 1 s / 10 s / 60 s (DEV-006)", growth 1.5, MTU 1472, readdir batch 1000, memory limit 1 GiB, per-file memory limit 32 MiB, registry refresh 60 s, registry alert after 60 s, attribute TTL 1 h, entry TTL 250 ms; `READ_BUF_SIZE` = 1 MiB + 8 KiB; validation rejects zero durations, `initial > max`, a per-file memory limit below 2 MiB or above the memory limit, and a memory limit below one read buffer; an overall deadline above 60 s is accepted with a warning. This file holds the DEV-006 test `block_deadline_default`
- [X] T037 Implement `crates/rustern-client/src/config.rs` (`ClientConfig`, `ReqTimeouts { initial, max, overall }`) to pass T036
- [X] T038 [P] Write failing tests in `crates/rustern-client/src/timeouts.rs`: the schedule is `backon::ExponentialBuilder` with `min_delay = initial`, `max_delay = max`, `factor = 1.5`, `without_max_times()`, no `total_delay`, backon jitter off; each value scaled by a uniform factor in [0.95, 1.05] from `fastrand` and capped at `max` again; property: every value lies in [0.95 × initial, max]; the iterator never ends; with a fixed `fastrand` seed the first values are 100 ms, 150 ms, 225 ms ±5% for shard defaults
- [X] T039 Implement `crates/rustern-client/src/timeouts.rs` to pass T038
- [X] T040 [P] Write failing tests in `crates/rustern-client/tests/udp_demux.rs` with an in-process fake shard (`tokio::net::UdpSocket` on `127.0.0.1:0`): responses are matched by request id; out-of-order responses reach the right callers; duplicate and late responses are counted (`rustern_udp_unmatched_total`) and dropped; datagrams that are short, carry the wrong protocol or an unknown kind are counted (`rustern_udp_malformed_total`) and dropped; dropping a caller's future removes its pending entry (map length returns to 0); request ids increase monotonically from 1; a request whose encoded size exceeds the MTU is rejected before sending; an ERROR response yields `ClientError::Tern`
- [X] T041 Implement `crates/rustern-client/src/udp.rs`: one shared `tokio::net::UdpSocket` per client, a receive task that decodes the frame header and routes the body to a `tokio::sync::oneshot::Sender` held in a `DashMap<u64, Sender<…>>`, an `AtomicU64` request-id counter, and a guard that removes the pending entry on drop (cancel-safe), to pass T040
- [X] T042 [P] Write failing tests in `crates/rustern-client/tests/shard_request.rs` using two fake shard sockets: the first N datagrams dropped → success after retries with per-attempt timeouts from `timeouts.rs` and no sleep between attempts; attempts alternate between Addr1 and Addr2; nothing answers → `ClientError::Timeout` no later than the overall deadline plus 50 ms; a `TIMEOUT` error response is retried like a lost response and any other error code is returned immediately (as upstream `go/client/metadatareq.go:96`); only types implementing `ReadOnlyRequest` compile as arguments (a `compile_fail` doctest with `CreateDirectoryInodeReq`)
- [X] T043 Implement `crates/rustern-client/src/shard.rs`: `async fn shard_request<Req: ReadOnlyRequest, Resp>(&self, shard: u8, req: &Req) -> Result<Resp, ClientError>` with the retry loop (`tokio::time::timeout` per attempt, overall deadline around the loop), to pass T042
- [X] T044 [P] Write failing tests in `crates/rustern-client/tests/registry.rs` with a fake TCP registry: `LocalShardsReq` returns 256 `ShardInfo` entries with two addresses each and the table is built from them; a response with a length field larger than 1 MiB is rejected before allocating; connection refused → `ClientError::Connect` naming the address; a silent registry → timeout within the shard overall deadline; with `registry_alert_after = 200ms` (config field, default 60 s) and the fake registry closed, periodic refreshes every 50 ms emit exactly one `tracing` error event with `alert = true` naming the registry address once failures have lasted more than 200 ms, and none while it answers (captured with `tracing::subscriber::with_default`)
- [X] T045 Implement `crates/rustern-client/src/registry.rs` (TCP request per call: connect, write the framed request, read the framed response with the length checked first) and `crates/rustern-client/src/addresses.rs` (`ShardTable` behind `std::sync::RwLock<Arc<…>>`, `refresh()` from the registry), to pass T044
- [X] T046 Implement `crates/rustern-client/src/lib.rs`: `Client::connect(config) -> Result<Client, ClientError>` binds the UDP socket, fetches the shard table within the shard overall deadline, and starts a periodic refresh every `registry_refresh` (60 s), emitting the alert event of T044 when refreshes have failed for longer than `registry_alert_after` (FR-016); module declarations; no FUSE types
- [X] T047 [P] Write failing tests in `crates/rustern-client/tests/metrics.rs`: install a `metrics_exporter_prometheus::PrometheusBuilder::new().build_recorder()` with `metrics::with_local_recorder` and run a current-thread tokio runtime inside the closure; one lookup against a fake shard gives `rustern_shard_requests_total{kind="LOOKUP",result="ok"} 1` and a `rustern_shard_request_seconds` sample in `handle.render()`; a duplicate response increments `rustern_udp_unmatched_total`; a refused registry gives `rustern_registry_requests_total{result="error"}`; every label value is from the constant lists exported by `metrics.rs` (no inode, block or address values in labels, FR-017)
- [X] T048 Add `crates/rustern-client/src/metrics.rs`: metric names `rustern_shard_requests_total{kind,result}`, `rustern_shard_request_seconds{kind}`, `rustern_registry_requests_total{result}`, `rustern_block_fetches_total{result}`, `rustern_block_fetch_seconds`, `rustern_block_bytes_total`, `rustern_udp_unmatched_total`, `rustern_udp_malformed_total`, recorded through `metrics` macros (no-ops when no recorder is installed); `tracing` spans at debug level around each request with kind, shard and attempt fields; label values from exported constant lists; call them from `shard.rs`, `registry.rs` and `udp.rs`, to pass T047

### FUSE adapter foundation (`rustern-fuse`)

- [X] T049 [P] Write failing tests in `crates/rustern-fuse/src/errno.rs` for `errno_of(&ClientError) -> fuser::Errno`, table from research R8: `FILE_NOT_FOUND`, `DIRECTORY_NOT_FOUND`, `NAME_NOT_FOUND`, `EDGE_NOT_FOUND` → ENOENT; `TYPE_IS_DIRECTORY` → EISDIR; `TYPE_IS_NOT_DIRECTORY` → ENOTDIR; `TIMEOUT` → ETIMEDOUT; `NOT_AUTHORISED`, `BAD_COOKIE` → EACCES; every other code → EIO; `Decode`, `CrcMismatch`, `Connect`, `Malformed`, `InvalidInode`, `Io` → EIO; `Timeout` from a shard or registry → ETIMEDOUT, from a block service → EIO. The match on `ClientError` has no wildcard arm
- [X] T050 Implement `crates/rustern-fuse/src/errno.rs` to pass T049 (the only place errors become errno)
- [X] T051 Define the async interface in `crates/rustern-fuse/src/bridge.rs`: `trait AsyncFilesystem: Send + Sync + 'static` modelled on `fuser::experimental::AsyncFilesystem` (same method names and argument order) with methods `lookup`, `getattr`, `readlink`, `open`, `read`, `release`, `opendir`, `readdir`, `readdirplus`, `releasedir`, `statfs`, each returning `impl Future<Output = Result<…, Errno>> + Send`; response structs like fuser's `LookupResponse`/`GetAttrResponse`; `read` returns a buffer type `impl AsRef<[u8]> + Send` so the reply borrows the pooled buffer (R12); and a skeleton `struct Bridge<F> { fs: Arc<F>, rt: tokio::runtime::Handle }` implementing `fuser::Filesystem` whose methods all reply ENOSYS, so T052 compiles and fails
- [X] T052 [P] Write `crates/rustern-fuse/tests/bridge_mount.rs` (needs `/dev/fuse`, not the cluster): mount a test-only in-memory `AsyncFilesystem` on a fresh directory under `std::env::temp_dir()` and check through `std::fs` that lookup, getattr, readdir, read and readlink return its data; a read that sleeps 2 s does not delay a concurrent `stat` (proves per-request tasks, Principle VI); an error from the test filesystem increments `rustern_fuse_ops_total{op,errno}` (the test binary installs a global `build_recorder()` recorder) and produces one log line with operation, inode and cause; unmount on drop leaves the directory usable
- [X] T053 Implement `Bridge` in `crates/rustern-fuse/src/bridge.rs`: copy arguments, spawn one task per request on `rt` and reply from the task (`read` replies with `ReplyData::data` on the returned buffer); `forget`/`batch_forget` are no-ops; every error reply goes through one helper that records `rustern_fuse_ops_total{op,errno}` and `rustern_fuse_op_seconds{op}` and logs the operation, inode and cause (ENOENT at debug, other errnos at warn; FR-016). Passes T052

### Cluster test harness and corpus

- [X] T054 [P] Create `scripts/make-corpus.sh <dest>`: build files in a local temp directory with `head -c N /dev/urandom > localfile` (redirects only on local disk), then copy into `<dest>` with `cp` (never redirect into the mount, see quickstart). Sizes: 0, 1, 255 (inline: upstream inlines spans under 256 bytes, `go/client/span.go:329`), 256, 4095, 4096, 4097, 65,536 and 65,537 (around the mirrored-span limit: default policy uses parity 1+4 for spans up to 65,536 bytes, `cpp/shard/ShardDB.cpp:4093`), 1 MiB, 5,120,000 and 5,120,001 (around the RS(10,4) limit), 30 MB and 100 MiB (multi-span), 1 GiB; at least 1,000 files across 100 directories nested up to depth 5; one directory `big/` with 10,000 entries; a 255-byte name; names with bytes `\xff\xfe` (non-UTF-8); symlinks: relative, absolute, dangling, to a directory; finally `cd <dest> && sha256sum` of every regular file into `<dest>/corpus.sha256`. Refuses to run if `<dest>` exists unless `--force`
- [X] T055 Create `crates/rustern/tests/common/mod.rs` (shared by cluster tests): `env()` reading `RUSTERN_TEST_REGISTRY` and `RUSTERN_TEST_DATA_DIR` with the defaults from Conventions and a clear failure if the registry does not accept TCP; `ref_corpus()` path; `struct Mount` that creates a fresh mount point under `std::env::temp_dir()`, spawns `env!("CARGO_BIN_EXE_rustern")` with `-registry <addr> <mountpoint>` plus extra args, waits up to 5 s for `mounted at` on stderr (collecting stderr lines for assertions), and on `Drop` sends SIGTERM, waits 10 s, then falls back to `fusermount3 -u`; helpers `walk(path) -> Vec<(relative OsString path, file type, size, mtime)>` and `compare_files(a, b)` reading both in 1 MiB chunks; `pgrep_tern(process, arg)` to find cluster PIDs (one `ternshard` process per shard with `-shard N`, `ternblocks` processes with `-log-file mnt/bs_N/log`), and `find_block_file(block_id)` that finds the file named `format!("{:016x}", block_id)` under `$RUSTERN_TEST_DATA_DIR/bs_*/*/with_crc/`
- [X] T056 Run the merge gates (`scripts/gate.sh`, which starts the cluster and writes the corpus)

**Checkpoint**: codecs proven against upstream, transport and bridge tested; user stories can start.

---

## Phase 3: User Story 1 - Mount and browse the namespace (Priority: P1) 🎯 MVP

**Goal**: `rustern -registry <addr> <mountpoint>` mounts the cluster read-only; directory
listings, names, types, sizes and mtimes match ternfuse; SIGINT/SIGTERM unmount cleanly.

**Independent Test**: mount the corpus with Rustern and compare a recursive listing with
the same listing through the ternfuse mount (`crates/rustern/tests/browse.rs`).

### Tests for User Story 1 (REQUIRED, written first) ⚠️

- [X] T057 [P] [US1] Write failing unit tests in `crates/rustern/src/cli.rs` per `contracts/cli.md`: every flag in the flags table parses in `-name value`, `-name=value`, `--name value` and `--name=value` forms (`-o` stays a short flag); defaults equal the table; Go durations `250ms`, `10s`, `1h`, `1m30s`, `1.5s`, `500us` parse and `10` or `1x` fail; `-o rw` is rejected and `ro,nosuid,nodev` are always present in the resulting options; `-o allow_other` yields options containing both `allow_other` and `default_permissions`, and options without `allow_other` do not gain `default_permissions` (FR-003); each ignored flag (`-close-tracker-object`, `-direct-mount`, `-direct-mount-strict`, `-profile-file`, `-syslog`, `-print-quiet-alerts`, `-xmon`) is accepted and produces a warning; an unknown flag or a missing mount point is an error with exit code 2
- [X] T058 [P] [US1] Write failing unit tests in `crates/rustern/src/main.rs` for `StartupError` → exit code: `InvalidArgs` → 2, `Registry { addr, .. }` → 1 with a message containing the address, `Mount { path, .. }` → 1
- [X] T059 [P] [US1] Write failing unit tests in `crates/rustern-fuse/src/attr.rs` per data-model.md "Attributes": kind from InodeId type bits; size from `StatFileResp.Size`, "directories report 0"; mtime from `StatFileResp.Mtime` / `StatDirectoryResp.Mtime` (TernTime, ns since epoch); atime from `StatFileResp.Atime`, "directories: mtime"; ctime = mtime; "mode 0555 plus type bits"; nlink 1; uid/gid 0, or the daemon's effective ids with `-set-uid`; TTL: files use `-attr-cache-time`, directories use `min(attr TTL, entry TTL)` (DEV-003)
- [X] T060 [P] [US1] Write failing unit tests in `crates/rustern-fuse/src/dirhandle.rs` with a fake pager: "Kernel offsets: 1 = ".", 2 = "..", 3 + n = n-th entry of this handle's listing"; paging follows `next_hash` until it is None ("None = exhausted"); "`seekdir` to an offset already passed re-lists from the start and skips to the position (bounded by entry count); entries are never returned twice for one position"; parent "from `StatDirectoryResp.Owner`, or self for the root"; the handle table rejects more than 65,536 open handles with EMFILE
- [X] T061 [P] [US1] Write failing cluster tests in `crates/rustern-client/tests/cluster_namespace.rs`: `lookup` of existing and missing names (missing → `Tern { code: NAME_NOT_FOUND }`); `stat_file`/`stat_directory` fields equal what the ternfuse mount reports; paging `read_dir` over `corpus/big/` with a 1472-byte MTU returns each of the 10,000 entries exactly once and the set equals `std::fs::read_dir` on the ternfuse mount
- [X] T062 [P] [US1] Write failing tests in `crates/rustern-client/tests/stat_coalescing.rs` with a fake shard that counts requests and answers after 100 ms (Principle VI): 64 concurrent `stat_directory` calls on one inode send exactly 1 request and all get its result; a call after the reply sends a new request (nothing cached); a failed request is returned to every waiter and the next call sends again; calls on different inodes are not coalesced; same checks for `stat_file`
- [X] T063 [P] [US1] Write failing cluster tests in `crates/rustern/tests/browse.rs`: the mount is ready within 5 s and stderr contains `mounted at <path>` (SC-004 part); `walk` of the Rustern mount equals `walk` of the ternfuse corpus (names as raw bytes, types, sizes, mtimes) (SC-001 listing part); `stat` of a missing name fails with ENOENT; the 255-byte and non-UTF-8 names are present; `corpus/big/` lists 10,000 entries once each through both `readdir` and `ls -l` (readdirplus); SIGINT and SIGTERM each unmount (mount point absent from `/proc/self/mountinfo`, `read_dir` on it succeeds, exit status 0); `-registry 127.0.0.1:1` exits 1 within the shard overall deadline with the address in the message and leaves no mount; an unknown flag exits 2; `kill -USR1` makes the log contain `rustern_fuse_ops_total`; the mount's line in `/proc/self/mountinfo` has `ro`, `nosuid` and `nodev`; `allow_other_default_permissions`: with `-o allow_other` the super options contain `allow_other` and `default_permissions` (fails with a message naming `user_allow_other` in `/etc/fuse.conf` when it is missing); `external_unmount_exits`: `fusermount3 -u <mountpoint>` while Rustern runs makes it exit 0 within 5 s with no mount left; `signal_parent`: a parent `sh -c 'trap "echo got-usr1" USR1; rustern … -signal-parent & wait; wait'` prints `got-usr1` after `mounted at`; `dir_attr_validity` (DEV-003): after `cp` of a new file into `$REF/scratch/` through ternfuse, the directory's mtime through Rustern changes within 300 ms
- [X] T064 [US1] Run T057–T063 and record in the commit message that they fail for the expected reason (missing code, not a broken harness)

### Implementation for User Story 1

- [X] T065 [P] [US1] Implement `crates/rustern-client/src/namespace.rs`: `lookup(dir, name) -> LookupResp`, `stat_file(id)`, `stat_directory(id)`, `read_dir(dir, start_hash) -> ReadDirResp` (one page; the caller pages) using `shard_request` on `id.shard()`; `stat_file` and `stat_directory` share in-flight requests through a `moka::future::Cache` keyed by inode (`try_get_with`, entry invalidated as soon as the load completes, so nothing is cached; research R6), to pass T061 and T062
- [X] T066 [P] [US1] Implement `crates/rustern-fuse/src/attr.rs` (`FileAttr` and TTL from stat responses and `AttrConfig`) to pass T059
- [X] T067 [P] [US1] Implement `crates/rustern-fuse/src/dirhandle.rs` (`DirHandle` with directory id, parent id, `next_hash: Option<NameHash>`, buffered entries, position counter; `DirHandles` table in a `DashMap<u64, Mutex<DirHandle>>` with a bounded count) to pass T060
- [X] T068 [US1] Implement `crates/rustern-fuse/src/fs.rs` `RusternFs: AsyncFilesystem` for the namespace operations of `contracts/filesystem.md`: `lookup` (`Lookup`, then `StatFile`/`StatDirectory`; symlinks use `StatFile`), replying with the entry TTL; a missing name replies with a negative entry (ino 0) cached for the entry TTL; `getattr`; `opendir` (`StatDirectory`, ENOTDIR for non-directories); `readdir`; `readdirplus` with each buffered entry stat-ed concurrently (`tokio::task::JoinSet` bounded by a `tokio::sync::Semaphore` of `readdir_batch_size`); `releasedir`; `statfs` with constant values and the read-only flag (`ST_RDONLY`); `init` requests readdirplus. Depends on T065–T067
- [X] T069 [US1] Implement `crates/rustern/src/cli.rs`: an argv shim rewriting `-name[=v]` to `--name[=v]` for the known long flag names before clap parses, the clap derive struct with every flag from `contracts/cli.md`, a Go duration parser (units `ns`, `us`, `µs`, `ms`, `s`, `m`, `h`, decimals, concatenated components), warnings for ignored flags, and conversion to `ClientConfig` plus FUSE options, to pass T057
- [X] T070 [US1] Implement `crates/rustern/src/main.rs` start-up per `contracts/cli.md`: parse flags (failure → stderr, exit 2); set up `tracing-subscriber` (stderr or `-log-file`; `-verbose` = debug, `-trace` = trace, otherwise info); install the `metrics-exporter-prometheus` recorder with `build_recorder()` and `metrics::set_global_recorder`, keeping its handle for the SIGUSR1 dump (no HTTP listener); build a multi-thread tokio runtime; `Client::connect` (failure → message naming the registry, exit 1, nothing mounted); mount with fuser 0.18's background session API (`Bridge` over `RusternFs`), `Config` with `n_threads = --threads` (default available CPUs capped at 16), options `ro`, `nosuid`, `nodev`, `FSName("ternfs")`, `Subtype("rustern")`, then the user's validated `-o` options from `cli.rs` (which adds `default_permissions` whenever `allow_other` is given, FR-003); no `AutoUnmount`; in `init`, `KernelConfig::set_max_readahead(1 MiB)` and `set_max_write(1 MiB)` (as ternfuse; confirm in fuser's source that this makes the kernel send reads of up to 1 MiB) and log the negotiated maximum read at info; log `mounted at <path>`; `StartupError` and exit codes, to pass T058
- [X] T071 [US1] Add the shutdown future to `crates/rustern/src/main.rs`: `tokio::select!` over `tokio::signal::unix` SIGINT, SIGTERM and SIGHUP (→ unmount, wait for in-flight requests up to 5 s, exit 0), the background session ending on its own (external `fusermount3 -u` → log at info, exit 0) and a fatal runtime error (→ unmount, exit 1); SIGUSR1, handled in a separate loop, renders the Prometheus handle and writes it to the log at info level
- [X] T072 [US1] Implement `-signal-parent` in `crates/rustern/src/main.rs`: after `mounted at`, send SIGUSR1 to the parent with `rustix::process::kill_process(getppid, Signal::Usr1)`; passes T063 `signal_parent`
- [X] T073 [US1] Fill in DEV-003's test path (`crates/rustern/tests/browse.rs::dir_attr_validity`) in `docs/deviations.md`
- [ ] T074 [US1] Run the merge gates

**Checkpoint**: the cluster mounts and browses correctly; MVP demonstrable.

---

## Phase 4: User Story 2 - Read file contents correctly (Priority: P1)

**Goal**: bytes read through Rustern equal the bytes ternfuse returns for every layout
(inline, mirrored, RS with all data blocks), sequentially, at random offsets and
concurrently; symlink targets read correctly.

**Independent Test**: client-level reads against the cluster (`crates/rustern-client/tests/cluster_read.rs`)
need no mount; mount-level checksum comparison (`crates/rustern/tests/differential.rs`)
uses the US1 mount.

### Tests for User Story 2 (REQUIRED, written first) ⚠️

- [X] T075 [P] [US2] Write failing unit tests in `crates/rustern-core/src/span.rs` for `Span::try_from(FetchedSpan, &[BlockService])`, rules quoted from data-model.md: "`parity.data_blocks` ≥ 1, `blocks.len() == data + parity`, `stripes` in 1..=15, `cell_size` a non-zero multiple of 4096"; "`size ≤ data_blocks × cell_size × stripes` (checked arithmetic)"; "every `block_service_ix` < `block_services.len()`"; "inline body length == `size`"; an inline body whose CRC does not match the header CRC is rejected ("verified against crc on decode"); each failure is a distinct `SpanError` variant
- [X] T076 [P] [US2] Write failing property tests in `crates/rustern-core/src/layout.rs`: for span offset `o`, stripe = `o / (D × cell)`, data block = `(o mod (D × cell)) / cell`, block offset = `stripe × cell + o mod cell`; `ranges(span, from, to)` returns page-aligned (4096) block ranges and output ranges whose union covers `[from, to)` exactly once (checked with the inverse mapping); bytes at or past `size` map to zero padding; for D = 1 (mirrored) every block holds the whole span
- [X] T077 [P] [US2] Write failing unit tests in `crates/rustern-client/src/blocks.rs` for the page stream scatter state and verifier (R12): the stream is `page_count` pages of 4096 data bytes each followed by a little-endian `u32` CRC32C; `Scatter::next_slices(&mut pages, &mut crcs)` yields the `IoSliceMut` list for the bytes still missing (data into the caller's 4096-byte page slices, CRC words into a side array, at most 1024 slices); proptest: for any split of the stream into chunks, filling through `Scatter` gives pages equal to the stream with CRC words stripped and the CRC array equal to the CRC words; a mismatch returns `CrcMismatch { block_id, page }` with the page index; a short stream returns `Malformed`; the page count comes from the request, never from the server
- [X] T078 [P] [US2] Write failing tests in `crates/rustern-client/tests/block_fetch.rs` with a fake TCP block service: the request frame is `u32 BLOCKS_REQ_PROTOCOL_VERSION, u64 block_service_id, u8 FETCH_BLOCK_WITH_CRC` plus the `FetchBlockWithCrcReq` body; an ERROR response yields `Tern`; a server that stalls hits the block timeout schedule and then `Timeout { service: Blocks }`; connection refused yields `Connect`
- [X] T079 [P] [US2] Write failing tests in `crates/rustern-client/src/cache.rs` for `SpanCache`: `get_or_load(key, loader)` called by 64 concurrent tasks with the same key runs the loader once and all receive the same span map; a failed load is not cached and the next call retries; the cache is keyed by inode and holds the file's whole span list sorted by offset; `span_at(offset)` finds the containing span by binary search, including the first and last byte of each span and offsets in gaps (an error); a 1,000-span list loads by paging until the response has no next offset; the cache is bounded by total span count (weigher). There is no block cache (R6, FR-011)
- [X] T080 [P] [US2] Write failing unit tests in `crates/rustern-client/src/bufpool.rs`: `BufPool::new(capacity)` with capacity `memory_limit / READ_BUF_SIZE`; `get()` returns a `READ_BUF_SIZE` buffer; a dropped buffer returns to the pool and the next `get` reuses it (allocation counter in the pool unchanged); the pool allocates only when empty and never frees a buffer while below capacity (after N concurrent gets and drops, N more gets allocate nothing); a returned buffer beyond capacity is freed (defensive; permits make it unreachable); with every `get` made while holding one buffer's worth of permits from a semaphore sized to the memory limit, the number of live buffers never exceeds the capacity (proptest over get/drop sequences)
- [X] T081 [P] [US2] Add a page-stream fuzz target `crates/rustern-client/fuzz/fuzz_targets/page_stream.rs` (fuzz crate excluded from the workspace) calling a `#[doc(hidden)] pub fn page_stream(&[u8])` in `crates/rustern-client/src/blocks.rs` (scatter, then verify, never panic), and `crates/rustern-client/tests/corpus_replay.rs` replaying `crates/rustern-client/fuzz/corpus/page_stream/`
- [X] T082 [P] [US2] Write failing cluster tests in `crates/rustern-client/tests/cluster_read.rs`: for every corpus size class, `client.open_file(inode)?.read(offset, len)` equals the bytes read from the ternfuse mount; 1,000 proptest cases (fixed seed) of random offsets and lengths, biased to cross span and stripe boundaries, on the 30 MB, 100 MiB and 1 GiB files; reads at end of file return 0 bytes and reads crossing it are short exactly at end of file; `read_link` returns each symlink target byte-exact
- [X] T083 [P] [US2] Write failing cluster tests in `crates/rustern/tests/differential.rs` (SC-001): `sha256sum -c corpus.sha256` inside the Rustern mount succeeds for every file; `compare_files` of every corpus file between the two mounts succeeds; `readlink` and following each symlink give the same results as through ternfuse
- [X] T084 [P] [US2] Write failing cluster tests in `crates/rustern/tests/random_reads.rs`: 10,000 random `pread` calls (fixed seed, sizes 1 B–1 MiB) on the multi-span files compared against the ternfuse mount
- [X] T085 [P] [US2] Write a failing cluster test in `crates/rustern-client/tests/read_alloc.rs` (`#[global_allocator] dhat::Alloc`, `dhat::Profiler::builder().testing()`): after 10 warm-up reads, 100 sequential 1 MiB reads of the 1 GiB file through `FileHandle::read` allocate under 64 KiB of heap in total per read (no allocation sized to file data: pooled buffer, no copy, R12); print allocations per read for `benchmarks.md` (CT-4)
- [X] T086 [P] [US2] Write failing cluster tests in `crates/rustern/tests/concurrency.rs`: 16 processes (`std::process::Command` running `sha256sum`) each read a different large file at once and every checksum matches; 16 threads read the same 100 MiB file at once on a fresh mount and `rustern_block_fetches_total` (from a SIGUSR1 dump) is no more than 3× the count for one reader alone on another fresh mount, and `rustern_block_bytes_total` is no more than 1.1 × the file size; a second read of the file after closing it adds no block fetches (kernel page cache sharing, FR-011); with `--file-memory-limit 2MiB`, 16 threads reading one 1 GiB file still all match their checksums while a concurrent `sha256sum` of a 100 MiB file finishes within 2× its time alone (FR-015 per-file limit)
- [X] T087 [US2] Run T075–T086 and record that they fail for the expected reason (done late: the unit tests were written together with their code and the cluster tests after it, so no red run was recorded; see the implementation notes)

### Implementation for User Story 2

- [X] T088 [P] [US2] Implement `crates/rustern-core/src/span.rs` (`Span { byte_offset, size, crc, body: Inline { data } | Blocks { parity, stripes, cell_size, blocks: Vec<BlockRef { service, block_id, crc }>, stripes_crc } }`) to pass T075
- [X] T089 [P] [US2] Implement `crates/rustern-core/src/layout.rs` (`locate`, `ranges`, `inverse`, all checked arithmetic, no allocation proportional to network values beyond the block count) to pass T076
- [X] T090 [P] [US2] Implement `crates/rustern-client/src/bufpool.rs` (a `deadpool::managed` `Manager` for `READ_BUF_SIZE` buffers, `QueueMode::Lifo`, `max_size` = capacity; superseded the first `crossbeam_queue::ArrayQueue`, which was FIFO; a `PooledBuf` that derefs to `[u8]` and returns itself on drop; buffers allocated lazily, never freed, no preallocation) to pass T080
- [X] T091 [US2] Implement `crates/rustern-client/src/blocks.rs`: `fetch_pages(service, block_id, page_start, pages: &mut [&mut [u8]]) -> Result<(), ClientError>` over a new `tokio::net::TcpStream` per fetch (`set_nodelay(true)`; later pooled, see research.md "Block-service TCP"), each attempt under the block timeout schedule and the whole fetch under the block overall deadline; after the response header reads exactly `pages.len() × 4100` bytes with `readable().await` + `try_read_vectored` through `Scatter`, so data lands in the caller's slices with no copy, then verifies every page in place, to pass T077 and T078
- [X] T092 [US2] Implement `crates/rustern-client/src/cache.rs`: `SpanCache` backed by `moka::future::Cache<InodeId, Arc<[Span]>>` with a weigher counting spans and `max_capacity = 1_048_576`, loading the whole span list through `try_get_with` and `span_at` by binary search; no block cache (R6); to pass T079
- [X] T093 [US2] Implement `crates/rustern-client/src/read.rs`: `Client::open_file(inode) -> FileHandle` and `FileHandle::read(offset, size) -> Result<ReadBuf, ClientError>` (`ReadBuf` = a `PooledBuf` covering the page-aligned range plus the requested sub-range, `AsRef<[u8]>`); `read` finds the spans overlapping the range with `SpanCache::span_at` (the file's span list is loaded on first use); inline spans are sliced from the verified body; block spans use `layout::ranges` and split the pooled buffer into disjoint 4096-byte page slices and fetch the needed pages of each data block concurrently with `futures_util::future::try_join_all` (no spawn, R12), using block 0 for mirrored spans; the assembled bytes are clamped at file size (short read only at end of file) and padding past span size is zeros; when one read covers a whole span, the span CRC is also checked with `crc::combine`/`zero_extend` (FR-009); read buffers are bounded by a global `tokio::sync::Semaphore` sized to the memory limit and a per-file semaphore sized to the per-file memory limit, each read acquiring `READ_BUF_SIZE` permits from both before taking a `PooledBuf` (validation guarantees both limits hold at least one buffer, so no acquisition can wait forever) (an `Arc` held by every `FileHandle` of the inode and found through a `dashmap` keyed by inode; `open_file` creates it, the last `FileHandle` dropped removes it; per-file permits are taken before global ones); a request larger than 1 MiB is rejected with EIO (the kernel never sends one after `init`); `Client::read_link(inode)` reads the whole symlink body. Passes T082 and T085
- [X] T094 [US2] Add to `crates/rustern-fuse/src/fs.rs`: `open` accepting `O_RDONLY`, storing `Client::open_file` in a `DashMap<u64, FileHandle>` under a new fh, replying `FOPEN_KEEP_CACHE` and sending no request (DEV-002); `read` calling `FileHandle::read` and returning the `ReadBuf` (the bridge replies with its slice); `release` removing the `FileHandle`; `readlink` (EINVAL when the inode is not a symlink); passes T083, T084 and T086
- [ ] T095 [US2] Run the merge gates

**Checkpoint**: US1 and US2 together are "the client works": browse plus correct bytes.

---

## Phase 5: User Story 3 - Fail safely, never wrongly (Priority: P2)

**Goal**: corrupt blocks, stopped services, malformed traffic and mutation attempts fail
only the affected request, with a log line; the mount never dies; unverified bytes are
never returned; mirrored spans fail over.

**Independent Test**: `crates/rustern/tests/faults.rs` and
`crates/rustern/tests/refuses_mutations.rs` against a mounted cluster.

### Tests for User Story 3 (REQUIRED, written first) ⚠️

- [X] T096 [P] [US3] Write failing cluster tests in `crates/rustern/tests/refuses_mutations.rs` (DEV-001, SC-006): on the Rustern mount, create, write via `open(O_WRONLY)`, `open(O_RDWR)`, `open(O_TRUNC)`, `open(O_CREAT)`, `O_TMPFILE`, mkdir, rmdir, unlink, rename, symlink, link, mknod, chmod, truncate, utimes, setxattr, removexattr, fallocate and `copy_file_range` into the mount all fail with EROFS; `getxattr` fails with ENODATA and `listxattr` returns an empty list; a snapshot of `corpus/` taken through ternfuse (listing, sizes, mtimes, atimes from `StatFile`, sha256) is identical before and after the test; `open_sends_no_mutation` (DEV-002): 100 open+read cycles on one file leave its atime unchanged through ternfuse
- [X] T097 [P] [US3] Write failing cluster tests in `crates/rustern/tests/faults.rs` (test group `cluster-serial`, each test on a fresh mount with `-overall-block-timeout 5s -overall-shard-timeout 3s`; after every test the mount, when alive, still answers `stat` on the root, SC-003) for block faults: `corrupt_block_eio` — take an RS file's first block id from `LocalFileSpans`, flip byte 0 of its file found by `find_block_file` (restore in `Drop`), reading that range fails with EIO, the log has an `alert = true` error line with the inode, block id and page, and other ranges of the file and other files read correctly (SC-002); `mirrored_failover` — corrupt the first block of a mirrored file, the read returns correct bytes and the log records the failover; `dead_block_service_eio` — SIGSTOP the `ternblocks` process holding an RS block (SIGCONT in `Drop`), the read fails with EIO within 5 s + 1 s and a mirrored file whose other copies are elsewhere still reads; `mirrored_all_copies_alert` — corrupt every copy of one block of a mirrored file: the read fails with EIO and the log has one `alert = true` line for the span
- [X] T098 [US3] Add metadata and process faults to `crates/rustern/tests/faults.rs` (same setup as T097): `stopped_shard` — SIGSTOP the `ternshard` process with `-shard N` for a directory's shard, `stat` of a name in it fails with ETIMEDOUT within 3 s + 1 s, after SIGCONT the same `stat` succeeds without restarting Rustern; `registry_stopped_after_start` — SIGSTOP `ternregistry`, reads of already-resolved paths still succeed; `interrupt_bounded_by_deadline` (DEV-005) — with the shard stopped, `timeout -s INT 1 stat <path>` returns no later than the shard overall deadline + 1 s; `sigkill_cleanup` — SIGKILL Rustern, `stat <mountpoint>` reports ENOTCONN, `fusermount3 -u` restores it
- [X] T099 [US3] Add concurrent-change and secrecy tests to `crates/rustern/tests/faults.rs` (same setup as T097): `dir_removed_while_listing` — create `$REF/scratch/rm-me/` with 3,000 files through ternfuse, open it on Rustern and read the first 100 entries, `rm -r` it through ternfuse, finish the listing: the result is ENOENT or a partial listing with no duplicate names; `open_file_deleted` — copy a 30 MB file into `$REF/scratch/` through ternfuse, open it on Rustern, read 1 MiB, delete it through ternfuse, read the rest: the bytes equal the original or the read fails with ENOENT or EIO, never different bytes; `logs_have_no_secrets` (FR-017) — with `-trace`, walk the corpus, read 10 files and run `corrupt_block_eio`'s fault, then no log field named like `cookie`, `certificate`, `secret` or `proof` has a value other than `<redacted>`, and every metric label value in a SIGUSR1 dump is from the constant lists in `rustern-client` `metrics.rs` and `rustern-fuse` `bridge.rs`
  - Done with 600 files instead of 3,000: ternfuse creates or unlinks about 15 files/s however many callers there are, so 3,000 took about 10 minutes, past the nextest limit of 300 s. 600 still spans many ReadDir batches; the test runs in 143 s.
- [X] T100 [P] [US3] Write failing cluster tests in `crates/rustern-client/tests/relay_faults.rs` (test group `cluster-serial`) with a test-only UDP relay in `crates/rustern-client/tests/support/relay.rs` placed between the client and one real shard (via `ClientConfig::shard_overrides`, T104): 30% loss, 30% duplication, reordering by random delays up to 50 ms, truncated datagrams, wrong request ids, wrong protocol numbers, and length fields larger than the datagram; every lookup either returns the same result as without the relay or fails with a typed error within the deadline; no panic; the unmatched and malformed counters increase. Random bit flips inside otherwise valid bodies are not tested: the protocol has no message checksum, so a flipped field is undetectable by any client
- [X] T101 [P] [US3] Write failing tests in `crates/rustern-client/tests/shard_failover.rs` with a fake registry and two fake shards: after the first shard stops answering, the client refreshes the shard table (at most once per second) and the request succeeds against the new address within the overall deadline (edge case "shard leader change"); `registry_down_after_start`: with the fake registry closed after start-up and the shard moved, the request fails with `Timeout` within the overall deadline, while requests to shards that did not move still succeed (edge case "Registry unreachable after start-up")
- [X] T102 [US3] Run T096–T101 and record that they fail for the expected reason
  - Red runs: `faults.rs` block faults failed first because piped logs carried ANSI colour, so `alert=true` never matched (fixed in `main.rs`), and the buffered read in `dead_block_service_eio` waited for the deadline twice (now O_DIRECT); `open_file_deleted` failed on `fs::copy` (EOPNOTSUPP on ternfuse, now `fs::write`); `shard_failover.rs` failed before T104 (no refresh on failure). Not red first: `open_sends_no_mutation` (passed before any change), the T105 logging (written before its tests), and `relay_faults.rs` (written after T104). Final run: 11/11 `faults.rs`, 1/1 `relay_faults.rs`, 3/3 `shard_failover.rs`, 2/2 `refuses_mutations.rs`.

### Implementation for User Story 3

- [X] T103 [US3] In `crates/rustern-fuse/src/bridge.rs`, override every mutating `fuser::Filesystem` method present in fuser 0.18 (`create`, `mknod`, `mkdir`, `unlink`, `rmdir`, `symlink`, `link`, `rename`, `write`, `setattr`, `setxattr`, `removexattr`, `fallocate`, `copy_file_range`, and `tmpfile` if the trait has it) to reply EROFS synchronously without spawning or contacting the cluster; `getxattr` → ENODATA, `listxattr` → empty; in `crates/rustern-fuse/src/fs.rs` `open` with a write access mode, `O_TRUNC` or `O_CREAT` → EROFS. Passes T096
- [X] T104 [US3] Add `shard_overrides: HashMap<u8, [SocketAddr; 2]>` to `ClientConfig` in `crates/rustern-client/src/config.rs` (test and debugging use; overrides registry addresses for those shards) and the on-failure refresh in `crates/rustern-client/src/shard.rs` and `addresses.rs` (`Timeout` or `Connect` on a shard triggers `refresh()`, rate-limited to once per second, and the retry loop picks up the new addresses). Passes T100 and T101
- [X] T105 [US3] Add mirrored failover to `crates/rustern-client/src/read.rs` (FR-010, DEV-004): for D = 1 spans, try blocks in order until one is verified, logging each failed block at warn with inode, block id, service and reason; a CRC mismatch and a mirrored span with no readable copy are logged as `tracing::error!(alert = true, …)` (FR-016); RS spans with a failed data block return the error (EIO), logged the same way. Passes T097 `mirrored_failover`, `mirrored_all_copies_alert` and `corrupt_block_eio`
- [X] T106 [US3] Make the rest of T097, T098 and T099 pass: confirm block and shard deadlines come from the configured timeouts, that every EIO and ETIMEDOUT reply has a warn log line with operation, inode and cause (FR-016), and that no fault path panics or exits
- [X] T107 [US3] Fill in the test paths for DEV-001, DEV-002, DEV-004, DEV-005 and DEV-006 in `docs/deviations.md`
- [ ] T108 [US3] Run the merge gates

**Checkpoint**: all three stories pass; the PoC is functionally complete.

---

## Phase 6: Performance, Interop & Polish

**Purpose**: measurements the spec requires (recorded, not gated), the soak test, and
documentation checks.

- [X] T109 [P] Add criterion micro-benchmarks: `crates/rustern-msgs/benches/codec.rs` (decode `LocalFileSpansResp` and a full `ReadDirResp`), `crates/rustern-core/benches/crc.rs` (`crc32c` vs `crc-fast` as a bench-only dev-dependency at 4 KiB, 128 KiB and 1 MiB, the comparison the Technology Baseline asks for), `crates/rustern-core/benches/layout.rs` (`ranges` for 4 KiB and 1 MiB reads)
- [X] T110 [P] Add `crates/rustern/examples/meta_latency.rs` (std only): measures p50/p99 of `stat` on an already-looked-up file, cold lookups of distinct names, and a full listing of `corpus/big/` (10,000 entries) on a given mount
- [X] T111 Create `scripts/bench.sh <ref-mount> <rustern-mount>`: `seq-read` with `fio` (1 GiB files, `bs=1M`, `numjobs` 1, 2, 4, 8, 16, `invalidate=1`; the 16-reader run repeated with `--memory-limit 256MiB` to show how much the limit matters), `rand-read` (`bs=4k` and `bs=128k`, 16 jobs, within a 1 GiB file), `meta-latency` and `readdir-latency` via `cargo run --release --example meta_latency`; CPU per GiB from `/proc/<pid>/stat` utime+stime of each daemon; peak RSS from `/proc/<pid>/status` `VmHWM`; writes `specs/001-fuse-client-poc/benchmarks.md` with one row per Performance Targets metric (Rustern, ternfuse, target, met or not)
- [X] T112 Run `scripts/bench.sh` against both mounts on the same cluster and commit `specs/001-fuse-client-poc/benchmarks.md` (SC-005); add UDP receive drop counts (`/proc/net/snmp` `RcvbufErrors` before and after) as input for the socket2 idea in `docs/future-features.md`
- [X] T113 Create `scripts/soak.sh [minutes=30]` (SC-003): mount with Rustern, run a mixed read load (`fio` random reads plus repeated `sha256sum -c corpus.sha256`) while every 60 s it SIGSTOPs then SIGCONTs a random `ternblocks` process and a random `ternshard` process for 10 s; at the end assert the mount is alive, no ENOTCONN was seen, every checksum failure happened inside a fault window as EIO (never a wrong hash), and peak RSS (recorded) stayed below `--memory-limit` + 256 MiB, which catches leaks; run it once and record the result in `specs/001-fuse-client-poc/benchmarks.md`
- [X] T114 [P] Add a check to `scripts/gate.sh` that every test name listed in `docs/deviations.md` exists as a test function in the named file
- [X] T115 [P] Write `README.md`: what Rustern is, PoC status (read-only), licence (`MIT OR Apache-2.0`, plus the Apache-2.0 WITH LLVM-exception parts listed in `NOTICE`), and a pointer to `specs/001-fuse-client-poc/quickstart.md`
- [X] T116 [P] Update `specs/001-fuse-client-poc/quickstart.md`: cluster tests run with `cargo nextest run --run-ignored all`, `scripts/cluster.sh` replaces the manual `ternrun` command (keep the command as a reference), the reference mount path is `ternfs/mnt/fuse_loc1/mnt`
- [X] T117 Follow `specs/001-fuse-client-poc/quickstart.md` end to end on a clean checkout, timing step 1 → browsable mount (SC-004: under 1 minute once the cluster is running), and fix any step that does not work as written
  - Run 2026-11-14 on the working tree (nothing is committed yet, so there is no clean checkout to clone). Cluster already up (`scripts/cluster.sh status` OK). Mount to browsable in 23 ms (SC-004); `sha256sum -c` all OK in 6.5 s; EROFS on `touch`/`mkdir`; ENOENT on a missing name; SIGUSR1 metrics dump; clean TERM unmount; `fusermount3 -u` after SIGKILL leaves the mount point usable. Fixed: step 1 now uses `scripts/cluster.sh`; cluster test commands need `--run-ignored all`. Not run: steps 7–8 (no `fio`); the `allow_other` browse test needs `user_allow_other` in `/etc/fuse.conf`.
- [x] T118 Resolve CT-1 with the project owner: accepted in clarify session 2026-11-14; Principle I amended (constitution 1.0.3) and CT-1 removed from plan.md Complexity Tracking
- [X] T120 [P] Goldens for every message type: move rsgen's message lists to `tools/msglist/` (shared by `tools/rsgen` and `tools/goldens`); `tools/goldens/fill.go` writes `testdata/goldens/messages.hex` (every struct, zero/random/max samples, both bodies of each union) and `tools/goldens/names.go` writes `testdata/goldens/names.txt` (every message kind and error code with its upstream name); rsgen emits `for_each_message!` and an `oversized` test module (each u8/u16 length-prefixed field one past its maximum fails to encode); `crates/rustern-msgs/tests/goldens.rs::every_message_type_round_trips` and `crates/rustern-msgs/tests/names.rs`
- [X] T121 [P] Mount the bridge without `ro` in `crates/rustern-fuse/tests/bridge_mount.rs::mutations_without_ro_get_erofs`: every mutating request reaches the bridge and gets EROFS, the in-memory filesystem is unchanged; the test filesystem also covers readdirplus, full readdir buffers (2,000 entries), listxattr and statfs
- [X] T122 Close the remaining coverage gaps (config and CLI validation, `-o` options, span encode mismatches, registry error replies, `-log-file`/`-verbose`, statfs on the real mount; unused helpers removed) and record every line left uncovered, with the reason, in `docs/coverage.md`
- [X] T123 Add a coverage step to `scripts/gate.sh`: in full mode the cluster and fault passes run under `cargo llvm-cov nextest`, then `cargo llvm-cov report --fail-under-lines` with a floor of 98% (`RUSTERN_COVERAGE_FLOOR`)
- [X] T124 Make the full gate pass for an unprivileged user: `browse.rs::allow_other_default_permissions` checks that `-o allow_other` fails at startup, naming `user_allow_other`, when /etc/fuse.conf lacks it (and checks the mount options only where it is set or as root); `faults.rs::logs_have_no_secrets` waits for the `alert=true` line before reading stderr (the old `> 1000 lines` check raced the stderr reader) and counts per-operation lines instead
- [ ] T125 Create `scripts/overnight.sh [hours=8]` (script done and smoke-tested, including 100 ms shaping and SIGTERM teardown; the first full night run is pending): runs the cluster and Rustern inside an unprivileged user+network+mount namespace (`unshare -Urnm`) so `tc netem` can shape loopback without root; cycles network phases (clean, delay with jitter, delay + loss, heavy loss + reordering, plus the SIGSTOP faults from `soak.sh`) under a constant mixed read load; records per phase: `sha256sum -c` results (no wrong byte, ever; read errors only inside fault windows), fio throughput and p50/p99, RSS and CPU samples and the metrics dump; fails on a wrong checksum, ENOTCONN, an exit, or RSS growth between the first and last clean phase; appends the report to `benchmarks.md`
- [X] T126 Profile the single-reader gap with 2 MiB kernel readahead (Rustern 1,477 vs ternfuse 1,739 MiB/s, `benchmarks.md` "Buffered reads and readahead"): find where the time goes (request sizes the kernel sends, readahead window, block fetch pipelining) and fix or record the cause
  - Cause: a stall of about 17 ms at every span boundary while the HDD block service starts its whole-block WILLNEED read. Fix: sequential reads warm the next span (page 0 of each data block). One reader at 2 MiB readahead is now 1.64x ternfuse (`benchmarks.md` "Span boundary stalls and next-span warming")
- [X] T127 Profile the p99 tail at 16 readers (1.5-2.1x ternfuse while p50 is 0.6-0.7x): queueing in the block connection pool, the read semaphore or the buffer pool; fix or record the cause
  - Cause: the same span-boundary stalls as T126 (each one a tail sample). With next-span warming the 16-reader p99 is 0.56-0.61x ternfuse (2026-11-15 `scripts/bench.sh` run); the build-flag A/B shows no change there, so the fix is the warming
- [ ] T128 Run `cargo-mutants` on the hand-written crates (generated.rs excluded) and add tests for surviving mutants, or record why each survivor is equivalent
- [ ] T129 Add Kani proofs for the arithmetic the coverage report calls impossible: `read.rs` `bounds()`, stripe/layout offsets in `layout.rs` and `span.rs`, and length decoding in `wire.rs`
- [ ] T130 Run each cargo-fuzz target (`shard_resp`, `registry_resp`, `blocks_resp`, `page_stream`) for at least 1 hour, minimise and commit the grown corpus, and fix any crash
- [ ] T131 Write `docs/deployment.md`: the unprivileged default, and each root-only tuning (`user_allow_other`, socket buffer sizes, transparent huge pages, `drop_caches`) with what it changes and how Rustern behaves without it
- [X] T132 Shutdown as root (also root in a user namespace) with a file open: fuser's plain unmount fails with EBUSY, so Rustern exited 1 and left a dead mount (ENOTCONN). `main.rs::serve` now detaches lazily on EBUSY, as `fusermount3 -u -z` does for unprivileged users; `browse.rs::sigterm_with_open_file_unmounts_as_root` covers it in an `unshare -Urm` namespace
- [X] T133 Check that an old Rustern does not unmount a new mount at the same path after an external unmount (fuser re-unmounts by path on drop): not reproduced, the fuser `is_mounted` check holds
- [X] T119 Run the merge gates (full `scripts/gate.sh` passed 2026-11-14 as an unprivileged user: 171 cluster and 12 fault tests, 98.61% line coverage)

---

## Dependencies & Execution Order

### Phase Dependencies

- **Setup (Phase 1)**: no dependencies.
- **Foundational (Phase 2)**: depends on Setup; blocks every story.
- **US1 (Phase 3)**: depends on Foundational.
- **US2 (Phase 4)**: depends on Foundational. Its client-level tests and code
  (T075–T082, T085, T088–T093) need nothing from US1; its mount-level tests and FUSE wiring
  (T083, T084, T086, T094) need the US1 mount (T068–T071).
- **US3 (Phase 5)**: depends on Foundational; mutation refusal (T096, T103) needs only
  the US1 mount; fault tests on reads (T097, T105) need US2's read path.
- **Polish (Phase 6)**: after US1–US3.

### Within each phase

- Tests are written and seen to fail before the implementation they cover.
- Generated codecs (T021–T024) before anything that sends a request.
- `rustern-core` types before `rustern-client` operations before `rustern-fuse` wiring
  before the binary.
- A phase is complete only when its final "Run the merge gates" task passes.

### Key task dependencies

- T017 → T019, T021, T023; T018 → T019, T025, T032; T021 → T022 → T024
- T037 → T039 → T043; T041 → T043 → T045 → T046; T047 → T048
- T050 → T051 → T052 → T053; T054, T055 → every cluster test
- T062 → T065; T065, T066, T067 → T068 → T070 → T071 → T072
- T080 → T090; T088, T089, T090 → T093; T091, T092 → T093 → T094
- T093 → T105; T068 → T103

---

## Parallel Opportunities

- Setup: T003, T006–T012 in parallel after T001–T002.
- Foundational: the wire tests (T016), goldens (T018), core tests (T030, T032), client
  tests (T034, T036, T038, T040, T042, T044) and errno tests (T049) touch different files
  and can all be written at once; implementations follow their tests.
- US1: T057–T063 in parallel; T065, T066, T067 in parallel.
- US2: T075–T086 in parallel; T088 and T089 in parallel.
- US3: T096, T100 and T101 in parallel with T097 (T097–T099 share `faults.rs`).
- Once Foundational is done, US2's client-level work (T075–T082, T085, T088–T093) can run in
  parallel with US1.

## Parallel Example: User Story 2

```bash
# Tests, all different files:
Task: "Span validation tests in crates/rustern-core/src/span.rs"          # T075
Task: "Layout property tests in crates/rustern-core/src/layout.rs"        # T076
Task: "Page stream verifier tests in crates/rustern-client/src/blocks.rs" # T077
Task: "Span cache tests in crates/rustern-client/src/cache.rs"            # T079

# Then the two pure core types together:
Task: "Implement crates/rustern-core/src/span.rs"                         # T088
Task: "Implement crates/rustern-core/src/layout.rs"                       # T089
```

---

## Implementation Strategy

### MVP first (User Story 1)

1. Phase 1 Setup, Phase 2 Foundational.
2. Phase 3 (US1): mount and browse. **Stop and validate**: `browse.rs` passes and the
   quickstart mount step works.

### Incremental delivery

1. US1 → mount and browse (MVP demo).
2. US2 → correct bytes: together with US1 this is "the client works".
3. US3 → safe failure, mutation refusal, failover.
4. Polish → benchmarks, soak, docs.

---

## Notes

- [P] tasks touch different files and do not depend on incomplete tasks.
- Commit after each task or logical group; never commit with a failing gate except the
  "record that they fail" commits, which contain tests only.
- Deferred work stays out of this feature: interrupt support, RS reconstruction, disk
  cache, socket tuning (`docs/future-features.md`).
- [X] T134 Lint the helper scripts in the gate: shellcheck, shfmt (`.editorconfig`), ruff (`ruff.toml`) and the hook tests; add a fuzz smoke (20 s per target) to the full gate
- [X] T135 GitHub CI (`.github/workflows/ci.yml`): `quick` and `full` jobs running `scripts/gate.sh`, upstream (the `ternfs/` submodule) built and cached per pin, shellcheck through reviewdog; both jobs pass on GitHub (run 43656101707: quick 1m20s, full 18m)
