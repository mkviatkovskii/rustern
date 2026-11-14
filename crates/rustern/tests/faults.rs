//! T097–T099 (US3, SC-002, SC-003, FR-010, FR-016, FR-017): block, metadata and process
//! faults, concurrent changes, and secrecy of logs and metrics. Every test runs on a
//! fresh mount with short deadlines and, when the mount is still alive, checks that it
//! answers `stat` on its root afterwards.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)] // test code

mod common;

/// nextest runs `cluster_serial::*` one at a time (`.config/nextest.toml`); the mutex does
/// the same under `cargo test`, since these tests stop shared cluster processes.
mod cluster_serial {
    use std::collections::HashSet;
    use std::fs::{self, File, OpenOptions};
    use std::io;
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::{FileExt, MetadataExt, OpenOptionsExt};
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::sync::{Mutex, MutexGuard};
    use std::time::{Duration, Instant};

    use rustern_client::{Client, ClientConfig};
    use rustern_core::inode::TernInode;
    use rustix::process::{Pid, Signal, kill_process};

    use super::common::{
        Mount, compare_files, env, find_block_file, pgrep_tern, ref_corpus, ternfuse_root, walk,
    };

    const FLAGS: &[&str] = &[
        "-overall-block-timeout",
        "5s",
        "-overall-shard-timeout",
        "3s",
    ];
    const BLOCK_DEADLINE: Duration = Duration::from_secs(5);
    const SHARD_DEADLINE: Duration = Duration::from_secs(3);
    const SLACK: Duration = Duration::from_secs(1);
    const EIO: i32 = 5;
    const ENOENT: i32 = 2;
    const ETIMEDOUT: i32 = 110;
    const ENOTCONN: i32 = 107;
    /// Files that may hold mirrored spans (sizes between the inline and RS thresholds).
    const MIRRORED_CANDIDATES: [&str; 5] = [
        "sizes/size_4096",
        "sizes/size_65536",
        "sizes/size_65537",
        "sizes/size_1048576",
        "sizes/size_5120000",
    ];
    /// Files that may hold RS spans.
    const RS_CANDIDATES: [&str; 3] = [
        "sizes/size_30000000",
        "sizes/size_5120001",
        "sizes/size_104857600",
    ];

    static SERIAL: Mutex<()> = Mutex::new(());

    fn serial() -> MutexGuard<'static, ()> {
        SERIAL
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// The mount still answers `stat` on its root (SC-003).
    fn alive(m: &Mount) {
        fs::metadata(&m.path).unwrap();
    }

    fn errno<T>(r: io::Result<T>) -> Option<i32> {
        r.err().and_then(|e| e.raw_os_error())
    }

    fn read_at(p: &Path, off: u64, len: usize) -> io::Result<Vec<u8>> {
        let f = File::open(p)?;
        let mut b = vec![0u8; len];
        let n = f.read_at(&mut b, off)?;
        b.truncate(n);
        Ok(b)
    }

    /// One O_DIRECT read: exactly one FUSE read, no readahead or retry by the kernel.
    fn read_direct(p: &Path, off: u64, len: usize) -> io::Result<usize> {
        let f = OpenOptions::new()
            .read(true)
            .custom_flags(rustix::fs::OFlags::DIRECT.bits() as i32)
            .open(p)?;
        let mut b = vec![0u8; len];
        f.read_at(&mut b, off)
    }

    /// A block span of a corpus file: offset, mirrored, `(block id, service id)`.
    type BlockSpan = (u64, bool, Vec<(u64, u64)>);

    /// Inode and block spans of `corpus/<rel>`, through the client library.
    fn layout(rel: &str) -> (u64, Vec<BlockSpan>) {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let client = Client::connect(ClientConfig::new(env().registry))
                .await
                .unwrap();
            let mut id = TernInode::ROOT;
            for c in Path::new("corpus").join(rel).components() {
                let r = client.lookup(id, c.as_os_str().as_bytes()).await.unwrap();
                id = TernInode::try_from(r.target_id).unwrap();
            }
            let h = client.open_file(id).await.unwrap();
            (id.id().0, h.block_layout())
        })
    }

    /// The first file among `candidates` with a span of the wanted kind.
    fn find_span(candidates: &[&'static str], mirrored: bool) -> (&'static str, u64, BlockSpan) {
        for rel in candidates {
            let (inode, spans) = layout(rel);
            if let Some(s) = spans.into_iter().find(|s| s.1 == mirrored) {
                return (rel, inode, s);
            }
        }
        panic!("no span with mirrored={mirrored} in {candidates:?}");
    }

    /// `bs_N` holding a block, from its on-disk path.
    fn block_dir(block: u64) -> String {
        let p = find_block_file(block).unwrap_or_else(|| panic!("no file for block {block:x}"));
        p.strip_prefix(env().data_dir)
            .unwrap()
            .components()
            .next()
            .unwrap()
            .as_os_str()
            .to_string_lossy()
            .into_owned()
    }

    /// Pid of the `ternblocks` process storing `block`.
    fn blocks_pid(block: u64) -> u32 {
        let log = env().data_dir.join(block_dir(block)).join("log");
        let pids = pgrep_tern("ternblocks", &format!("-log-file {}", log.display()));
        assert_eq!(pids.len(), 1, "ternblocks for {}", log.display());
        pids[0]
    }

    /// Byte 0 of a block's file flipped; restored on drop.
    struct Corrupt {
        path: PathBuf,
        byte: u8,
    }

    impl Corrupt {
        fn new(block: u64) -> Self {
            let path = find_block_file(block).unwrap_or_else(|| panic!("block {block:x}"));
            let f = OpenOptions::new()
                .read(true)
                .write(true)
                .open(&path)
                .unwrap();
            let mut b = [0u8];
            f.read_exact_at(&mut b, 0).unwrap();
            f.write_all_at(&[!b[0]], 0).unwrap();
            f.sync_all().unwrap();
            Self { path, byte: b[0] }
        }
    }

    impl Drop for Corrupt {
        fn drop(&mut self) {
            let f = OpenOptions::new().write(true).open(&self.path).unwrap();
            f.write_all_at(&[self.byte], 0).unwrap();
            f.sync_all().unwrap();
        }
    }

    /// Processes stopped with SIGSTOP; continued on drop.
    struct Stopped(Vec<u32>);

    impl Stopped {
        fn new(pids: Vec<u32>) -> Self {
            assert!(!pids.is_empty(), "nothing to stop");
            for p in &pids {
                kill_process(Pid::from_raw(*p as i32).unwrap(), Signal::STOP).unwrap();
            }
            Self(pids)
        }
    }

    impl Drop for Stopped {
        fn drop(&mut self) {
            for p in &self.0 {
                let _ = kill_process(Pid::from_raw(*p as i32).unwrap(), Signal::CONT);
            }
        }
    }

    fn alert_lines(m: &Mount) -> Vec<String> {
        m.stderr()
            .into_iter()
            .filter(|l| l.contains("alert=true"))
            .collect()
    }

    /// Reads `[off, off+len)` of `rel` on the mount and through ternfuse; they must match.
    fn same_range(m: &Mount, rel: &str, off: u64, len: usize) {
        let got = read_at(&m.join("corpus").join(rel), off, len).unwrap();
        let want = read_at(&ref_corpus().join(rel), off, len).unwrap();
        assert!(got == want, "{rel} [{off}, +{len}) differs");
    }

    // ---- T097: block faults ----

    #[test]
    #[ignore = "needs cluster: run via scripts/gate.sh"]
    fn corrupt_block_eio() {
        let _g = serial();
        let (rel, inode, (off, _, blocks)) = find_span(&RS_CANDIDATES, false);
        let block = blocks[0].0;
        let _c = Corrupt::new(block);
        let m = Mount::new(FLAGS);
        let e = read_at(&m.join("corpus").join(rel), off, 4096);
        assert_eq!(errno(e), Some(EIO), "{rel} at {off}");
        assert!(m.wait_stderr("alert=true"), "no alert line");
        let line = alert_lines(&m).into_iter().next().unwrap();
        for want in [
            format!("inode={inode}"),
            format!("block_id={block}"),
            "page=".to_owned(),
        ] {
            assert!(line.contains(&want), "{want} missing: {line}");
        }
        // Other spans of the file and other files are fine.
        let (_, spans) = layout(rel);
        for (o, _, _) in spans.iter().filter(|s| s.0 != off).take(2) {
            same_range(&m, rel, *o, 65536);
        }
        compare_files(
            &m.join("corpus/sizes/size_65537"),
            &ref_corpus().join("sizes/size_65537"),
        )
        .unwrap();
        alive(&m);
    }

    #[test]
    #[ignore = "needs cluster: run via scripts/gate.sh"]
    fn mirrored_failover() {
        let _g = serial();
        let (rel, inode, (_, _, blocks)) = find_span(&MIRRORED_CANDIDATES, true);
        assert!(blocks.len() >= 2, "{rel}: one copy");
        let _c = Corrupt::new(blocks[0].0);
        let m = Mount::new(FLAGS);
        compare_files(&m.join("corpus").join(rel), &ref_corpus().join(rel)).unwrap();
        assert!(m.wait_stderr("mirrored copy failed"), "failover not logged");
        let line = m
            .stderr()
            .into_iter()
            .find(|l| l.contains("mirrored copy failed"))
            .unwrap();
        for want in [
            format!("inode={inode}"),
            format!("block_id={}", blocks[0].0),
            format!("service={}", blocks[0].1),
        ] {
            assert!(line.contains(&want), "{want} missing: {line}");
        }
        assert!(
            alert_lines(&m).is_empty(),
            "a recovered read is not an alert"
        );
        alive(&m);
    }

    #[test]
    #[ignore = "needs cluster: run via scripts/gate.sh"]
    fn dead_block_service_eio() {
        let _g = serial();
        let (rel, _, (off, _, blocks)) = find_span(&RS_CANDIDATES, false);
        let pid = blocks_pid(blocks[0].0);
        // A mirrored span whose first copy lives on another process.
        let mut other = None;
        for cand in MIRRORED_CANDIDATES {
            let (_, spans) = layout(cand);
            if spans
                .iter()
                .any(|s| s.1 && blocks_pid(s.2[0].0) != pid && s.2.len() >= 2)
                && spans
                    .iter()
                    .all(|s| s.2.iter().any(|b| blocks_pid(b.0) != pid))
            {
                other = Some(cand);
                break;
            }
        }
        let m = Mount::new(FLAGS);
        let _s = Stopped::new(vec![pid]);
        // O_DIRECT: a buffered read may reach the filesystem twice (readahead, then the
        // kernel's synchronous retry), each waiting for the deadline.
        let start = Instant::now();
        let e = read_direct(&m.join("corpus").join(rel), off, 4096);
        let took = start.elapsed();
        assert_eq!(errno(e), Some(EIO), "{rel} at {off}");
        assert!(took <= BLOCK_DEADLINE + SLACK, "EIO after {took:?}");
        if let Some(cand) = other {
            compare_files(&m.join("corpus").join(cand), &ref_corpus().join(cand)).unwrap();
        } else {
            eprintln!("no mirrored file with copies off pid {pid}; skipped that half");
        }
        alive(&m);
    }

    #[test]
    #[ignore = "needs cluster: run via scripts/gate.sh"]
    fn mirrored_all_copies_alert() {
        let _g = serial();
        let (rel, inode, (off, _, blocks)) = find_span(&MIRRORED_CANDIDATES, true);
        let _c: Vec<Corrupt> = blocks.iter().map(|b| Corrupt::new(b.0)).collect();
        let m = Mount::new(FLAGS);
        let e = read_direct(&m.join("corpus").join(rel), off, 4096);
        assert_eq!(errno(e), Some(EIO), "{rel} at {off}");
        assert!(m.wait_stderr("alert=true"));
        std::thread::sleep(Duration::from_millis(200));
        let alerts = alert_lines(&m);
        assert_eq!(alerts.len(), 1, "{alerts:#?}");
        assert!(
            alerts[0].contains(&format!("inode={inode}")),
            "{}",
            alerts[0]
        );
        alive(&m);
    }

    // ---- T098: metadata and process faults ----

    /// A directory, two names in it not yet looked up on the mount, and its shard's pid.
    fn shard_target(m: &Mount) -> (PathBuf, [String; 2], u32) {
        let rel = Path::new("tree/d00");
        let dir = m.join("corpus").join(rel);
        let shard = fs::metadata(&dir).unwrap().ino().to_le_bytes()[0];
        let mut names: Vec<String> = fs::read_dir(ref_corpus().join(rel))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        let pids = pgrep_tern("ternshard", &format!("-shard {shard}"));
        assert_eq!(pids.len(), 1, "ternshard -shard {shard}");
        (dir, [names[0].clone(), names[1].clone()], pids[0])
    }

    #[test]
    #[ignore = "needs cluster: run via scripts/gate.sh"]
    fn stopped_shard() {
        let _g = serial();
        let m = Mount::new(FLAGS);
        let (dir, [name, _], pid) = shard_target(&m);
        {
            let _s = Stopped::new(vec![pid]);
            let start = Instant::now();
            let r = fs::symlink_metadata(dir.join(&name));
            let took = start.elapsed();
            assert_eq!(errno(r), Some(ETIMEDOUT));
            assert!(took <= SHARD_DEADLINE + SLACK, "ETIMEDOUT after {took:?}");
        }
        fs::symlink_metadata(dir.join(&name)).unwrap();
        alive(&m);
    }

    #[test]
    #[ignore = "needs cluster: run via scripts/gate.sh"]
    fn registry_stopped_after_start() {
        let _g = serial();
        let m = Mount::new(FLAGS);
        let rels = ["sizes/size_65537", "sizes/size_5120001", "names/"];
        let files: Vec<PathBuf> = rels[..2].iter().map(|r| m.join("corpus").join(r)).collect();
        for f in &files {
            fs::metadata(f).unwrap();
        }
        let _s = Stopped::new(pgrep_tern("ternregistry", "-replica 0"));
        for r in &rels[..2] {
            compare_files(&m.join("corpus").join(r), &ref_corpus().join(r)).unwrap();
        }
        alive(&m);
    }

    #[test]
    #[ignore = "needs cluster: run via scripts/gate.sh"]
    fn interrupt_bounded_by_deadline() {
        let _g = serial();
        let m = Mount::new(FLAGS);
        let (dir, [_, name], pid) = shard_target(&m);
        let _s = Stopped::new(vec![pid]);
        let start = Instant::now();
        let out = Command::new("timeout")
            .args(["-s", "INT", "1", "stat"])
            .arg(dir.join(name))
            .output()
            .unwrap();
        let took = start.elapsed();
        assert!(!out.status.success());
        assert!(
            took <= SHARD_DEADLINE + SLACK,
            "stat returned after {took:?}"
        );
        drop(_s);
        alive(&m);
    }

    #[test]
    #[ignore = "needs cluster: run via scripts/gate.sh"]
    fn sigkill_cleanup() {
        let _g = serial();
        let mut m = Mount::new(FLAGS);
        m.signal(Signal::KILL);
        assert!(m.wait_exit(Duration::from_secs(5)).is_some());
        assert_eq!(errno(fs::metadata(&m.path)), Some(ENOTCONN));
        let st = Command::new("fusermount3")
            .arg("-u")
            .arg(&m.path)
            .status()
            .unwrap();
        assert!(st.success());
        assert!(fs::metadata(&m.path).unwrap().is_dir());
    }

    // ---- T099: concurrent changes and secrecy ----

    fn scratch(name: &str) -> PathBuf {
        let p = ternfuse_root()
            .join("scratch")
            .join(format!("{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&p);
        let _ = fs::remove_file(&p);
        p
    }

    #[test]
    #[ignore = "needs cluster: run via scripts/gate.sh"]
    fn dir_removed_while_listing() {
        let _g = serial();
        let dir = scratch("rm-me");
        fs::create_dir(&dir).unwrap();
        // 600 files span many ReadDir batches (a few dozen short names per 1472-byte response) and
        // several kernel readdir calls. ternfuse creates or unlinks only about 15 files per
        // second however many callers there are, so 3,000 (as first planned) took 10 minutes.
        let files: Vec<PathBuf> = (0..600).map(|i| dir.join(format!("f{i:04}"))).collect();
        let parallel = |op: fn(&Path)| {
            std::thread::scope(|s| {
                for chunk in files.chunks(50) {
                    s.spawn(move || chunk.iter().for_each(|p| op(p)));
                }
            });
        };
        parallel(|p| drop(File::create(p).unwrap()));
        let m = Mount::new(FLAGS);
        let on_mount = m.join("scratch").join(dir.file_name().unwrap());
        let mut it = fs::read_dir(&on_mount).unwrap();
        let mut names = Vec::new();
        for _ in 0..100 {
            names.push(it.next().unwrap().unwrap().file_name());
        }
        parallel(|p| fs::remove_file(p).unwrap());
        fs::remove_dir(&dir).unwrap();
        for e in it {
            match e {
                Ok(e) => names.push(e.file_name()),
                Err(e) => {
                    assert_eq!(e.raw_os_error(), Some(ENOENT), "{e}");
                    break;
                }
            }
        }
        let unique: HashSet<_> = names.iter().collect();
        assert_eq!(unique.len(), names.len(), "duplicate names");
        assert!(names.len() <= files.len());
        alive(&m);
    }

    #[test]
    #[ignore = "needs cluster: run via scripts/gate.sh"]
    fn open_file_deleted() {
        let _g = serial();
        let src = ref_corpus().join("sizes/size_30000000");
        let copy = scratch("del-me");
        let want = fs::read(&src).unwrap();
        // Plain writes: ternfuse supports neither copy_file_range nor fchmod.
        fs::write(&copy, &want).unwrap();
        let m = Mount::new(FLAGS);
        let f = File::open(m.join("scratch").join(copy.file_name().unwrap())).unwrap();
        const MIB: usize = 1 << 20;
        let mut buf = vec![0u8; MIB];
        let n = f.read_at(&mut buf, 0).unwrap();
        assert_eq!(&buf[..n], &want[..n]);
        fs::remove_file(&copy).unwrap();
        let mut off = n;
        let mut failed = None;
        while off < want.len() {
            match f.read_at(&mut buf, off as u64) {
                Ok(0) => break,
                Ok(k) => {
                    assert!(buf[..k] == want[off..off + k], "different bytes at {off}");
                    off += k;
                }
                Err(e) => {
                    failed = e.raw_os_error();
                    break;
                }
            }
        }
        assert!(
            failed.is_none() && off == want.len() || matches!(failed, Some(ENOENT | EIO)),
            "stopped at {off} with {failed:?}"
        );
        alive(&m);
    }

    /// Every `<name>=<value>` or `<name>: <value>` whose name contains a secret word.
    fn secret_values(line: &str) -> Vec<(String, String)> {
        let is_ident = |c: char| c.is_ascii_alphanumeric() || c == '_';
        let mut out = Vec::new();
        for word in ["cookie", "certificate", "secret", "proof"] {
            for (at, _) in line.match_indices(word) {
                let start = line[..at].rfind(|c| !is_ident(c)).map_or(0, |i| i + 1);
                let end = line[at..]
                    .find(|c| !is_ident(c))
                    .map_or(line.len(), |i| at + i);
                let rest = &line[end..];
                let value = rest
                    .strip_prefix('=')
                    .or_else(|| rest.strip_prefix(": "))
                    .map(|v| v.split([' ', ',', '}', ')']).next().unwrap_or(""));
                if let Some(v) = value {
                    out.push((line[start..end].to_owned(), v.to_owned()));
                }
            }
        }
        out
    }

    /// Every `key="value"` label pair in a Prometheus text line.
    fn labels(line: &str) -> Vec<(String, String)> {
        let Some(inner) = line
            .split_once('{')
            .and_then(|(_, r)| r.rsplit_once('}'))
            .map(|(i, _)| i)
        else {
            return Vec::new();
        };
        inner
            .split("\",")
            .filter_map(|kv| {
                let (k, v) = kv.split_once('=')?;
                Some((k.trim().to_owned(), v.trim_matches('"').to_owned()))
            })
            .collect()
    }

    #[test]
    #[ignore = "needs cluster: run via scripts/gate.sh"]
    fn logs_have_no_secrets() {
        let _g = serial();
        let (rel, _, (off, _, blocks)) = find_span(&RS_CANDIDATES, false);
        let mut flags = FLAGS.to_vec();
        flags.push("-trace");
        let m = Mount::new(&flags);
        assert!(walk(&m.join("corpus")).len() > 1000);
        let mut read = 0;
        for e in fs::read_dir(m.join("corpus/sizes")).unwrap() {
            let p = e.unwrap().path();
            if fs::metadata(&p).unwrap().len() <= 30_000_000 && read < 10 {
                fs::read(&p).unwrap();
                read += 1;
            }
        }
        {
            let _c = Corrupt::new(blocks[0].0);
            assert_eq!(
                errno(read_direct(&m.join("corpus").join(rel), off, 4096)),
                Some(EIO)
            );
        }
        // The alert is logged after everything above, so once it is read, so is the rest.
        assert!(m.wait_stderr("alert=true"), "no alert line");
        let log = m.stderr();
        let traced = log
            .iter()
            .filter(|l| l.contains(" DEBUG ") || l.contains(" TRACE "))
            .count();
        assert!(
            traced > 100,
            "trace logging is on: {traced} per-operation lines"
        );
        for l in &log {
            for (name, value) in secret_values(l) {
                assert!(value.starts_with("<redacted>"), "{name}={value} in: {l}");
            }
        }

        let mut allowed: HashSet<String> = rustern_fuse::bridge::OPS
            .iter()
            .chain(&rustern_fuse::errno::ERRNO_NAMES)
            .chain(&rustern_client::metrics::RESULTS)
            .chain(&[rustern_fuse::bridge::OK])
            .map(|s| (*s).to_owned())
            .collect();
        allowed.extend(rustern_client::metrics::shard_kinds().map(str::to_owned));
        let dump = m.metrics();
        let mut checked = 0;
        for l in &dump {
            for (k, v) in labels(l) {
                if k == "quantile" || k == "le" {
                    assert!(v.parse::<f64>().is_ok() || v == "+Inf", "{l}");
                } else {
                    assert!(
                        allowed.contains(&v),
                        "label {k}={v} not from a constant list: {l}"
                    );
                }
                checked += 1;
            }
        }
        assert!(checked > 0, "no labels in dump: {dump:#?}");
        alive(&m);
    }
}
