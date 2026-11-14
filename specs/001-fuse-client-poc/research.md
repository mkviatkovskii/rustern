# Research: Read-Only FUSE Client Proof of Concept

**Feature**: 001-fuse-client-poc | **Date**: 2026-11-14

Download counts are from crates.io on 2026-11-14 (total / last 90 days). The bar from
FR-018 is "millions of downloads". Upstream facts refer to `ternfs/` at commit `5550891`.

## R1. FUSE library

- **Decision**: `fuser` 0.18 (6.6M / 2.0M), default features (pure-Rust mount through
  `fusermount3`, no libfuse), used through an async interface. `rustern-fuse` defines its
  own `AsyncFilesystem` trait, modelled on fuser's `experimental::AsyncFilesystem` (same
  shape and names), covering every operation the PoC serves. A bridge of about 150 lines
  implements fuser's `Filesystem` trait: each callback copies its arguments, spawns a task
  on the shared `tokio` runtime, awaits the async method and sends the reply (replies are
  `Send`). fuser runs `Config::n_threads` event loops. When fuser's async API is stable
  and covers these operations, the bridge is deleted and the trait replaced by fuser's.
- **Why not fuser's `experimental` async API as is** (checked in the 0.18 source,
  `src/experimental.rs`): it covers only `lookup`, `getattr`, `read` and `readdir`; there
  is no `open` (needed to refuse write access modes with EROFS), `opendir`/`releasedir`
  (listing handles), `readlink`, `readdirplus` or `statfs`, and the missing operations
  cannot be added beside it because its `TokioAdapter` is the `Filesystem` impl. It also
  builds a private runtime that the client cannot share, and is documented as "Expect
  this to change". Internally it does what the bridge above does.
- **Rationale**: the only general-purpose FUSE crate above the bar; actively released
  (0.17 added multiple event loops and the experimental async API; 0.18 fixed in-process
  unmount with `AutoUnmount`).
- **Known limitation**: FUSE_INTERRUPT is answered with ENOSYS (`src/request.rs:146`), so
  the kernel stops sending interrupts. Recorded as DEV-005; deadlines bounded by DEV-006.
  Follow-up before v1: upstream an `interrupt()` dispatch to `fuser` (via `[patch]` meanwhile)
  or switch backend.
- **Mount options**: `ro,nosuid,nodev`, `FSName("ternfs")`, `Subtype("rustern")`.
  User `-o` options are appended; if they include `allow_other`, `default_permissions`
  is added (FR-003), because without it FUSE performs no permission checks and every
  local user could read every file.
  `AutoUnmount` is not used: fuser requires `allow_other`/`allow_root` for it, which needs
  `user_allow_other` in `/etc/fuse.conf` and conflicts with least privilege (Principle IV).
  Unmount happens in-process on SIGINT/SIGTERM/fatal error; after SIGKILL the documented
  clean-up is `fusermount3 -u <mountpoint>`.
- **Alternatives considered**: `fuse3` 0.10 (0.3M; async-native, handles interrupts, below
  bar); `fuse-backend-rs` (1.9M; virtio-fs oriented, sync); `rfuse3` (28K), `polyfuse`,
  `easy_fuser` (too small); fuser `experimental` async API (see above).

## R2. Async runtime

- **Decision**: `tokio` 1.x (1.19B), multi-thread runtime.
- **Rationale**: Technology Baseline default; ecosystem (`tokio-util`, `moka`, `backon`,
  `metrics` exporters) assumes it.
- **Alternatives considered**: `smol` (29M, last release 2024-09); `async-std`
  (discontinued, README points to smol); io_uring runtimes `compio` (1.1M), `monoio`
  (1.7M, last release 2024-08), `glommio` (0.2M, 2024-03), `tokio-uring` (3.6M,
  2024-05). io_uring stays an option behind an I/O trait if benchmarks justify it.

## R3. Protocol codecs (Principle II: generated, never transcribed)

- **Upstream facts**: messages are Go structs in `go/msgs/msgs.go`; `go/bincodegen`
  reflects over them and emits Go, C++ and kmod codecs. Encoding: little-endian fixed-width
  scalars; `string`/`[]byte` = u8 length + bytes; `Blob` and lists = u16 length (u32 with
  the `len32` tag); fixed byte arrays raw; nested structs inline. Two union types
  (`FetchedSpan`, `FetchedFullSpan`) are hand-written upstream, keyed on `StorageClass`.
- **Decision**: a Go generator in this repository, `tools/rsgen`, adapted from
  `go/bincodegen` (reuses its type lists and reflection walk) that emits one Rust file,
  `crates/rustern-msgs/src/generated.rs`: structs, enums for message kinds and error codes,
  protocol constants, and `Encode`/`Decode` impls as straight-line code. A hand-written
  runtime of about 150 lines (`crates/rustern-msgs/src/wire.rs`) provides the primitives on
  `bytes::{Buf, BufMut}`. The two unions are hand-written in Rust, as upstream does in Go.
  The generator runs at development time (Go is installed); the output is committed and
  the gate fails if regenerating changes it.
- **Robustness rules built into the generated decoder**: every read is bounds-checked and
  returns `DecodeError`; before allocating a list of `n` elements, the decoder checks that
  at least `n × min_encoded_size(elem)` bytes remain, so a length field cannot cause an
  allocation larger than the datagram; names and strings are `Bytes`, not `String`
  (TernFS names may be non-UTF-8).
- **Licence**: generator and generated file are derived from Apache-2.0 WITH
  LLVM-exception sources; both carry the upstream copyright line and SPDX header, a
  `NOTICE` file lists the sources, and `rustern-msgs` declares
  `(MIT OR Apache-2.0) AND Apache-2.0 WITH LLVM-exception`.
- **Alternatives considered**: `binrw` (17.7M) or `deku` (8.4M) with generated
  attribute-annotated structs — workable, but the length-prefix pattern needs
  temp/calc attribute pairs on every list, decoding goes through `Read + Seek`, and the
  panic/allocation behaviour on hostile input would have to be audited per attribute.
  Straight-line generated code is as small to maintain and easier to prove panic-free.
  `bincode` crate (377M): different wire format (fixed u64 or varint lengths), not
  compatible. Hand-written codecs: forbidden by Principle II. A Rust generator parsing Go
  source: more code than adapting `bincodegen`.

## R4. Transport

- **Upstream facts**:
  - Shards and CDC: UDP, one datagram per request/response. Request frame: `u32 protocol`
    (`SHARD_REQ_PROTOCOL_VERSION` = 0x414853), `u64 request_id`, `u8 kind`, body. Response:
    `u32 protocol` (0x1414853), `u64 request_id`, `u8 kind`, body; kind `ERROR` carries a
    `u16` error code. Default MTU 1472. Responses are matched by request id; early or
    duplicate responses are tolerated.
  - Registry: TCP; request `u32 protocol`, `u32 length` (1 + body), `u8 kind`, body.
  - Block services: TCP; request `u32 protocol`, `u64 block_service_id`, `u8 kind`, body;
    response `u32 protocol`, `u8 kind` (or ERROR + `u16`), body, then for
    `FetchBlockWithCrc` a stream of pages, each 4096 data bytes followed by a `u32` CRC32C.
  - Block service addresses come back inside `LocalFileSpansResp.BlockServices`, so reads
    need the registry only for shard addresses (`LocalShardsReq`, 256 entries, two
    addresses each).
  - Default timeouts (`go/client/client.go:155`): shard initial 100 ms, max 2 s, overall
    10 s; CDC 1 s / 10 s / 1 min; blocks 1 s / 10 s / 5 min; growth 1.5, jitter 0.1.
- **Decision**: `tokio::net` sockets; one shared UDP socket per client with a receive
  task demultiplexing by request id into `tokio::sync::oneshot` senders held in a
  `dashmap` (454M); frames built with `bytes`. Block-service TCP: started as one connection
  per block fetch; T112's benchmarks showed connect plus the service's per-connection setup
  cost about two thirds of each fetch (fetch p50 527 µs, 182 µs pooled), so
  `blocks::Conns` keeps up to 64 idle connections per address (a `dashmap` of
  `Vec<_>` stacks, each guarded by its shard's lock; last in, first out). Each connection carries one fetch at a time
  (upstream pipelines many on one), so small reads never queue behind large ones; a
  connection returns to the pool only after a complete response, and a pooled one the
  service has closed (`-connection-idle-timeout`, 10 min) is replaced at once, without the
  retry wait. `deadpool`/`bb8` were not used for connections: both return a connection on drop, so a fetch
  cancelled by its timeout would need a hand-written guard against reusing a half-read
  stream.
  Addr1/Addr2 alternate between attempts.
- **Retry schedule**: upstream does not sleep between attempts. It resends as soon as an
  attempt times out, and each attempt's timeout grows (`go/core/timing/timeouts.go`:
  initial, growth 1.5, capped at max, until the overall deadline). `backon` (114M) is used
  as the schedule generator only: an `ExponentialBuilder` with `min_delay` = initial,
  `max_delay` = max, `factor` 1.5, `without_max_times()` (the default is 3 attempts) and no
  `total_delay` yields the per-attempt timeouts. Each attempt runs under
  `tokio::time::timeout` with the next value; the whole request runs under the overall
  deadline (capped at 60 s for caller-facing requests, DEV-006), so the last attempt gets
  whatever time remains. backon's own jitter is not enabled: it adds 0–100% of the current
  delay, which would let a timeout reach twice the configured `-max-*-timeout`. Instead each
  value is scaled by a uniform factor in [0.95, 1.05] with `fastrand` (already a backon
  dependency) and capped at max again, matching upstream's ±5% jitter on growth closely enough; this affects retry
  timing only, not the wire format or results, and needs no deviation entry.
- **Not used in the PoC**: `socket2` (already compiled as a `tokio` dependency). It is
  needed only to enlarge the UDP receive buffer; it is added if the benchmarks show
  dropped datagrams or better latency (`docs/future-features.md`, "Socket tuning").
- **Deferred**: batched UDP via `quinn-udp` (423M; sendmmsg/recvmmsg, GSO/GRO). Principle V
  asks for batched syscalls; the PoC records this under its Principle V exception and
  benchmarks first.
- **Alternatives considered**: `tower` (838M) for timeout/retry/limit middleware — more
  abstraction than four request kinds need; `backoff` (117M, last release 2021);
  `tokio-retry` (107M). `tokio-util` codecs (952M) — TCP framing here is two fixed headers,
  not worth a codec layer.

## R5. Read path and verification

- **Upstream facts**: spans are `INLINE` (body in metadata, CRC in span header) or block
  spans with `Parity` (low nibble data blocks D, high nibble parity blocks P), `Stripes`
  (< 16) and `CellSize` (multiple of 4096). D = 1 means mirrored (all blocks identical).
  Layout: span offset `o` → stripe `o / (D × cell)`, data block `(o mod (D × cell)) / cell`,
  block offset `stripe × cell + o mod cell`; bytes past the span size are zero padding.
  `FetchBlockWithCrc` returns per-page CRCs; upstream verifies every page and, when a whole
  span is assembled, the span CRC (with zero extension).
- **Decision**:
  - Inline spans: verify the body against the header CRC before use.
  - Block spans: fetch only the data blocks covering the requested range, page-aligned,
    with `FetchBlockWithCrc`; verify every page CRC; reject the whole fetch on any
    mismatch. When a read assembles a complete span, also verify the span CRC.
  - Mirrored spans fail over to the next mirror on error or mismatch (FR-010). RS spans
    with a failed data block return EIO (DEV-004).
  - CRC32C: `crc32c` (100M; hardware-accelerated, has `crc32c_append` and
    `crc32c_combine`). `crc-fast` (80M) is the benchmark alternative named by the baseline;
    the PoC first picked `crc32c` for its combine primitive, but `crc-fast` has one too
    (`checksum_combine`), and the T109 benchmark showed `crc-fast` about 2x faster at
    128 KiB and up and 35% faster on 4 KiB pages, so the PoC uses `crc-fast` (default
    features off, `std`). Upstream golden vectors cover sum, append, zero-extend and xor. `crc32fast` (868M) is not a candidate: it computes
    CRC-32 with the IEEE polynomial (zip, gzip, PNG), while TernFS uses CRC-32C
    (Castagnoli), which gives different values.
- **Principle I interpretation (accepted 2026-11-14, constitution 1.0.3)**: page CRCs
  are stored by the block service, which checked the whole-block CRC from metadata when
  the block was written. Verifying against metadata alone would require fetching whole
  stripes for every read (megabytes per 4 KiB random read). The PoC verifies page CRCs
  on every read, as upstream does, plus metadata CRCs for inline data and whenever a
  read covers a whole span. Upstream ternfuse's read path never checks the span CRC
  (`readMirrored` has `TODO check CRC for entire stripes`) and trusts kernel-cached
  pages during RS recovery; Rustern does neither.

## R6. Caching and in-flight sharing

- **Decision**: no in-process block cache. Each FUSE read fetches exactly the
  page-aligned range it covers, as ternfuse does (`readMirrored`, `client/span.go`).
  Sharing between readers comes from the kernel page cache: `open` replies with
  `FOPEN_KEEP_CACHE`, and Linux locks a missing page while its FUSE read is in flight,
  so other readers of the same page wait for that read instead of issuing their own
  (FR-011). FR-009 makes this safe: only verified bytes ever reach the kernel.
- **Span maps**: `moka` (161M) `future::Cache` keyed by inode; the value is the file's
  whole span list sorted by offset (loaded by paging `LocalFileSpans` from offset 0),
  searched by binary search for the span containing a file offset. Bounded by total
  span count with a weigher (1,048,576 spans); `try_get_with` loads a span map once for
  concurrent callers with no hand-written single-flight code. Kernel caches use the TTLs in the spec's Caching and
  Validity table.
- **Memory**: read buffers are bounded by a global semaphore sized to `--memory-limit`
  and a per-file semaphore sized to `--file-memory-limit` (FR-015), each read holding
  one buffer's worth of permits (R12); per-file permits are taken first, so a file at
  its limit holds no global permits while it waits. There is no cached block data to
  account for. The limit is not left to the OS: unbounded growth ends in the OOM killer
  taking the daemon and the mount with it (Principle III). The default is 1 GiB: it is a
  ceiling, not a reservation (use grows to the peak of reads in flight × 1 MiB and stays
  there, since pooled buffers are kept for reuse, R12), and file data
  itself is cached in the reclaimable kernel page cache, outside this limit.
  `scripts/bench.sh` also runs at 256 MiB to show how much the limit matters.
- **Alternatives considered**: a `moka` block cache keyed by exact page range (shares
  only identical requests; duplicates the kernel cache), keyed by aligned chunks (more
  sharing, but read amplification and a second copy of data the kernel already holds),
  or by whole block (megabytes per 4 KiB read); `quick_cache` (34M; no async loader);
  `foyer` (14.7M / 4.3M; memory plus local-disk hybrid, pre-1.0). A disk tier is out
  of PoC scope and is recorded in `docs/future-features.md`.
- **Stat coalescing (Principle VI)**: the kernel coalesces parallel lookups of one name,
  but not `getattr`: when a directory's 250 ms attribute TTL expires, every process that
  stats it at that moment causes its own `StatDirectory`. `stat_file` and
  `stat_directory` therefore go through a `moka` cache keyed by inode with
  `try_get_with`, and the entry is invalidated as soon as the load completes, so callers
  share a request only while it is in flight and nothing is cached.

## R12. Buffers and copies (Principle V hot-path rules)

- **Upstream fact (fuser 0.18)**: `ReplyData::data(&[u8])` borrows the slice and sends it
  with one `writev` together with the reply header (`reply.rs`, `ll/reply.rs`
  `ResponseSlice`). That is the single copy of file data into the kernel.
- **Decision**: the read path makes no other copy of file data.
  - **Read size**: 1 MiB per kernel request, as ternfuse (`go/ternfuse/ternfuse.go:1406`:
    `MaxWrite` and `MaxReadAhead` = 1 MiB). In fuser's `init`,
    `KernelConfig::set_max_readahead(1 MiB)` and `set_max_write(1 MiB)`; 1 MiB is also
    Linux's default per-request ceiling (`fs.fuse.max_pages_limit` = 256 pages). The
    negotiated value is checked in fuser's source when T070 is implemented and logged at
    start-up. Larger requests would need a sysctl change and are left to benchmarks.
  - Each read gets one fixed-size buffer (1 MiB + 2 pages, enough for any page-aligned
    range of a maximal read) from a `deadpool` pool (171M downloads; `QueueMode::Lifo`), returned on drop. The buffers
    are slices of one anonymous `memmap2` mapping of the memory limit advised
    `MADV_HUGEPAGE` (+2% for one reader, -5% CPU per GiB at 16; benchmarks.md). The pool was first a
    `crossbeam-queue` `ArrayQueue`, which is FIFO: after a 16-reader burst, a lone reader
    cycled through every idle buffer (about 30 MiB, more than the CPU cache) and lost 8% of
    its throughput (1,760 to 1,620 MiB/s); last in, first out cut that to about 2%. A buffer is allocated the first
    time no idle one is available and is never freed: after a burst the idle buffers stay
    in the pool for the next reads (up to the 1 GiB default idle, accepted), so steady
    state makes no allocation. Buffers are not preallocated at start-up, so an idle mount
    uses no buffer memory. The pool holds at most `memory_limit / buffer size` buffers,
    and that count is the pool's `max_size`: a read waits for a buffer while all are in
    use, so pooled plus in-flight buffers never exceed `--memory-limit`. Small reads use a whole buffer too (at most about 1,000 reads in
    flight at the default limit), which keeps the accounting exact.
  - The buffer is split into disjoint 4096-byte `&mut` page slices. Each block fetch
    receives the slices its pages map to (for RS spans these are interleaved by cell).
  - The page stream is read with `TcpStream::try_read_vectored`: each page's 4096 data
    bytes land directly in their slice and the 4-byte CRC words in a small side array,
    so stripping CRCs needs no copy. Pages are verified in place.
  - Fetches of one read run concurrently in the request's task with
    `futures-util` `try_join_all` over futures borrowing their slices (no spawn, no
    `'static` buffers).
  - The FUSE reply is the slice of the buffer at the requested offset.
  - Metadata datagrams are received into one reused buffer per socket and decoded
    zero-copy (`Bytes` slices for names).
- **Remaining per-request allocations** (one tokio task per FUSE request, one `oneshot`
  per metadata request, decoded response structs, the `try_join_all` list) are counted
  with `dhat` in one cluster test (`read_alloc`) and recorded under plan CT-4.

## R7. Namespace mapping

- **Decision**: FUSE inode number = TernFS `InodeId`, except FUSE ino 1 ↔
  `ROOT_DIR_INODE_ID` (`DIRECTORY << 61`); TernFS ids always have type bits set, so no
  collision. No per-inode state is kept, so `forget` is a no-op. Lookup = `LookupReq` then
  `StatFile`/`StatDirectory` for attributes. Directory listing on an open handle pages
  through `ReadDirReq` by `NameHash`; offsets returned to the kernel are positions in the
  handle's listing, so `seekdir` is stable for the handle's lifetime. Readdirplus stats
  entries concurrently, bounded by `-readdir-batch-size` (default 1000, as upstream).
- **Presented metadata** (as ternfuse): mode 0555 plus type bits for every inode; uid/gid
  0 unless `-set-uid`, which uses the effective uid/gid of the daemon; nlink 1; mtime and
  atime from the shard; ctime = mtime.

## R8. Errors and errno

- **Decision**: generated `TernError` enum; one function `errno_of(&ClientError) -> Errno`
  in `rustern-fuse`. Table follows the ternfuse behavioural reference (facts, not code):
  `*_NOT_FOUND` → ENOENT, `TYPE_IS_DIRECTORY` → EISDIR, `TYPE_IS_NOT_DIRECTORY` → ENOTDIR,
  `TIMEOUT` → ETIMEDOUT, `NOT_AUTHORISED`/`BAD_COOKIE` → EACCES, internal/unknown → EIO.
  Client-side failures: decode error, CRC mismatch, connection failure, deadline → EIO
  (deadline on metadata → ETIMEDOUT as upstream). Mutations → EROFS before any request.
- Typed errors via `thiserror` (1.85B) in every crate, including the binary. `main` has a
  small `StartupError` enum (invalid arguments, registry unreachable, mount failed), and
  the exit code (2 for invalid arguments, 1 otherwise) is chosen by matching on it, which
  a typed enum makes exhaustive. `anyhow` is not used: its strengths (type-erased errors,
  context chains) add nothing for three error sites, and the baseline only permits it,
  it does not require it.

## R9. CLI, logging, metrics

- **Decision**: `clap` 4 (1.34B) derive API. ternfuse uses Go's `flag` package, which
  accepts `-name` and `--name`; clap only parses `--name`. A small argv shim rewrites
  `-name[=v]` to `--name[=v]` for known long flags before clap parses (FR-002).
  Supported flags: `-registry`, `-o`, `-log-file`, `-verbose`, `-trace`, `-mtu`,
  `-attr-cache-time`, `-dir-entry-cache-time`, `-readdir-batch-size`, `-set-uid`,
  `-signal-parent`, the nine `-{initial,max,overall}-{shard,cdc,block}-timeout` flags,
  plus Rustern-only `--memory-limit`, `--file-memory-limit`, `--threads`. Positional: mount point.
- `tracing` (1.05B) + `tracing-subscriber`; `metrics` (129M) +
  `metrics-exporter-prometheus` (59M) with a handle rendered to the log on SIGUSR1 (as
  ternfuse dumps stats on SIGUSR1); no HTTP endpoint in the PoC (the SIGUSR1 dump is enough to
  compare with ternfuse; a scrape endpoint can be added when Rustern is deployed).
  `build_recorder()` does not spawn the exporter's upkeep task, so `main.rs` calls
  `PrometheusHandle::run_upkeep()` every 5 s (the exporter's own default). Without it every
  histogram sample stays buffered until the next render: about 0.4 MiB/s of growth at
  1.8 GiB/s of reads (found in the 2026-11-14 benchmark, see benchmarks.md).
- Secrets: block certificates and cookies are wrapped in a newtype whose `Debug` prints
  `<redacted>` (FR-017).

## R10. Testing toolchain

- **Decision**: `cargo-nextest` (runner), `proptest` (246M), `cargo-fuzz` on a nightly
  toolchain installed with rustup (fuzzing only; the project toolchain stays stable), a
  stable `cargo test` target that replays every fuzz corpus, `criterion` (340M) for
  micro-benchmarks. Model tests with `loom` (79M) of the request-id demultiplexer are
  postponed (plan CT-6, `docs/future-features.md`); in the PoC it is covered by
  `udp_demux` and the concurrency stress tests. ThreadSanitizer is not run (plan CT-6): every Rustern crate forbids `unsafe`, so a data
  race in Rustern code is a compile error; `dhat` (dev-only, one test) counts allocations per
  read for the Principle V hot-path rules.
- **Cluster tests**: upstream `ternrun -leader-only` from `ternfs/build/alpine`, started by
  the gate script with a data directory on disk (not tmpfs: the cluster pre-allocates
  ~19 GB). Corpus written through the ternfuse mount that `ternrun` creates, using `cp`
  (redirect-based writes hit an unexplained ENOTSUP in ternfuse; see quickstart).
- **Fault injection**: corrupt a stored block by flipping a byte under
  `<data-dir>/bs_N/*/with_crc/`; dead block service = SIGKILL its process; slow/unresponsive
  = SIGSTOP/SIGCONT; malformed responses, loss, duplication and reordering through a
  test-only fault-injecting UDP relay between the client and a shard. `turmoil` (16.5M)
  was considered; it needs the whole client on its simulated sockets and fake servers,
  which is more code than a relay for this scope.
- **Macro benchmarks**: `fio` (to be installed) against both mounts on the same cluster.
- **Golden vectors**: `tools/goldens`, a Go program importing upstream `msgs`, encodes a
  sample of every message type the PoC uses (and CRC cases) into `testdata/goldens/*.hex`;
  Rust tests decode, re-encode and compare byte for byte.

## R11. Workspace and gates

- **Decision**: crates `rustern-msgs` (protocol), `rustern-core` (CRC helpers, span layout
  arithmetic, inode ids), `rustern-client` (async client, no FUSE types), `rustern-fuse`
  (adapter, no protocol logic), `rustern` (binary). One `rustern-` prefix for all: the
  names are free on crates.io (`tern-core` is taken), and `tern-*` or `ternfs-*` would read
  as official upstream crates. Lints once in `[workspace.lints]`:
  `unsafe_code = "forbid"`; clippy `unwrap_used`, `expect_used`, `panic`,
  `indexing_slicing`, `arithmetic_side_effects` = deny; `clippy.toml` allows them in tests.
  `deny.toml` for advisories, licences (allow MIT, Apache-2.0, Apache-2.0 WITH
  LLVM-exception, BSD-2/3, ISC, Unicode-3.0, Zlib), bans and sources. Dependency
  direction check: `cargo tree -i fuser` must show only `rustern-fuse` and `rustern`.
- `scripts/gate.sh` runs every merge gate and fails, naming the tool, if one is missing;
  `--quick` skips cluster suites for the edit loop. Toolchain pinned in
  `rust-toolchain.toml` (1.100.0).
- `ternfs/` stays git-ignored; its commit is checked by the gate against the pin.
