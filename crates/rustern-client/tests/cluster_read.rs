//! T082: file reads against the local cluster, compared with the ternfuse mount.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)] // test code

use std::fs::File;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::FileExt;
use std::path::{Path, PathBuf};

use proptest::prelude::*;
use proptest::test_runner::{Config, RngAlgorithm, TestRng, TestRunner};
use rustern_client::{Client, ClientConfig, FileHandle, MAX_READ};
use rustern_core::inode::TernInode;

const SIZES: [u64; 15] = [
    0,
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
];

fn registry() -> String {
    std::env::var("RUSTERN_TEST_REGISTRY").unwrap_or_else(|_| "127.0.0.1:10001".into())
}

fn corpus() -> PathBuf {
    let data = std::env::var_os("RUSTERN_TEST_DATA_DIR").map_or_else(
        || Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ternfs/mnt"),
        PathBuf::from,
    );
    let p = data.join("fuse_loc1/mnt/corpus");
    assert!(
        p.join("corpus.sha256").is_file(),
        "no corpus at {}; run scripts/make-corpus.sh",
        p.display()
    );
    p
}

fn rt() -> tokio::runtime::Runtime {
    tokio::runtime::Runtime::new().unwrap()
}

async fn connect() -> Client {
    Client::connect(ClientConfig::new(registry()))
        .await
        .unwrap_or_else(|e| panic!("registry {} (RUSTERN_TEST_REGISTRY): {e}", registry()))
}

async fn resolve(client: &Client, rel: &str) -> TernInode {
    let mut dir = TernInode::ROOT;
    for c in Path::new("corpus").join(rel).components() {
        let r = client
            .lookup(dir, c.as_os_str().as_bytes())
            .await
            .unwrap_or_else(|e| panic!("lookup {c:?}: {e}"));
        dir = TernInode::try_from(r.target_id).unwrap();
    }
    dir
}

async fn open(client: &Client, rel: &str) -> FileHandle {
    client
        .open_file(resolve(client, rel).await)
        .await
        .unwrap_or_else(|e| panic!("open {rel}: {e}"))
}

/// `len` bytes at `offset` through ternfuse (short at end of file).
fn reference(f: &File, offset: u64, len: u32) -> Vec<u8> {
    let mut buf = vec![0u8; len as usize];
    let mut got = 0;
    while got < buf.len() {
        let n = f.read_at(&mut buf[got..], offset + got as u64).unwrap();
        if n == 0 {
            break;
        }
        got += n;
    }
    buf.truncate(got);
    buf
}

async fn check(h: &FileHandle, f: &File, offset: u64, len: u32, what: &str) {
    let got = h
        .read(offset, len)
        .await
        .unwrap_or_else(|e| panic!("{what} [{offset}+{len}]: {e}"));
    let want = reference(f, offset, len);
    assert!(
        got.as_ref() == want.as_slice(),
        "{what}: [{offset}+{len}] differs (got {} bytes, want {})",
        got.as_ref().len(),
        want.len()
    );
}

#[test]
#[ignore = "needs cluster: run via scripts/gate.sh"]
fn every_size_class_matches() {
    let corpus = corpus();
    rt().block_on(async {
        let client = connect().await;
        for n in SIZES {
            let rel = format!("sizes/size_{n}");
            let h = open(&client, &rel).await;
            assert_eq!(h.size(), n, "{rel}");
            eprintln!("{rel}: {} spans", h.span_offsets().len());
            let f = File::open(corpus.join(&rel)).unwrap();
            let mut o = 0;
            while o < n {
                check(&h, &f, o, MAX_READ, &rel).await;
                o += u64::from(MAX_READ);
            }
        }
        for rel in ["tree/d07/a/b/c/d/f9", "tree/d19/f3", "big/e299", "big/e0"] {
            let h = open(&client, rel).await;
            check(&h, &File::open(corpus.join(rel)).unwrap(), 0, MAX_READ, rel).await;
        }
    });
}

/// Offsets anywhere, or within a page of a span start, a page, a 1 MiB or 5,120,000 B
/// multiple (span and stripe boundaries).
fn offsets(size: u64, spans: Vec<u64>) -> impl Strategy<Value = u64> {
    let near = |base: u64| (-4100i64..4100).prop_map(move |d| base.saturating_add_signed(d));
    let any = 0..size;
    let span = proptest::sample::select(spans).prop_flat_map(near);
    let page = (0..size / 4096 + 1).prop_flat_map(move |k| near(k * 4096));
    let mib = (0..size / (1 << 20) + 1).prop_flat_map(move |k| near(k << 20));
    let rs = (0..size / 5_120_000 + 1).prop_flat_map(move |k| near(k * 5_120_000));
    prop_oneof![any, span, page, mib, rs].prop_map(move |o| o.min(size + 10))
}

#[test]
#[ignore = "needs cluster: run via scripts/gate.sh"]
fn random_reads_match() {
    let corpus = corpus();
    let rt = rt();
    let client = rt.block_on(connect());
    for rel in [
        "sizes/size_30000000",
        "sizes/size_104857600",
        "sizes/size_1073741824",
    ] {
        let h = rt.block_on(open(&client, rel));
        let f = File::open(corpus.join(rel)).unwrap();
        let mut runner = TestRunner::new_with_rng(
            Config {
                cases: 1000,
                failure_persistence: None,
                ..Config::default()
            },
            TestRng::deterministic_rng(RngAlgorithm::ChaCha),
        );
        let lens = prop_oneof![1u32..=8192, 1u32..=MAX_READ, Just(MAX_READ)];
        runner
            .run(&(offsets(h.size(), h.span_offsets()), lens), |(o, len)| {
                rt.block_on(check(&h, &f, o, len, rel));
                Ok(())
            })
            .unwrap();
    }
}

#[test]
#[ignore = "needs cluster: run via scripts/gate.sh"]
fn end_of_file_reads_are_short() {
    rt().block_on(async {
        let client = connect().await;
        for n in [1u64, 4097, 5_120_001, 30_000_000] {
            let h = open(&client, &format!("sizes/size_{n}")).await;
            assert!(
                h.read(n, 4096).await.unwrap().as_ref().is_empty(),
                "at EOF {n}"
            );
            assert!(
                h.read(n + 1_000_000, 1).await.unwrap().as_ref().is_empty(),
                "past EOF {n}"
            );
            let back = n.min(100);
            assert_eq!(
                h.read(n - back, 4096).await.unwrap().as_ref().len() as u64,
                back,
                "crossing EOF {n}"
            );
        }
        let h = open(&client, "sizes/size_0").await;
        assert!(h.read(0, MAX_READ).await.unwrap().as_ref().is_empty());
    });
}

#[test]
#[ignore = "needs cluster: run via scripts/gate.sh"]
fn read_link_matches() {
    let corpus = corpus();
    rt().block_on(async {
        let client = connect().await;
        for name in ["relative", "absolute", "dangling", "dir"] {
            let rel = format!("links/{name}");
            let got = client
                .read_link(resolve(&client, &rel).await)
                .await
                .unwrap();
            let want = std::fs::read_link(corpus.join(&rel)).unwrap();
            assert_eq!(got, want.as_os_str().as_bytes(), "{rel}");
        }
    });
}
