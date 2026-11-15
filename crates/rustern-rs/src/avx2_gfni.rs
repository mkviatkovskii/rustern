// Copyright 2026 Mikhail Kviatkovskii
// Ported from TernFS go/core/rs/rs_core.c and rs.cpp (upstream 5550891) by XTX Markets
// Technologies Limited, modified; see NOTICE.
//
// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception

//! GFNI kernel on 256-bit vectors: `vgf2p8mulb`, 32 bytes at a time (upstream
//! `rs_compute_parity_gfni` and `rs_recover_matmul_gfni`). GFNI uses the same polynomial
//! (0x11B) as TernFS, so one instruction multiplies 32 bytes by 32 coefficients.

use core::arch::x86_64::{__m256i, _mm256_gf2p8mul_epi8, _mm256_setzero_si256, _mm256_xor_si256};

crate::matmul_kernels!(Alderlake;
    width 32, vec __m256i, zero _mm256_setzero_si256(), xor _mm256_xor_si256,
    table |row| row.coef.iter().map(|c| bytemuck::cast::<_, __m256i>([*c; 32])).collect(),
    mul |x, c| _mm256_gf2p8mul_epi8(x, *c),
);
