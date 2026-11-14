//! T081: replays the `page_stream` fuzz corpus on stable Rust, so seeds and past crashes
//! stay covered without nightly.
#![allow(clippy::unwrap_used, clippy::panic)] // test code

use std::path::Path;

use rustern_client::ClientError;
use rustern_client::blocks::page_stream;

#[test]
fn replay_page_stream_corpus() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/page_stream");
    let mut replayed = 0;
    for file in std::fs::read_dir(&dir).unwrap() {
        let path = file.unwrap().path();
        let name = path.file_name().unwrap().to_str().unwrap().to_owned();
        let r = page_stream(&std::fs::read(&path).unwrap());
        // Known seeds also pin the outcome; any other file (a past crash) must just return.
        match name.as_str() {
            n if n.starts_with("valid") || n == "trailing" => assert!(r.is_ok(), "{n}: {r:?}"),
            "bad_crc_page3" => {
                assert!(
                    matches!(r, Err(ClientError::CrcMismatch { page: 3, .. })),
                    "{r:?}"
                );
            }
            "zeros" => assert!(
                matches!(r, Err(ClientError::CrcMismatch { page: 0, .. })),
                "{r:?}"
            ),
            "short_by_one" | "header_only" | "empty" => assert!(r.is_err(), "{name}"),
            _ => {}
        }
        replayed += 1;
    }
    assert!(replayed >= 10, "only {replayed} corpus files");
}
