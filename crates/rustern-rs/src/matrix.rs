// Copyright 2026 Mikhail Kviatkovskii
// Ported from TernFS go/core/rs/rs_core.c (upstream 5550891) by XTX Markets Technologies
// Limited, modified; see NOTICE.
//
// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception

//! The encoding matrix and the recovery coefficients (`rs_cauchy_matrix`,
//! `rs_gf_invert_matrix` and `rs_recover_mat` in upstream `rs_core.c`).
//!
//! Upstream stores the B×D matrix column-major in one buffer: column `b` maps the D data
//! blocks to block `b`. Here each column is its own `Vec`, which keeps the index arithmetic
//! out of the code; the values are identical.

use crate::gf;

/// Columns of the systematic encoding matrix: identity for the D data blocks, then the
/// Cauchy columns for the P parity blocks, scaled so that every entry of the first parity
/// column and of the first data row is 1. Parity block 0 is therefore the XOR of the data,
/// which the kernels rely on.
pub(crate) fn encoding(d: u8, p: u8) -> Vec<Vec<u8>> {
    let b = d.saturating_add(p);
    let mut cols: Vec<Vec<u8>> = (0..b)
        .map(|col| {
            (0..d)
                .map(|row| {
                    if col < d {
                        u8::from(col == row)
                    } else {
                        gf::inv(col ^ row)
                    }
                })
                .collect()
        })
        .collect();
    let parity = cols.get_mut(usize::from(d)..).unwrap_or_default();
    // Scale each parity column by the inverse of its first entry.
    for col in parity.iter_mut() {
        let factor = gf::inv(col.first().copied().unwrap_or(0));
        col.iter_mut().for_each(|v| *v = gf::mul(*v, factor));
    }
    // Scale rows 1.. by the inverse of their entry in the first parity column. The factors
    // are taken before any row is scaled, as upstream computes each row's factor first.
    let factors: Vec<u8> = parity
        .first()
        .map(|first| first.iter().map(|v| gf::inv(*v)).collect())
        .unwrap_or_default();
    for col in parity.iter_mut() {
        for (v, f) in col.iter_mut().zip(&factors).skip(1) {
            *v = gf::mul(*v, *f);
        }
    }
    cols
}

/// Inverse of a square matrix given as rows, by Gauss-Jordan elimination with row swaps
/// (upstream `rs_gf_invert_matrix`, from ISA-L `ec_base.c`). `None` if singular.
pub(crate) fn invert(mut a: Vec<Vec<u8>>) -> Option<Vec<Vec<u8>>> {
    let n = a.len();
    let mut out: Vec<Vec<u8>> = (0..n)
        .map(|i| (0..n).map(|j| u8::from(i == j)).collect())
        .collect();
    for i in 0..n {
        let pivot_row =
            (i..n).find(|&j| a.get(j).and_then(|r| r.get(i)).is_some_and(|v| *v != 0))?;
        a.swap(i, pivot_row);
        out.swap(i, pivot_row);
        let factor = gf::inv(*a.get(i)?.get(i)?);
        let a_i: Vec<u8> = a.get(i)?.iter().map(|v| gf::mul(*v, factor)).collect();
        let out_i: Vec<u8> = out.get(i)?.iter().map(|v| gf::mul(*v, factor)).collect();
        for (j, (a_j, out_j)) in a.iter_mut().zip(out.iter_mut()).enumerate() {
            if j == i {
                a_j.clone_from(&a_i);
                out_j.clone_from(&out_i);
                continue;
            }
            let t = a_j.get(i).copied().unwrap_or(0);
            for (v, p) in a_j.iter_mut().zip(&a_i) {
                *v ^= gf::mul(t, *p);
            }
            for (v, p) in out_j.iter_mut().zip(&out_i) {
                *v ^= gf::mul(t, *p);
            }
        }
    }
    Some(out)
}

/// Coefficients that compute block `want` from the D blocks in `have` (block indices in
/// increasing order): `want = Σ coef[k] · have[k]`. `None` if the blocks are not independent,
/// which cannot happen for D distinct blocks of a Cauchy-derived matrix.
pub(crate) fn recovery(cols: &[Vec<u8>], have: &[usize], want: usize) -> Option<Vec<u8>> {
    // Row k maps the data blocks to have[k].
    let data_to_have: Vec<Vec<u8>> = have
        .iter()
        .map(|&b| cols.get(b).cloned())
        .collect::<Option<_>>()?;
    let have_to_data = invert(data_to_have)?;
    let data_to_want = cols.get(want)?;
    let mut coef = vec![0u8; have.len()];
    for (w, row) in data_to_want.iter().zip(&have_to_data) {
        for (c, h) in coef.iter_mut().zip(row) {
            *c ^= gf::mul(*w, *h);
        }
    }
    Some(coef)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_parity_column_and_first_row_are_ones() {
        for d in 2..=15u8 {
            for p in 1..=15u8 {
                let cols = encoding(d, p);
                assert_eq!(cols.len(), usize::from(d + p));
                assert!(
                    cols.get(usize::from(d))
                        .is_some_and(|c| c.iter().all(|v| *v == 1))
                );
                assert!(
                    cols.iter()
                        .skip(usize::from(d))
                        .all(|c| c.first() == Some(&1))
                );
            }
        }
    }

    #[test]
    fn invert_identity_and_singular() {
        let id = vec![vec![1, 0], vec![0, 1]];
        assert_eq!(invert(id.clone()), Some(id));
        assert_eq!(invert(vec![vec![1, 1], vec![1, 1]]), None);
        // Needs a row swap: the first pivot is 0.
        let a = vec![vec![0, 1], vec![1, 0]];
        assert_eq!(invert(a.clone()), Some(a));
    }
}
