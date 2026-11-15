// Copyright 2026 Mikhail Kviatkovskii
//
// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception

//! Every kernel against `testdata/goldens/rs.hex`: parity computed and blocks recovered by
//! upstream `go/core/rs` (`tools/goldens/rs.go`).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
#![allow(clippy::arithmetic_side_effects)]

use rustern_rs::{Level, Rs};

const GOLDENS: &str = include_str!("../../../testdata/goldens/rs.hex");

fn unhex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

fn blocks(bytes: &[u8], n: usize) -> Vec<&[u8]> {
    assert_eq!(bytes.len() % n, 0);
    bytes.chunks(bytes.len() / n).collect()
}

fn levels() -> Vec<Level> {
    let levels: Vec<Level> = Level::ALL.into_iter().filter(|l| l.available()).collect();
    eprintln!("kernels under test: {levels:?}");
    levels
}

struct Case<'a> {
    kind: &'a str,
    d: u8,
    p: u8,
    fields: Vec<&'a str>,
    desc: &'a str,
}

fn cases() -> Vec<Case<'static>> {
    let cases: Vec<Case> = GOLDENS
        .lines()
        .filter(|l| !l.starts_with('#') && !l.is_empty())
        .map(|l| {
            let (body, desc) = l.split_once(" # ").unwrap();
            let mut f = body.split(' ');
            let kind = f.next().unwrap();
            let d = f.next().unwrap().parse().unwrap();
            let p = f.next().unwrap().parse().unwrap();
            Case {
                kind,
                d,
                p,
                fields: f.collect(),
                desc,
            }
        })
        .collect();
    assert!(
        cases.len() >= 140,
        "rs.hex looks truncated: {} cases",
        cases.len()
    );
    cases
}

#[test]
fn parity_matches_upstream() {
    let levels = levels();
    let mut n = 0;
    for c in cases().iter().filter(|c| c.kind == "parity") {
        let rs = Rs::get(c.p << 4 | c.d).unwrap();
        let data_bytes = unhex(c.fields[0]);
        let want_bytes = unhex(c.fields[1]);
        let data = blocks(&data_bytes, c.d.into());
        let want = blocks(&want_bytes, c.p.into());
        for level in &levels {
            let size = data[0].len();
            let mut out = vec![vec![0xa5u8; size]; c.p.into()];
            let mut refs: Vec<&mut [u8]> = out.iter_mut().map(|v| v.as_mut_slice()).collect();
            rs.compute_parity_with(*level, &data, &mut refs).unwrap();
            for (i, (got, want)) in out.iter().zip(&want).enumerate() {
                assert_eq!(
                    got.as_slice(),
                    *want,
                    "{} parity block {i} with {level:?}",
                    c.desc
                );
            }
        }
        n += 1;
    }
    assert!(n >= 70);
}

#[test]
fn recover_matches_upstream() {
    let levels = levels();
    let mut n = 0;
    for c in cases().iter().filter(|c| c.kind == "recover") {
        let rs = Rs::get(c.p << 4 | c.d).unwrap();
        let mask = u32::from_str_radix(c.fields[0], 16).unwrap();
        let want_block: u8 = c.fields[1].parse().unwrap();
        let have_bytes = unhex(c.fields[2]);
        let want = unhex(c.fields[3]);
        let have = blocks(&have_bytes, c.d.into());
        for level in &levels {
            let mut out = vec![0x5au8; want.len()];
            rs.recover_with(*level, mask, &have, want_block, &mut out)
                .unwrap();
            assert_eq!(out, want, "{} block {want_block} with {level:?}", c.desc);
        }
        n += 1;
    }
    assert!(n >= 70);
}
