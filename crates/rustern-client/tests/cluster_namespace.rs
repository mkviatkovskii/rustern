//! T061: namespace requests against the local cluster, compared with the ternfuse mount.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)] // test code

use std::collections::BTreeSet;
use std::ffi::OsString;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use rustern_client::{Client, ClientConfig, ClientError};
use rustern_core::inode::{InodeKind, TernInode};
use rustern_msgs::{ErrorCode, NameHash};

fn registry() -> String {
    std::env::var("RUSTERN_TEST_REGISTRY").unwrap_or_else(|_| "127.0.0.1:10001".into())
}

/// The reference corpus through ternfuse.
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

/// Resolves `rel` under `corpus` from the root, one `lookup` per component.
async fn resolve(client: &Client, rel: &Path) -> TernInode {
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

fn mtime_ns(m: &std::fs::Metadata) -> u64 {
    u64::try_from(
        m.modified()
            .unwrap()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
    )
    .unwrap()
}

#[test]
#[ignore = "needs cluster: run via scripts/gate.sh"]
fn lookup_existing_and_missing() {
    let corpus = corpus();
    rt().block_on(async {
        let client = connect().await;
        let id = resolve(&client, Path::new("sizes/size_4096")).await;
        assert_eq!(id.kind(), InodeKind::File);
        assert_eq!(
            id.to_fuse(),
            std::fs::metadata(corpus.join("sizes/size_4096"))
                .unwrap()
                .ino()
        );

        let sizes = resolve(&client, Path::new("sizes")).await;
        match client.lookup(sizes, b"no-such-name").await {
            Err(ClientError::Tern { code, .. }) => assert_eq!(code, ErrorCode::NAME_NOT_FOUND),
            other => panic!("{other:?}"),
        }
    });
}

#[test]
#[ignore = "needs cluster: run via scripts/gate.sh"]
fn stats_match_ternfuse() {
    let corpus = corpus();
    rt().block_on(async {
        let client = connect().await;
        for rel in [
            "sizes/size_0",
            "sizes/size_5120001",
            "tree/d03/a/b/f7",
            "big/e42",
        ] {
            let m = std::fs::metadata(corpus.join(rel)).unwrap();
            let st = client
                .stat_file(resolve(&client, Path::new(rel)).await)
                .await
                .unwrap();
            assert_eq!(st.size, m.len(), "{rel}");
            assert_eq!(st.mtime.0, mtime_ns(&m), "{rel}");
        }
        for rel in ["sizes", "tree/d03/a/b", "empty"] {
            let m = std::fs::metadata(corpus.join(rel)).unwrap();
            let id = resolve(&client, Path::new(rel)).await;
            let st = client.stat_directory(id).await.unwrap();
            assert_eq!(st.mtime.0, mtime_ns(&m), "{rel}");
            let parent = std::fs::metadata(corpus.join(rel).parent().unwrap()).unwrap();
            assert_eq!(
                TernInode::try_from(st.owner).unwrap().to_fuse(),
                parent.ino(),
                "{rel}"
            );
        }
    });
}

#[test]
#[ignore = "needs cluster: run via scripts/gate.sh"]
fn read_dir_pages_over_10000_entries() {
    let corpus = corpus();
    rt().block_on(async {
        let client = connect().await;
        assert_eq!(client.config().mtu, 1472);
        let big = resolve(&client, Path::new("big")).await;
        let mut names = Vec::new();
        let mut pages = 0;
        let mut next = NameHash(0);
        loop {
            let page = client.read_dir(big, next).await.unwrap();
            pages += 1;
            names.extend(
                page.results
                    .iter()
                    .map(|e| OsString::from_vec(e.name.to_vec())),
            );
            if page.next_hash.0 == 0 {
                break;
            }
            next = page.next_hash;
        }
        assert!(pages > 1, "one page only");
        let set: BTreeSet<_> = names.iter().cloned().collect();
        assert_eq!(set.len(), names.len(), "an entry was returned twice");
        assert_eq!(names.len(), 10_000);
        let want: BTreeSet<OsString> = std::fs::read_dir(corpus.join("big"))
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(set, want);
    });
}
