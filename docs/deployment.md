# Deploying Rustern

Rustern runs as an ordinary user. Everything it needs works without root; each tuning
below is optional, needs root once on the host (not for Rustern itself), and Rustern
behaves predictably without it. Command-line flags are in
`specs/001-fuse-client-poc/contracts/cli.md`.

## Host requirements

- Linux with `/dev/fuse` and `fusermount3` (package `fuse3`). The mount is made through
  the setuid `fusermount3`; Rustern never needs `CAP_SYS_ADMIN`.
- On x86-64, a CPU with x86-64-v3 (Haswell, 2013, or later). Release builds use it
  (`.cargo/config.toml`); an older CPU stops the binary with SIGILL at start.
- Network access to the registry and the block services (TCP) and the shards (UDP).

## The unprivileged default

```sh
rustern -registry <host:port> <mountpoint>
```

- The mount is `ro,nosuid,nodev` and visible only to the user who mounted it (FUSE's
  default). Other users, root included, get EACCES.
- Unmount: SIGINT, SIGTERM or SIGHUP (Rustern unmounts and exits 0), or
  `fusermount3 -u <mountpoint>`. After SIGKILL the mount point reports "transport
  endpoint is not connected" until `fusermount3 -u <mountpoint>`.
- Memory: read buffers are bounded by `--memory-limit` (default 1 GiB) and per file by
  `--file-memory-limit` (default 32 MiB). Measured resident memory stays near 90 MiB
  under 16 readers (`benchmarks.md`).

## Root-only tunings

| Tuning | What it changes | Without it |
|---|---|---|
| `user_allow_other` in `/etc/fuse.conf` | Lets `-o allow_other` share the mount with other users. Rustern adds `default_permissions`, so the kernel enforces the uid, gid and mode it shows. | `-o allow_other` fails at start-up with a message naming `user_allow_other`; Rustern never mounts without an option that was asked for. Without `-o allow_other` nothing changes. |
| Kernel readahead, `/sys/class/bdi/<dev>/read_ahead_kb` | Raises the window the kernel reads ahead of buffered (non-O_DIRECT) sequential reads. The kernel offers 128 KiB in FUSE INIT and a daemon may only lower it. At 2 MiB, one cold reader goes from 1,160 to 2,720 MiB/s (`benchmarks.md`, "Span boundary stalls"). | 128 KiB. Reads are correct; one buffered sequential reader is slower. O_DIRECT readers and readers with large requests are unaffected. |
| UDP receive buffers, `net.core.rmem_default` / `rmem_max` | The kernel's default and maximum socket receive buffer. Rustern uses the system default (it does not set `SO_RCVBUF`). | No drops were measured at the default (`/proc/net/snmp` `RcvbufErrors` 0 across the benchmark run). If drops appear, they show as retried metadata requests (higher p99), never wrong data. |
| Transparent huge pages, `/sys/kernel/mm/transparent_hugepage/enabled` | Rustern advises its read-buffer mapping `MADV_HUGEPAGE`, which takes effect when the mode is `always` or `madvise` (the common default). About 2% for one reader, 5% less CPU per GiB at 16 (`benchmarks.md`, "Huge-page read buffers"). | With `never`, buffers use 4 KiB pages; reads are correct and slightly slower. |
| `vm.drop_caches` | Benchmarks only: `scripts/bench.sh` drops the host page cache between runs so each starts cold. | `bench.sh` warms both clients' caches with an untimed read instead. Rustern itself never uses it. |

### Setting readahead per mount

The INIT reply sets the window when the mount is made, so the value must be written after
mounting:

```sh
dev=$(mountpoint -d <mountpoint>)        # e.g. 0:123
echo 2048 | sudo tee /sys/class/bdi/$dev/read_ahead_kb
```

A udev rule on `SUBSYSTEM=="bdi"` can set it host-wide, but it must delay the write past
the INIT reply (for example with `systemd-run --on-active=1`), because a write at `add`
time is undone by it. A post-mount step in the unit that starts Rustern is the
deterministic way.

## Running as root

Root needs nothing above. On shutdown with a file still open, a plain unmount fails with
EBUSY; Rustern then detaches lazily (as `fusermount3 -u -z` does), so the process exits
and no dead mount is left behind.
