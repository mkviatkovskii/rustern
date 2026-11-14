# Rustern

A FUSE client for [TernFS](https://github.com/XTXMarkets/ternfs), written in Rust. It
talks to the TernFS registry, shards and block services directly over the TernFS wire
protocol, and serves the filesystem through the kernel's FUSE interface.

## Status

Proof of concept, **read-only**. Browsing, `stat`, symlinks and verified reads of
inline, mirrored and erasure-coded files work. Every mutating operation fails with EROFS.
A missing or corrupt block in an erasure-coded span fails the read with EIO (no parity
reconstruction yet). Differences from the reference client (`ternfuse`) are listed in
[docs/deviations.md](docs/deviations.md); planned work is in
[docs/future-features.md](docs/future-features.md).

## Layout

| Crate | Role |
|-------|------|
| `rustern-msgs` | Wire types and codecs, generated from upstream `go/msgs` by `tools/rsgen` |
| `rustern-core` | CRC, span layout and other pure logic |
| `rustern-client` | Async client: registry, shards, block services, file reads |
| `rustern-fuse` | FUSE bridge (the only crate that depends on `fuser`) |
| `rustern` | The `rustern` binary |

## Getting started

Follow [specs/001-fuse-client-poc/quickstart.md](specs/001-fuse-client-poc/quickstart.md):
start a local cluster, write the test corpus, mount with

```sh
cargo run --release -p rustern -- -registry 127.0.0.1:10001 /tmp/rustern-mnt
```

and run the gates with `scripts/gate.sh`.

## Licence

Licensed under either of [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your
option. Parts derived from TernFS are under Apache-2.0 WITH LLVM-exception; they are
listed in [NOTICE](NOTICE).
