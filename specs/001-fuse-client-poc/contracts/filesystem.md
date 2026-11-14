# Contract: filesystem operations served by the mount

The mount is read-only (`ro,nosuid,nodev`). "Shard" means the metadata shard owning the
inode (`inode_id & 0xff`).

| FUSE operation | Behaviour | Cluster requests | Errors |
|----------------|-----------|------------------|--------|
| `init` | Negotiate; request readdirplus and keep-cache | none | — |
| `lookup` | Resolve name in directory, return attributes | `Lookup`, then `StatFile`/`StatDirectory` | ENOENT, ENOTDIR, EIO, ETIMEDOUT |
| `getattr` | Return attributes | `StatFile`/`StatDirectory` | ENOENT, EIO, ETIMEDOUT |
| `readlink` | Return target bytes | span fetch of the symlink inode | EINVAL (not a symlink), EIO |
| `open` | Accept `O_RDONLY` only; create a file handle (the per-file memory limit is shared by all handles of the inode); reply with keep-cache; send no set-time request (DEV-002) | none | EROFS for write access modes, `O_TRUNC`, `O_CREAT` |
| `read` | Return verified bytes from a pooled buffer with no copy before the reply (R12); short read only at end of file | `LocalFileSpans` (cached), `FetchBlockWithCrc` | EIO (unreadable or corrupt block, malformed response), EINVAL (negative offset) |
| `release` | Drop handle state | none | — |
| `opendir` | Create listing handle; reply with cache-dir and keep-cache, so the kernel serves relistings until the directory mtime changes (seen within the directory attribute TTL, DEV-003) | `StatDirectory` | ENOENT, ENOTDIR, EIO |
| `readdir` / `readdirplus` | Entries with stable offsets; plus attributes for readdirplus | `ReadDir` pages; `Stat*` per entry for plus | ENOENT (directory removed while listing), EIO |
| `releasedir` | Drop listing handle | none | — |
| `statfs` | Constant values, read-only flag set | none | — |
| `forget`, `batch_forget` | No-op (no per-inode state) | none | — |
| `create`, `mknod`, `mkdir`, `unlink`, `rmdir`, `symlink`, `link`, `rename`, `write`, `setattr`, `setxattr`, `removexattr`, `fallocate`, `copy_file_range`, `tmpfile` | Refused before any request | none | EROFS |
| `getxattr`, `listxattr` | Not supported | none | ENODATA / empty list |
| anything else | Not implemented | none | ENOSYS |

Cross-cutting guarantees:

- No request is answered with bytes that failed CRC verification (FR-009).
- No mutating request is ever sent to the cluster (FR-012, SC-006).
- Every non-zero errno is logged with operation, inode and cause (FR-016).
- Attribute and entry TTLs follow the spec's Caching and Validity table.
