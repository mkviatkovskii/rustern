// Copyright 2026 Mikhail Kviatkovskii
// Ported from TernFS go/core/rs/rs_core.c and rs.cpp (upstream 5550891) by XTX Markets
// Technologies Limited, modified; see NOTICE.
//
// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception

//! GFNI kernel on 512-bit vectors (x86-64-v4 plus GFNI): `vgf2p8mulb`, 64 bytes at a time.
//! Not in upstream, which stops at 256-bit GFNI.

use core::arch::x86_64::{__m512i, _mm512_gf2p8mul_epi8, _mm512_setzero_si512, _mm512_xor_si512};

crate::matmul_kernels!(Avx512;
    width 64, vec __m512i, zero _mm512_setzero_si512(), xor _mm512_xor_si512,
    table |row| row.coef.iter().map(|c| bytemuck::cast::<_, __m512i>([*c; 64])).collect(),
    mul |x, c| _mm512_gf2p8mul_epi8(x, *c),
);
