// Copyright 2026 Mikhail Kviatkovskii
// Ported from TernFS go/core/rs/rs_core.c (upstream 5550891) by XTX Markets Technologies
// Limited, modified; see NOTICE.
//
// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception

//! Portable kernel: split-nibble table lookups, one byte at a time (upstream
//! `rs_compute_parity_scalar` and `rs_recover_matmul_scalar`). Also handles the tails shorter
//! than a vector for the SIMD kernels, and is the reference they are tested against.

use crate::Row;

fn nibble(table: &[u8; 32], i: u8) -> u8 {
    table.get(usize::from(i & 0x0f)).copied().unwrap_or(0)
}

/// `outs[r] = Σ_d rows[r].coef[d] · ins[d]`, byte by byte. All slices have the same length.
pub(crate) fn matmul(rows: &[Row], ins: &[&[u8]], outs: &mut [&mut [u8]]) {
    for (row, out) in rows.iter().zip(outs.iter_mut()) {
        out.fill(0);
        for (input, (lo, hi)) in ins.iter().zip(row.lo.iter().zip(&row.hi)) {
            if row.ones {
                out.iter_mut().zip(input.iter()).for_each(|(o, x)| *o ^= x);
            } else {
                for (o, x) in out.iter_mut().zip(input.iter()) {
                    *o ^= nibble(lo, *x) ^ nibble(hi, x.wrapping_shr(4));
                }
            }
        }
    }
}
