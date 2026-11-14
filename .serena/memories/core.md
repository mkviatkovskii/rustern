# Rustern core

Rust FUSE client for TernFS (XTXMarkets). Governing doc: `.specify/memory/constitution.md` (supersedes all other conventions).

## State (2026-11-14)
- Pre-implementation: only `src/main.rs` (hello world). First feature = `specs/001-fuse-client-poc/` (read-only FUSE PoC); `plan.md` there defines the target layout, `tasks.md` the work order.
- Planned: `src/main.rs` moves into `crates/rustern`.

## Source map (planned, per plan.md)
- `crates/rustern-msgs` wire codecs; `src/generated.rs` is generator output (committed, never hand-edit), `wire.rs` hand-written primitives.
- `crates/rustern-core` InodeId, span layout arithmetic, CRC helpers.
- `crates/rustern-client` async TernFS client; NO FUSE types.
- `crates/rustern-fuse` fuser adapter (bridge fuser Filesystem -> AsyncFilesystem, errno, TTLs); NO protocol logic.
- `crates/rustern` binary: CLI, signals, logging, metrics.
- `tools/rsgen` (Go, adapted from upstream `go/bincodegen`), `tools/goldens` (Go golden vectors -> `testdata/goldens/*.hex`).
- `scripts/gate.sh` = single merge gate. `docs/deviations.md` records every deviation from ternfuse (DEV-xxx, each with a test); `docs/future-features.md` deferred ideas.
- `ternfs/` = gitignored upstream checkout, pinned at commit `5550891`; reference only, not part of the build except via `tools/go.mod` replace.

## Invariants
- Request paths never panic (no unwrap/expect/indexing; lint-enforced). `unsafe_code = "forbid"`.
- Every byte returned to kernel CRC32C-verified; failed stat never served as placeholder.
- Wire formats bit-exact with upstream at pin; proven by golden vectors.
- PoC is read-only: every mutation -> EROFS, no mutating request ever sent.
- Allocations from network input bounded by received length.

## Further memories
- Toolchain, crate picks, dependency policy: `mem:tech_stack`
- Licensing/derivation rules, error and spec-kit workflow conventions: `mem:conventions`
- Running a local TernFS cluster, build/test commands: `mem:suggested_commands`
- Merge-gate commands to run before declaring a task done: `mem:task_completion`
