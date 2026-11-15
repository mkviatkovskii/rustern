# Vendored crates

## fearless_simd 1.2.0 + `fearless_simd.patch`

`fearless_simd/` is the crates.io release 1.2.0 (Apache-2.0 OR MIT, linebender), unpacked
unchanged, with `fearless_simd.patch` applied on top. The root `Cargo.toml` points
`[patch.crates-io]` at it, so a plain `cargo build` works offline and the change can be
reviewed in place. Most of the 5 MB is the crate's generated SIMD code, which is untouched.

The patch (three files: new `src/x86_ext.rs`, plus `src/kernel_macros.rs` and `src/lib.rs`)
adds two kernel-only tokens for `fearless_simd::kernel!`, so `rustern-rs` can run its SIMD
kernels with no `unsafe` of its own. Each is named after, and has the feature set of, the GCC
`-march` / rustc `-C target-cpu` of that name (pruned of implied features, like upstream's
`Avx512`, which is `icelake-server`):

| Token | `-march` | Adds to x86-64-v3 | Typical CPUs |
|---|---|---|---|
| `Alderlake` | `alderlake` (minus Key Locker) | GFNI, VAES, VPCLMULQDQ, AVX-VNNI, SHA | Alder/Raptor/Meteor/Arrow/Lunar Lake |
| `SkylakeAvx512` | `skylake-avx512` | AVX-512 F, BW, CD, DQ, VL | Skylake-SP, Cascade/Cooper Lake, every later AVX-512 CPU |

Upstream's own `Avx512` token needs the whole Ice Lake feature set, so Skylake-SP class
servers and the AVX2-only consumer cores with GFNI fall back to `Avx2` without the patch.
`rustern-rs` uses `Alderlake` for its 256-bit GFNI kernel, `SkylakeAvx512` for its 512-bit
split-nibble kernel and upstream's `Avx512` for its 512-bit GFNI kernel. The tokens come from
run-time detection (`Level::as_alderlake`, `Level::as_skylake_avx512`; the latter is free on an
`Avx512` level); the patch's only `unsafe` is inside fearless_simd, next to that detection.

Drop the directory and the `[patch.crates-io]` entry once upstream ships equivalent tokens.

To check or regenerate (from the repository root):

```sh
cp -r ~/.cargo/registry/src/index.crates.io-*/fearless_simd-1.2.0 /tmp/fs && rm -f /tmp/fs/.cargo*
patch -d /tmp/fs -p1 < third_party/fearless_simd.patch
diff -r /tmp/fs third_party/fearless_simd   # no output: the copy is 1.2.0 + the patch
(cd third_party/fearless_simd && cargo test --lib)
```
