# Contract: `rustern` command line and process behaviour

## Invocation

```text
rustern [flags] <mountpoint>
```

Flags accept both `-name value` / `-name=value` (as Go's `flag` package, so ternfuse
command lines work unchanged) and `--name value` / `--name=value`.

| Flag | Type | Default | ternfuse-compatible |
|------|------|---------|---------------------|
| `-registry` | `host:port` | required | yes |
| `-o` | comma-separated mount options | none | yes; `rw` is rejected, `ro,nosuid,nodev` are always applied, `allow_other` adds `default_permissions` (FR-003) |
| `-log-file` | path | stderr | yes |
| `-verbose`, `-trace` | bool | false | yes |
| `-mtu` | u16 | 1472 | yes |
| `-attr-cache-time` | duration | 1h | yes (directories capped, DEV-003) |
| `-dir-entry-cache-time` | duration | 250ms | yes |
| `-readdir-batch-size` | int | 1000 | yes |
| `-set-uid` | bool | false | yes |
| `-signal-parent` | bool | false | yes: SIGUSR1 to parent once mounted |
| `-initial-shard-timeout`, `-max-shard-timeout`, `-overall-shard-timeout` | duration | 100ms, 2s, 10s | yes |
| `-initial-cdc-timeout`, `-max-cdc-timeout`, `-overall-cdc-timeout` | duration | 1s, 10s, 60s | yes |
| `-initial-block-timeout`, `-max-block-timeout`, `-overall-block-timeout` | duration | 1s, 10s, 60s | yes (overall default differs, DEV-006) |
| `--memory-limit` | bytes | 1GiB | Rustern only |
| `--file-memory-limit` | bytes | 32MiB | Rustern only; at least 2 MiB (one read buffer of 1 MiB + 8 KiB) and at most `--memory-limit` |
| `--threads` | int | CPUs, max 16 | Rustern only |

ternfuse flags that only affect writes or privileged features (`-close-tracker-object`,
`-direct-mount`, `-direct-mount-strict`, `-profile-file`, `-syslog`,
`-print-quiet-alerts`, `-xmon`) are accepted and ignored with a warning, so existing
command lines keep working. Unknown flags are an error (exit 2).

Durations use Go syntax (`250ms`, `10s`, `1h`).

## Start-up

1. Parse flags; invalid input → message on stderr, exit 2.
2. Contact the registry and fetch shard addresses within the overall shard deadline.
   Failure → message naming the registry address, exit 1, nothing mounted.
3. Mount. Failure (e.g. mount point missing, `/dev/fuse` unavailable) → exit 1.
   A mount option the system refuses (e.g. `allow_other` without `user_allow_other` in
   `/etc/fuse.conf`) is a failure too: the message gives the reason, and Rustern never
   mounts without an option that was asked for.
4. Log "mounted at <path>"; with `-signal-parent`, send SIGUSR1 to the parent.

## Signals

| Signal | Behaviour |
|--------|-----------|
| SIGINT, SIGTERM, SIGHUP | Unmount, wait for in-flight requests up to 5 s, exit 0 |
| External unmount (`fusermount3 -u`, `umount`) | Log at info, exit 0 |
| SIGUSR1 | Dump counters and latency histograms to the log |
| SIGKILL | Process dies; mount point shows "transport endpoint is not connected" until `fusermount3 -u <mountpoint>` |

## Exit codes

| Code | Meaning |
|------|---------|
| 0 | Clean unmount after a signal or an external unmount |
| 1 | Start-up failure or fatal runtime error (after unmounting) |
| 2 | Invalid command line |
