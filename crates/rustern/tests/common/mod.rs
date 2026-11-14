//! Shared by the cluster tests: environment, the `rustern` mount, tree walks and cluster
//! process lookup.
#![allow(
    dead_code,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)] // test code

use std::ffi::OsString;
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Read};
use std::net::{TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant, SystemTime};

/// Cluster coordinates from the environment (tasks.md, Conventions).
#[derive(Debug, Clone)]
pub struct Env {
    /// `RUSTERN_TEST_REGISTRY`, default `127.0.0.1:10001`.
    pub registry: String,
    /// `RUSTERN_TEST_DATA_DIR`, default `ternfs/mnt` at the workspace root.
    pub data_dir: PathBuf,
}

/// Reads the environment and checks the registry accepts TCP; fails loudly otherwise.
pub fn env() -> Env {
    let registry =
        std::env::var("RUSTERN_TEST_REGISTRY").unwrap_or_else(|_| "127.0.0.1:10001".into());
    let data_dir = std::env::var_os("RUSTERN_TEST_DATA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| workspace_root().join("ternfs/mnt"));
    let addr = registry
        .to_socket_addrs()
        .ok()
        .and_then(|mut a| a.next())
        .unwrap_or_else(|| panic!("RUSTERN_TEST_REGISTRY={registry} is not an address"));
    if let Err(e) = TcpStream::connect_timeout(&addr, Duration::from_secs(2)) {
        panic!(
            "registry {registry} (RUSTERN_TEST_REGISTRY) does not accept TCP: {e}; \
             start the cluster with scripts/cluster.sh start"
        );
    }
    Env { registry, data_dir }
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

/// The reference corpus, read through ternfuse.
pub fn ref_corpus() -> PathBuf {
    let p = env().data_dir.join("fuse_loc1/mnt/corpus");
    assert!(
        p.join("corpus.sha256").is_file(),
        "no corpus at {} (RUSTERN_TEST_DATA_DIR); run scripts/make-corpus.sh",
        p.display()
    );
    p
}

static MOUNTS: AtomicUsize = AtomicUsize::new(0);

/// A running `rustern` mount, unmounted on drop.
pub struct Mount {
    /// Mount point.
    pub path: PathBuf,
    child: Child,
    stderr: Arc<Mutex<Vec<String>>>,
}

impl Mount {
    /// Mounts the cluster on a fresh directory with `extra` flags; waits for `mounted at`.
    pub fn new(extra: &[&str]) -> Self {
        Self::try_new(extra).unwrap_or_else(|e| panic!("{e}"))
    }

    /// Like [`Mount::new`] but returns the collected stderr when the daemon does not mount.
    pub fn try_new(extra: &[&str]) -> Result<Self, String> {
        let env = env();
        let n = MOUNTS.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("rustern-test-{}-{n}", std::process::id()));
        fs::create_dir_all(&path).unwrap();
        let mut child = Command::new(env!("CARGO_BIN_EXE_rustern"))
            .args(["-registry", &env.registry])
            .args(extra)
            .arg(&path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let stderr = Arc::new(Mutex::new(Vec::new()));
        let (tx, rx) = mpsc::channel();
        let pipe = child.stderr.take().unwrap();
        let lines = Arc::clone(&stderr);
        std::thread::spawn(move || {
            for line in BufReader::new(pipe).lines() {
                let Ok(line) = line else { break };
                let mounted = line.contains("mounted at");
                // Store before signalling, so `stderr()` holds the line once `new` returns.
                lines.lock().unwrap().push(line);
                if mounted {
                    let _ = tx.send(());
                }
            }
        });
        let start = Instant::now();
        loop {
            if rx.recv_timeout(Duration::from_millis(50)).is_ok() {
                return Ok(Self {
                    path,
                    child,
                    stderr,
                });
            }
            let exited = child.try_wait().unwrap();
            if exited.is_some() || start.elapsed() > Duration::from_secs(5) {
                if exited.is_none() {
                    let _ = child.kill();
                    let _ = child.wait();
                }
                std::thread::sleep(Duration::from_millis(100)); // let the reader drain
                let _ = fs::remove_dir(&path);
                let log = stderr.lock().unwrap().join("\n");
                return Err(format!(
                    "rustern did not mount ({exited:?}); stderr:\n{log}"
                ));
            }
        }
    }

    /// Stderr lines so far.
    pub fn stderr(&self) -> Vec<String> {
        self.stderr.lock().unwrap().clone()
    }

    /// `path` inside the mount.
    pub fn join(&self, p: impl AsRef<Path>) -> PathBuf {
        self.path.join(p)
    }

    /// The daemon's pid.
    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    /// Sends `sig` to the daemon.
    pub fn signal(&self, sig: rustix::process::Signal) {
        let pid = rustix::process::Pid::from_child(&self.child);
        rustix::process::kill_process(pid, sig).unwrap();
    }

    /// The daemon's exit status, waiting up to `wait`.
    pub fn wait_exit(&mut self, wait: Duration) -> Option<ExitStatus> {
        let start = Instant::now();
        loop {
            if let Some(s) = self.child.try_wait().unwrap() {
                return Some(s);
            }
            if start.elapsed() > wait {
                return None;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// Sends SIGUSR1 and returns the log lines of the metrics dump it produces.
    pub fn metrics(&self) -> Vec<String> {
        let before = self.stderr().len();
        self.signal(rustix::process::Signal::USR1);
        let start = Instant::now();
        loop {
            if let Some(i) = self.stderr()[before..]
                .iter()
                .position(|l| l.contains("metrics:"))
            {
                std::thread::sleep(Duration::from_millis(200));
                return self.stderr()[before + i..].to_vec();
            }
            assert!(start.elapsed() < Duration::from_secs(5), "no metrics dump");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// Waits up to 2 s for a stderr line containing `needle`.
    pub fn wait_stderr(&self, needle: &str) -> bool {
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(2) {
            if self.stderr().iter().any(|l| l.contains(needle)) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        false
    }
}

/// The mount point's line in `/proc/self/mountinfo`, if mounted.
pub fn mountinfo(path: &Path) -> Option<String> {
    let want = format!(" {} ", path.display());
    fs::read_to_string("/proc/self/mountinfo")
        .unwrap()
        .lines()
        .find(|l| l.contains(&want))
        .map(str::to_owned)
}

/// Root of the ternfuse mount (`$RUSTERN_TEST_DATA_DIR/fuse_loc1/mnt`).
pub fn ternfuse_root() -> PathBuf {
    env().data_dir.join("fuse_loc1/mnt")
}

impl Drop for Mount {
    fn drop(&mut self) {
        if matches!(self.child.try_wait(), Ok(Some(_))) {
            if mountinfo(&self.path).is_some() {
                let _ = Command::new("fusermount3")
                    .arg("-u")
                    .arg(&self.path)
                    .status();
            }
            let _ = fs::remove_dir(&self.path);
            return;
        }
        let pid = rustix::process::Pid::from_child(&self.child);
        let _ = rustix::process::kill_process(pid, rustix::process::Signal::TERM);
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(10) {
            if matches!(self.child.try_wait(), Ok(Some(_))) {
                let _ = fs::remove_dir(&self.path);
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let _ = Command::new("fusermount3")
            .arg("-u")
            .arg(&self.path)
            .status();
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = fs::remove_dir(&self.path);
    }
}

/// One entry of a tree walk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Walked {
    /// Path relative to the walk root.
    pub path: OsString,
    /// Type (symlinks are not followed).
    pub kind: fs::FileType,
    /// Size (0 for directories).
    pub size: u64,
    /// Modification time.
    pub mtime: SystemTime,
}

/// Every entry under `root` (not `root` itself), sorted by path; symlinks not followed.
pub fn walk(root: &Path) -> Vec<Walked> {
    let mut out = Vec::new();
    let mut stack = vec![PathBuf::new()];
    while let Some(rel) = stack.pop() {
        for e in fs::read_dir(root.join(&rel)).unwrap() {
            let e = e.unwrap();
            let rel = rel.join(e.file_name());
            let m = fs::symlink_metadata(root.join(&rel)).unwrap();
            let kind = m.file_type();
            if kind.is_dir() {
                stack.push(rel.clone());
            }
            out.push(Walked {
                path: rel.into_os_string(),
                kind,
                size: if kind.is_dir() { 0 } else { m.len() },
                mtime: m.modified().unwrap(),
            });
        }
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

/// Compares two files in 1 MiB chunks; `Err` names the first differing offset.
pub fn compare_files(a: &Path, b: &Path) -> Result<(), String> {
    const CHUNK: usize = 1 << 20;
    let (mut fa, mut fb) = (File::open(a).unwrap(), File::open(b).unwrap());
    let (mut ba, mut bb) = (vec![0u8; CHUNK], vec![0u8; CHUNK]);
    let mut offset = 0u64;
    loop {
        let na = read_full(&mut fa, &mut ba).map_err(|e| format!("{}: {e}", a.display()))?;
        let nb = read_full(&mut fb, &mut bb).map_err(|e| format!("{}: {e}", b.display()))?;
        if ba[..na] != bb[..nb] {
            let at = ba[..na]
                .iter()
                .zip(&bb[..nb])
                .take_while(|(x, y)| x == y)
                .count();
            return Err(format!(
                "{} and {} differ at offset {} (lengths {na}/{nb} in this chunk)",
                a.display(),
                b.display(),
                offset + at as u64
            ));
        }
        if na == 0 {
            return Ok(());
        }
        offset += na as u64;
    }
}

fn read_full(f: &mut File, buf: &mut [u8]) -> std::io::Result<usize> {
    let mut n = 0;
    while n < buf.len() {
        match f.read(&mut buf[n..])? {
            0 => break,
            k => n += k,
        }
    }
    Ok(n)
}

/// Pids of `process` (binary name) whose command line contains `arg` as whole tokens,
/// e.g. `pgrep_tern("ternshard", "-shard 3")`.
pub fn pgrep_tern(process: &str, arg: &str) -> Vec<u32> {
    let want = format!(" {arg} ");
    let mut pids = Vec::new();
    for e in fs::read_dir("/proc").unwrap().flatten() {
        let Some(pid) = e.file_name().to_str().and_then(|s| s.parse::<u32>().ok()) else {
            continue;
        };
        let Ok(cmdline) = fs::read(e.path().join("cmdline")) else {
            continue;
        };
        let argv: Vec<String> = cmdline
            .split(|b| *b == 0)
            .filter(|a| !a.is_empty())
            .map(|a| String::from_utf8_lossy(a).into_owned())
            .collect();
        let Some(bin) = argv.first() else { continue };
        if Path::new(bin).file_name().is_some_and(|n| n == process)
            && format!(" {} ", argv.join(" ")).contains(&want)
        {
            pids.push(pid);
        }
    }
    pids
}

/// The on-disk file of `block_id`: `$RUSTERN_TEST_DATA_DIR/bs_*/*/with_crc/**/<016x>`.
pub fn find_block_file(block_id: u64) -> Option<PathBuf> {
    let name = format!("{block_id:016x}");
    let data = env().data_dir;
    let mut stack: Vec<PathBuf> = fs::read_dir(&data)
        .ok()?
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with("bs_"))
        .flat_map(|bs| fs::read_dir(bs.path()).into_iter().flatten().flatten())
        .map(|d| d.path().join("with_crc"))
        .filter(|p| p.is_dir())
        .collect();
    while let Some(dir) = stack.pop() {
        for e in fs::read_dir(&dir).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if e.file_name() == name.as_str() {
                return Some(p);
            }
        }
    }
    None
}

/// Sum of every sample of metric `name` (all label sets) in a dump; 0 when absent.
pub fn metric_sum(dump: &[String], name: &str) -> f64 {
    dump.iter()
        .filter_map(|l| {
            let (k, v) = l.trim().rsplit_once(' ')?;
            let k = k.split('{').next()?;
            (k == name).then(|| v.parse::<f64>().ok()).flatten()
        })
        .sum()
}
