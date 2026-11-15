# Research: Write Support

Phase 0 of `/speckit-plan` for [spec.md](spec.md). Each decision is in Decision / Rationale /
Alternatives form. References are to the upstream pin `5550891` (`ternfs/`).

## W1. Reed-Solomon implementation

**Decision**: An in-house port of upstream `go/core/rs` (`rs_core.c`, `rs.cpp`, `gf_tables.c`)
lives in its own crate, `crates/rustern-rs` (done, commit `0007da2`). It covers GF(2^8) with the Rijndael polynomial 0x11B,
log and exp tables, and the upstream matrix construction:

1. Identity rows first.
2. Cauchy rows `m[col*D+row] = gf_inv(col ^ row)`.
3. Columns normalised by `gf_inv(m[col*D])`, and parity rows 1.. by `gf_inv(m[D*D+row])`, so
   parity 0 is the XOR of the data cells.

The encoder takes D data cells and writes P parity cells. The decoder, which reconstructs any D
of D+P, is part of the same port. Its write-path use is only for tests; read-path reconstruction
(DEV-004, CT-2) can adopt it later without a new port. The files are derived from Apache-2.0
WITH LLVM-exception code: each carries the upstream copyright line, the SPDX identifier and a
`NOTICE` entry, and `rustern-rs` declares the combined licence. Unlike upstream, bad arguments
return an error instead of aborting, and recovery takes exactly D blocks (upstream reads past
its buffers when given more).

Correctness evidence:

- Golden vectors (`testdata/goldens/rs.hex`) from `tools/goldens` running upstream `rs.go`:
  parity and a random recovery for every layout TernFS allows (D 2..10, P 1..6), block sizes
  around the vector widths (1 to 4103 bytes), and layouts up to D=15, P=15. Every kernel the
  CPU supports is checked against them.
- Property tests: all kernels produce identical parity, any D of the D+P blocks recover the
  data, and parity 0 == XOR of the data.

**Rationale**: The constitution baseline requires the port. Bit-identity is a release gate
(Principle II).

**Alternatives considered**:

- `reed-solomon-turbo` 0.1.1, suggested by the user:
  - It uses GF(2^8) with polynomial 0x11D and the Backblaze/klauspost Vandermonde-derived
    matrix.
  - It exposes no custom-polynomial or custom-matrix API, so its parity cannot be made
    bit-identical with TernFS (0x11B, normalised Cauchy, XOR first parity). Files it encoded
    would read back wrong through ternfuse and the kernel module.
  - It is also two months old with 264 downloads, so it does not meet the baseline's
    "widely used" bar.
  - Rejected.
- `reed-solomon-erasure` and `reed-solomon-novelpoly` use different fields or matrices.
  `reed-solomon-simd` uses GF(2^16) with FFT-based Leopard codes. None is compatible.
- `galois_2p8` supports arbitrary polynomials but has no SIMD and no matrix that matches. The
  matrix is the part that must match, and the field is about 50 lines. Not worth a dependency.

## W2. GF(2^8) multiply kernel and SIMD selection

**Decision**: five kernels, chosen at run time by CPU detection (`rustern_rs::Level::best`), in
safe Rust with `#![forbid(unsafe_code)]` (done, commit `0007da2`):

| Level | Kernel | Bytes per step | Token (feature set) |
|---|---|---|---|
| `Scalar` | split-nibble tables, two 16-entry lookups per byte | 1 | none; the reference in tests and the path on non-x86 targets |
| `Avx2` | split-nibble with `vpshufb` (upstream `rs_compute_parity_avx2`) | 32 | fearless_simd `Avx2` (x86-64-v3) |
| `Avx2Gfni` | `vgf2p8mulb` (upstream `rs_compute_parity_gfni`) | 32 | `Alderlake` (`-march=alderlake` without Key Locker) |
| `Avx512` | split-nibble with 512-bit `vpshufb` (not in upstream) | 64 | `SkylakeAvx512` (`-march=skylake-avx512`, x86-64-v4) |
| `Avx512Gfni` | 512-bit `vgf2p8mulb` (not in upstream) | 64 | fearless_simd `Avx512` (`icelake-server`) |

`best()` prefers GFNI over split-nibble and 512-bit over 256-bit. TernFS uses GFNI's polynomial
(0x11B), so one `vgf2p8mulb` multiplies 32 or 64 bytes by their coefficients.

- **How the kernels avoid `unsafe`**: `fearless_simd::kernel!` turns a detected CPU-feature
  token into the `#[target_feature]` context that makes the intrinsics safe to call (Rust 1.87+).
  Loads and stores go through `slice::as_chunks` and `bytemuck::cast` to `__m256i`/`__m512i`
  (bytemuck `avx512_simd` feature), with no raw pointers. One macro (`simd_body!`) holds the
  matrix-multiply loop; each kernel file only supplies its table and multiply (about 15 lines).
  Tails shorter than a vector go to the scalar kernel.
- **fearless_simd patch**: upstream fearless_simd 1.2.0 has `Avx2` and an Ice Lake `Avx512`
  token only. Skylake-SP class servers (AVX-512 F/BW/CD/DQ/VL, no Ice Lake extensions, as on the
  production work machines) and Alder Lake class desktops (GFNI without AVX-512) would fall back
  to `Avx2`. `third_party/fearless_simd` is the 1.2.0 release with
  `third_party/fearless_simd.patch` applied (about 400 lines, three files). It adds two
  kernel-only tokens, `Alderlake` and `SkylakeAvx512`, named after the GCC `-march` CPUs whose
  feature sets they require. The root `Cargo.toml` points `[patch.crates-io]` at the copy, so
  plain `cargo` works with no fork. The patch's `unsafe` (token construction after
  `is_x86_feature_detected!`) lives in fearless_simd, not in our crates (CT-9). It is to be
  offered upstream; the copy is dropped once equivalent tokens ship.
- **Tests**: goldens and property tests run every level the CPU has; `cargo test` inside
  `third_party/fearless_simd` covers the tokens and their detection. The AVX-512 kernels have
  not run yet: the development machine (i9-13900K) has no AVX-512. They need a run on a
  Skylake-SP work machine (`Avx512`) and on an Ice Lake or Zen 4 machine, or Intel SDE
  (`Avx512Gfni`), before the merge.

**Benchmark** (`crates/rustern-rs/benches/rs.rs` and `tools/rsbench` for upstream's cgo build;
µs per stripe of D 1 MiB blocks, one core of an i9-13900K; recovery rebuilds data block 0
without parity 0, so the multiply kernel runs rather than plain XOR):

| Case | upstream AVX2 | upstream GFNI | `Avx2` | `Avx2Gfni` |
|---|---|---|---|---|
| parity D4 P2 | 134 | 95 | 120 | 110 |
| parity D10 P4 | 900 | 565 | 732 | 425 |
| parity D8 P3 | 462 | 283 | 376 | 203 |
| recover D4 P2 | 93 | 69 | 105 | 68 |
| recover D10 P4 | 258 | 166 | 330 | 227 |
| recover D8 P3 | 182 | 126 | 246 | 131 |

Parity is 15–28% faster than upstream; recovery is up to 37% slower, likely because the kernel
loads all D inputs per chunk even for one output row. Write paths only encode, so recovery
tuning waits for degraded reads (CT-2). D10 P4 parity with `Avx2Gfni` is about 23 GiB/s of data
per core, far above the write throughput target, so RS is not the bottleneck.

**Rationale**: runtime detection with a tested scalar fallback is what Principle V asks for, and
it reaches GFNI and AVX-512 without an `unsafe` crate of our own. fearless_simd is a maintained,
widely used linebender crate whose `kernel!` macro exists for exactly this use.

**Alternatives considered**:

- AVX2 only, selected by `cfg(target_feature = "avx2")` under the x86-64-v3 build flag (the
  first version of this decision): safe, but leaves GFNI (1.7–1.9× faster) and AVX-512 unused
  and caps RS at the build's baseline target CPU.
- A designated `unsafe` crate with hand-written `is_x86_feature_detected!` dispatch: costs Miri,
  two reviewers per change and a TSan run, for what fearless_simd already audits.
- A fork of fearless_simd: a vendored release plus a reviewable patch is easier to diff against
  upstream and to drop later.
- Portable SIMD (`std::simd`): nightly only, and has no GFNI.

## W3. Span layout, CRCs and parity for writes

**Decision**: `rustern-core::span` gains a write-side layout. It ports
`ComputeSpanParameters` and `prepareSpanInitiateReqWithParams` from `go/client/span.go`:

- `S` comes from the stripe policy for the span size and `D, P` from the span policy entry;
  `blockSize = ceil(size/D)`.
- `cellSize = ceil(blockSize/S)`, rounded up to 4 KiB; then `blockSize = cellSize*S`.
- The storage class comes from the block policy for `blockSize`.
- Data is padded with zeros to `S*D*cellSize`. Data cell `(s,d)` is at `D*cellSize*s + cellSize*d`.
- Mirroring (`D == 1`): one CRC per stripe cell, repeated for every block.
- RS: per-cell CRCs `Crcs[B*s+b]`. The span CRC is `crc32c(data)` zero-extended to `size`.

The CRC of the padding and of gap zeros is computed with zero-extend. Zeros are never hashed
byte by byte. Proof comes from golden `AddSpanInitiateReq` bodies (parity, CRC arrays,
`cellSize`, `stripes`) generated by upstream `prepareSpanInitiateReq` for the corpus sizes,
plus property tests: the span CRC equals the CRC of the bytes the read path returns.

**Rationale**: The shard re-derives the span CRC from the cell CRCs (`ShardDB.cpp`
`_checkSpanBody`) and rejects mismatches with BAD_SPAN_BODY. Bit-identity is checked twice,
by the goldens and by the shard.

**Alternatives considered**: Hand-deriving the layout from the docs. Rejected: Principle II
requires goldens anyway, and the port is about 100 lines.

## W4. Gaps and extending truncate without allocation

**Decision**: Zeros are never materialised beyond one static 4 KiB zero page.

- **Inside the span being filled**: a gap advances the fill offset over zeroed buffer memory.
  The pooled span buffer is zeroed when it is recycled (see W7), and that is the only zeroing.
- **A gap that reaches the span size limit**: the span is cut and sent with
  `Size = span size` but data blocks covering only the bytes actually written, rounded up to
  one cell. `AddSpanInitiate` allows `Size` greater than the data blocks ("trailing zeros are
  added ... cheap creation of gaps"), and the shard zero-extends the CRC (negative or positive
  extension).
- **An all-zero span**: one zero page per data cell, written from the static page.
- **A small all-zero tail**: `AddInlineSpan` with INLINE storage and a body shorter than
  `size`. The shard rejects only `size == 0` and `size < body.size()`. That the missing
  bytes read as zeros through every client is a to-verify item, and until it is verified the
  tail is sent as a full body.

EMPTY storage cannot be used: the shard rejects EMPTY spans with `size != 0`.

**Rationale**: Principle III ("sparse writes and extending truncates stream zeros"). A 1 TiB
`ftruncate` costs about 10k span requests and about 10k page-sized block writes, not 1 TiB of
memory or traffic.

**Alternatives considered**:

- Buffering zeros like data: unbounded traffic, and allocation driven by the caller.
- EMPTY spans: rejected by the shard for a non-zero size.

**To verify in tests**: the read path of feature 001 returns zeros for a span whose size
exceeds its data blocks (`layout.rs` handles trailing zeros for the last stripe). A dedicated
cluster test writes a gap span and reads it through Rustern, ternfuse and the kernel module.

## W5. File writer architecture

**Decision**: There is one `FileWriter` per file being written, created at `create` or
`O_TMPFILE`.

- **State machine**: a sans-IO, synchronous, exhaustively matched enum
  (`WriteState::{Open, Publishing, Published, Failed(Errno), Abandoned}`, see
  [data-model.md](data-model.md)). Its `step(event) -> Vec<Action>` function has no I/O and no
  `await`.
- **Driver**: one tokio task per file, the actor, owns the state and the span pipeline. It
  receives commands on a bounded `tokio::sync::mpsc` (`Write`, `Flush`, `Fsync`, `Truncate`,
  `Release`, `Link`, `Stat`), each with a `oneshot` for the reply.
- **FUSE handlers**: they send a command and await the reply, and hold no lock across an
  `await`.
- **Store completions** (`SpanStored`, `SpanFailed`) re-enter the actor as events on the same
  mailbox.

`write` data is copied once from the FUSE request buffer into the pooled span buffer before
the reply. The copy happens in the FUSE handler under the actor's buffer lease (see W7), so
the mailbox carries a buffer handle, not the bytes.

**Rationale**:

- One owner per file makes concurrency easy to reason about. Writes, flushes, fsyncs and
  releases on one file arrive from several FUSE threads, and the mailbox totally orders them.
- The interleaving space is then the set of orderings of mailbox events, which the model test
  enumerates exhaustively (W13).
- Principle I asks for an explicit lifecycle type, and Principle VI for no global lock and for
  model tests.

**Alternatives considered**:

- A `Mutex<FileState>` per inode, locked by each handler: the lock would be held across span
  stores (forbidden), or the code would need hand-written condition variables.
- A global writer task: a global serialisation point.
- Writing through the kernel page cache with writeback: see W10.

## W6. Span pipeline and block writes

**Decision**: Within one file:

1. The actor fills span k+1 while span k is stored. CRC and parity for a full span are computed
   on `tokio::task::spawn_blocking`, so they never run on a reactor thread (Principle VI), in
   parallel with the previous span's block writes.
2. Spans are initiated in order. The shard rejects `AddSpanInitiate` while the last span is not
   CLEAN (`LAST_SPAN_STATE_NOT_CLEAN`), so initiate k+1 waits for certify k. Across files,
   spans are fully concurrent.
3. Each span is stored as follows:
   - `AddSpanInitiate`, with the blacklist built from earlier failures, returns block
     placements and write certificates.
   - All `WriteBlock` requests run in parallel over the existing per-block-service TCP
     connection pools (`blocks.rs` `Conns`). Each sends `WriteBlockReq{BlockId, Crc, Size,
     Certificate}`, streams the block's cells as a vectored write straight from the span and
     parity buffers, and reads back `WriteBlockResp{Proof}`.
   - `AddSpanCertify{FileId, Cookie, ByteOffset, Proofs}`.
4. On a block write failure or timeout, the client follows `go/client/span.go`
   `writeSpanBlocks`:
   - It blacklists the failing block's failure domain.
   - It moves the dirty span aside with `ConstructFile{FILE, Note:"bad_add_span_attempt"}` and
     `MoveSpan`, so the cluster's GC reclaims it.
   - It retries the span, up to 5 attempts within the overall block deadline (DEV-006, 60 s).
   - When attempts or the deadline run out, the span fails with EIO.
   - `COULD_NOT_PICK_BLOCK_SERVICES` on a first attempt (empty blacklist) maps to ENOSPC. After
     blacklisting it maps to EIO.
5. Data smaller than 256 bytes takes the inline path (`AddInlineSpan`, INLINE storage), as
   upstream does.

**Rationale**:

- Bit-for-bit and step-for-step parity with the upstream writer keeps the cluster's GC and
  scrubber assumptions intact.
- The vectored write keeps the only data copy as the one from the kernel (Principle V).
- Parallel block writes satisfy Principle VI.

**Alternatives considered**:

- `AddSpanAtLocationInitiate`: internal, used for multi-location replication. Not used by
  clients.
- Pipelining initiates across spans of one file: the shard forbids it.

## W7. Write memory and backpressure

**Decision**: There are two bounds, both enforced by a `tokio::sync::Semaphore` measured in
bytes:

- `--write-memory-limit`, default 1 GiB, covers all span buffers plus parity buffers in the
  process.
- `--file-write-spans`, default 2, is the number of span buffers one file may hold: one
  filling and one storing. With 1 the pipeline is serial.

Span buffers come from a dedicated pool of `Vec<u8>` sized to the largest span the file's
policy allows (at most `MAXIMUM_SPAN_SIZE` = 100 MiB). Recycled buffers are re-zeroed only up
to their used length. A file whose next span cannot get a permit makes `write` wait, as
backpressure. A request never allocates beyond the permit. Parity buffers (`P*S*cellSize`) are
taken from the same semaphore when a span is sealed.

**Rationale**:

- FR-019. About 2 GiB per client is acceptable: 1 GiB for reads (feature 001) plus 1 GiB for
  writes.
- With 10 MiB spans, 1 GiB lets about 50 files stream at full pipeline depth.

**Alternatives considered**:

- Allocating per span: steady-state allocation of up to 100 MiB per span, which Principle V
  forbids.
- Reusing the 001 read buffer pool (1 MiB + 8 KiB buffers): RS needs contiguous cells up to
  the block size, and splitting cells across pool buffers complicates the kernel for no gain.

## W8. Publish point and the pending-publish registry (DEV-007)

**Decision**: The publish point works as follows.

- **flush** (each `close`): the actor seals the partly filled span and stores and certifies
  every span written so far before replying. Errors go to `close` (FR-004, FR-005). A flush
  does not publish.
- **release** (last close): the actor seals, stores and certifies whatever was written after
  the last flush (normally nothing), then sends `LinkFile{FileId, Cookie, OwnerId, Name}`. A
  publish failure at this point has no caller. It is logged at warn with the file id and name
  (never the cookie) and counted in `rustern_publish_failures_total{reason}`.
- **Pending-publish registry**: a `DashMap<(DirId, Name), PendingEntry>` holding entries for
  files being written that have a name. Each entry has a `tokio::sync::watch` of
  `{Open{flushed_idle: bool}, Publishing, Done(Result)}`. `flushed_idle` is true after a flush
  completes and before the next write.
  - A **lookup** or **readdir** through this mount that sees an entry with
    `Publishing | flushed_idle` waits on the watch, up to `--publish-wait` (default 1 s), for
    `Done`. On `Done(Ok)` it continues with the published file. On a timeout or anything else
    it answers ENOENT (lookup) or the listing without the name (readdir), never a negative
    dentry with a TTL.
  - An entry that is `Open` and not flushed-idle answers ENOENT at once: the file is still
    being written, and the name is invisible as in ternfuse.

**Rationale**:

- The kernel sends FLUSH on every `close` and RELEASE when the last reference to the open file
  drops. `close()` can return before the daemon has processed the RELEASE, and the kernel never
  says whether a FLUSH is the last one. "flushed and idle" is the signal that a RELEASE is
  imminent, and the bounded wait closes the race.
- `cmd > file` works: the shell's early `close` stores nothing and does not publish, and `cmd`
  keeps writing through the duplicated descriptor.

**Alternatives considered**:

- Publishing at the first flush (ternfuse): breaks `cmd > file`, reproduced on the baseline
  cluster (EOPNOTSUPP from every later write, an empty file published).
- An eBPF close tracker: needs root (Principle IV).
- Publishing at release without waiting: `close(); stat()` would race.

## W9. Overwrite option and LinkFile replace semantics (DEV-010)

**Decision**: `--overwrite` makes `open(O_TRUNC)` of a published file, and `O_CREAT|O_TRUNC` or
`creat` on an existing name, create a new file being written whose intended name is the
existing one. Its release publishes with `LinkFile`. The shard's `_createCurrentEdge`
(`ShardDB.cpp` ~2011) replaces a non-locked current file edge and turns the old one into a
snapshot edge. That is the same mechanism upstream nfsd relies on for replacement on close.

`MORE_RECENT_CURRENT_EDGE`, when the existing edge is newer (another client published after
us), is logged and counted as a publish failure, and the newer file stays. `CANNOT_OVERRIDE_NAME`
(a directory holds the name) at `open` time gives EISDIR before any file is constructed. At
publish time it is a publish failure.

Without `--overwrite`, a write-mode `open` of a published file gives EPERM (DEV-009). Readers
that opened the old file keep reading it: its spans are immutable, and the snapshot edge keeps
it alive until the snapshot policy deletes it.

`FUSE_ATOMIC_O_TRUNC` is negotiated at init, so the kernel delivers `O_TRUNC` in OPEN instead
of a separate SETATTR(size=0).

**Rationale**: The spec clarification (FR-011). It reuses cluster semantics: no client-side
delete and no window in which the name is missing.

**Alternatives considered**: Unlink then link: leaves a window with no file, and loses the old
file if the new one fails.

## W10. Kernel cache and FUSE settings for writes

**Decision**:

- Handles of files being written are opened with `FOPEN_DIRECT_IO`. Writes bypass the page
  cache and arrive as received, in requests up to `max_write`.
- `max_write` is 1 MiB, with `max_pages` negotiated (as 001 negotiates reads).
- No writeback cache.
- Attributes of a file being written are replied with zero validity.
- On publish, rename, unlink, rmdir, mkdir, symlink and `utimensat`, the bridge calls fuser's
  `Notifier::inval_entry` and `inval_inode` for the affected (parent, name) and inode.
- Reads on a being-written handle return EOPNOTSUPP (FR-010).

**Rationale**:

- Writeback caching would let the kernel reorder and coalesce writes, and turn short writes
  into page-sized read-modify-write. TernFS files are append-only, so the page cache gives
  nothing on the write path.
- `DIRECT_IO` also gives each `write()` a single FUSE request, so an error maps to the call
  that caused it.

**Alternatives considered**: `writeback_cache` for small-write coalescing. Rejected: it changes
error attribution and flush ordering. Small writes are already coalesced in the span buffer.

## W11. Namespace operations and idempotent reconciliation

**Decision**: Requests mirror ternfuse's request choice and add upstream's reconciliation from
`go/client/shardreq.go`, `cdcreq.go` and `metadatareq.go`:

| Operation | Request | Reconciliation after a retried attempt |
|---|---|---|
| create | `ConstructFile{FILE, Note: name}` (shard of the parent) | none needed: a lost response leaks an unnamed transient file that GC reclaims |
| publish | `LinkFile` | shard is idempotent (`_applyLinkFile`: FILE_NOT_FOUND with the current edge pointing at our file = success) |
| mkdir | CDC `MakeDirectory{OwnerId, Name}` | EEXIST on attempt > 0: look the name up; if it is a directory created within the attempt window, success (upstream has none; Rustern addition) |
| rmdir | CDC `SoftUnlinkDirectory` after `Lookup` | `checkDeletedEdge` (FullReadDir backwards, deletion edge + snapshot edge) |
| unlink | shard `SoftUnlinkFile` after `Lookup`, then hard unlink (`SameShardHardFileUnlink` / CDC `CrossShardHardUnlinkFile`) when the snapshot policy allows immediate deletion, errors ignored as ternfuse does | `checkDeletedEdge` on EDGE_NOT_FOUND |
| rename, same directory | shard `SameDirectoryRename` | `checkNewEdgeAfterRename` (60 s creation-time fuzz), always applied |
| rename, file across directories | CDC `RenameFile` | same |
| rename, directory | CDC `RenameDirectory` | same |
| symlink | `ConstructFile{SYMLINK}`, `AddInlineSpan` (target), `LinkFile` | as create and publish |
| utimensat | shard `SetTime` with bit 63 set on each time that is to be set (`UTIME_OMIT` leaves it clear) | idempotent |

Names are validated before sending: length ≤ 255 bytes, no `/` and no NUL, not `.` or `..`.
Violations give ENAMETOOLONG or EINVAL (FR-017).

The immediate-deletion predicate is taken from the cluster's own snapshot-policy evaluation
(`go/cleanup`), not from ternfuse's `canDeleteFileImmediately`. As written, that ternfuse
function returns true for a time-only policy with non-zero retention (`DeleteAfterVersions`
inactive), which looks like a defect. A task compares the two rules. If they disagree, the
difference is recorded as DEV-014.

**Rationale**: FR-013 to FR-015, Principle III (reconciliation as upstream does it).

**Alternatives considered**: Not reconciling mkdir: a lost response would turn into a spurious
EEXIST, which FR-015 forbids.

## W12. Namespace operations on a name that is being written (DEV-012)

**Decision**:

- `unlink` of a name whose file is still being written succeeds. It removes the pending entry
  and marks the writer `Abandoned`: it stays writable through open descriptors and is never
  published, which is POSIX unlink-while-open.
- `rename` of such a name, or onto it, waits like a lookup (W8) when the file is flushed-idle.
  Otherwise it fails with EBUSY.
- In ternfuse both give ENOENT (the name is not in the shard yet) and the file is published
  later anyway.

**Rationale**: The kernel dentry for the name exists (the create reply instantiated it).
ENOENT would contradict what `ls` through the same mount just showed, and publishing after a
successful unlink is a resurrection.

**Alternatives considered**: Retargeting the pending name on rename: needs a cross-directory
`LinkFile` owner change for spans already certified under the old owner's shard. Possible, but
not needed for any listed tool, since rename-after-close is covered by the wait.

## W13. Concurrency verification

**Decision**: The per-file state machine (W5) is sans-IO. `crates/rustern-client/tests/
write_model.rs` enumerates every ordering of bounded event sets exhaustively:

- events: up to 3 writes, 2 flushes, 1 fsync, 1 truncate, 1 release, store completions and
  failures, 1 interrupt;
- assertions: the invariants in [data-model.md](data-model.md), such as "never published with
  an unacknowledged write", "a failure is sticky", "exactly one publish", "release after
  Failed publishes nothing";
- a proptest strategy samples longer sequences.

The async driver is a thin loop over the mailbox. It is exercised by stress tests (16 writers
plus concurrent fsync, flush and stat on one file) and by the fault suite. The pending-publish
registry uses `DashMap` and `tokio::sync::watch` only, with no custom primitive.

**Rationale**: Principle VI requires model tests for every state machine. Because the mailbox
totally orders events, the orderings of the mailbox are exactly the interleavings, and an
exhaustive enumeration over them is a complete model check of the logic, without making tokio
types generic for `loom` or `shuttle`.

**Alternatives considered**: `shuttle` over the async driver: needs shuttle's own channels and
executor instead of tokio's throughout the driver. It would check tokio's mpsc rather than our
logic.

## W14. Baseline measurements (ternfuse, local cluster, 2026-11-15)

Measured with `fio` psync, `bs=1M`, `--create_on_open=1 --fallocate=none`, on ternfuse
`5550891` mounted by `ternrun` (no close tracker), in the cluster's default root policy. CPU is
ternfuse's user+system time from `/proc/<pid>/stat`.

| Workload | Result |
|---|---|
| seq-write, 1 writer × 4 GiB | 90 MiB/s, 2.67 CPU-s/GiB |
| seq-write, 4 writers × 1 GiB | 218 MiB/s, 2.46 CPU-s/GiB |
| seq-write, 16 writers × 1 GiB | 295 MiB/s, 2.64 CPU-s/GiB |
| small-files, 16 writers × 200 × 4 KiB | ≈ 11 files/s |
| mkdir, sequential | mean 636 ms (the cluster's metadata writes are slow on WSL2: RocksDB WAL fsync) |

Not yet measured; `scripts/bench.sh write` produces them before US4 (task phase):

- `seq-write-mirror`;
- 2 and 8 writers;
- `meta-write-latency` p50 and p99 for close, rename and unlink;
- `mixed-rw`.

`dd of=` and `cmd > file` cannot be measured on ternfuse: they fail with EOPNOTSUPP after the
first flush publishes an empty file (DEV-007 evidence). The benchmark uses `fio`, which does
not duplicate the descriptor.

## W15. O_TMPFILE through fuser

**Decision**: fuser 0.18.0, the latest release (2026-07-22), has no `FUSE_TMPFILE` (opcode 51)
in its request decoder. The kernel's `O_TMPFILE` therefore gets ENOSYS and then EOPNOTSUPP.

Plan: contribute decoding of `FUSE_TMPFILE` to fuser upstream. The payload is
`fuse_create_in` plus a name, the same as CREATE, so the change is a `Filesystem::tmpfile`
method. Until a release ships it, Rustern carries a `[patch.crates-io]` to a pinned fork
commit, with `cargo-deny` `sources` allowing that one git URL. The patch is removed when a
fuser release includes the change.

If the change is not accepted by the time this feature merges, FR-008 ships as EOPNOTSUPP with
a deviation row, and the rest of the `O_TMPFILE` work (writer state without a name, `linkat`
publish, EXDEV) stays tested through the client library API. The outcome is recorded in
Complexity Tracking (CT-8).

**Rationale**: FR-008 is v1 scope. A fork for one opcode is the smallest change. Raw
`/dev/fuse` handling would mean replacing fuser.

**Alternatives considered**: `fuse3` crate: async, but also has no tmpfile op, and swapping the
FUSE library is a separate benchmark-driven decision (Technology Baseline).

## W16. Upstream test suites

**Decision**:

- `terntests` (upstream `go/terntests`, the filesystem tests) is built from `ternfs/go` and run
  by `scripts/gate.sh` against a Rustern mount, with exclusions in `docs/coverage.md`: tests
  that need xattrs, kernel-module-only behaviour or the close tracker.
- POSIX conformance uses `pjdfstest` (Rust rewrite, built outside the workspace in
  `scripts/posix.sh`), with the subset for create, open, write, rename, unlink, mkdir, rmdir,
  symlink and utimensat. Exclusions (chmod/chown/link/mknod/truncate-shrink and
  random-write cases) are listed with their TernFS reason.

CT-5 from feature 001 is closed by this feature.

**Rationale**: Principle VII, SC-006.

**Alternatives considered**: Hand-written POSIX checks only: weaker and not comparable with
other filesystems.

## W17. Read-only gate

**Decision**: Feature 001 only sends requests in the read-only list (`tools/rsgen` `readOnly`)
and refuses every FUSE mutation with EROFS. A mount mode is introduced:
`MountMode::{ReadOnly, ReadWrite}`, with `-o ro` selecting ReadOnly. The client library's
mutating API takes a `WriteAccess` token that only the `ReadWrite` mount constructs, so a
read-only mount cannot send a mutation by construction. `refuses_mutations.rs` keeps running
with `-o ro`.

**Rationale**: FR-018. The 001 guarantee stays checkable.

**Alternatives considered**: A runtime flag check in each handler: one forgotten check sends a
mutation.
