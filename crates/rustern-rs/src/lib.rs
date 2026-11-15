// Copyright 2026 Mikhail Kviatkovskii
// Ported from TernFS go/core/rs (rs.h, rs.cpp, rs_core.c, rs.go; upstream 5550891) by XTX
// Markets Technologies Limited, modified; see NOTICE.
//
// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception

//! Reed-Solomon coding byte-compatible with TernFS (`go/core/rs`): GF(2^8) with the
//! Rijndael polynomial, a systematic matrix whose first parity block is the XOR of the data
//! (RAID5-style) and Cauchy rows for the rest.
//!
//! Five kernels: scalar, split-nibble `vpshufb` on 256-bit (AVX2) and 512-bit (AVX-512)
//! vectors, and GFNI `vgf2p8mulb` on 256-bit and 512-bit vectors. Upstream has scalar, AVX2
//! and 256-bit GFNI. [`Level::best`] picks the fastest one the CPU supports, detected at run
//! time. The SIMD kernels are safe Rust: `fearless_simd::kernel!` turns a detected CPU token
//! into the `#[target_feature]` context the intrinsics need. The tokens are fearless_simd's
//! `Avx2` and `Avx512` (Ice Lake) plus `Alderlake` and `SkylakeAvx512` from our patch
//! (`third_party/fearless_simd.patch`), each the feature set of the GCC `-march` of that name.
//!
//! Differences from upstream: bad arguments return [`RsError`] instead of aborting, and
//! recovery requires exactly D blocks (upstream reads past its buffers with more). Upstream
//! also avoids GFNI under Valgrind; here [`Rs::compute_parity_with`] takes an explicit level.

use std::sync::OnceLock;

#[cfg(target_arch = "x86_64")]
mod avx2;
#[cfg(target_arch = "x86_64")]
mod avx2_gfni;
#[cfg(target_arch = "x86_64")]
mod avx512;
#[cfg(target_arch = "x86_64")]
mod avx512_gfni;
mod gf;
mod matrix;
mod scalar;

/// Defines one `fearless_simd::kernel!` per input count D in 2..=15 for `$token` (`kernel!`
/// functions cannot be generic, and a fixed D unrolls the per-chunk loop over the inputs),
/// plus `pub(crate) fn matmul(token, rows, ins, outs)` dispatching on `ins.len()`; any other
/// count goes to the scalar kernel. The rest of the arguments describe the vector code, see
/// [`simd_body`].
#[cfg(target_arch = "x86_64")]
macro_rules! matmul_kernels {
    ($token:ident; $($simd:tt)*) => {
        $crate::matmul_kernels!(@gen $token; [$($simd)*];
            2 matmul_2, 3 matmul_3, 4 matmul_4, 5 matmul_5, 6 matmul_6, 7 matmul_7, 8 matmul_8,
            9 matmul_9, 10 matmul_10, 11 matmul_11, 12 matmul_12, 13 matmul_13, 14 matmul_14,
            15 matmul_15);
    };
    (@gen $token:ident; $simd:tt; $($d:literal $name:ident),*) => {
        $(fearless_simd::kernel!(
            fn $name(
                t: $token,
                rows: &[$crate::Row],
                ins: &[&[u8]; $d],
                outs: &mut [&mut [u8]],
            ) {
                $crate::simd_body!($d, rows, ins, outs; $simd)
            }
        );)*

        /// `outs[r] = Σ_d rows[r].coef[d] · ins[d]`. All slices have the same length.
        pub(crate) fn matmul(
            t: fearless_simd::$token,
            rows: &[$crate::Row],
            ins: &[&[u8]],
            outs: &mut [&mut [u8]],
        ) {
            match ins.len() {
                $($d => match <&[&[u8]; $d]>::try_from(ins) {
                    Ok(ins) => $name(t, rows, ins, outs),
                    Err(_) => $crate::scalar::matmul(rows, ins, outs),
                },)*
                _ => $crate::scalar::matmul(rows, ins, outs),
            }
        }
    };
}
#[cfg(target_arch = "x86_64")]
pub(crate) use matmul_kernels;

/// The kernel body shared by all SIMD levels, for `$d` inputs of `$w`-byte vectors `$v`:
/// `table` maps a [`Row`] to its per-input coefficients in vector form (computed once per
/// call), `mul` multiplies input vector `x` by such a coefficient `c` (a reference), `xor`
/// adds. Rows whose coefficients are all 1 (parity block 0) only XOR. Per chunk, the D input
/// vectors are loaded once and reused for every output row; the tails (under `$w` bytes) go
/// to the scalar kernel. Loads and stores are `bytemuck` casts of `[u8; $w]` chunks.
#[cfg(target_arch = "x86_64")]
macro_rules! simd_body {
    ($d:literal, $rows:ident, $ins:ident, $outs:ident; [
        width $w:literal, vec $v:ty, zero $zero:expr, xor $xor:path,
        table |$row:ident| $table:expr,
        mul |$x:ident, $c:ident| $mul:expr $(,)?
    ]) => {{
        const D: usize = $d;
        let zero: $v = $zero;
        let split = $ins.map(|s| s.as_chunks::<$w>());
        let tables: Vec<Vec<_>> = $rows.iter().map(|$row| $table).collect();
        let mut out_split: Vec<(&mut [[u8; $w]], &mut [u8])> =
            $outs.iter_mut().map(|o| o.as_chunks_mut::<$w>()).collect();
        let chunks = split.first().map_or(0, |(c, _)| c.len());
        for i in 0..chunks {
            let x: [$v; D] = split
                .each_ref()
                .map(|(c, _)| c.get(i).map_or(zero, |c| bytemuck::cast(*c)));
            for ((row, table), (out, _)) in $rows.iter().zip(&tables).zip(out_split.iter_mut()) {
                let acc = if row.ones {
                    x.iter().fold(zero, |a, v| $xor(a, *v))
                } else {
                    x.iter().zip(table).fold(zero, |a, (&$x, $c)| $xor(a, $mul))
                };
                if let Some(dst) = out.get_mut(i) {
                    *dst = bytemuck::cast(acc);
                }
            }
        }
        let tail_ins = split.map(|(_, t)| t);
        let mut tail_outs: Vec<&mut [u8]> = out_split.into_iter().map(|(_, t)| t).collect();
        $crate::scalar::matmul($rows, &tail_ins, &mut tail_outs);
    }};
}
#[cfg(target_arch = "x86_64")]
pub(crate) use simd_body;

/// Kernel selection, as upstream `rs_cpu_level` plus the two AVX-512 levels.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Level {
    Scalar,
    /// x86-64 with AVX2 (the x86-64-v3 level): split-nibble `vpshufb`, 32 bytes at a time.
    Avx2,
    /// GFNI `vgf2p8mulb`, 32 bytes at a time. Needs the Alder Lake feature set.
    Avx2Gfni,
    /// AVX-512 F, BW, CD, DQ, VL: split-nibble `vpshufb`, 64 bytes at a time. Needs the
    /// Skylake-SP feature set.
    Avx512,
    /// GFNI `vgf2p8mulb`, 64 bytes at a time. Needs the Ice Lake AVX-512 feature set.
    Avx512Gfni,
}

impl Level {
    pub const ALL: [Level; 5] = [
        Level::Scalar,
        Level::Avx2,
        Level::Avx2Gfni,
        Level::Avx512,
        Level::Avx512Gfni,
    ];

    /// Whether this level can run on this CPU.
    pub fn available(self) -> bool {
        #[cfg(target_arch = "x86_64")]
        let simd = fearless_simd::Level::new();
        match self {
            Level::Scalar => true,
            #[cfg(target_arch = "x86_64")]
            Level::Avx2 => simd.as_avx2().is_some(),
            #[cfg(target_arch = "x86_64")]
            Level::Avx2Gfni => simd.as_alderlake().is_some(),
            #[cfg(target_arch = "x86_64")]
            Level::Avx512 => simd.as_skylake_avx512().is_some(),
            #[cfg(target_arch = "x86_64")]
            Level::Avx512Gfni => simd.as_avx512().is_some(),
            #[cfg(not(target_arch = "x86_64"))]
            Level::Avx2 | Level::Avx2Gfni | Level::Avx512 | Level::Avx512Gfni => false,
        }
    }

    /// The fastest available level: GFNI before split-nibble, 512-bit before 256-bit.
    pub fn best() -> Level {
        static BEST: OnceLock<Level> = OnceLock::new();
        *BEST.get_or_init(|| {
            [
                Level::Avx512Gfni,
                Level::Avx2Gfni,
                Level::Avx512,
                Level::Avx2,
            ]
            .into_iter()
            .find(|l| l.available())
            .unwrap_or(Level::Scalar)
        })
    }
}

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum RsError {
    #[error("parity {0:#04x}: Reed-Solomon needs at least 2 data blocks and 1 parity block")]
    BadParity(u8),
    #[error("expected {expected} {what} blocks, got {got}")]
    BlockCount {
        what: &'static str,
        expected: usize,
        got: usize,
    },
    #[error("block sizes differ: {got} bytes, expected {expected}")]
    BlockSize { expected: usize, got: usize },
    #[error("have mask {have:#x} must name exactly {data} of the {blocks} blocks")]
    HaveMask { have: u32, data: u8, blocks: u8 },
    #[error("want block {want} is out of range or in have mask {have:#x}")]
    WantBlock { want: u8, have: u32 },
    #[error("kernel {0:?} is not available on this build or CPU")]
    Unavailable(Level),
    #[error("recovery matrix is singular")]
    Singular,
}

/// One output row of a matrix product: `out = Σ_d coef[d] · in[d]`, with the split-nibble
/// tables of each coefficient precomputed.
pub(crate) struct Row {
    pub(crate) coef: Vec<u8>,
    pub(crate) lo: Vec<[u8; 32]>,
    pub(crate) hi: Vec<[u8; 32]>,
    /// All coefficients are 1: the row is a plain XOR (parity block 0).
    pub(crate) ones: bool,
}

impl Row {
    fn new(coef: Vec<u8>) -> Row {
        let (lo, hi) = coef.iter().map(|c| gf::expand(*c)).unzip();
        let ones = coef.iter().all(|c| *c == 1);
        Row { coef, lo, hi, ones }
    }
}

/// Encoder and decoder for one parity layout (upstream `struct rs`). Immutable; get it with
/// [`Rs::get`].
pub struct Rs {
    parity: u8,
    data: u8,
    blocks: u8,
    /// Encoding matrix columns, one per block (see `matrix::encoding`).
    cols: Vec<Vec<u8>>,
    /// The parity columns as kernel rows.
    parity_rows: Vec<Row>,
}

fn matmul(
    level: Level,
    rows: &[Row],
    ins: &[&[u8]],
    outs: &mut [&mut [u8]],
) -> Result<(), RsError> {
    #[cfg(target_arch = "x86_64")]
    let simd = fearless_simd::Level::new();
    let unavailable = RsError::Unavailable(level);
    match level {
        Level::Scalar => scalar::matmul(rows, ins, outs),
        #[cfg(target_arch = "x86_64")]
        Level::Avx2 => avx2::matmul(simd.as_avx2().ok_or(unavailable)?, rows, ins, outs),
        #[cfg(target_arch = "x86_64")]
        Level::Avx2Gfni => {
            avx2_gfni::matmul(simd.as_alderlake().ok_or(unavailable)?, rows, ins, outs)
        }
        #[cfg(target_arch = "x86_64")]
        Level::Avx512 => avx512::matmul(
            simd.as_skylake_avx512().ok_or(unavailable)?,
            rows,
            ins,
            outs,
        ),
        #[cfg(target_arch = "x86_64")]
        Level::Avx512Gfni => {
            avx512_gfni::matmul(simd.as_avx512().ok_or(unavailable)?, rows, ins, outs);
        }
        #[cfg(not(target_arch = "x86_64"))]
        Level::Avx2 | Level::Avx2Gfni | Level::Avx512 | Level::Avx512Gfni => {
            return Err(unavailable);
        }
    }
    Ok(())
}

fn check_sizes(size: usize, blocks: impl Iterator<Item = usize>) -> Result<(), RsError> {
    match blocks.into_iter().find(|len| *len != size) {
        Some(got) => Err(RsError::BlockSize {
            expected: size,
            got,
        }),
        None => Ok(()),
    }
}

fn check_count(what: &'static str, expected: u8, got: usize) -> Result<(), RsError> {
    if got == usize::from(expected) {
        Ok(())
    } else {
        Err(RsError::BlockCount {
            what,
            expected: usize::from(expected),
            got,
        })
    }
}

impl Rs {
    /// The coder for a TernFS parity byte (data blocks in the low nibble, parity blocks in
    /// the high nibble), built once per layout and cached for the life of the process.
    pub fn get(parity: u8) -> Result<&'static Rs, RsError> {
        static CACHE: [OnceLock<Rs>; 256] = [const { OnceLock::new() }; 256];
        let (data, parity_blocks) = (parity & 0x0f, parity.wrapping_shr(4));
        if data < 2 || parity_blocks < 1 {
            return Err(RsError::BadParity(parity));
        }
        let slot = CACHE
            .get(usize::from(parity))
            .ok_or(RsError::BadParity(parity))?;
        Ok(slot.get_or_init(|| {
            let cols = matrix::encoding(data, parity_blocks);
            let parity_rows = cols
                .iter()
                .skip(usize::from(data))
                .map(|c| Row::new(c.clone()))
                .collect();
            Rs {
                parity,
                data,
                blocks: data.saturating_add(parity_blocks),
                cols,
                parity_rows,
            }
        }))
    }

    pub fn parity(&self) -> u8 {
        self.parity
    }

    pub fn data_blocks(&self) -> usize {
        usize::from(self.data)
    }

    pub fn parity_blocks(&self) -> usize {
        usize::from(self.blocks.saturating_sub(self.data))
    }

    pub fn blocks(&self) -> usize {
        usize::from(self.blocks)
    }

    /// Writes all P parity blocks for the D data blocks (upstream `ComputeParityInto`).
    /// Every block has the same length.
    pub fn compute_parity(&self, data: &[&[u8]], parity: &mut [&mut [u8]]) -> Result<(), RsError> {
        self.compute_parity_with(Level::best(), data, parity)
    }

    /// [`Rs::compute_parity`] with a given kernel.
    pub fn compute_parity_with(
        &self,
        level: Level,
        data: &[&[u8]],
        parity: &mut [&mut [u8]],
    ) -> Result<(), RsError> {
        check_count("data", self.data, data.len())?;
        check_count(
            "parity",
            self.blocks.saturating_sub(self.data),
            parity.len(),
        )?;
        let size = data.first().map_or(0, |d| d.len());
        check_sizes(
            size,
            data.iter()
                .map(|d| d.len())
                .chain(parity.iter().map(|p| p.len())),
        )?;
        matmul(level, &self.parity_rows, data, parity)
    }

    /// Computes block `want_block` into `out` from the D blocks named by the bitmask
    /// `have_blocks` (bit i = block i, data blocks first), given in `have` in increasing
    /// block order (upstream `RecoverInto`).
    pub fn recover(
        &self,
        have_blocks: u32,
        have: &[&[u8]],
        want_block: u8,
        out: &mut [u8],
    ) -> Result<(), RsError> {
        self.recover_with(Level::best(), have_blocks, have, want_block, out)
    }

    /// [`Rs::recover`] with a given kernel.
    pub fn recover_with(
        &self,
        level: Level,
        have_blocks: u32,
        have: &[&[u8]],
        want_block: u8,
        out: &mut [u8],
    ) -> Result<(), RsError> {
        let mask_err = RsError::HaveMask {
            have: have_blocks,
            data: self.data,
            blocks: self.blocks,
        };
        if have_blocks.checked_shr(u32::from(self.blocks)).unwrap_or(0) != 0
            || have_blocks.count_ones() != u32::from(self.data)
        {
            return Err(mask_err);
        }
        let want_bit = 1u32.checked_shl(u32::from(want_block)).unwrap_or(0);
        if want_block >= self.blocks || have_blocks & want_bit != 0 {
            return Err(RsError::WantBlock {
                want: want_block,
                have: have_blocks,
            });
        }
        check_count("have", self.data, have.len())?;
        let size = have.first().map_or(0, |h| h.len());
        check_sizes(size, have.iter().map(|h| h.len()).chain([out.len()]))?;
        let have_ix: Vec<usize> = (0..self.blocks)
            .filter(|b| have_blocks & 1u32.checked_shl(u32::from(*b)).unwrap_or(0) != 0)
            .map(usize::from)
            .collect();
        let coef = matrix::recovery(&self.cols, &have_ix, usize::from(want_block))
            .ok_or(RsError::Singular)?;
        matmul(level, &[Row::new(coef)], have, &mut [out])
    }
}
