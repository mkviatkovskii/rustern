// Copyright 2026 Mikhail Kviatkovskii
// Ported from TernFS go/core/rs/rs_core.c (upstream 5550891) by XTX Markets Technologies
// Limited, modified; see NOTICE.
//
// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception

//! AVX-512 kernel (x86-64-v4: F, BW, CD, DQ, VL; no GFNI): the AVX2 split-nibble multiply
//! on 512-bit vectors, 64 bytes at a time. Not in upstream. `vpshufb` looks up within each
//! 128-bit lane, so the 32-byte `lo`/`hi` tables (already two lanes) are repeated twice.

use core::arch::x86_64::{
    __m512i, _mm512_and_si512, _mm512_set1_epi8, _mm512_setzero_si512, _mm512_shuffle_epi8,
    _mm512_srli_epi16, _mm512_xor_si512,
};

crate::matmul_kernels!(SkylakeAvx512;
    width 64, vec __m512i, zero _mm512_setzero_si512(), xor _mm512_xor_si512,
    table |row| row.lo.iter().zip(&row.hi)
        .map(|(l, h)| (bytemuck::cast::<_, __m512i>([*l, *l]), bytemuck::cast::<_, __m512i>([*h, *h])))
        .collect(),
    mul |x, c| {
        let mask = _mm512_set1_epi8(0x0f);
        let x_lo = _mm512_and_si512(x, mask);
        let x_hi = _mm512_and_si512(_mm512_srli_epi16::<4>(x), mask);
        _mm512_xor_si512(_mm512_shuffle_epi8(c.0, x_lo), _mm512_shuffle_epi8(c.1, x_hi))
    },
);
