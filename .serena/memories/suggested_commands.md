# Suggested commands

## Build/test (Rust)
- `cargo build`, `cargo nextest run`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt`.
- Gate: `scripts/gate.sh` (full) / `scripts/gate.sh --quick` (planned; see `mem:task_completion`).
- Fuzzing needs nightly: `cargo +nightly fuzz run <target>` in `crates/rustern-msgs/fuzz`.
- One-time tools: `cargo install --locked cargo-nextest cargo-deny cargo-fuzz`; `rustup toolchain install nightly`.

## Local TernFS cluster (from specs/001-fuse-client-poc/quickstart.md)
- Build upstream (Docker): `cd ternfs && ./build.sh alpine`.
- Run: `cd ternfs && ./build/alpine/ternrun -leader-only -data-dir "$PWD/mnt" -binaries-dir "$PWD/build/alpine/"`; wait for `operational, mounted at .../fuse_loc1/mnt` (that is the ternfuse reference mount). Registry at `127.0.0.1:10001`.
- Data dir needs ~20 GB, must not be on tmpfs.
- Corpus: `scripts/make-corpus.sh ternfs/mnt/fuse_loc1/mnt/corpus`; compare trees with `scripts/diff-trees.sh`; benchmarks `scripts/bench.sh` (fio).
- Unmount stuck FUSE mounts: `fusermount3 -u <mountpoint>`.

## Environment
- Linux under WSL2: needs `/dev/fuse` and `fusermount3`.
