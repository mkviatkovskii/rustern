// Copyright 2026 Mikhail Kviatkovskii
//
// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception

//! Parity and recovery throughput per kernel, in data bytes per second (D blocks of 1 MiB,
//! about one TernFS cell stripe), for the layouts TernFS deployments use.
#![allow(
    missing_docs,
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use rustern_rs::{Level, Rs};
use std::hint::black_box;

const BLOCK: usize = 1 << 20;

fn blocks(n: usize) -> Vec<Vec<u8>> {
    (0..n)
        .map(|b| {
            (0..BLOCK)
                .map(|i| (i.wrapping_mul(31) ^ b.wrapping_mul(97)) as u8)
                .collect()
        })
        .collect()
}

fn rs(c: &mut Criterion) {
    for (d, p) in [(4u8, 2u8), (10, 4), (8, 3)] {
        let rs = Rs::get(p << 4 | d).unwrap();
        let data = blocks(d.into());
        let data_refs: Vec<&[u8]> = data.iter().map(|v| v.as_slice()).collect();
        let mut parity = vec![vec![0u8; BLOCK]; p.into()];

        let mut g = c.benchmark_group(format!("rs_parity/D{d}P{p}"));
        g.throughput(Throughput::Bytes((BLOCK * usize::from(d)) as u64));
        for level in Level::ALL.into_iter().filter(|l| l.available()) {
            g.bench_function(BenchmarkId::from_parameter(format!("{level:?}")), |b| {
                b.iter(|| {
                    let mut refs: Vec<&mut [u8]> =
                        parity.iter_mut().map(|v| v.as_mut_slice()).collect();
                    rs.compute_parity_with(level, black_box(&data_refs), &mut refs)
                        .unwrap();
                });
            });
        }
        g.finish();

        // Recover data block 0 without parity block 0 (the XOR row), so the multiply kernel
        // runs: have = data blocks 1..D-1 and parity block 1 (tools/rsbench does the same).
        let have: Vec<&[u8]> = data_refs[1..]
            .iter()
            .copied()
            .chain([parity[1].as_slice()])
            .collect();
        let mask = ((1u32 << d) - 2) | (1 << (d + 1));
        let mut out = vec![0u8; BLOCK];
        let mut g = c.benchmark_group(format!("rs_recover/D{d}P{p}"));
        g.throughput(Throughput::Bytes((BLOCK * usize::from(d)) as u64));
        for level in Level::ALL.into_iter().filter(|l| l.available()) {
            g.bench_function(BenchmarkId::from_parameter(format!("{level:?}")), |b| {
                b.iter(|| {
                    rs.recover_with(level, mask, black_box(&have), 0, &mut out)
                        .unwrap()
                });
            });
        }
        g.finish();
    }
}

criterion_group!(benches, rs);
criterion_main!(benches);
