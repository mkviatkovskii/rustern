# Contract: `rustern` command line, changes for write support

This document is a delta against
[001 contracts/cli.md](../../001-fuse-client-poc/contracts/cli.md). Anything not listed here
is unchanged: invocation, flag syntax, timeouts, signals, exit codes and start-up order.

## Changed flags

| Flag | Type | Default | ternfuse-compatible |
|------|------|---------|---------------------|
| `-o` | comma-separated mount options | none | yes. The mount is read-write by default. `ro` gives the 001 read-only mount, where every mutation gets EROFS. `rw` is accepted and is the default. `nosuid,nodev` are always applied. `allow_other` adds `default_permissions` and fails explicitly when refused (FR-018) |

## New flags (Rustern only)

| Flag | Type | Default | Meaning |
|------|------|---------|---------|
| `--overwrite` | bool | false | `open` with `O_TRUNC` of a published file (including `creat` and `O_CREAT\|O_TRUNC` on an existing name) writes a replacement that is published over the old file at release (FR-011, DEV-010). Without it, write-mode `open` of a published file gets EPERM (DEV-009) |
| `--accept-matching-attrs` | bool | false | `chmod`, `chown` and `chgrp` succeed with no effect when the requested values equal the uid, gid and mode the mount presents, and fail with EPERM otherwise. Without it they all get EOPNOTSUPP (FR-016, DEV-011) |
| `--write-memory-limit` | bytes | 1GiB | Ceiling on span and parity buffers across all files being written. Writers wait at the limit (FR-019). Minimum 256MiB, so that one largest-possible span (100 MiB) plus its parity always fits. A smaller value exits with code 2 |
| `--file-write-spans` | int | 2 | Span buffers one file may hold: one filling, the rest storing. 1 disables the fill-while-store pipeline (FR-020). Range 1–8 |
| `--publish-wait` | duration | 1s | How long a lookup, listing or rename through this mount waits for a file whose descriptors were all closed to be published (FR-004, research W8). 0 disables waiting |

Feature 001's `--memory-limit` and `--file-memory-limit` still bound read buffers only. The
total buffer ceiling is `--memory-limit + --write-memory-limit` (default 2 GiB).

## ternfuse flags

- `-close-tracker-object` is still accepted and ignored with a warning. The publish point
  never depends on close tracking (DEV-007).
- `-direct-mount` and `-direct-mount-strict` are unchanged: accepted and ignored with a
  warning.
- The other ternfuse write-related behaviour has no flag in ternfuse, so there is nothing to
  map.

## Signals and shutdown

| Signal | Behaviour |
|--------|-----------|
| SIGINT, SIGTERM, SIGHUP | Stop accepting new mutations (EIO, logged once), wait up to the 5 s grace for in-flight span stores, publishes and namespace requests, then unmount and exit 0. Files still being written are not published. Their count is logged at warn (`shutdown: N files being written were not published`) and counted (FR-021) |

## Metrics added to the SIGUSR1 dump and the exporter

| Metric | Kind | Labels |
|---|---|---|
| `rustern_write_bytes_total` | counter | — |
| `rustern_span_store_seconds` | histogram | `storage` (inline, mirrored, rs) |
| `rustern_span_store_attempts_total` | counter | `outcome` (ok, retry, failed) |
| `rustern_publish_seconds` | histogram | — |
| `rustern_publish_failures_total` | counter | `reason` (exists, more_recent, cookie, timeout, other) |
| `rustern_write_buffer_bytes` | gauge | — |
| `rustern_write_backpressure_seconds` | histogram | — |
| `rustern_files_being_written` | gauge | — |
| `rustern_fuse_requests_total`, `rustern_fuse_errors_total` | counter (existing) | new `op` values for every mutation |

Cookies, certificates and proofs never appear in logs or metrics. The metric names follow the
001 naming. If the 001 exporter uses different prefixes, these rows follow those prefixes.
