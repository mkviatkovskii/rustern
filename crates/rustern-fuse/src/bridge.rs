//! Async bridge over `fuser::Filesystem` (research R2, Principle VI): every request is
//! copied, run as its own task on the tokio runtime and replied to from that task, so a
//! slow read never holds up a `stat`.
//!
//! [`AsyncFilesystem`] follows `fuser::experimental::AsyncFilesystem` (method names and
//! argument order) but drops the unused request context and returns [`FsError`], which
//! keeps the cause next to the errno for the log line.

use std::ffi::{OsStr, OsString};
use std::fmt;
use std::future::Future;
use std::io;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use fuser::{
    BsdFileFlags, CopyFileRangeFlags, Errno, FileAttr, FileHandle, FileType, Filesystem,
    FopenFlags, Generation, INodeNo, InitFlags, KernelConfig, LockOwner, OpenFlags, RenameFlags,
    ReplyAttr, ReplyCreate, ReplyData, ReplyDirectory, ReplyDirectoryPlus, ReplyEmpty, ReplyEntry,
    ReplyOpen, ReplyStatfs, ReplyWrite, ReplyXattr, Request, TimeOrNow, WriteFlags,
};
use rustern_client::ClientError;
use tokio::runtime::Handle;

use crate::errno::{errno_name, errno_of};

/// An errno and, when it came from the client, its cause.
#[derive(Debug)]
pub struct FsError {
    /// Returned to the kernel.
    pub errno: Errno,
    /// Logged.
    pub cause: Option<ClientError>,
}

impl From<Errno> for FsError {
    fn from(errno: Errno) -> Self {
        Self { errno, cause: None }
    }
}

impl From<ClientError> for FsError {
    fn from(e: ClientError) -> Self {
        Self {
            errno: errno_of(&e),
            cause: Some(e),
        }
    }
}

impl fmt::Display for FsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.cause {
            Some(c) => c.fmt(f),
            None => f.write_str(errno_name(self.errno)),
        }
    }
}

/// Result of an [`AsyncFilesystem`] method.
pub type FsResult<T> = Result<T, FsError>;

/// Reply to `lookup`, and the attributes part of a `readdirplus` entry.
#[derive(Debug, Clone)]
pub struct LookupResponse {
    /// How long the kernel may cache the entry and attributes.
    pub ttl: Duration,
    /// Attributes.
    pub attr: FileAttr,
    /// Inode generation.
    pub generation: Generation,
}

/// Reply to `getattr`.
#[derive(Debug, Clone)]
pub struct GetAttrResponse {
    /// How long the kernel may cache the attributes.
    pub ttl: Duration,
    /// Attributes.
    pub attr: FileAttr,
}

/// Reply to `open` / `opendir`.
#[derive(Debug, Clone, Copy)]
pub struct OpenResponse {
    /// File handle passed back on later calls.
    pub fh: FileHandle,
    /// Open flags for the kernel.
    pub flags: FopenFlags,
}

/// One `readdir` entry.
#[derive(Debug, Clone)]
pub struct DirEntry {
    /// Inode.
    pub ino: INodeNo,
    /// Offset of the next entry.
    pub offset: u64,
    /// Type.
    pub kind: FileType,
    /// Name.
    pub name: OsString,
}

/// One `readdirplus` entry.
#[derive(Debug, Clone)]
pub struct DirEntryPlus {
    /// Offset of the next entry.
    pub offset: u64,
    /// Name.
    pub name: OsString,
    /// Entry and attributes.
    pub entry: LookupResponse,
}

/// Reply to `statfs`.
#[derive(Debug, Clone, Copy, Default)]
#[allow(missing_docs)]
pub struct StatfsResponse {
    pub blocks: u64,
    pub bfree: u64,
    pub bavail: u64,
    pub files: u64,
    pub ffree: u64,
    pub bsize: u32,
    pub namelen: u32,
    pub frsize: u32,
}

/// The read-only operations the bridge forwards.
pub trait AsyncFilesystem: Send + Sync + 'static {
    /// Buffer returned by `read`; the reply borrows it (R12).
    type Buf: AsRef<[u8]> + Send + 'static;

    /// Ask the kernel for `readdirplus` (directory entries with attributes).
    const READDIRPLUS: bool = false;

    /// Looks up `name` in `parent`.
    fn lookup(
        &self,
        parent: INodeNo,
        name: &OsStr,
    ) -> impl Future<Output = FsResult<LookupResponse>> + Send;
    /// Attributes of `ino`.
    fn getattr(
        &self,
        ino: INodeNo,
        fh: Option<FileHandle>,
    ) -> impl Future<Output = FsResult<GetAttrResponse>> + Send;
    /// Symlink target.
    fn readlink(&self, ino: INodeNo) -> impl Future<Output = FsResult<Vec<u8>>> + Send;
    /// Opens a file.
    fn open(
        &self,
        ino: INodeNo,
        flags: OpenFlags,
    ) -> impl Future<Output = FsResult<OpenResponse>> + Send;
    /// Reads up to `size` bytes at `offset`.
    fn read(
        &self,
        ino: INodeNo,
        fh: FileHandle,
        offset: u64,
        size: u32,
    ) -> impl Future<Output = FsResult<Self::Buf>> + Send;
    /// Closes a file.
    fn release(&self, ino: INodeNo, fh: FileHandle) -> impl Future<Output = FsResult<()>> + Send;
    /// Opens a directory.
    fn opendir(
        &self,
        ino: INodeNo,
        flags: OpenFlags,
    ) -> impl Future<Output = FsResult<OpenResponse>> + Send;
    /// Entries after `offset`; the bridge sends as many as fit.
    fn readdir(
        &self,
        ino: INodeNo,
        fh: FileHandle,
        offset: u64,
    ) -> impl Future<Output = FsResult<Vec<DirEntry>>> + Send;
    /// Entries with attributes after `offset`.
    fn readdirplus(
        &self,
        ino: INodeNo,
        fh: FileHandle,
        offset: u64,
    ) -> impl Future<Output = FsResult<Vec<DirEntryPlus>>> + Send;
    /// Closes a directory.
    fn releasedir(&self, ino: INodeNo, fh: FileHandle)
    -> impl Future<Output = FsResult<()>> + Send;
    /// Filesystem statistics.
    fn statfs(&self, ino: INodeNo) -> impl Future<Output = FsResult<StatfsResponse>> + Send;
}

/// `rustern_fuse_ops_total{op,errno}`.
pub const FUSE_OPS: &str = "rustern_fuse_ops_total";
/// `rustern_fuse_op_seconds{op}`.
pub const FUSE_OP_SECONDS: &str = "rustern_fuse_op_seconds";
/// `errno` label value of a successful operation.
pub const OK: &str = "OK";
/// Every `op` label value.
pub const OPS: [&str; 27] = [
    "lookup",
    "getattr",
    "readlink",
    "open",
    "read",
    "release",
    "opendir",
    "readdir",
    "readdirplus",
    "releasedir",
    "statfs",
    "getxattr",
    "listxattr",
    "create",
    "mknod",
    "mkdir",
    "unlink",
    "rmdir",
    "symlink",
    "link",
    "rename",
    "write",
    "setattr",
    "setxattr",
    "removexattr",
    "fallocate",
    "copy_file_range",
];

/// Kernel reply types: all can carry an errno.
trait ErrorReply {
    fn fail(self, e: Errno);
}

macro_rules! error_reply {
    ($($t:ty),*) => {$(
        impl ErrorReply for $t {
            fn fail(self, e: Errno) {
                self.error(e);
            }
        }
    )*};
}
error_reply!(
    ReplyCreate,
    ReplyWrite,
    ReplyXattr,
    ReplyEntry,
    ReplyAttr,
    ReplyData,
    ReplyOpen,
    ReplyEmpty,
    ReplyDirectory,
    ReplyDirectoryPlus,
    ReplyStatfs
);

/// Records one finished operation; logs failures (ENOENT and ENODATA, which are routine, at
/// debug; others at warn).
fn finish(op: &'static str, ino: INodeNo, start: Instant, err: Option<&FsError>) {
    let errno = err.map_or(OK, |e| errno_name(e.errno));
    metrics::counter!(FUSE_OPS, "op" => op, "errno" => errno).increment(1);
    metrics::histogram!(FUSE_OP_SECONDS, "op" => op).record(start.elapsed().as_secs_f64());
    match err {
        Some(e) if e.errno == Errno::ENOENT || e.errno == Errno::ENODATA => {
            tracing::debug!(op, ino = ino.0, errno, cause = %e, "operation failed");
        }
        Some(e) => tracing::warn!(op, ino = ino.0, errno, cause = %e, "operation failed"),
        None => {}
    }
}

/// Answers a request without the runtime: mutations (DEV-001) and extended attributes.
fn answer<R: ErrorReply>(op: &'static str, ino: INodeNo, errno: Errno, reply: R) {
    finish(op, ino, Instant::now(), Some(&FsError::from(errno)));
    reply.fail(errno);
}

/// `fuser::Filesystem` over an [`AsyncFilesystem`].
#[derive(Debug)]
pub struct Bridge<F> {
    fs: Arc<F>,
    rt: Handle,
    max_read: u32,
}

impl<F: AsyncFilesystem> Bridge<F> {
    /// Runs `fs` on `rt`, asking the kernel for reads of up to [`rustern_client::MAX_READ`].
    pub fn new(fs: Arc<F>, rt: Handle) -> Self {
        Self {
            fs,
            rt,
            max_read: rustern_client::MAX_READ,
        }
    }

    /// Spawns `call` and replies from the task: `ok` on success, the errno otherwise.
    fn run<T, R, Fut>(
        &self,
        op: &'static str,
        ino: INodeNo,
        reply: R,
        call: impl FnOnce(Arc<F>) -> Fut + Send + 'static,
        ok: impl FnOnce(R, T) + Send + 'static,
    ) where
        T: Send + 'static,
        R: ErrorReply + Send + 'static,
        Fut: Future<Output = FsResult<T>> + Send + 'static,
    {
        let fs = Arc::clone(&self.fs);
        self.rt.spawn(async move {
            let start = Instant::now();
            match call(fs).await {
                Ok(v) => {
                    finish(op, ino, start, None);
                    ok(reply, v);
                }
                Err(e) => {
                    finish(op, ino, start, Some(&e));
                    reply.fail(e.errno);
                }
            }
        });
    }
}

/// Background requests the kernel may keep in flight (kernel default 12).
const MAX_BACKGROUND: u16 = 256;

/// Sets a kernel limit to `want`, or the closest value the kernel allows.
macro_rules! negotiate {
    ($config:ident . $set:ident ( $want:expr )) => {
        match $config.$set($want) {
            Ok(_) => $want,
            Err(nearest) => {
                let _ = $config.$set(nearest);
                nearest
            }
        }
    };
}

impl<F: AsyncFilesystem> Filesystem for Bridge<F> {
    fn init(&mut self, _req: &Request, config: &mut KernelConfig) -> io::Result<()> {
        let readahead = negotiate!(config.set_max_readahead(self.max_read));
        let write = negotiate!(config.set_max_write(self.max_read));
        // Background (readahead) requests the kernel keeps in flight. The default 12 lets
        // readers queued on one file's memory limit hold every slot and stall other files
        // (FR-015); the congestion threshold follows at 3/4.
        let background = negotiate!(config.set_max_background(MAX_BACKGROUND));
        let plus = F::READDIRPLUS
            && config
                .add_capabilities(InitFlags::FUSE_DO_READDIRPLUS | InitFlags::FUSE_READDIRPLUS_AUTO)
                .is_ok();
        tracing::info!(
            max_readahead = readahead,
            max_write = write,
            max_background = background,
            readdirplus = plus,
            "FUSE session initialised"
        );
        Ok(())
    }

    fn lookup(&self, _req: &Request, parent: INodeNo, name: &OsStr, reply: ReplyEntry) {
        let name = name.to_os_string();
        self.run(
            "lookup",
            parent,
            reply,
            move |fs| async move { fs.lookup(parent, &name).await },
            |r, e: LookupResponse| r.entry(&e.ttl, &e.attr, e.generation),
        );
    }

    // `batch_forget` defaults to calling this. No inode table, so nothing to forget.
    fn forget(&self, _req: &Request, _ino: INodeNo, _nlookup: u64) {}

    fn getattr(&self, _req: &Request, ino: INodeNo, fh: Option<FileHandle>, reply: ReplyAttr) {
        self.run(
            "getattr",
            ino,
            reply,
            move |fs| async move { fs.getattr(ino, fh).await },
            |r, a: GetAttrResponse| r.attr(&a.ttl, &a.attr),
        );
    }

    fn readlink(&self, _req: &Request, ino: INodeNo, reply: ReplyData) {
        self.run(
            "readlink",
            ino,
            reply,
            move |fs| async move { fs.readlink(ino).await },
            |r, target: Vec<u8>| r.data(&target),
        );
    }

    fn open(&self, _req: &Request, ino: INodeNo, flags: OpenFlags, reply: ReplyOpen) {
        self.run(
            "open",
            ino,
            reply,
            move |fs| async move { fs.open(ino, flags).await },
            |r, o: OpenResponse| r.opened(o.fh, o.flags),
        );
    }

    fn read(
        &self,
        _req: &Request,
        ino: INodeNo,
        fh: FileHandle,
        offset: u64,
        size: u32,
        _flags: OpenFlags,
        _lock_owner: Option<LockOwner>,
        reply: ReplyData,
    ) {
        self.run(
            "read",
            ino,
            reply,
            move |fs| async move { fs.read(ino, fh, offset, size).await },
            |r, buf: F::Buf| r.data(buf.as_ref()),
        );
    }

    /// Read-only: nothing to flush. Answered here rather than with fuser's default ENOSYS,
    /// which logs a "not implemented" warning (and makes the kernel stop sending flushes).
    fn flush(
        &self,
        _req: &Request,
        _ino: INodeNo,
        _fh: FileHandle,
        _lock_owner: LockOwner,
        reply: ReplyEmpty,
    ) {
        reply.ok();
    }

    fn release(
        &self,
        _req: &Request,
        ino: INodeNo,
        fh: FileHandle,
        _flags: OpenFlags,
        _lock_owner: Option<LockOwner>,
        _flush: bool,
        reply: ReplyEmpty,
    ) {
        self.run(
            "release",
            ino,
            reply,
            move |fs| async move { fs.release(ino, fh).await },
            |r, ()| r.ok(),
        );
    }

    fn opendir(&self, _req: &Request, ino: INodeNo, flags: OpenFlags, reply: ReplyOpen) {
        self.run(
            "opendir",
            ino,
            reply,
            move |fs| async move { fs.opendir(ino, flags).await },
            |r, o: OpenResponse| r.opened(o.fh, o.flags),
        );
    }

    fn readdir(
        &self,
        _req: &Request,
        ino: INodeNo,
        fh: FileHandle,
        offset: u64,
        reply: ReplyDirectory,
    ) {
        self.run(
            "readdir",
            ino,
            reply,
            move |fs| async move { fs.readdir(ino, fh, offset).await },
            |mut r, entries: Vec<DirEntry>| {
                for e in entries {
                    if r.add(e.ino, e.offset, e.kind, &e.name) {
                        break;
                    }
                }
                r.ok();
            },
        );
    }

    fn readdirplus(
        &self,
        _req: &Request,
        ino: INodeNo,
        fh: FileHandle,
        offset: u64,
        reply: ReplyDirectoryPlus,
    ) {
        self.run(
            "readdirplus",
            ino,
            reply,
            move |fs| async move { fs.readdirplus(ino, fh, offset).await },
            |mut r, entries: Vec<DirEntryPlus>| {
                for e in entries {
                    let l = &e.entry;
                    if r.add(l.attr.ino, e.offset, &e.name, &l.ttl, &l.attr, l.generation) {
                        break;
                    }
                }
                r.ok();
            },
        );
    }

    fn releasedir(
        &self,
        _req: &Request,
        ino: INodeNo,
        fh: FileHandle,
        _flags: OpenFlags,
        reply: ReplyEmpty,
    ) {
        self.run(
            "releasedir",
            ino,
            reply,
            move |fs| async move { fs.releasedir(ino, fh).await },
            |r, ()| r.ok(),
        );
    }

    fn statfs(&self, _req: &Request, ino: INodeNo, reply: ReplyStatfs) {
        self.run(
            "statfs",
            ino,
            reply,
            move |fs| async move { fs.statfs(ino).await },
            |r, s: StatfsResponse| {
                r.statfs(
                    s.blocks, s.bfree, s.bavail, s.files, s.ffree, s.bsize, s.namelen, s.frsize,
                );
            },
        );
    }

    // DEV-001: the mount is read-only (the `ro` option makes the kernel refuse most of these
    // itself); every mutating request that still arrives is refused here.

    fn create(
        &self,
        _req: &Request,
        parent: INodeNo,
        _name: &OsStr,
        _mode: u32,
        _umask: u32,
        _flags: i32,
        reply: ReplyCreate,
    ) {
        answer("create", parent, Errno::EROFS, reply);
    }

    fn mknod(
        &self,
        _req: &Request,
        parent: INodeNo,
        _name: &OsStr,
        _mode: u32,
        _umask: u32,
        _rdev: u32,
        reply: ReplyEntry,
    ) {
        answer("mknod", parent, Errno::EROFS, reply);
    }

    fn mkdir(
        &self,
        _req: &Request,
        parent: INodeNo,
        _name: &OsStr,
        _mode: u32,
        _umask: u32,
        reply: ReplyEntry,
    ) {
        answer("mkdir", parent, Errno::EROFS, reply);
    }

    fn unlink(&self, _req: &Request, parent: INodeNo, _name: &OsStr, reply: ReplyEmpty) {
        answer("unlink", parent, Errno::EROFS, reply);
    }

    fn rmdir(&self, _req: &Request, parent: INodeNo, _name: &OsStr, reply: ReplyEmpty) {
        answer("rmdir", parent, Errno::EROFS, reply);
    }

    fn symlink(
        &self,
        _req: &Request,
        parent: INodeNo,
        _link_name: &OsStr,
        _target: &Path,
        reply: ReplyEntry,
    ) {
        answer("symlink", parent, Errno::EROFS, reply);
    }

    fn link(
        &self,
        _req: &Request,
        ino: INodeNo,
        _newparent: INodeNo,
        _newname: &OsStr,
        reply: ReplyEntry,
    ) {
        answer("link", ino, Errno::EROFS, reply);
    }

    fn rename(
        &self,
        _req: &Request,
        parent: INodeNo,
        _name: &OsStr,
        _newparent: INodeNo,
        _newname: &OsStr,
        _flags: RenameFlags,
        reply: ReplyEmpty,
    ) {
        answer("rename", parent, Errno::EROFS, reply);
    }

    fn write(
        &self,
        _req: &Request,
        ino: INodeNo,
        _fh: FileHandle,
        _offset: u64,
        _data: &[u8],
        _write_flags: WriteFlags,
        _flags: OpenFlags,
        _lock_owner: Option<LockOwner>,
        reply: ReplyWrite,
    ) {
        answer("write", ino, Errno::EROFS, reply);
    }

    fn setattr(
        &self,
        _req: &Request,
        ino: INodeNo,
        _mode: Option<u32>,
        _uid: Option<u32>,
        _gid: Option<u32>,
        _size: Option<u64>,
        _atime: Option<TimeOrNow>,
        _mtime: Option<TimeOrNow>,
        _ctime: Option<SystemTime>,
        _fh: Option<FileHandle>,
        _crtime: Option<SystemTime>,
        _chgtime: Option<SystemTime>,
        _bkuptime: Option<SystemTime>,
        _flags: Option<BsdFileFlags>,
        reply: ReplyAttr,
    ) {
        answer("setattr", ino, Errno::EROFS, reply);
    }

    fn setxattr(
        &self,
        _req: &Request,
        ino: INodeNo,
        _name: &OsStr,
        _value: &[u8],
        _flags: i32,
        _position: u32,
        reply: ReplyEmpty,
    ) {
        answer("setxattr", ino, Errno::EROFS, reply);
    }

    fn removexattr(&self, _req: &Request, ino: INodeNo, _name: &OsStr, reply: ReplyEmpty) {
        answer("removexattr", ino, Errno::EROFS, reply);
    }

    fn fallocate(
        &self,
        _req: &Request,
        ino: INodeNo,
        _fh: FileHandle,
        _offset: u64,
        _length: u64,
        _mode: i32,
        reply: ReplyEmpty,
    ) {
        answer("fallocate", ino, Errno::EROFS, reply);
    }

    fn copy_file_range(
        &self,
        _req: &Request,
        _ino_in: INodeNo,
        _fh_in: FileHandle,
        _offset_in: u64,
        ino_out: INodeNo,
        _fh_out: FileHandle,
        _offset_out: u64,
        _len: u64,
        _flags: CopyFileRangeFlags,
        reply: ReplyWrite,
    ) {
        answer("copy_file_range", ino_out, Errno::EROFS, reply);
    }

    /// TernFS has no extended attributes: every name is absent (not "unsupported", which
    /// would make tools like `cp -a` warn).
    fn getxattr(&self, _req: &Request, ino: INodeNo, _name: &OsStr, _size: u32, reply: ReplyXattr) {
        answer("getxattr", ino, Errno::ENODATA, reply);
    }

    /// An empty list.
    fn listxattr(&self, _req: &Request, ino: INodeNo, size: u32, reply: ReplyXattr) {
        finish("listxattr", ino, Instant::now(), None);
        if size == 0 {
            reply.size(0);
        } else {
            reply.data(&[]);
        }
    }
}
