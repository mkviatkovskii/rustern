# Data Model: Read-Only FUSE Client Proof of Concept

Wire types are generated from upstream `go/msgs` (research R3) and are not repeated here.
This document covers the client-side model built on them.

## InodeId (`rustern-core`)

- `u64`. Top 3 bits: type (`DIRECTORY` = 1, `FILE` = 2, `SYMLINK` = 3); low byte: shard id.
- Validation: type bits must be 1–3, otherwise the id is rejected as malformed (EIO).
- FUSE mapping: ino 1 ↔ `ROOT_DIR_INODE_ID` (`1 << 61`); every other id maps to itself.

## Attributes (`rustern-client`)

| Field | Source | Notes |
|-------|--------|-------|
| kind | InodeId type bits | file, directory, symlink |
| size | `StatFileResp.Size` | directories report 0 |
| mtime | `StatFileResp.Mtime` / `StatDirectoryResp.Mtime` | TernTime (ns since epoch) |
| atime | `StatFileResp.Atime` | directories: mtime |
| owner | configuration | uid/gid 0, or daemon's effective ids with `-set-uid` |
| mode | constant | 0555 plus type bits (as ternfuse) |

A failed stat is an error; no placeholder attributes are ever produced (FR-006).

## Directory listing handle (`rustern-fuse`)

- Fields: directory id, parent id (from `StatDirectoryResp.Owner`, or self for the root),
  `next_hash: Option<NameHash>` (None = exhausted), buffered entries, position counter.
- Kernel offsets: 1 = ".", 2 = "..", 3 + n = n-th entry of this handle's listing.
- `seekdir` to an offset already passed re-lists from the start and skips to the position
  (bounded by entry count); entries are never returned twice for one position.
- Readdirplus: each buffered entry is stat-ed concurrently, at most
  `readdir_batch_size` in flight.

## Span (`rustern-core` layout, `rustern-client` cache)

```text
Span
├── byte_offset: u64          file offset of the first byte
├── size: u32                 logical bytes (may exceed what is stored: the rest are zeros)
├── crc: Crc                  CRC32C of the logical bytes, zero-extended
└── body
    ├── Inline { data }                       verified against crc on decode
    └── Blocks { parity, stripes, cell_size,
                 blocks: [ {service, block_id, crc} ], stripes_crc }
```

Validation on decode (all failures → EIO, logged with inode and offset):

- `parity.data_blocks` ≥ 1, `blocks.len() == data + parity`, `stripes` in 1..=15,
  `cell_size` a non-zero multiple of 4096.
- `stripes_crc.len() == stripes`; block size `cell_size × stripes` fits `u32`.
- `size` may exceed the capacity `data_blocks × cell_size × stripes` (upstream
  `sizeWithZeros`, `go/client/span.go`): bytes past capacity are zeros. `byte_offset + size`
  must not overflow.
- every `block_service_ix` < `block_services.len()`.
- inline body length ≤ `size`; `crc32c(body)` zero-extended to `size` equals `crc`
  (upstream `createInlineSpan`).
- blocks on services whose flags forbid reading (stale, no-read, decommissioned) are
  skipped, as upstream `CanRead`.

Layout functions (pure, property-tested): `locate(span, span_offset) -> (block_ix,
block_offset)` and `ranges(span, from, to) -> [(block_ix, page-aligned block range,
output range)]`, with the inverse used in tests to prove every byte maps to exactly one
block position.

## Block fetch (`rustern-client`)

- Key: `(block_id, page_start, page_count)`.
- Value: `Bytes` of verified data pages (CRC words stripped).
- State per attempt: `Pending → Verified | Failed(reason)`; reasons: connect, timeout,
  protocol, page CRC mismatch (with page index), server error code.
- Mirrored spans try blocks in order until one is `Verified`; RS spans fail on the first
  `Failed` data block.

## Request lifecycle (`rustern-client`)

```text
Created ──send──▶ InFlight ──response(kind ok)──▶ Done(Ok)
                     │  ├──response(ERROR code)──▶ Done(TernError)
                     │  └──attempt timeout──▶ Backoff ──▶ InFlight (other address)
                     └──overall deadline──▶ Done(Timeout)
```

- Request ids: monotonically increasing `u64` from 1 per client.
- Unmatched, duplicate, or late responses are counted and dropped.
- Dropping the caller's future removes the pending entry (cancel-safe).

## Configuration (`rustern`)

| Setting | Flag | Default |
|---------|------|---------|
| Registry | `-registry host:port` | required |
| Mount point | positional | required |
| Shard timeouts | `-{initial,max,overall}-shard-timeout` | 100 ms / 2 s / 10 s |
| CDC timeouts | `-{initial,max,overall}-cdc-timeout` | 1 s / 10 s / 60 s |
| Block timeouts | `-{initial,max,overall}-block-timeout` | 1 s / 10 s / 60 s (DEV-006) |
| Attribute TTL | `-attr-cache-time` | 1 h files; directories use the entry TTL (DEV-003) |
| Entry TTL | `-dir-entry-cache-time` | 250 ms |
| Readdir batch | `-readdir-batch-size` | 1000 |
| MTU | `-mtu` | 1472 |
| Memory limit | `--memory-limit` | 1 GiB of read buffers (pooled and in flight), counted as whole buffers of 1 MiB + 8 KiB |
| Per-file memory limit | `--file-memory-limit` | 32 MiB of read buffers per file, shared by its open handles; 2 MiB ≤ value ≤ memory limit (at least one read buffer, so no read can wait forever) |
| Maximum read | none (fixed) | 1 MiB per kernel read request, as ternfuse (`MaxReadAhead`, `MaxWrite` = 1 MiB) |
| Registry refresh | none (internal `ClientConfig`) | 60 s |
| Registry alert after | none (internal `ClientConfig`) | 60 s of failed refreshes |
| FUSE threads | `--threads` | number of CPUs, max 16 |
