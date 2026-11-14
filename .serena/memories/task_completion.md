# Task completion

Once `scripts/gate.sh` exists, it is the single source of truth: run `scripts/gate.sh --quick` for ordinary changes, full `scripts/gate.sh` before merge (cluster suites need a running local cluster, see `mem:suggested_commands`).

Until then, run:
- `cargo fmt --check`
- `cargo clippy --workspace --all-targets -- -D warnings` (panic-family lints denied outside tests via `[workspace.lints]`/`clippy.toml`)
- `cargo nextest run --workspace`
- `cargo deny check` once `deny.toml` exists

Also: new dependency -> add Dependency Justification row in the active plan; derived-from-upstream file -> SPDX header + `NOTICE` entry (`mem:conventions`); ternfuse deviation -> `docs/deviations.md` entry with test. Benchmarks are recorded, not merge-blocking (CT-4).
