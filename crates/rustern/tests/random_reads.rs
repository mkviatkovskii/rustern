//! T084 (US2): 10,000 random `pread`s on the multi-span files, compared with ternfuse.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)] // test code

mod common;

use std::fs::File;
use std::os::unix::fs::FileExt;

use common::{Mount, ref_corpus};

const FILES: [&str; 3] = [
    "sizes/size_30000000",
    "sizes/size_104857600",
    "sizes/size_1073741824",
];
const READS: usize = 10_000;
const MAX: usize = 1 << 20;

/// One `pread` (the kernel may split it; it is short only at end of file).
fn pread(f: &File, buf: &mut [u8], offset: u64) -> usize {
    let mut got = 0;
    while got < buf.len() {
        match f.read_at(&mut buf[got..], offset + got as u64).unwrap() {
            0 => break,
            n => got += n,
        }
    }
    got
}

#[test]
#[ignore = "needs cluster: run via scripts/gate.sh"]
fn random_preads_match_ternfuse() {
    let reference = ref_corpus();
    let m = Mount::new(&[]);
    let files: Vec<(File, File, u64)> = FILES
        .iter()
        .map(|rel| {
            let ours = File::open(m.join("corpus").join(rel)).unwrap();
            let theirs = File::open(reference.join(rel)).unwrap();
            let size = theirs.metadata().unwrap().len();
            (ours, theirs, size)
        })
        .collect();
    let mut rng = fastrand::Rng::with_seed(0x7e2f_5eed);
    let (mut a, mut b) = (vec![0u8; MAX], vec![0u8; MAX]);
    for i in 0..READS {
        let (ours, theirs, size) = &files[i % files.len()];
        // Lengths log-uniform over 1 B..=1 MiB; offsets anywhere up to just past the end.
        let len = (1usize << rng.u32(0..=20)).min(MAX);
        let len = rng.usize(len.div_ceil(2)..=len);
        let offset = rng.u64(0..=*size + 10);
        let na = pread(ours, &mut a[..len], offset);
        let nb = pread(theirs, &mut b[..len], offset);
        assert_eq!(
            na,
            nb,
            "read {i}: {} [{offset}+{len}]",
            FILES[i % files.len()]
        );
        assert!(
            a[..na] == b[..nb],
            "read {i}: {} [{offset}+{len}] differs",
            FILES[i % files.len()]
        );
    }
}
