//! T083 (US2, SC-001): every corpus file read through a Rustern mount equals ternfuse.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)] // test code

mod common;

use std::process::Command;

use common::{Mount, compare_files, ref_corpus, walk};

#[test]
#[ignore = "needs cluster: run via scripts/gate.sh"]
fn sha256sum_check_passes() {
    ref_corpus();
    let m = Mount::new(&[]);
    let out = Command::new("sha256sum")
        .args(["--quiet", "-c", "corpus.sha256"])
        .current_dir(m.join("corpus"))
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
#[ignore = "needs cluster: run via scripts/gate.sh"]
fn every_file_matches_ternfuse() {
    let reference = ref_corpus();
    let m = Mount::new(&[]);
    let ours = m.join("corpus");
    let mut files = 0;
    for e in walk(&reference).into_iter().filter(|e| e.kind.is_file()) {
        compare_files(&ours.join(&e.path), &reference.join(&e.path)).unwrap();
        files += 1;
    }
    assert!(files > 10_000, "only {files} files");
}

#[test]
#[ignore = "needs cluster: run via scripts/gate.sh"]
fn symlinks_match_ternfuse() {
    let reference = ref_corpus();
    let m = Mount::new(&[]);
    let ours = m.join("corpus/links");
    for name in ["relative", "absolute", "dangling", "dir"] {
        assert_eq!(
            std::fs::read_link(ours.join(name)).unwrap(),
            std::fs::read_link(reference.join("links").join(name)).unwrap(),
            "{name}"
        );
    }
    // Following them: a file in the corpus, a file outside it, nothing, a directory.
    compare_files(&ours.join("relative"), &reference.join("sizes/size_1")).unwrap();
    assert_eq!(
        std::fs::read(ours.join("absolute")).unwrap(),
        std::fs::read("/etc/passwd").unwrap()
    );
    assert_eq!(
        std::fs::metadata(ours.join("dangling")).unwrap_err().kind(),
        std::io::ErrorKind::NotFound
    );
    let mut a: Vec<_> = std::fs::read_dir(ours.join("dir"))
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    let mut b: Vec<_> = std::fs::read_dir(reference.join("tree"))
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    a.sort();
    b.sort();
    assert_eq!(a, b);
    // readlink on a regular file is EINVAL.
    let e = std::fs::read_link(m.join("corpus/sizes/size_1")).unwrap_err();
    assert_eq!(
        e.raw_os_error(),
        Some(rustix::io::Errno::INVAL.raw_os_error())
    );
}

#[test]
#[ignore = "needs cluster: run via scripts/gate.sh"]
fn write_opens_are_erofs() {
    ref_corpus();
    let m = Mount::new(&[]);
    let p = m.join("corpus/sizes/size_1");
    for opts in [
        std::fs::OpenOptions::new().write(true).clone(),
        std::fs::OpenOptions::new().read(true).write(true).clone(),
        std::fs::OpenOptions::new()
            .write(true)
            .truncate(true)
            .clone(),
    ] {
        let e = opts.open(&p).unwrap_err();
        assert_eq!(
            e.raw_os_error(),
            Some(rustix::io::Errno::ROFS.raw_os_error()),
            "{opts:?}"
        );
    }
}
