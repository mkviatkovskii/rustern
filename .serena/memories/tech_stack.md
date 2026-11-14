# Tech stack

- Rust stable 1.100.0, edition 2024, pinned via `rust-toolchain.toml` (planned). Linux only, x86_64 primary.
- Cargo workspace, library-first, five crates (see `mem:core`). Shared deps and `[workspace.lints]` in root `Cargo.toml`.
- Go 1.27: dev-time only, for `tools/rsgen` and `tools/goldens`; `tools/go.mod` uses `replace github.com/XTXMarkets/ternfs/go => ../ternfs/go`.
- Core crates: fuser 0.18, tokio 1 (multi-thread), bytes, dashmap, moka (future; span-map cache, single-flight), backon (retry schedule as iterator only), fastrand, crc32c (incl. combine), rustix, thiserror, clap (derive), tracing, metrics + metrics-exporter-prometheus.
- Errors: thiserror everywhere; anyhow NOT used (plan R8), despite constitution allowing it in main.
- Test: cargo-nextest, proptest, cargo-fuzz (nightly only), criterion, loom (if custom sync), fio for macro benchmarks.
- Dependency policy: new crates outside the constitution's Technology Baseline need a decision record; every dependency gets a row in the plan's Dependency Justification table; cargo-deny must be clean; `Cargo.lock` committed.
- fuser's experimental async API is too narrow; project uses its own small bridge (plan R1). fuser declines FUSE_INTERRUPT (DEV-005).
- `.python-version` (3.14) exists only for spec-kit tooling, not the product.
