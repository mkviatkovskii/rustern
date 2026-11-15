// Copyright 2026 Mikhail Kviatkovskii
// Ported from TernFS go/core/rs and cpp/rs/gf_tables.py (upstream 5550891) by XTX Markets
// Technologies Limited, modified; see NOTICE.
//
// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception

//! GF(2^8) with the Rijndael polynomial x^8 + x^4 + x^3 + x + 1 (0x11B) and generator 3,
//! as `cpp/rs/gf_tables.py`. The tables are computed at compile time instead of being checked
//! in; the goldens and the unit tests below pin them to upstream `gf_tables.c`.

const POLY: u16 = 0x11B;
const GENERATOR: u8 = 0x03;

/// Carry-less multiply then reduce, as `gf_mul` in `gf_tables.py`. Only used to build tables.
#[expect(
    clippy::arithmetic_side_effects,
    reason = "const-evaluated: an overflow fails the build, it cannot happen at run time"
)]
const fn mul_slow(x: u8, y: u8) -> u8 {
    let mut out: u16 = 0;
    let mut i = 0;
    while i < 8 {
        if (y >> i) & 1 != 0 {
            out ^= (x as u16) << i;
        }
        i += 1;
    }
    let mut i = 14;
    while i > 7 {
        if (out >> i) & 1 != 0 {
            out ^= POLY << (i - 8);
        }
        i -= 1;
    }
    out as u8
}

struct Tables {
    exp: [u8; 256],
    log: [u8; 256],
    inv: [u8; 256],
}

#[expect(
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing,
    reason = "const-evaluated: an overflow or out-of-bounds index fails the build, it cannot happen at run time"
)]
const fn tables() -> Tables {
    let mut exp = [0u8; 256];
    let mut x = 1u8;
    let mut i = 0;
    while i < 256 {
        exp[i] = x;
        x = mul_slow(x, GENERATOR);
        i += 1;
    }
    // As upstream: exp[255] == 1 overwrites log[1] with 255, and log[0] stays 0.
    let mut log = [0u8; 256];
    let mut i = 0;
    while i < 256 {
        log[exp[i] as usize] = i as u8;
        i += 1;
    }
    let mut inv = [0u8; 256];
    let mut i = 1;
    while i < 256 {
        inv[i] = exp[255 - log[i] as usize];
        i += 1;
    }
    Tables { exp, log, inv }
}

const TABLES: Tables = tables();

fn at(table: &[u8; 256], i: usize) -> u8 {
    table.get(i).copied().unwrap_or(0)
}

/// Multiplicative inverse; `inv(0) == 0` as upstream.
pub(crate) fn inv(x: u8) -> u8 {
    at(&TABLES.inv, usize::from(x))
}

/// Product in GF(2^8).
pub(crate) fn mul(x: u8, y: u8) -> u8 {
    if x == 0 || y == 0 {
        return 0;
    }
    let i = usize::from(at(&TABLES.log, usize::from(x)))
        .wrapping_add(usize::from(at(&TABLES.log, usize::from(y))));
    at(&TABLES.exp, if i > 254 { i.wrapping_sub(255) } else { i })
}

/// Split-nibble tables for multiplying by `c`, as upstream `gf_mul_expand_factor`: `lo[i] =
/// i·c` and `hi[i] = (i << 4)·c` for i < 16, each repeated in both 128-bit lanes for `vpshufb`.
/// Then `x·c == lo[x & 15] ^ hi[x >> 4]`.
pub(crate) fn expand(c: u8) -> ([u8; 32], [u8; 32]) {
    let mut lo = [0u8; 32];
    let mut hi = [0u8; 32];
    for (i, (l, h)) in (0u8..16).cycle().zip(lo.iter_mut().zip(hi.iter_mut())) {
        *l = mul(i, c);
        *h = mul(i.wrapping_shl(4), c);
    }
    (lo, hi)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tables_match_upstream_gf_tables_c() {
        // First and last rows of each table in go/core/rs/gf_tables.c.
        assert_eq!(
            TABLES.exp.get(..8),
            Some(&[0x01, 0x03, 0x05, 0x0f, 0x11, 0x33, 0x55, 0xff][..])
        );
        assert_eq!(
            TABLES.exp.get(248..),
            Some(&[0x1c, 0x24, 0x6c, 0xb4, 0xc7, 0x52, 0xf6, 0x01][..])
        );
        assert_eq!(
            TABLES.log.get(..8),
            Some(&[0x00, 0xff, 0x19, 0x01, 0x32, 0x02, 0x1a, 0xc6][..])
        );
        assert_eq!(
            TABLES.inv.get(..8),
            Some(&[0x00, 0x01, 0x8d, 0xf6, 0xcb, 0x52, 0x7b, 0xd1][..])
        );
    }

    #[test]
    fn mul_matches_slow_multiply_and_inverse() {
        for x in 0..=255u8 {
            for y in 0..=255u8 {
                assert_eq!(mul(x, y), mul_slow(x, y), "{x} * {y}");
            }
            if x != 0 {
                assert_eq!(mul(x, inv(x)), 1, "inv({x})");
            }
            for c in [0u8, 1, 2, 0x8d, 0xff] {
                let (lo, hi) = expand(c);
                let i = usize::from(x & 15);
                let j = usize::from(x >> 4);
                assert_eq!(
                    lo.get(i).zip(hi.get(j)).map(|(l, h)| l ^ h),
                    Some(mul(x, c))
                );
                assert_eq!(lo.get(i.wrapping_add(16)), lo.get(i));
            }
        }
    }
}
