//! Replays every fuzz corpus file through its target's `fuzz_support` function on stable
//! Rust, so seeds and past crashes stay covered without nightly.

#![allow(clippy::unwrap_used, clippy::panic)] // test code

use std::path::Path;

use rustern_msgs::fuzz_support;

#[test]
fn replay_corpus() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus");
    let mut replayed = 0;
    for target in std::fs::read_dir(&root).unwrap() {
        let target = target.unwrap().path();
        let name = target.file_name().unwrap().to_str().unwrap().to_owned();
        let run: fn(&[u8]) = match name.as_str() {
            "shard_resp" => fuzz_support::shard_resp,
            "registry_resp" => fuzz_support::registry_resp,
            "blocks_resp" => fuzz_support::blocks_resp,
            other => panic!("corpus for unknown fuzz target {other}"),
        };
        for file in std::fs::read_dir(&target).unwrap() {
            run(&std::fs::read(file.unwrap().path()).unwrap());
            replayed += 1;
        }
    }
    assert!(replayed >= 20, "only {replayed} corpus files");
}
