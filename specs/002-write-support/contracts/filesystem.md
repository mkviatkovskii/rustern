# Contract: filesystem operations, changes for write support

This is a delta against
[001 contracts/filesystem.md](../../001-fuse-client-poc/contracts/filesystem.md).

- With `-o ro`, the 001 table applies unchanged, including EROFS for every mutation.
- With the default read-write mount, the rows below replace the 001 rows for the same
  operations. The rows not listed (`readlink`, `opendir`, `releasedir`, `forget`, xattr reads)
  are unchanged.

"Writer" is the per-file state machine in [data-model.md](../data-model.md). "Pending"
means an entry in the pending-publish registry (research W8).

| FUSE operation | Behaviour | Cluster requests | Errors |
|----------------|-----------|------------------|--------|
| `init` | As 001, plus `ATOMIC_O_TRUNC`, `max_write` 1 MiB, no writeback cache | none | — |
| `lookup` | As 001. If (dir, name) is pending: wait up to `--publish-wait` when Publishing or flushed-idle, then answer with the published file or ENOENT. Still being written: ENOENT at once. Never a negative entry with TTL for a pending name | as 001 | as 001 |
| `getattr` | Writer present: size = bytes accepted (FR-012), with mode and owner presented as for published files (001 attribute rules), and zero attribute validity. Otherwise as 001 | none for a writer | as 001 |
| `create` | Validate name; `ConstructFile{FILE}` on the parent's shard; resolve directory policies; start a writer; insert pending; reply entry (zero attr validity) and a handle with `FOPEN_DIRECT_IO`. Existing name: `O_EXCL` → EEXIST; otherwise as `open` of that file with the same flags | `ConstructFile`, policy lookups (cached) | EEXIST, ENAMETOOLONG, EINVAL, ENOTDIR, EIO |
| `tmpfile` (`O_TMPFILE`) | As `create` with no name and no pending entry (needs the fuser change, research W15) | `ConstructFile` | EOPNOTSUPP until supported (CT-8), EIO |
| `open` | Read-only access: as 001. Write access to a writer's inode (another `open` while the file is being written): new handle on the same writer. Write access to a published file: EPERM (DEV-009), except `O_TRUNC` with `--overwrite`, which starts a replacement writer for the same name (DEV-010). `O_APPEND` alone is still EPERM | as `create` for a replacement | EPERM, EISDIR, EIO |
| `read` | Writer handle or a writer's inode: EOPNOTSUPP (FR-010). Otherwise as 001 | as 001 | EOPNOTSUPP, as 001 |
| `write` | Send to the writer; backpressure at memory limits; reply after the data is copied into the span buffer | `AddSpanInitiate`, `WriteBlock` ×B, `AddSpanCertify` (or `AddInlineSpan`), asynchronously as spans fill | EINVAL (offset before end), the sticky errno of a Failed writer, EBADF on a read-only handle |
| `flush` | Writer: seal the partial span and wait until everything is certified (FR-004). No publish. Read handle: OK | span requests | EIO, ENOSPC (sticky) |
| `fsync`, `fdatasync` | As flush (FR-007). Published file: OK | span requests | as flush |
| `release` | Last handle of a named writer: seal, store, certify, `LinkFile` (or the replacement link), invalidate (parent, name), resolve pending. Errors are logged and counted, the reply is OK. Unnamed or Abandoned writer: drop it | span requests, `LinkFile` | none to the caller (FR-005) |
| `link` (`linkat`) | Source is an unnamed writer (`O_TMPFILE`) and the target directory is its owner: publish under the name and reply after `LinkFile` (errors reach `linkat`). Different directory: EXDEV. Source is a published file: EPERM (hard links are out of scope) | as release | EXDEV, EPERM, EEXIST, EIO |
| `setattr` size | Writer: extend with zeros (W4); shrink → EOPNOTSUPP (FR-006). Published file: EPERM | span requests | EOPNOTSUPP, EPERM |
| `setattr` atime/mtime | Published file: `SetTime` (bit 63 marks a set field; `UTIME_NOW` uses the daemon clock); invalidate the inode. Writer: stored in the writer and applied with `SetTime` after publish | `SetTime` | ENOENT, EIO |
| `setattr` mode/uid/gid | EOPNOTSUPP. With `--accept-matching-attrs`: equal to presented values → OK, no request; else EPERM (DEV-011) | none | EOPNOTSUPP, EPERM |
| `mkdir` | Validate; CDC `MakeDirectory`; reconcile EEXIST after a retry (W11); invalidate (parent, name) | `MakeDirectory` | EEXIST, ENOENT, ENOTDIR, ENAMETOOLONG, EIO |
| `rmdir` | `Lookup`, CDC `SoftUnlinkDirectory`; reconcile (W11) | `Lookup`, `SoftUnlinkDirectory` | ENOTEMPTY, ENOENT, ENOTDIR, EBUSY, EIO |
| `unlink` | Pending name: abandon the writer, remove pending, OK (DEV-012). Otherwise `Lookup`, `SoftUnlinkFile`, hard unlink when the snapshot policy allows immediate deletion (errors ignored, as ternfuse); reconcile (W11); invalidate | `Lookup`, `SoftUnlinkFile`, `SameShardHardFileUnlink` / `CrossShardHardUnlinkFile` | ENOENT, EISDIR, EIO |
| `rename` (and `rename2` with flags 0) | Source or target pending: wait as lookup when flushed-idle, else EBUSY (DEV-012). Same directory: `SameDirectoryRename`. File across directories: CDC `RenameFile`. Directory: CDC `RenameDirectory`. Target file exists: replaced (old one becomes a snapshot). Reconcile (W11). Invalidate both entries | `Lookup` of the source, then one of the three | EINVAL (own subtree), ENOTEMPTY, EISDIR, ENOTDIR, EEXIST, EBUSY, ENOENT, EIO |
| `rename2` with `RENAME_NOREPLACE` | Lookup target first: exists → EEXIST; otherwise as rename (best effort: a concurrent create by another client can still be replaced, documented) | as rename | as rename |
| `rename2` with `RENAME_EXCHANGE` / `RENAME_WHITEOUT` | Not supported | none | EINVAL |
| `symlink` | Validate; `ConstructFile{SYMLINK}`, `AddInlineSpan` (target ≤ inline limit; longer targets via one block span as upstream), `LinkFile`; invalidate | as listed | EEXIST, ENAMETOOLONG, EIO |
| `mknod` | Regular file (`S_IFREG`): as create then release (empty file published). Other types: EPERM | as create | EPERM |
| `statfs` | As 001 without the read-only flag on read-write mounts | none | — |
| `setxattr`, `removexattr` | Out of scope | none | EOPNOTSUPP |
| `fallocate`, `copy_file_range` | Out of scope; the kernel falls back to read/write for `copy_file_range` | none | EOPNOTSUPP |

## Cross-cutting guarantees

- A name appears in the cluster only through `LinkFile` of a writer whose every span is
  certified. No `LinkFile` is sent for a Failed or Abandoned writer (FR-001, FR-005).
- Every success reply to `flush`, `fsync` or `linkat` follows certification of all data
  accepted before the request (FR-004, FR-007).
- Names are validated before any request is sent (FR-017).
- Every mutation retried after a lost response goes through the reconciliation in research
  W11 (FR-015).
- Each non-zero errno is logged at debug with op, inode and cause, and counted in
  `rustern_fuse_errors_total{op,errno}` (FR-022). Publish failures at release are logged at
  warn.
- Kernel entries and attributes affected by a mutation through this mount are invalidated
  before the reply, or before the pending entry resolves for a publish.
