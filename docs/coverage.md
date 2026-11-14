# Test coverage

`scripts/gate.sh` (full mode) runs every test, the cluster and fault passes included, under
`cargo llvm-cov` and fails below the line floor (`RUSTERN_COVERAGE_FLOOR`, default 98%).

Measured 2026-11-14 on the whole suite: **98.6% of lines** (8,795 lines, 123 not run).
`generated.rs` runs every line; its missed regions are `?` after encoding fixed-size
fields, which cannot fail.

To reproduce:

```sh
cargo llvm-cov clean --workspace
cargo llvm-cov nextest --no-report --workspace --run-ignored all -E 'not test(/^cluster_serial::/)'
cargo llvm-cov nextest --no-report --workspace --run-ignored all -E 'test(/^cluster_serial::/)'
cargo llvm-cov report --text   # or --html
```

Branch coverage (`--branch`) needs a nightly toolchain and is not gated.

## Lines left uncovered, and why

Each group below is a reason. A line that fits none of them is a missing test.

### Panics in test code

Test helpers end their `match` with `other => panic!(...)` for inputs the test data never
contains: `blocks.rs` (fake block service), `bufpool.rs`, `crc.rs` (golden op names),
`span.rs`, `dirhandle.rs` (fake pager), `frame.rs` (golden frame kinds).

### Checks the kernel makes first

`fs.rs`: lookup under a non-directory (ENOTDIR), readlink on a non-symlink (EINVAL), an
open for writing or truncation (EROFS: the mount is `ro`, so the kernel refuses first;
`bridge_mount.rs::mutations_without_ro_get_erofs` covers the bridge without `ro`), open
of a directory (EISDIR), opendir of a file (ENOTDIR). The VFS rejects all of these before
sending a request.

### Values that cannot occur on a 64-bit host

`usize::try_from(u64)` and `u32::try_from` of in-memory lengths: `wire.rs` (`get_len`),
`frame.rs` (registry message over 4 GiB), `cli.rs` (byte sizes), `bufpool.rs` (memory
limit overflow), `read.rs` `bounds()` (checked arithmetic on offsets already
validated by `Span::try_from`), `layout.rs:67`. `spans.rs` `smaller` runs only at
compile time (it computes `MIN_ENCODED_SIZE`).

### Operating-system failures with no unprivileged trigger

`udp.rs`: binding the UDP socket, a receive error, the receive task ending.
`bufpool.rs`: the buffer mapping running out, `MADV_HUGEPAGE` refused (it is accepted
wherever transparent huge pages are compiled in). `main.rs`: installing the metrics
recorder twice, `-signal-parent` with no parent or a parent that cannot be signalled,
the FUSE session thread panicking or failing (`StartupError::Session`). `bridge.rs`
`forget`: a no-op, called only when the kernel evicts inodes (forcing that needs
`drop_caches`, which needs root).

### Races too narrow to hit on purpose

`fs.rs:232-233` (an entry removed between `readdir` and its `stat` in readdirplus;
`faults.rs::dir_removed_while_listing` covers the plain readdir case), `udp.rs:157`
(a response for a request removed by its caller between lookup and send),
`read.rs:687`, `addresses.rs:45`, `lib.rs:63,145` (a refresh racing shutdown),
`dirhandle.rs:159`.

### Responses a real or fake server does not send

`shard.rs`: a response of another kind, a send error (other than the MTU check), the
loop ending without an attempt. `blocks.rs`: a block response header or kind other than
`FETCH_BLOCK_WITH_CRC`/`ERROR`, a decode error after a valid header. `error.rs`:
`FrameError::Protocol` (the UDP demultiplexer drops wrong-protocol datagrams before this
mapping; `udp_demux.rs` counts them). `read.rs`: `ReadTooLarge` (the bridge caps reads at
`max_read`), "file spans did not advance", an inline span in the block path, a block span
with no blocks, `closed()` (the read semaphore is never closed). `errno.rs:43`
(`ClientError::Shared` wraps only errors already mapped). `config.rs:183` (test
constant).

### Logging-only lines

`main.rs:97` (warnings for ignored flags; the text is checked by
`cli.rs::tests::ignored_flags_warn`), `cli.rs:251`.
