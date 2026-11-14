//! T052: the bridge mounted over an in-memory filesystem (needs `/dev/fuse` and
//! `fusermount3`, not the cluster).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)] // test code

use std::ffi::{OsStr, OsString};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use fuser::{
    Errno, FileAttr, FileHandle, FileType, FopenFlags, Generation, INodeNo, MountOption, OpenFlags,
};
use rustern_client::ClientError;
use rustern_fuse::bridge::{
    AsyncFilesystem, Bridge, DirEntry, DirEntryPlus, FsResult, GetAttrResponse, LookupResponse,
    OpenResponse, StatfsResponse,
};

const HELLO: &[u8] = b"hello world\n";

/// name, inode, type, size
const FILES: [(&str, u64, FileType, u64); 5] = [
    ("hello", 2, FileType::RegularFile, HELLO.len() as u64),
    ("link", 3, FileType::Symlink, 5),
    ("slow", 4, FileType::RegularFile, 4),
    ("broken", 5, FileType::RegularFile, 10),
    ("dir", 6, FileType::Directory, 0),
];

fn attr(ino: u64, kind: FileType, size: u64) -> FileAttr {
    FileAttr {
        ino: INodeNo(ino),
        size,
        blocks: size.div_ceil(512),
        atime: SystemTime::UNIX_EPOCH,
        mtime: SystemTime::UNIX_EPOCH,
        ctime: SystemTime::UNIX_EPOCH,
        crtime: SystemTime::UNIX_EPOCH,
        kind,
        perm: 0o555,
        nlink: 1,
        uid: 0,
        gid: 0,
        rdev: 0,
        blksize: 4096,
        flags: 0,
    }
}

fn find(ino: INodeNo) -> FsResult<FileAttr> {
    if ino.0 == 1 {
        return Ok(attr(1, FileType::Directory, 0));
    }
    FILES
        .iter()
        .find(|f| f.1 == ino.0)
        .map(|f| attr(f.1, f.2, f.3))
        .ok_or_else(|| Errno::ENOENT.into())
}

/// Attributes are never cached, so every `stat` reaches the filesystem.
const TTL: Duration = Duration::ZERO;

/// Files in `dir`: enough to fill several readdir buffers (getdents asks for 32 KiB).
const MANY: u64 = 2000;

/// `.`, `..` and the children of directory `ino`.
fn children(ino: INodeNo) -> FsResult<Vec<(String, u64, FileType)>> {
    let mut all = vec![
        (".".to_owned(), ino.0, FileType::Directory),
        ("..".to_owned(), 1, FileType::Directory),
    ];
    match ino.0 {
        1 => all.extend(FILES.iter().map(|f| (f.0.to_owned(), f.1, f.2))),
        6 => {
            all.extend((0..MANY).map(|i| (format!("file-{i:04}"), 100 + i, FileType::RegularFile)))
        }
        _ => return Err(Errno::ENOTDIR.into()),
    }
    Ok(all)
}

/// `PLUS` asks the kernel for readdirplus.
struct TestFs<const PLUS: bool>;

impl<const PLUS: bool> AsyncFilesystem for TestFs<PLUS> {
    type Buf = Vec<u8>;

    const READDIRPLUS: bool = PLUS;

    async fn lookup(&self, parent: INodeNo, name: &OsStr) -> FsResult<LookupResponse> {
        if parent.0 != 1 {
            return Err(Errno::ENOENT.into());
        }
        let f = FILES
            .iter()
            .find(|f| OsStr::new(f.0) == name)
            .ok_or(Errno::ENOENT)?;
        Ok(LookupResponse {
            ttl: TTL,
            attr: attr(f.1, f.2, f.3),
            generation: Generation(0),
        })
    }

    async fn getattr(&self, ino: INodeNo, _fh: Option<FileHandle>) -> FsResult<GetAttrResponse> {
        Ok(GetAttrResponse {
            ttl: TTL,
            attr: find(ino)?,
        })
    }

    async fn readlink(&self, ino: INodeNo) -> FsResult<Vec<u8>> {
        match ino.0 {
            3 => Ok(b"hello".to_vec()),
            _ => Err(Errno::EINVAL.into()),
        }
    }

    async fn open(&self, ino: INodeNo, _flags: OpenFlags) -> FsResult<OpenResponse> {
        // Direct I/O for `broken` so the kernel sends exactly one read.
        let flags = if ino.0 == 5 {
            FopenFlags::FOPEN_DIRECT_IO
        } else {
            FopenFlags::empty()
        };
        Ok(OpenResponse {
            fh: FileHandle(ino.0),
            flags,
        })
    }

    async fn read(
        &self,
        ino: INodeNo,
        _fh: FileHandle,
        offset: u64,
        size: u32,
    ) -> FsResult<Vec<u8>> {
        let data: &[u8] = match ino.0 {
            2 => HELLO,
            4 => {
                tokio::time::sleep(Duration::from_secs(2)).await;
                b"slow"
            }
            5 => return Err(ClientError::Malformed { what: "test cause" }.into()),
            _ => return Err(Errno::EIO.into()),
        };
        let start = usize::try_from(offset).unwrap().min(data.len());
        let end = (start + size as usize).min(data.len());
        Ok(data[start..end].to_vec())
    }

    async fn release(&self, _ino: INodeNo, _fh: FileHandle) -> FsResult<()> {
        Ok(())
    }

    async fn opendir(&self, ino: INodeNo, _flags: OpenFlags) -> FsResult<OpenResponse> {
        Ok(OpenResponse {
            fh: FileHandle(ino.0),
            flags: FopenFlags::empty(),
        })
    }

    async fn readdir(&self, ino: INodeNo, _fh: FileHandle, offset: u64) -> FsResult<Vec<DirEntry>> {
        Ok(children(ino)?
            .into_iter()
            .enumerate()
            .skip(usize::try_from(offset).unwrap())
            .map(|(i, (name, ino, kind))| DirEntry {
                ino: INodeNo(ino),
                offset: i as u64 + 1,
                kind,
                name: OsString::from(name),
            })
            .collect())
    }

    async fn readdirplus(
        &self,
        ino: INodeNo,
        _fh: FileHandle,
        offset: u64,
    ) -> FsResult<Vec<DirEntryPlus>> {
        Ok(children(ino)?
            .into_iter()
            .enumerate()
            .skip(usize::try_from(offset).unwrap())
            .map(|(i, (name, ino, kind))| DirEntryPlus {
                offset: i as u64 + 1,
                name: OsString::from(name),
                entry: LookupResponse {
                    ttl: TTL,
                    attr: attr(ino, kind, 0),
                    generation: Generation(0),
                },
            })
            .collect())
    }

    async fn releasedir(&self, _ino: INodeNo, _fh: FileHandle) -> FsResult<()> {
        Ok(())
    }

    async fn statfs(&self, _ino: INodeNo) -> FsResult<StatfsResponse> {
        Ok(StatfsResponse {
            bsize: 4096,
            namelen: 255,
            ..StatfsResponse::default()
        })
    }
}

/// Log output of the whole test binary.
static LOG: Mutex<Vec<u8>> = Mutex::new(Vec::new());

struct LogWriter;

impl Write for LogWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        LOG.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Sorted names in `dir`.
fn listed(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    names
}

/// What `listed` returns for `dir`.
fn many() -> Vec<String> {
    (0..MANY).map(|i| format!("file-{i:04}")).collect()
}

fn mount_dir(test: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("rustern-{test}-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn bridge_serves_an_async_filesystem() {
    if !Path::new("/dev/fuse").exists() {
        eprintln!("skipping: no /dev/fuse");
        return;
    }
    let recorder = metrics_exporter_prometheus::PrometheusBuilder::new().build_recorder();
    let metrics = recorder.handle();
    metrics::set_global_recorder(recorder).unwrap();
    tracing_subscriber::fmt()
        .with_writer(|| LogWriter)
        .with_ansi(false)
        .with_max_level(tracing::Level::DEBUG)
        .init();

    let rt = tokio::runtime::Runtime::new().unwrap();
    let dir = mount_dir("bridge");
    let mut config = fuser::Config::default();
    config.mount_options = vec![MountOption::RO, MountOption::FSName("rustern-test".into())];
    let session = fuser::spawn_mount(
        Bridge::new(Arc::new(TestFs::<false>), rt.handle().clone()),
        &dir,
        &config,
    )
    .unwrap();

    // lookup, getattr, readdir, read, readlink.
    assert_eq!(listed(&dir.join("dir")), many());
    assert_eq!(listed(&dir), ["broken", "dir", "hello", "link", "slow"]);
    let meta = std::fs::metadata(dir.join("hello")).unwrap();
    assert_eq!(meta.len(), HELLO.len() as u64);
    assert!(meta.is_file());
    assert_eq!(std::fs::read(dir.join("hello")).unwrap(), HELLO);
    assert_eq!(
        std::fs::read_link(dir.join("link")).unwrap(),
        Path::new("hello")
    );
    assert_eq!(std::fs::read(dir.join("link")).unwrap(), HELLO);
    assert!(std::fs::metadata(dir.join("missing")).is_err());

    // A slow read does not delay a concurrent stat.
    let slow_path = dir.join("slow");
    let slow = std::thread::spawn(move || {
        let start = Instant::now();
        let data = std::fs::read(slow_path).unwrap();
        (data, start.elapsed())
    });
    std::thread::sleep(Duration::from_millis(200));
    let start = Instant::now();
    std::fs::metadata(dir.join("hello")).unwrap();
    let stat_time = start.elapsed();
    let (data, slow_time) = slow.join().unwrap();
    assert_eq!(data, b"slow");
    assert!(slow_time >= Duration::from_secs(2), "{slow_time:?}");
    assert!(
        stat_time < Duration::from_millis(500),
        "stat took {stat_time:?}"
    );

    // An error is counted and logged once with operation, inode and cause.
    let err = std::fs::read(dir.join("broken")).unwrap_err();
    assert_eq!(err.raw_os_error(), Some(libc_eio()));
    std::thread::sleep(Duration::from_millis(50));
    let text = metrics.render();
    let line = text
        .lines()
        .find(|l| l.starts_with(r#"rustern_fuse_ops_total{op="read",errno="EIO"}"#))
        .unwrap_or_else(|| panic!("{text}"));
    assert!(line.ends_with(" 1"), "{line}");
    let log = String::from_utf8(LOG.lock().unwrap().clone()).unwrap();
    let failures: Vec<&str> = log
        .lines()
        .filter(|l| l.contains("operation failed") && l.contains("ino=5"))
        .collect();
    assert_eq!(failures.len(), 1, "{log}");
    assert!(
        failures[0].contains("read") && failures[0].contains("test cause"),
        "{}",
        failures[0]
    );
    assert!(failures[0].contains("WARN"), "{}", failures[0]);

    // Unmount on drop leaves the directory usable.
    drop(session);
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
    std::fs::remove_dir(&dir).unwrap();
}

/// EROFS on Linux.
const EROFS: i32 = 30;

fn refused<T: std::fmt::Debug>(op: &str, r: std::io::Result<T>) {
    let e = r.expect_err(op);
    assert_eq!(e.raw_os_error(), Some(EROFS), "{op}: {e}");
}

/// DEV-001 without the `ro` mount option: the kernel forwards every mutation, the bridge
/// refuses each one with EROFS, and the filesystem is unchanged afterwards.
#[test]
fn mutations_without_ro_get_erofs() {
    use rustix::fs::{CWD, FallocateFlags, Mode, XattrFlags};
    use std::fs::OpenOptions;
    use std::os::unix::fs::PermissionsExt;

    if !Path::new("/dev/fuse").exists() {
        eprintln!("skipping: no /dev/fuse");
        return;
    }
    let rt = tokio::runtime::Runtime::new().unwrap();
    let dir = mount_dir("mutations");
    let mut config = fuser::Config::default();
    config.mount_options = vec![MountOption::FSName("rustern-test".into())];
    let session = fuser::spawn_mount(
        Bridge::new(Arc::new(TestFs::<true>), rt.handle().clone()),
        &dir,
        &config,
    )
    .unwrap();
    let p = |name: &str| dir.join(name);

    refused(
        "create",
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(p("new")),
    );
    refused(
        "mknod",
        rustix::fs::mknodat(CWD, p("fifo"), rustix::fs::FileType::Fifo, Mode::RUSR, 0)
            .map_err(std::io::Error::from),
    );
    refused("mkdir", std::fs::create_dir(p("newdir")));
    refused("unlink", std::fs::remove_file(p("hello")));
    refused("rmdir", std::fs::remove_dir(p("dir")));
    refused("symlink", std::os::unix::fs::symlink("hello", p("newlink")));
    refused("link", std::fs::hard_link(p("hello"), p("newhard")));
    refused("rename", std::fs::rename(p("hello"), p("renamed")));
    refused(
        "setattr",
        std::fs::set_permissions(p("hello"), std::fs::Permissions::from_mode(0o644)),
    );
    refused(
        "setxattr",
        rustix::fs::setxattr(p("hello"), "user.k", b"v", XattrFlags::empty())
            .map_err(std::io::Error::from),
    );
    refused(
        "removexattr",
        rustix::fs::removexattr(p("hello"), "user.k").map_err(std::io::Error::from),
    );

    // Opening for writing is allowed; the writes themselves are refused.
    let mut out = OpenOptions::new().write(true).open(p("hello")).unwrap();
    refused("write", out.write(b"x"));
    refused(
        "fallocate",
        rustix::fs::fallocate(&out, FallocateFlags::empty(), 0, 16).map_err(std::io::Error::from),
    );
    let input = std::fs::File::open(p("slow")).unwrap();
    refused(
        "copy_file_range",
        rustix::fs::copy_file_range(&input, Some(&mut 0), &out, Some(&mut 0), 4)
            .map_err(std::io::Error::from),
    );
    drop((out, input));

    // Read-only requests still work: statfs, listxattr (size probe and data), readdirplus.
    let st = rustix::fs::statvfs(&dir).unwrap();
    assert_eq!((st.f_bsize, st.f_namemax), (4096, 255));
    assert_eq!(
        rustix::fs::listxattr(p("hello"), &mut [0u8; 0][..]).unwrap(),
        0
    );
    assert_eq!(
        rustix::fs::listxattr(p("hello"), &mut [0u8; 64][..]).unwrap(),
        0
    );
    assert_eq!(listed(&p("dir")), many());

    // Nothing changed.
    assert_eq!(listed(&dir), ["broken", "dir", "hello", "link", "slow"]);
    assert_eq!(std::fs::read(p("hello")).unwrap(), HELLO);
    let meta = std::fs::metadata(p("hello")).unwrap();
    assert_eq!(meta.permissions().mode() & 0o777, 0o555);

    drop(session);
    std::fs::remove_dir(&dir).unwrap();
}

/// EIO on Linux.
const fn libc_eio() -> i32 {
    5
}
