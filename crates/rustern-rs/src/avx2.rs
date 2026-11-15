// Copyright 2026 Mikhail Kviatkovskii
// Ported from TernFS go/core/rs/rs_core.c (upstream 5550891) by XTX Markets Technologies
// Limited, modified; see NOTICE.
//
// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception

//! AVX2 kernel: split-nibble multiply with `vpshufb`, 32 bytes at a time (upstream
//! `gf_mul_expanded_avx2`, `rs_compute_parity_avx2`, `rs_recover_matmul_avx2`). `x · c` is
//! `lo[x & 15] ^ hi[x >> 4]`, with `lo`/`hi` the 16-entry products of `c` in both lanes.

use core::arch::x86_64::{
    __m256i, _mm256_and_si256, _mm256_set1_epi8, _mm256_setzero_si256, _mm256_shuffle_epi8,
    _mm256_srli_epi16, _mm256_xor_si256,
};

crate::matmul_kernels!(Avx2;
    width 32, vec __m256i, zero _mm256_setzero_si256(), xor _mm256_xor_si256,
    table |row| row.lo.iter().zip(&row.hi)
        .map(|(l, h)| (bytemuck::cast::<_, __m256i>(*l), bytemuck::cast::<_, __m256i>(*h)))
        .collect(),
    mul |x, c| {
        let mask = _mm256_set1_epi8(0x0f);
        let x_lo = _mm256_and_si256(x, mask);
        let x_hi = _mm256_and_si256(_mm256_srli_epi16::<4>(x), mask);
        _mm256_xor_si256(_mm256_shuffle_epi8(c.0, x_lo), _mm256_shuffle_epi8(c.1, x_hi))
    },
);
