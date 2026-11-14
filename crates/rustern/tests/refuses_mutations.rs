//! T096 (US3, DEV-001, DEV-002, SC-006): every mutation through the mount fails with
//! EROFS and the cluster is unchanged; opening and reading sends no mutation.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)] // test code

mod common;

use std::path::Path;
use std::process::Command;

use common::{Mount, ref_corpus, walk};
use rustix::fs::{AtFlags, CWD, FileType, Mode, OFlags, Timespec, Timestamps, XattrFlags};
use rustix::io::Errno;

fn erofs<T: std::fmt::Debug>(what: &str, r: Result<T, Errno>) {
    assert_eq!(r.err(), Some(Errno::ROFS), "{what}");
}

fn io_erofs<T: std::fmt::Debug>(what: &str, r: std::io::Result<T>) {
    assert_eq!(
        r.err().and_then(|e| e.raw_os_error()),
        Some(Errno::ROFS.raw_os_error()),
        "{what}"
    );
}

/// A command fails and says why.
fn command_erofs(args: &[&str]) {
    let out = Command::new(args[0]).args(&args[1..]).output().unwrap();
    assert!(!out.status.success(), "{args:?} succeeded");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("Read-only file system"), "{args:?}: {err}");
}

fn every_mutation_fails(root: &Path) {
    let file = root.join("sizes/size_4096");
    let dir = root.join("tree/d00");
    let new = root.join("new-name");
    let fd = rustix::fs::open(&file, OFlags::RDONLY, Mode::empty()).unwrap();

    io_erofs("create", std::fs::File::create(&new));
    for (what, flags) in [
        ("O_WRONLY", OFlags::WRONLY),
        ("O_RDWR", OFlags::RDWR),
        ("O_TRUNC", OFlags::RDONLY | OFlags::TRUNC),
    ] {
        erofs(
            what,
            rustix::fs::open(&file, flags, Mode::from_raw_mode(0o644)),
        );
    }
    // As on a read-only mount, O_CREAT on an existing file without write access opens it:
    // the kernel resolves the name first and never sends a create.
    rustix::fs::open(
        &file,
        OFlags::RDONLY | OFlags::CREATE,
        Mode::from_raw_mode(0o644),
    )
    .unwrap();
    erofs(
        "O_CREAT new",
        rustix::fs::open(
            &new,
            OFlags::RDONLY | OFlags::CREATE,
            Mode::from_raw_mode(0o644),
        ),
    );
    erofs(
        "O_TMPFILE",
        rustix::fs::open(
            &dir,
            OFlags::RDWR | OFlags::TMPFILE,
            Mode::from_raw_mode(0o600),
        ),
    );
    io_erofs("mkdir", std::fs::create_dir(&new));
    io_erofs("rmdir", std::fs::remove_dir(root.join("empty")));
    io_erofs("unlink", std::fs::remove_file(&file));
    io_erofs("rename", std::fs::rename(&file, &new));
    io_erofs("symlink", std::os::unix::fs::symlink("x", &new));
    io_erofs("link", std::fs::hard_link(&file, &new));
    erofs(
        "mknod",
        rustix::fs::mknodat(CWD, &new, FileType::Fifo, Mode::from_raw_mode(0o644), 0),
    );
    erofs(
        "chmod",
        rustix::fs::chmod(&file, Mode::from_raw_mode(0o600)),
    );
    erofs(
        "fchmod",
        rustix::fs::fchmod(&fd, Mode::from_raw_mode(0o600)),
    );
    // The kernel refuses ftruncate on a read-only descriptor before asking the filesystem.
    assert_eq!(
        rustix::fs::ftruncate(&fd, 0).err(),
        Some(Errno::INVAL),
        "ftruncate"
    );
    command_erofs(&["truncate", "-s", "0", file.to_str().unwrap()]);
    let ts = Timestamps {
        last_access: Timespec {
            tv_sec: 1,
            tv_nsec: 0,
        },
        last_modification: Timespec {
            tv_sec: 1,
            tv_nsec: 0,
        },
    };
    erofs(
        "utimes",
        rustix::fs::utimensat(CWD, &file, &ts, AtFlags::empty()),
    );
    erofs(
        "setxattr",
        rustix::fs::setxattr(&file, "user.x", b"1", XattrFlags::empty()),
    );
    erofs("removexattr", rustix::fs::removexattr(&file, "user.x"));
    command_erofs(&["fallocate", "-l", "8192", file.to_str().unwrap()]);
    command_erofs(&[
        "cp",
        file.to_str().unwrap(),
        root.join("sizes/size_1").to_str().unwrap(),
    ]);
    // copy_file_range needs a writable descriptor in the mount, which open refuses above;
    // into the mount from outside, `cp` uses it (or falls back to write) and fails.

    let mut buf = [0u8; 64];
    assert_eq!(
        rustix::fs::getxattr(&file, "user.x", &mut buf[..]).unwrap_err(),
        Errno::NODATA,
        "getxattr"
    );
    assert_eq!(
        rustix::fs::listxattr(&file, &mut buf[..]).unwrap(),
        0,
        "listxattr"
    );
    assert_eq!(
        rustix::fs::listxattr(&dir, &mut [0u8; 0][..]).unwrap(),
        0,
        "listxattr size"
    );
}

#[test]
#[ignore = "needs cluster: run via scripts/gate.sh"]
fn mutations_fail_and_cluster_is_unchanged() {
    let reference = ref_corpus();
    let before = walk(&reference);
    let m = Mount::new(&[]);
    every_mutation_fails(&m.join("corpus"));
    drop(m);
    assert!(walk(&reference) == before, "the corpus changed");
    let out = Command::new("sha256sum")
        .args(["--quiet", "-c", "corpus.sha256"])
        .current_dir(&reference)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
}

#[test]
#[ignore = "needs cluster: run via scripts/gate.sh"]
fn open_sends_no_mutation() {
    let reference = ref_corpus();
    let rel = "sizes/size_65537";
    let atime = |p: &Path| std::fs::metadata(p).unwrap().accessed().unwrap();
    let before = atime(&reference.join(rel));
    let m = Mount::new(&[]);
    for _ in 0..100 {
        let data = std::fs::read(m.join("corpus").join(rel)).unwrap();
        assert_eq!(data.len(), 65537);
    }
    drop(m);
    // ternfuse caches attributes; a fresh lookup through a new path component is enough
    // because its attribute TTL is short, but read the atime from a new process to be sure.
    let out = Command::new("stat")
        .args(["-c", "%X.%x"])
        .arg(reference.join(rel))
        .output()
        .unwrap();
    assert!(out.status.success());
    assert_eq!(
        atime(&reference.join(rel)),
        before,
        "atime changed: {}",
        String::from_utf8_lossy(&out.stdout)
    );
}
