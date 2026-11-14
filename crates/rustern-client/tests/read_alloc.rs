//! T085 (CT-4, R12): steady-state 1 MiB reads allocate nothing sized to file data.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)] // test code

use std::os::unix::ffi::OsStrExt;
use std::path::Path;

use rustern_client::{Client, ClientConfig, MAX_READ};
use rustern_core::inode::TernInode;

#[global_allocator]
static ALLOC: dhat::Alloc = dhat::Alloc;

const WARMUP: u64 = 10;
const READS: u64 = 100;
const PER_READ_LIMIT: u64 = 64 * 1024;

#[test]
#[ignore = "needs cluster: run via scripts/gate.sh"]
fn sequential_reads_allocate_little() {
    let _profiler = dhat::Profiler::builder().testing().build();
    let registry =
        std::env::var("RUSTERN_TEST_REGISTRY").unwrap_or_else(|_| "127.0.0.1:10001".into());
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        let client = Client::connect(ClientConfig::new(registry)).await.unwrap();
        let mut id = TernInode::ROOT;
        for c in Path::new("corpus/sizes/size_1073741824").components() {
            let r = client.lookup(id, c.as_os_str().as_bytes()).await.unwrap();
            id = TernInode::try_from(r.target_id).unwrap();
        }
        let h = client.open_file(id).await.unwrap();
        let step = u64::from(MAX_READ);
        for i in 0..WARMUP {
            assert_eq!(
                h.read(i * step, MAX_READ).await.unwrap().as_ref().len(),
                MAX_READ as usize
            );
        }
        let before = dhat::HeapStats::get();
        for i in WARMUP..WARMUP + READS {
            let b = h.read(i * step, MAX_READ).await.unwrap();
            assert_eq!(b.as_ref().len(), MAX_READ as usize);
        }
        let after = dhat::HeapStats::get();
        let bytes = (after.total_bytes - before.total_bytes) / READS;
        let blocks = (after.total_blocks - before.total_blocks) / READS;
        eprintln!("per 1 MiB read: {blocks} allocations, {bytes} bytes");
        assert!(bytes < PER_READ_LIMIT, "{bytes} bytes allocated per read");
        assert!(
            after.max_bytes < before.max_bytes + 4 * MAX_READ as usize,
            "peak heap grew from {} to {}",
            before.max_bytes,
            after.max_bytes
        );
    });
}
