# Data Model: Write Support

Entities that feature 002 adds to the client library (`rustern-client`), the core crate
(`rustern-core`) and the FUSE adapter (`rustern-fuse`). Decisions are in
[research.md](research.md) (W*n*).

## MountMode and WriteAccess (W17)

| Field | Type | Rule |
|---|---|---|
| `MountMode` | `ReadOnly \| ReadWrite` | `-o ro` → `ReadOnly`; default `ReadWrite` (FR-018) |
| `WriteAccess` | zero-sized token, constructor private to `rustern-client::Client::writer()` | `Client::writer()` returns `None` in `ReadOnly`; every mutating client API takes `&WriteAccess` |
| `WriteOptions` | `{ overwrite: bool, accept_matching_attrs: bool, publish_wait: Duration }` | from `--overwrite`, `--accept-matching-attrs` and `--publish-wait` ([contracts/cli.md](contracts/cli.md)) |

## DirectoryPolicies (resolved)

Resolved once when a file is created (Caching and Validity: held for the file's lifetime).

| Field | Source | Validation |
|---|---|---|
| `span: Vec<(max_size: u32, parity: Parity)>` | `SPAN_POLICY` entry of the nearest ancestor that sets it | non-empty, ascending `max_size` ≤ `MAXIMUM_SPAN_SIZE` (100 MiB), `Parity` valid (D ≥ 1, D+P ≤ 16) |
| `block: Vec<(min_size: u32, storage_class)>` | `BLOCK_POLICY` | storage class not EMPTY or INLINE |
| `stripe: target_stripe_size: u32` | `STRIPE_POLICY` | > 0; stripes are clamped to 1..15 |
| `snapshot: SnapshotPolicy` | `SNAPSHOT_POLICY` | used by unlink (W11) |

Any violation is EIO with an alert-level log (cluster misconfiguration), never a panic.

## SpanLayout (write side, `rustern-core::span`, W3)

A pure function of `(size, policies)`:

| Field | Rule |
|---|---|
| `parity: (D, P)` | first span-policy entry with `max_size ≥ size` |
| `stripes: S` | from the stripe policy for `size`, clamped to `[1, 15]` |
| `cell_size` | `ceil(ceil(size/D)/S)` rounded up to 4096 |
| `block_size` | `cell_size * S` |
| `storage_class` | block policy for `block_size` |
| `data_len` | `S * D * cell_size` ≥ written bytes (zero padded) |
| `cell_crcs: [Crc; S*(D+P)]` | row-major `Crcs[B*s + b]`; mirrored: one CRC per stripe repeated |
| `span_crc` | crc32c of written bytes, zero-extended to `size` |

Invariant (property-tested): `span_crc == crc32c(bytes the read path returns for the span)`,
and the shard check (`_checkSpanBody`) holds: the span CRC equals the cell CRCs appended and
zero-extended, and parity-0 CRCs equal the XOR combination.

## SpanBuffer (W7)

| Field | Type | Rule |
|---|---|---|
| `buf` | pooled `Vec<u8>`, capacity = file's max span size | taken with a byte permit from the write semaphore; returned zeroed up to `filled` |
| `start` | file offset, multiple of 4096 | equals the previous span's end |
| `filled` | bytes of data or explicit zeros in `buf` | ≤ capacity |
| `implicit_zeros` | `u64` | trailing zeros not in `buf` (gap at the span end, W4) |
| `permit` | `OwnedSemaphorePermit` | released on drop (cancel-safe) |

Sealing produces a **SealedSpan**: `{ start, size = filled + implicit_zeros, layout, buf,
parity: Option<pooled buf>, kind: Inline | Blocks }`. `Inline` when `size < 256`.

## FileWriter state machine (Principle I, W5)

One per file being written. It is owned by its actor task, and only the actor touches it.

```text
                 write / truncate(extend) / flush / fsync
                ┌──────────────┐
                ▼              │
  create ──► Open ─────────────┘
              │  │  release                   LinkFile ok
              │  └──────────────► Publishing ────────────► Published
              │                       │ LinkFile err (logged, counted)
              │  store/certify error  ▼
              ├──────────────────► Failed(errno) ◄─── store error while Publishing
              │  unlink of the name / O_TMPFILE closed without linkat
              └──────────────────► Abandoned
```

```rust
enum WriteState {
    Open { size: u64, flushed_idle: bool },
    Publishing,
    Published { creation_time: TernTime },
    Failed(Errno),          // sticky: the errno the first failing caller saw
    Abandoned,              // never published; writable through open descriptors
}
```

Fields of `FileWriter`, besides the state:

| Field | Type | Rule |
|---|---|---|
| `id` | `InodeId` (FILE) | from `ConstructFile`; the same inode number before and after publish |
| `cookie` | `Cookie` | never logged (FR-017 of 001, Principle IV) |
| `owner` | `InodeId` (directory) | directory the file was created in |
| `name` | `Option<Name>` | `None` for `O_TMPFILE` until `linkat` |
| `replaces` | `Option<(InodeId, TernTime)>` | set under `--overwrite` (W9) |
| `policies` | `Arc<DirectoryPolicies>` | resolved at create |
| `size` | `u64` | bytes accepted so far (FR-012) |
| `certified` | `u64` | end offset of the last certified span |
| `filling` | `Option<SpanBuffer>` | the span being filled |
| `in_flight` | bounded `VecDeque<SpanStore>` | ≤ `--file-write-spans` − 1 |
| `handles` | `u32` | open FUSE handles; release of the last one triggers publish |

### Events and transitions

| State | Event | Guard | Next state | Actions |
|---|---|---|---|---|
| Open | `Write{off, len}` | `off == size` | Open (`flushed_idle=false`) | copy into `filling`; seal and store when full |
| Open | `Write{off, len}` | `off > size` | Open | gap as zeros (W4), then the write |
| Open | `Write{off, len}` | `off < size` | Open | reply EINVAL (FR-002) |
| Open | `Truncate{new}` | `new ≥ size` | Open | extend with zeros (W4) |
| Open | `Truncate{new}` | `new < size` | Open | reply EOPNOTSUPP (FR-006) |
| Open | `Flush` / `Fsync` | — | Open (`flushed_idle=true` after a flush) | seal the partial span; wait until `certified == size`; reply |
| Open | `Release` | `handles == 1`, `name` is some | Publishing | seal, store, certify, then `LinkFile` |
| Open | `Release` | `handles == 1`, `name` is none | Abandoned | nothing is published (FR-008) |
| Open | `Release` | `handles > 1` | Open | `handles -= 1` |
| Open | `Link{dir, name}` (linkat) | `name` none, `dir == owner` | Publishing → Published | seal, store, certify, `LinkFile`; reply to `linkat` (errors reach it) |
| Open | `Link{dir, ..}` | `dir != owner` | Open | reply EXDEV |
| Open | `SpanFailed(e)` | — | Failed(e) | fail every pending reply with `e` |
| Open | `Unlinked` | — | Abandoned | remove the pending entry |
| Publishing | `LinkOk(t)` | — | Published | invalidate (parent, name); resolve watchers `Done(Ok)` |
| Publishing | `LinkErr(e)` | — | Failed(e) | warn log + `rustern_publish_failures_total`; watchers `Done(Err)` |
| Failed(e) | any command | — | Failed(e) | reply `e` (sticky, FR-005); `Release` drops the writer |
| Abandoned | `Write` / `Flush` / `Fsync` | — | Abandoned | accepted (data stored, never published) |
| Abandoned | `Release` | last handle | — | drop the writer; the transient file expires in the cluster |
| Published | — | — | — | the writer is dropped; the inode is an ordinary published file |

Interrupts and cancellation: a FUSE handler that is dropped (unmount, or a future interrupt
support) drops its `oneshot` receiver only. The actor completes or fails the operation on its
own, so a dropped caller can never leave a half-applied state. Span permits and buffers are
RAII and released on every path.

### Invariants (checked by the model test, W13)

1. Published ⇒ every byte acknowledged to `write` is in a certified span, and no byte that
   was not acknowledged is (no publish of unacknowledged data).
2. At most one `LinkFile` is issued per writer.
3. Failed and Abandoned are terminal for publishing: no `LinkFile` after entering them.
4. Every reply to `Flush` or `Fsync` with success happens after `certified == size` at the
   time the command was received.
5. `Failed(e)` replies `e` to every later command.
6. Spans are initiated in increasing offset order, and span k+1 only after span k is
   certified.
7. Total permits held ≤ `--write-memory-limit`, and per file ≤ `--file-write-spans` spans.

## SpanStore (one attempt sequence for one span, W6)

| Field | Type | Rule |
|---|---|---|
| `span` | `SealedSpan` | |
| `attempt` | `u8` | 1..=5 |
| `blacklist` | `Vec<BlacklistEntry>` | failure domains of failed blocks |
| `deadline` | `Instant` | overall block deadline (DEV-006) |
| `blocks` | `Vec<(BlockServiceId, BlockId, Certificate)>` | from `AddSpanInitiateResp` |
| `proofs` | `Vec<BlockProof>` | collected from `WriteBlockResp`; never logged |

The outcome is `SpanStored{end}` or `SpanFailed(errno)`. A dirty attempt is moved with
`ConstructFile{bad_add_span_attempt}` and `MoveSpan` before the retry.

## PendingEntry (W8)

The key is `(DirId, Name)` in a `DashMap`. The value is a `tokio::sync::watch::Sender<PendingState>`:

```rust
enum PendingState { Open { flushed_idle: bool }, Publishing, Done(Result<InodeId, Errno>) }
```

It is inserted at create (named files only) and removed at `Done` or `Abandoned`. Lookup,
readdir, rename and unlink consult it ([contracts/filesystem.md](contracts/filesystem.md)).
Two writers creating the same name in one mount each have a writer. The registry holds the
later one, and the earlier one's publish still happens: cluster rules decide which name wins,
as the edge cases describe.

## Errno mapping for mutations (one place: `rustern-fuse::errno`)

| Cluster error | errno | Note |
|---|---|---|
| NAME_NOT_FOUND, FILE_NOT_FOUND, DIRECTORY_NOT_FOUND, OLD/NEW_DIRECTORY_NOT_FOUND | ENOENT | after reconciliation (W11) |
| CANNOT_OVERRIDE_NAME, NAME_IS_LOCKED | EEXIST | |
| MORE_RECENT_CURRENT_EDGE | EEXIST | publish failure when it happens at release |
| DIRECTORY_NOT_EMPTY | ENOTEMPTY | |
| TYPE_IS_DIRECTORY / TYPE_IS_NOT_DIRECTORY | EISDIR / ENOTDIR | |
| LOOP_IN_DIRECTORY_RENAME | EINVAL | ternfuse: ELOOP; POSIX: EINVAL (DEV-013) |
| COULD_NOT_PICK_BLOCK_SERVICES, first attempt | ENOSPC | W6; ternfuse: EIO (DEV-013) |
| COULD_NOT_PICK_BLOCK_SERVICES after blacklisting, block write exhausted, BAD_BLOCK_PROOF, BAD_SPAN_BODY | EIO | BAD_* also raise an alert (client bug) |
| LAST_SPAN_STATE_NOT_CLEAN, MTIME_IS_TOO_RECENT, FILE_IS_TRANSIENT | EBUSY | as ternfuse |
| BAD_COOKIE, NOT_AUTHORISED | EIO | ternfuse: EACCES; a cookie error is a client or GC-expiry fault (2 h idle transient deadline), not a permission (DEV-013) |
| TIMEOUT / overall deadline | EIO | as the 001 read path |
| name validation | ENAMETOOLONG / EINVAL | before sending (FR-017) |
| write behind end | EINVAL | FR-002 |
| shrink, read of a being-written file, xattr, fallocate, mknod non-regular, hard link of a published file | EOPNOTSUPP / EPERM | [contracts/filesystem.md](contracts/filesystem.md) |
