# Quickstart: Read-Only FUSE Client Proof of Concept

Validation guide for feature 001. Contracts: [cli.md](contracts/cli.md),
[filesystem.md](contracts/filesystem.md).

## Prerequisites

- Linux with `/dev/fuse` and `fusermount3`.
- On x86-64, a CPU with x86-64-v3 (Haswell, 2013, or later: AVX2, BMI2, FMA). `.cargo/config.toml`
  builds for it, and the binary stops with SIGILL on older CPUs.
- Rust toolchain from `rust-toolchain.toml` (installed by rustup on first `cargo` run).
- Go (for `tools/rsgen` and `tools/goldens`).
- Upstream in `ternfs/`, a submodule pinned at commit `5550891`
  (`git submodule update --init --depth 1`), built with `cd ternfs && ./build.sh alpine` (Docker).
- Gate tools: `cargo install --locked cargo-nextest cargo-deny cargo-fuzz`, and
  `rustup toolchain install nightly` (fuzzing only). `fio` for the benchmarks.
- `user_allow_other` in `/etc/fuse.conf` (only the `allow_other` mount test needs it).
- About 20 GB free disk for the cluster data directory. Do not put it on tmpfs.

## 1. Start a local cluster

```sh
scripts/cluster.sh start    # also: stop, status
```

The script runs upstream `ternrun` in the background with the data directory
`ternfs/mnt` (override with `RUSTERN_TEST_DATA_DIR`) and waits until it is operational.
For reference, the command it runs is:

```sh
ternfs/build/alpine/ternrun -leader-only -data-dir "$PWD/ternfs/mnt" \
  -binaries-dir "$PWD/ternfs/build/alpine/"
```

The reference (ternfuse) mount is `ternfs/mnt/fuse_loc1/mnt`. The registry listens on
`127.0.0.1:10001`.

## 2. Write a test corpus through ternfuse

```sh
scripts/make-corpus.sh ternfs/mnt/fuse_loc1/mnt/corpus
```

Creates ≥ 1,000 files and 100 directories (empty, inline-sized, single-span,
multi-span and 1 GiB files, symlinks, long and non-UTF-8 names) using `cp` from local
files, and writes `corpus.sha256`. Shell redirects (`> file`) currently fail with ENOTSUP
on ternfuse; the script avoids them.

## 3. Mount with Rustern

```sh
mkdir -p /tmp/rustern-mnt
cargo run --release -p rustern -- -registry 127.0.0.1:10001 /tmp/rustern-mnt
```

Expected: `mounted at /tmp/rustern-mnt` within 5 seconds.

## 4. Validate

| Check | Command | Expected |
|-------|---------|----------|
| Listing matches | `cargo nextest run -p rustern --test browse --run-ignored all` | pass: names, types, sizes and mtimes equal the ternfuse mount |
| Contents match | `cd /tmp/rustern-mnt/corpus && sha256sum -c corpus.sha256` | all OK |
| Random reads | `cargo nextest run -p rustern --test random_reads --run-ignored all` | pass |
| Writes refused | `touch /tmp/rustern-mnt/x; mkdir /tmp/rustern-mnt/d` | "Read-only file system" |
| Missing name | `stat /tmp/rustern-mnt/nope` | "No such file or directory" |
| Clean shutdown | `kill -TERM <pid>`; `ls /tmp/rustern-mnt` | empty directory, no "transport endpoint" error |
| Recovery after SIGKILL | `kill -KILL <pid>`; `fusermount3 -u /tmp/rustern-mnt` | mount point usable |
| Metrics dump | `kill -USR1 <pid>` | counters and histograms in the log |

## 5. Full gate

```sh
scripts/gate.sh          # every merge gate, starts and stops its own cluster
scripts/gate.sh --quick  # fmt, clippy, unit/property tests, corpus replay, cargo-deny
```

The gate fails, naming the tool, if any required tool is missing. It also fails if
`tools/rsgen` output differs from the committed `crates/rustern-msgs/src/generated.rs`, or if
`ternfs/` is not at the pinned commit.

## 6. Fault scenarios (run by the gate, can be run alone)

```sh
cargo nextest run -p rustern --test faults --run-ignored all
cargo nextest run -p rustern-client --test relay_faults --run-ignored all
```

Cluster tests are `#[ignore]`d so that `cargo nextest run` works without a cluster; they run
with `cargo nextest run --workspace --run-ignored all`. Fault tests (`mod cluster_serial`)
stop cluster processes and corrupt blocks, so run them on their own, after the rest, as
`scripts/gate.sh` does:

```sh
cargo nextest run --workspace --run-ignored all -E 'not test(/^cluster_serial::/)'
cargo nextest run --workspace --run-ignored all -E 'test(/^cluster_serial::/)'
```

They also share the `cluster-serial` test group (`.config/nextest.toml`), so they run one
at a time.

Covers: corrupted block (EIO, other files readable), killed block service (EIO within
the block deadline; mirrored spans still readable), stopped shard (error after the shard
deadline, recovery after SIGCONT), malformed/lost/duplicated/reordered datagrams via the
test relay, SIGKILL of Rustern followed by `fusermount3 -u`.

## 7. Benchmarks

```sh
scripts/bench.sh ternfs/mnt/fuse_loc1/mnt /tmp/rustern-mnt
```

Needs `fio`. Runs `seq-read`, `rand-read`, `meta-latency` and `readdir-latency` against
both mounts (the script mounts Rustern on the second path itself) and writes
`specs/001-fuse-client-poc/benchmarks.md`. Results are recorded, not gated (spec:
Performance Targets). Micro-benchmarks: `cargo bench -p rustern-core --bench crc --bench layout` and
`cargo bench -p rustern-msgs --bench codec`.

## 8. Soak

```sh
scripts/soak.sh 30
```

Needs `fio`. 30 minutes of mixed reads while block services and shards are stopped and
resumed; appends the result to `benchmarks.md` (SC-003).
