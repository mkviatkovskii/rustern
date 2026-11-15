// Copyright 2026 Mikhail Kviatkovskii
//
// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception

//! Properties over random layouts, sizes and erasures: all kernels agree, parity 0 is the XOR
//! of the data, any D of the B blocks recover every other block, and bad arguments are
//! errors rather than panics.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
#![allow(clippy::arithmetic_side_effects)]

use proptest::prelude::*;
use rustern_rs::{Level, Rs, RsError};

fn layout() -> impl Strategy<Value = (u8, u8, usize, Vec<u8>, u64)> {
    (2u8..=15, 1u8..=15, 0usize..300).prop_flat_map(|(d, p, size)| {
        (
            Just(d),
            Just(p),
            Just(size),
            proptest::collection::vec(any::<u8>(), usize::from(d) * size),
            any::<u64>(),
        )
    })
}

fn encode(rs: &Rs, level: Level, data: &[&[u8]], size: usize) -> Vec<Vec<u8>> {
    let mut parity = vec![vec![0u8; size]; rs.parity_blocks()];
    let mut refs: Vec<&mut [u8]> = parity.iter_mut().map(|v| v.as_mut_slice()).collect();
    rs.compute_parity_with(level, data, &mut refs).unwrap();
    parity
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    #[test]
    fn kernels_agree_and_any_d_blocks_recover((d, p, size, bytes, seed) in layout()) {
        let rs = Rs::get(p << 4 | d).unwrap();
        let data: Vec<&[u8]> = if size == 0 { vec![&[][..]; d.into()] } else { bytes.chunks(size).collect() };
        let parity = encode(rs, Level::Scalar, &data, size);
        for level in Level::ALL.into_iter().filter(|l| l.available()) {
            prop_assert_eq!(&encode(rs, level, &data, size), &parity, "{:?}", level);
        }
        let mut xor = vec![0u8; size];
        for block in &data {
            xor.iter_mut().zip(block.iter()).for_each(|(x, b)| *x ^= b);
        }
        prop_assert_eq!(&parity[0], &xor);

        // Pick D surviving blocks from a seeded shuffle and recover each missing one.
        let all: Vec<&[u8]> = data.iter().copied().chain(parity.iter().map(|v| v.as_slice())).collect();
        let b = all.len();
        let mut order: Vec<usize> = (0..b).collect();
        let mut s = seed;
        for i in (1..b).rev() {
            s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            order.swap(i, (s >> 33) as usize % (i + 1));
        }
        let mut have_ix: Vec<usize> = order[..d.into()].to_vec();
        have_ix.sort_unstable();
        let mask = have_ix.iter().fold(0u32, |m, i| m | 1 << i);
        let have: Vec<&[u8]> = have_ix.iter().map(|i| all[*i]).collect();
        for want in order[usize::from(d)..].iter() {
            for level in Level::ALL.into_iter().filter(|l| l.available()) {
                let mut out = vec![0xffu8; size];
                rs.recover_with(level, mask, &have, *want as u8, &mut out).unwrap();
                prop_assert_eq!(out.as_slice(), all[*want], "block {} with {:?}", want, level);
            }
        }
    }
}

#[test]
fn bad_arguments_are_errors() {
    assert_eq!(Rs::get(0x11).err(), Some(RsError::BadParity(0x11)));
    assert_eq!(Rs::get(0x02).err(), Some(RsError::BadParity(0x02)));
    let rs = Rs::get(0x24).unwrap(); // D=4, P=2
    let a = [0u8; 8];
    let short = [0u8; 7];
    let mut p0 = [0u8; 8];
    let mut p1 = [0u8; 8];
    assert!(matches!(
        rs.compute_parity(&[&a, &a, &a], &mut [&mut p0, &mut p1]),
        Err(RsError::BlockCount { .. })
    ));
    assert!(matches!(
        rs.compute_parity(&[&a, &a, &a, &short], &mut [&mut p0, &mut p1]),
        Err(RsError::BlockSize { .. })
    ));
    assert!(matches!(
        rs.compute_parity(&[&a[..]; 4], &mut [&mut p0]),
        Err(RsError::BlockCount { .. })
    ));
    let mut out = [0u8; 8];
    let have = [&a[..]; 4];
    // Too few, too many, and out-of-range blocks in the mask.
    assert!(matches!(
        rs.recover(0b0111, &have, 5, &mut out),
        Err(RsError::HaveMask { .. })
    ));
    assert!(matches!(
        rs.recover(0b11111, &have, 5, &mut out),
        Err(RsError::HaveMask { .. })
    ));
    assert!(matches!(
        rs.recover(0b100_0111, &have, 3, &mut out),
        Err(RsError::HaveMask { .. })
    ));
    // Want block out of range, or one we already have.
    assert!(matches!(
        rs.recover(0b1111, &have, 6, &mut out),
        Err(RsError::WantBlock { .. })
    ));
    assert!(matches!(
        rs.recover(0b1111, &have, 200, &mut out),
        Err(RsError::WantBlock { .. })
    ));
    assert!(matches!(
        rs.recover(0b1111, &have, 2, &mut out),
        Err(RsError::WantBlock { .. })
    ));
    assert!(matches!(
        rs.recover(0b1111, &have, 4, &mut short.clone()),
        Err(RsError::BlockSize { .. })
    ));
    assert!(rs.recover(0b1111, &have, 4, &mut out).is_ok());
}

#[test]
fn best_level_is_available() {
    assert!(Level::best().available());
    eprintln!("best: {:?}", Level::best());
}
