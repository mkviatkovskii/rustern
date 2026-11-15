# Future features

Ideas deferred from earlier features. Each becomes a `/speckit-specify` feature when it
is picked up; the notes here are starting points, not decisions.

## Local-disk block cache (foyer)

**Origin**: feature 001 research R6. **Status**: idea.

Cache verified block pages on a local SSD as well as in memory, so repeated reads of
large files are served locally instead of from block services.

- **Library**: [`foyer`](https://crates.io/crates/foyer) (14.7M downloads, 4.3M in the
  last 90 days at 2026-11-14), a hybrid memory plus disk cache with request coalescing.
  Pre-1.0 (0.22) with frequent breaking releases, so pin the version and re-check
  maturity when the feature starts. Feature 001 has no in-process block cache and relies
  on the kernel page cache (R6), so this feature adds the block-cache layer itself.
- **Why it is safe for validity**: TernFS files are immutable once written and block ids
  are never reused, so a page cached under its block id cannot go stale. The cache key
  should still include the cluster identity, so one cache directory never serves another
  cluster's data.
- **Obligations it brings**:
  - Principle I: bytes read back from local disk are verified again before being
    returned, so page CRCs are stored with the cached data.
  - Principle IV: the cache directory is created with mode 0700 and owned by the daemon's
    user; decide whether it is wiped on start, on unmount, or never; document that
    cached file contents persist on local disk.
  - Principle III: a full, slow or failing cache disk degrades to "cache miss", never to a
    failed read; size and I/O concurrency are bounded.
  - Configuration: cache directory, size limit, enable flag (off by default); eviction
    and restart behaviour covered by tests.
- **To measure**: hit-rate and latency benefit on `seq-read` and `rand-read` re-runs, and
  the write amplification on the cache disk.

## Socket tuning with socket2

**Origin**: feature 001 research R4. **Status**: idea, decided by benchmark.

Feature 001 uses plain `tokio::net` sockets. [`socket2`](https://crates.io/crates/socket2)
(already compiled as a `tokio` dependency) exposes options `tokio` does not, chiefly a
larger UDP receive buffer (`SO_RCVBUF`) for the shared metadata socket.

- **To measure**: UDP receive drops (`/proc/net/snmp` `RcvbufErrors`) and `meta-latency`
  and `readdir-latency` p99 under 16 concurrent readers and large readdirplus fan-out,
  with the default buffer and with enlarged buffers.
- **Adopt if**: drops occur at the default size or p99 improves measurably; otherwise
  record the result and close the idea.
- **Related**: batched UDP syscalls via `quinn-udp` (CT-4) are measured in the same run.

## FUSE interrupt support

**Origin**: feature 001 DEV-005 / CT-3. **Status**: required before v1.

`fuser` answers FUSE_INTERRUPT with ENOSYS, so a request blocked on the cluster cannot be
cancelled by a signal and waits up to the overall deadline (capped at 60 s, DEV-006).
Options: add an `interrupt()` dispatch to `fuser` and upstream it (carried through
`[patch.crates-io]` meanwhile), or switch FUSE backend.

## Concurrency model tests (loom)

**Origin**: feature 001 analysis C3 / CT-6. **Status**: required before v1.

Model-check the UDP request demultiplexer with [`loom`](https://crates.io/crates/loom):
insert, complete and drop-guard over the pending-request table, with three threads
(a caller that gives up, a receiver completing its id, a duplicate response). Assert a
response is delivered at most once, a completion after the caller gave up is dropped, and
the table ends empty. Needs the table made generic over its map type so a loom
`Mutex<HashMap>` can stand in for `DashMap`. ThreadSanitizer stays unnecessary while every
crate forbids `unsafe`.

## Degraded reads (Reed-Solomon reconstruction)

**Origin**: feature 001 DEV-004 / CT-2. **Status**: required before v1.

Reconstruct missing or corrupt data blocks of RS spans from parity, using an in-house
port of the upstream `rs` implementation proven bit-identical by golden vectors
(Technology Baseline).

The decoder (`crates/rustern-rs`, feature 002) is in place. Before wiring it in, restore
upstream's single-output-row loop for recovery: a degraded read rebuilds one block, and
our generic matrix-multiply kernels run it up to 37% slower than upstream (`tools/rsbench`,
i9-13900K: recover D10P4 227 vs 166 µs on GFNI, D8P3 246 vs 182 µs on AVX2).

## Fewer copies and syscalls on the FUSE reply path

**Origin**: 2026-11-14 benchmark discussion. **Status**: investigated; FUSE over io_uring
is the only open item.

A read copies file data twice: block service socket → Rustern's pooled buffer (CRC words
go to a side array, pages are verified in place), then `writev` on `/dev/fuse` → the
reader's pages (O_DIRECT) or the page cache. fuser 0.18 sends the reply with that one
`writev`; the `Vec`-copying sender in its `reply.rs` is test-only.

- **splice / vmsplice to `/dev/fuse`: rejected.** Splicing the socket straight to
  `/dev/fuse` would skip userspace, but then pages cannot be CRC-verified (Principle I)
  and the per-page CRC words would reach the kernel with the data. Splicing from the
  verified buffer (`vmsplice` to a pipe, `splice` to `/dev/fuse`) keeps the copy: the
  kernel copies pipe pages into the destination unless it can steal them
  (`SPLICE_F_MOVE`/`GIFT`), and Linux 6.18 has no `fuse_try_move_page`. Same one copy,
  two syscalls instead of one.
- **FUSE over io_uring: open.** `CONFIG_FUSE_IO_URING` is built in (off until
  `/sys/module/fuse/parameters/enable_uring` is set). The payload is still copied from the
  ring buffer (`fuse_uring_copy_from_ring`), so the copy count does not change; the gain
  would be fewer syscalls and context switches, with per-CPU queues. fuser 0.18 has no
  support, so this means another FUSE layer. Ties in with the FUSE backend choice in
  "FUSE interrupt support".
- **io_uring zero-copy receive** (`CONFIG_IO_URING_ZCRX`) would remove the socket copy,
  but needs NIC header split and flow steering; not possible on loopback.
