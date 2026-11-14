//! T110: metadata latency on a mounted TernFS corpus (Rustern or the reference client).
//!
//! `cargo run --release --example meta_latency -- <mount> [iterations=1000] [writable-mount]`
//!
//! Prints `metric p50_us p99_us n` lines for:
//! - `cold-lookup`: `stat` of `min(iterations, 10,000)` distinct names in `corpus/big/`
//!   that are not in the kernel's dentry cache: never looked up, or looked up longer ago
//!   than the entry TTL (the kernel then asks the client again, which does the same work);
//! - `stat`: `iterations` repeated `stat`s of one already looked-up file;
//! - `readdir`: `iterations / 50` full listings of `corpus/big/` (10,000 entries): after
//!   the first, the kernel's directory cache serves them;
//! - `readdir-changed` (with `writable-mount`, a ternfuse mount of the same cluster): 20
//!   listings, each after a file was created and removed in `corpus/big/` through it, so the
//!   cached listing is stale and the client lists from the shards.
#![allow(
    missing_docs,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)] // example code

use std::fs;
use std::path::Path;
use std::time::{Duration, Instant};

fn report(metric: &str, mut samples: Vec<Duration>) {
    samples.sort_unstable();
    let pick = |q: f64| {
        let i = ((samples.len() as f64 - 1.0) * q).round() as usize;
        samples[i].as_secs_f64() * 1e6
    };
    println!(
        "{metric} {:.2} {:.2} {}",
        pick(0.50),
        pick(0.99),
        samples.len()
    );
}

fn timed(f: impl FnOnce()) -> Duration {
    let start = Instant::now();
    f();
    start.elapsed()
}

fn main() {
    let mut args = std::env::args().skip(1);
    let mount = args
        .next()
        .expect("usage: meta_latency <mount> [iterations]");
    let n: usize = args.next().map_or(1000, |s| s.parse().expect("iterations"));
    let corpus = Path::new(&mount).join("corpus");
    let big = corpus.join("big");

    let cold = (0..n.min(10_000))
        .map(|i| {
            let p = big.join(format!("e{i}"));
            timed(|| {
                fs::metadata(&p).unwrap();
            })
        })
        .collect();
    report("cold-lookup", cold);

    let file = corpus.join("sizes/size_4096");
    fs::metadata(&file).unwrap();
    let warm = (0..n)
        .map(|_| {
            timed(|| {
                fs::metadata(&file).unwrap();
            })
        })
        .collect();
    report("stat", warm);

    // One untimed listing first: on a fresh mount it fills the kernel's directory cache,
    // which a long-lived mount already has; uncached listings are `readdir-changed`.
    assert_eq!(fs::read_dir(&big).unwrap().count(), 10_000);
    let listings = (0..(n / 50).max(5))
        .map(|_| {
            timed(|| {
                let count = fs::read_dir(&big).unwrap().map(Result::unwrap).count();
                assert_eq!(count, 10_000);
            })
        })
        .collect();
    report("readdir", listings);

    if let Some(writable) = args.next() {
        let touch = Path::new(&writable).join("corpus/big/.meta-latency");
        let changed = (0..20)
            .map(|_| {
                fs::File::create_new(&touch).unwrap();
                fs::remove_file(&touch).unwrap();
                // Past the directory attribute TTL (250 ms by default), so the new mtime is seen.
                std::thread::sleep(Duration::from_millis(400));
                timed(|| {
                    let count = fs::read_dir(&big).unwrap().map(Result::unwrap).count();
                    assert_eq!(count, 10_000);
                })
            })
            .collect();
        report("readdir-changed", changed);
    }
}
