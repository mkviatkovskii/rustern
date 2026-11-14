//! Checks of the shared cluster-test harness itself.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)] // test code

mod common;

use std::fs;

use common::*;

#[test]
fn walk_and_compare_on_local_files() {
    let d = std::env::temp_dir().join(format!("rustern-harness-{}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(d.join("sub")).unwrap();
    fs::write(d.join("a"), vec![7u8; 3 << 20]).unwrap();
    let mut b = vec![7u8; 3 << 20];
    b[(2 << 20) + 5] = 0;
    fs::write(d.join("sub/b"), &b).unwrap();
    std::os::unix::fs::symlink("a", d.join("l")).unwrap();

    let w = walk(&d);
    let names: Vec<_> = w
        .iter()
        .map(|e| e.path.to_str().unwrap().to_owned())
        .collect();
    assert_eq!(names, ["a", "l", "sub", "sub/b"]);
    assert!(w[1].kind.is_symlink());
    assert_eq!(w[2].size, 0);
    assert_eq!(w[3].size, 3 << 20);

    compare_files(&d.join("a"), &d.join("a")).unwrap();
    let err = compare_files(&d.join("a"), &d.join("sub/b")).unwrap_err();
    assert!(err.contains(&format!("offset {}", (2 << 20) + 5)), "{err}");
    fs::remove_dir_all(&d).unwrap();
}

#[test]
#[ignore = "needs cluster: run via scripts/gate.sh"]
fn cluster_is_reachable_and_has_a_corpus() {
    let corpus = ref_corpus();
    assert!(walk(&corpus).len() > 1000);
    assert_eq!(pgrep_tern("ternshard", "-shard 7").len(), 1);
    assert_eq!(pgrep_tern("ternblocks", "-failure-domain 0").len(), 1);
    assert!(find_block_file(u64::MAX).is_none());
}
