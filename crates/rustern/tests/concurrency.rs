//! T086 (US2): concurrent readers through one mount: checksums, request sharing through
//! the kernel page cache (FR-011) and the per-file memory limit (FR-015).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)] // test code

mod common;

use std::collections::HashMap;
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use common::{Mount, metric_sum, ref_corpus};

const FILE_100M: &str = "sizes/size_104857600";
const FILE_1G: &str = "sizes/size_1073741824";

/// `./rel` → sha256 from `corpus.sha256`.
fn expected(reference: &Path) -> HashMap<String, String> {
    // Lossy: two names are not UTF-8.
    String::from_utf8_lossy(&std::fs::read(reference.join("corpus.sha256")).unwrap())
        .lines()
        .map(|l| {
            let (h, p) = l.split_once("  ").unwrap();
            (p.trim_start_matches("./").to_owned(), h.to_owned())
        })
        .collect()
}

fn sha256(path: &Path) -> String {
    let out = Command::new("sha256sum").arg(path).output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout)
        .unwrap()
        .split(' ')
        .next()
        .unwrap()
        .to_owned()
}

/// Runs `sha256sum` of every path at once; returns each digest.
fn sha256_all(paths: &[std::path::PathBuf]) -> Vec<String> {
    std::thread::scope(|s| {
        let hs: Vec<_> = paths.iter().map(|p| s.spawn(move || sha256(p))).collect();
        hs.into_iter().map(|h| h.join().unwrap()).collect()
    })
}

/// `(block fetches, block bytes)` so far.
fn block_counters(m: &Mount) -> (f64, f64) {
    let dump = m.metrics();
    (
        metric_sum(&dump, "rustern_block_fetches_total"),
        metric_sum(&dump, "rustern_block_bytes_total"),
    )
}

#[test]
#[ignore = "needs cluster: run via scripts/gate.sh"]
fn sixteen_processes_on_different_files() {
    let reference = ref_corpus();
    let want = expected(&reference);
    let m = Mount::new(&[]);
    let mut rels: Vec<String> = [
        1,
        255,
        256,
        4095,
        4096,
        4097,
        65536,
        65537,
        1_048_576,
        5_120_000,
        5_120_001,
        30_000_000,
        104_857_600,
        1_073_741_824,
    ]
    .iter()
    .map(|n| format!("sizes/size_{n}"))
    .collect();
    rels.extend(["tree/d00/a/b/c/d/f9".to_owned(), "tree/d11/a/f5".to_owned()]);
    assert_eq!(rels.len(), 16);
    let paths: Vec<_> = rels.iter().map(|r| m.join("corpus").join(r)).collect();
    for (rel, got) in rels.iter().zip(sha256_all(&paths)) {
        assert_eq!(got, want[rel], "{rel}");
    }
}

#[test]
#[ignore = "needs cluster: run via scripts/gate.sh"]
fn readers_of_one_file_share_fetches() {
    let reference = ref_corpus();
    let want = &expected(&reference)[FILE_100M];
    let size = std::fs::metadata(reference.join(FILE_100M)).unwrap().len() as f64;

    // One reader alone, on a fresh mount (empty page cache for this inode... the kernel
    // keys the page cache by mount, so a new mount starts cold).
    let (single, _) = {
        let m = Mount::new(&[]);
        assert_eq!(&sha256(&m.join("corpus").join(FILE_100M)), want);
        block_counters(&m)
    };
    assert!(single > 0.0);

    let m = Mount::new(&[]);
    let paths = vec![m.join("corpus").join(FILE_100M); 16];
    for got in sha256_all(&paths) {
        assert_eq!(&got, want);
    }
    let (fetches, bytes) = block_counters(&m);
    assert!(
        fetches <= 3.0 * single,
        "16 readers: {fetches} fetches vs {single} alone"
    );
    assert!(
        bytes <= 1.1 * size,
        "16 readers fetched {bytes} bytes of a {size}-byte file"
    );

    // Closed and read again: served from the page cache (FOPEN_KEEP_CACHE).
    assert_eq!(&sha256(&paths[0]), want);
    let (again, _) = block_counters(&m);
    assert_eq!(again, fetches, "a re-read after close fetched blocks");
}

#[test]
#[ignore = "needs cluster: run via scripts/gate.sh"]
fn per_file_limit_does_not_starve_other_files() {
    let reference = ref_corpus();
    let want = expected(&reference);

    let alone = {
        let m = Mount::new(&[]);
        let start = Instant::now();
        assert_eq!(sha256(&m.join("corpus").join(FILE_100M)), want[FILE_100M]);
        start.elapsed()
    };

    let m = Mount::new(&["--file-memory-limit", "2MiB"]);
    let big = vec![m.join("corpus").join(FILE_1G); 16];
    let small = m.join("corpus").join(FILE_100M);
    let (big_sums, small_time) = std::thread::scope(|s| {
        let readers = s.spawn(|| sha256_all(&big));
        std::thread::sleep(Duration::from_millis(200)); // the big readers are going
        let start = Instant::now();
        assert_eq!(sha256(&small), want[FILE_100M]);
        let t = start.elapsed();
        (readers.join().unwrap(), t)
    });
    for got in big_sums {
        assert_eq!(got, want[FILE_1G]);
    }
    // A fair CPU share, not starvation: the 16 readers hash on the same CPUs as the small
    // one (and the cluster), so few CPUs stretch the small read; a starved one would wait
    // out the 16 GiB of big reads. 2x at least, as slack on any machine.
    let cpus = std::thread::available_parallelism().map_or(1, std::num::NonZero::get);
    let share = u32::try_from((big.len() + 1).div_ceil(cpus).max(2)).unwrap();
    let limit = (alone * share).max(Duration::from_millis(500));
    assert!(
        small_time <= limit,
        "{small_time:?} with 16 readers of another file vs {alone:?} alone"
    );
}
