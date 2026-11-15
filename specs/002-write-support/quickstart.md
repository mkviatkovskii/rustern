# Quickstart: Write Support

Validation guide for feature 002. Contracts: [cli.md](contracts/cli.md),
[filesystem.md](contracts/filesystem.md). The lifecycle and errnos are in
[data-model.md](data-model.md).

## Prerequisites

As [001 quickstart](../001-fuse-client-poc/quickstart.md), plus:

- `pjdfstest` built by `scripts/posix.sh build` (Rust rewrite, built outside the workspace).
- `terntests` built from `ternfs/go/terntests` by `scripts/gate.sh` (Go).
- `fio`, `rsync`, `tar` and GNU coreutils for the tool script.
- About 40 GB free in the cluster data directory. The 4 GiB corpus files are written twice
  (once per client) and kept until the run ends.

## 1. Start the cluster and create policy directories

```sh
scripts/cluster.sh start
scripts/cluster.sh policies      # new: creates /mirror (Parity(1,1)), /rs (Parity(4,2)),
                                 # /inline (inline-only span policy) on the cluster via terncli set-dir-info
```

## 2. Mount Rustern read-write

```sh
mkdir -p /tmp/rustern-mnt
cargo run --release -p rustern -- -registry 127.0.0.1:10001 /tmp/rustern-mnt
# with the opt-in options:
cargo run --release -p rustern -- -registry 127.0.0.1:10001 --overwrite --accept-matching-attrs /tmp/rustern-mnt
```

Expected: `mounted at /tmp/rustern-mnt` within 5 seconds. The ternfuse mount is
`ternfs/mnt/fuse_loc1/mnt`.

## 3. Validate

| Check | Command | Expected |
|---|---|---|
| Write and read back (US1-1) | `head -c 3G /dev/urandom > /tmp/r; cp /tmp/r /tmp/rustern-mnt/rs/a; sha256sum /tmp/r ternfs/mnt/fuse_loc1/mnt/rs/a` | equal sums |
| Redirect (US1-3, DEV-007) | `seq 1 1000000 > /tmp/rustern-mnt/mirror/s; wc -l ternfs/mnt/fuse_loc1/mnt/mirror/s` | `1000000`; on ternfuse the same command fails with EOPNOTSUPP |
| Invisible until published (US1-2) | `exec 3>/tmp/rustern-mnt/rs/p; echo x >&3; ls ternfs/mnt/fuse_loc1/mnt/rs/p; exec 3>&-; ls -l /tmp/rustern-mnt/rs/p` | first `ls` fails with ENOENT; second shows size 2 |
| Gap and truncate (US1-4, US1-5) | `cargo nextest run -p rustern --test write_lifecycle` | pass |
| Default refuses overwrite (US1-7, DEV-009) | `echo y > /tmp/rustern-mnt/rs/a` | `Operation not permitted`, content unchanged |
| Overwrite option (US1-8, DEV-010) | with `--overwrite`: `cp /etc/hosts /tmp/rustern-mnt/rs/a; cmp /etc/hosts ternfs/mnt/fuse_loc1/mnt/rs/a` | identical |
| Namespace script (US2) | `cargo nextest run -p rustern --test namespace_differential` | trees, types, sizes and symlink targets equal on both clients |
| Tool script (US3, SC-003) | `scripts/tools.sh /tmp/rustern-mnt ternfs/mnt/fuse_loc1/mnt` | same trees; `cp -p`, `rsync -a` fail with EOPNOTSUPP by default and succeed with `--accept-matching-attrs` |
| O_TMPFILE (US3-4) | `cargo nextest run -p rustern --test tmpfile` | pass, or EOPNOTSUPP rows while CT-8 is open |
| Write corpus differential (SC-001) | `scripts/gate.sh differential-write` | every corpus file equal through Rustern, ternfuse and (if loaded) the kernel module, both directions |
| Fault suite (SC-002) | `scripts/gate.sh faults-write` | no published file with wrong content; no visible name for any failed file |
| terntests (SC-006) | `scripts/gate.sh terntests` | pass except the exclusions in `docs/coverage.md` |
| POSIX subset | `scripts/posix.sh run /tmp/rustern-mnt` | pass except documented exclusions |
| Read-only mount (FR-018) | `rustern ... -o ro ...` then `cargo nextest run -p rustern --test refuses_mutations` | pass (001 behaviour) |
| Benchmarks (SC-004) | `scripts/bench.sh write` | the Performance Targets table filled in `benchmarks.md`, both clients |
| Soak (SC-005) | `scripts/soak.sh --write 30m` | mount alive, no wrong byte, RSS within limits plus margin |

## 4. Observe

- `kill -USR1 <pid>`: the dump includes `rustern_span_store_seconds`,
  `rustern_publish_seconds` and `rustern_publish_failures_total`.
- `-verbose` logs each mutation's errno with op, inode and cause. Publish failures at
  release are logged at warn even without `-verbose`.

## 5. Clean up

```sh
fusermount3 -u /tmp/rustern-mnt
scripts/cluster.sh stop
```
