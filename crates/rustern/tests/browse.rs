//! T063 (US1): browsing the cluster through a Rustern mount, compared with ternfuse.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)] // test code

mod common;

use std::collections::BTreeSet;
use std::ffi::OsString;
use std::io::{BufRead, BufReader};
use std::os::unix::ffi::OsStringExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use common::{Mount, env, mountinfo, ref_corpus, ternfuse_root, walk};
use rustix::process::Signal;

/// The shard overall deadline (contracts/cli.md defaults) plus slack for process start.
const STARTUP_DEADLINE: Duration = Duration::from_secs(12);

fn names(dir: &Path) -> Vec<OsString> {
    std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect()
}

/// Super options: the field after ` - fstype source `.
fn super_options(line: &str) -> Vec<String> {
    let (_, sup) = line.split_once(" - ").unwrap();
    sup.split(' ')
        .nth(2)
        .unwrap()
        .split(',')
        .map(str::to_owned)
        .collect()
}

/// Mount options: the sixth field.
fn mount_options(line: &str) -> Vec<String> {
    line.split(' ')
        .nth(5)
        .unwrap()
        .split(',')
        .map(str::to_owned)
        .collect()
}

#[test]
#[ignore = "needs cluster: run via scripts/gate.sh"]
fn ready_within_5s_and_walk_matches() {
    let reference = ref_corpus();
    let start = Instant::now();
    let m = Mount::new(&[]);
    assert!(
        start.elapsed() < Duration::from_secs(5),
        "{:?}",
        start.elapsed()
    );
    let want = format!("mounted at {}", m.path.display());
    assert!(
        m.stderr().iter().any(|l| l.contains(&want)),
        "{:?}",
        m.stderr()
    );

    let ours = walk(&m.join("corpus"));
    let theirs = walk(&reference);
    assert!(theirs.len() > 1000, "corpus too small: {}", theirs.len());
    assert_eq!(ours.len(), theirs.len());
    for (a, b) in ours.iter().zip(&theirs) {
        assert_eq!(a, b);
    }
}

#[test]
#[ignore = "needs cluster: run via scripts/gate.sh"]
fn missing_name_is_enoent() {
    ref_corpus();
    let m = Mount::new(&[]);
    let e = std::fs::metadata(m.join("corpus/no-such-name")).unwrap_err();
    assert_eq!(
        e.raw_os_error(),
        Some(rustix::io::Errno::NOENT.raw_os_error())
    );
    let e = std::fs::metadata(m.join("corpus/sizes/size_1/below-a-file")).unwrap_err();
    assert_eq!(
        e.raw_os_error(),
        Some(rustix::io::Errno::NOTDIR.raw_os_error())
    );
}

#[test]
#[ignore = "needs cluster: run via scripts/gate.sh"]
fn long_and_non_utf8_names() {
    ref_corpus();
    let m = Mount::new(&[]);
    let got: BTreeSet<OsString> = names(&m.join("corpus/names")).into_iter().collect();
    for want in [
        OsString::from("n".repeat(255)),
        OsString::from_vec(b"\xff\xfe".to_vec()),
        OsString::from_vec(b"bad\xff\xfename".to_vec()),
    ] {
        assert!(got.contains(&want), "{want:?} missing from {got:?}");
        std::fs::symlink_metadata(m.join("corpus/names").join(&want)).unwrap();
    }
    let e = std::fs::metadata(m.join("corpus/names").join("n".repeat(256))).unwrap_err();
    assert_eq!(
        e.raw_os_error(),
        Some(rustix::io::Errno::NAMETOOLONG.raw_os_error())
    );
}

#[test]
#[ignore = "needs cluster: run via scripts/gate.sh"]
fn big_directory_readdir_and_ls_l() {
    let reference = ref_corpus();
    let m = Mount::new(&[]);
    let want: BTreeSet<OsString> = names(&reference.join("big")).into_iter().collect();
    assert_eq!(want.len(), 10_000);

    let got = names(&m.join("corpus/big"));
    assert_eq!(got.len(), 10_000);
    assert_eq!(got.iter().cloned().collect::<BTreeSet<_>>(), want);

    // `ls -l` goes through readdirplus.
    let out = Command::new("ls")
        .args(["-l", "--quoting-style=literal"])
        .arg(m.join("corpus/big"))
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let lines: Vec<&[u8]> = out
        .stdout
        .split(|b| *b == b'\n')
        .filter(|l| !l.is_empty() && !l.starts_with(b"total "))
        .collect();
    assert_eq!(lines.len(), 10_000);
    let listed: BTreeSet<OsString> = lines
        .iter()
        .map(|l| OsString::from_vec(l.rsplit(|b| *b == b' ').next().unwrap().to_vec()))
        .collect();
    assert_eq!(listed, want);
}

/// `READ_DIR` shard requests so far.
fn read_dir_requests(m: &Mount) -> f64 {
    m.metrics()
        .iter()
        .filter(|l| {
            l.trim()
                .starts_with("rustern_shard_requests_total{kind=\"READ_DIR\"")
        })
        .filter_map(|l| l.trim().rsplit_once(' ')?.1.parse::<f64>().ok())
        .sum()
}

#[test]
#[ignore = "needs cluster: run via scripts/gate.sh"]
fn relisting_is_cached_until_the_directory_changes() {
    ref_corpus();
    let m = Mount::new(&[]);
    let big = m.join("corpus/big");
    assert_eq!(names(&big).len(), 10_000);
    let first = read_dir_requests(&m);
    assert!(first > 0.0);
    // FOPEN_CACHE_DIR: the kernel serves the second listing.
    assert_eq!(names(&big).len(), 10_000);
    assert_eq!(read_dir_requests(&m), first, "relisting reached the shards");

    // A change made elsewhere shows once the directory attributes expire (250 ms TTL): the
    // new mtime drops the cached listing.
    let scratch = ternfuse_root().join("scratch");
    std::fs::create_dir_all(&scratch).unwrap();
    let ours = m.join("scratch");
    let _ = names(&ours); // cached
    let local = std::env::temp_dir().join(format!("rustern-rl-{}", std::process::id()));
    std::fs::write(&local, b"x").unwrap();
    let name = OsString::from(format!(
        "r-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let st = Command::new("cp")
        .arg(&local)
        .arg(scratch.join(&name))
        .status()
        .unwrap();
    std::fs::remove_file(&local).unwrap();
    assert!(st.success());
    let start = Instant::now();
    while !names(&ours).contains(&name) {
        assert!(
            start.elapsed() < Duration::from_millis(500),
            "new file not listed after {:?}",
            start.elapsed()
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    let _ = std::fs::remove_file(scratch.join(&name));
}

fn unmounts_on(sig: Signal) {
    ref_corpus();
    let mut m = Mount::new(&[]);
    assert!(mountinfo(&m.path).is_some());
    m.signal(sig);
    let status = m
        .wait_exit(Duration::from_secs(10))
        .unwrap_or_else(|| panic!("no exit after {sig:?}"));
    assert!(status.success(), "{status:?}: {:?}", m.stderr());
    assert!(mountinfo(&m.path).is_none(), "still mounted after {sig:?}");
    std::fs::read_dir(&m.path).unwrap();
}

#[test]
#[ignore = "needs cluster: run via scripts/gate.sh"]
fn sigint_unmounts() {
    unmounts_on(Signal::INT);
}

#[test]
#[ignore = "needs cluster: run via scripts/gate.sh"]
fn sigterm_unmounts() {
    unmounts_on(Signal::TERM);
}

/// SIGTERM with a file open, as root: fuser's plain unmount fails with EBUSY there, and
/// rustern must detach rather than exit 1 and leave a dead mount. Root comes from a user +
/// mount namespace, so no privileges are needed; skipped where those are disabled.
#[test]
#[ignore = "needs cluster: run via scripts/gate.sh"]
fn sigterm_with_open_file_unmounts_as_root() {
    ref_corpus();
    let dir = std::env::temp_dir().join(format!("rustern-nsroot-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let script = r#"
        "$0" -registry "$1" "$2" 2> "$2.log" & p=$!
        for _ in $(seq 100); do mountpoint -q "$2" && break; sleep 0.05; done
        mountpoint -q "$2" || { echo "no mount"; cat "$2.log"; exit 2; }
        exec 3< "$2/corpus/corpus.sha256"
        kill -TERM $p; wait $p; echo "exit $?"
        if mountpoint -q "$2"; then echo "still mounted"; umount -l "$2"; else echo unmounted; fi
    "#;
    let out = std::process::Command::new("unshare")
        .args(["-Urm", "--propagation", "private", "sh", "-c", script])
        .arg(env!("CARGO_BIN_EXE_rustern"))
        .arg(&env().registry)
        .arg(&dir)
        .output()
        .unwrap();
    let _ = std::fs::remove_file(dir.with_extension("log"));
    let _ = std::fs::remove_dir(&dir);
    let stderr = String::from_utf8_lossy(&out.stderr);
    if stderr.contains("unshare failed") {
        eprintln!("skipped: no user namespaces here: {stderr}");
        return;
    }
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "exit 0\nunmounted\n",
        "{stderr}"
    );
}

#[test]
#[ignore = "needs cluster: run via scripts/gate.sh"]
fn bad_registry_exits_1() {
    let dir = std::env::temp_dir().join(format!("rustern-badreg-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let start = Instant::now();
    let out = Command::new(env!("CARGO_BIN_EXE_rustern"))
        .args(["-registry", "127.0.0.1:1"])
        .arg(&dir)
        .stdin(Stdio::null())
        .output()
        .unwrap();
    let elapsed = start.elapsed();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(1), "{stderr}");
    assert!(elapsed < STARTUP_DEADLINE, "{elapsed:?}");
    assert!(stderr.contains("127.0.0.1:1"), "{stderr}");
    assert!(mountinfo(&dir).is_none());
    std::fs::remove_dir(&dir).unwrap();
}

#[test]
#[ignore = "needs cluster: run via scripts/gate.sh"]
fn unknown_flag_exits_2() {
    let out = Command::new(env!("CARGO_BIN_EXE_rustern"))
        .args([
            "-registry",
            &env().registry,
            "-no-such-flag",
            "/nonexistent",
        ])
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(2),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
#[ignore = "needs cluster: run via scripts/gate.sh"]
fn usr1_dumps_metrics() {
    ref_corpus();
    let m = Mount::new(&[]);
    std::fs::metadata(m.join("corpus/sizes")).unwrap();
    m.signal(Signal::USR1);
    assert!(m.wait_stderr("rustern_fuse_ops_total"), "{:?}", m.stderr());
}

#[test]
#[ignore = "needs cluster: run via scripts/gate.sh"]
fn mountinfo_is_ro_nosuid_nodev() {
    ref_corpus();
    let m = Mount::new(&[]);
    let line = mountinfo(&m.path).unwrap();
    let opts = mount_options(&line);
    for o in ["ro", "nosuid", "nodev"] {
        assert!(opts.iter().any(|x| x == o), "{o} missing: {line}");
    }
    assert!(line.contains(" - fuse.rustern ternfs "), "{line}");
}

/// `-log-file` takes the log off stderr (without colour) and `-verbose` adds debug lines;
/// `statfs` on the mount reports the block size and name limit.
#[test]
#[ignore = "needs cluster: run via scripts/gate.sh"]
fn log_file_verbose_and_statfs() {
    ref_corpus();
    let tmp = std::env::temp_dir();
    let dir = tmp.join(format!("rustern-logfile-{}", std::process::id()));
    let log = tmp.join(format!("rustern-logfile-{}.log", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let _ = std::fs::remove_file(&log);
    let child = Command::new(env!("CARGO_BIN_EXE_rustern"))
        .args(["-registry", &env().registry, "-verbose", "-log-file"])
        .arg(&log)
        .arg(&dir)
        .stdin(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let start = Instant::now();
    while mountinfo(&dir).is_none() {
        assert!(start.elapsed() < STARTUP_DEADLINE, "not mounted");
        std::thread::sleep(Duration::from_millis(20));
    }
    let st = rustix::fs::statvfs(&dir).unwrap();
    assert_eq!((st.f_bsize, st.f_namemax), (4096, 255));
    // ENOENT is logged at debug.
    assert!(std::fs::metadata(dir.join("no-such-name")).is_err());

    rustix::process::kill_process(rustix::process::Pid::from_child(&child), Signal::TERM).unwrap();
    let out = child.wait_with_output().unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{stderr}");
    assert!(stderr.is_empty(), "{stderr}");
    let text = std::fs::read_to_string(&log).unwrap();
    assert!(text.contains("mounted at"), "{text}");
    assert!(text.contains("DEBUG"), "{text}");
    assert!(!text.contains('\x1b'), "{text}");
    std::fs::remove_file(&log).unwrap();
    std::fs::remove_dir(&dir).unwrap();
}

/// `-o allow_other` adds `default_permissions` where the host allows it (root, or
/// `user_allow_other` in /etc/fuse.conf); elsewhere, the usual case for an unprivileged
/// user, the mount fails at startup with an error that names the missing setting.
#[test]
#[ignore = "needs cluster: run via scripts/gate.sh"]
fn allow_other_default_permissions() {
    ref_corpus();
    let conf = std::fs::read_to_string("/etc/fuse.conf").unwrap_or_default();
    let allowed = conf.lines().any(|l| l.trim() == "user_allow_other");
    if !allowed && !rustix::process::geteuid().is_root() {
        let Err(err) = Mount::try_new(&["-o", "allow_other"]) else {
            panic!("allow_other mounted without user_allow_other");
        };
        assert!(
            err.contains("did not mount (Some("),
            "exited at startup: {err}"
        );
        assert!(
            err.contains("cannot mount") && err.contains("user_allow_other"),
            "{err}"
        );
        return;
    }
    let m = Mount::new(&["-o", "allow_other"]);
    let line = mountinfo(&m.path).unwrap();
    let sup = super_options(&line);
    for o in ["allow_other", "default_permissions"] {
        assert!(sup.iter().any(|x| x == o), "{o} missing: {line}");
    }
}

/// When the system refuses `-o allow_other` (no `user_allow_other` in `/etc/fuse.conf`, not
/// root), Rustern exits 1 naming the reason and does not mount without the option.
/// `allow_other_default_permissions` stays mandatory; this checks the refusal path where the
/// environment lets us observe it.
#[test]
#[ignore = "needs cluster: run via scripts/gate.sh"]
fn allow_other_refused_is_explicit() {
    let conf = std::fs::read_to_string("/etc/fuse.conf").unwrap_or_default();
    if conf.lines().any(|l| l.trim() == "user_allow_other") || rustix::process::geteuid().is_root()
    {
        eprintln!("allow_other is permitted here; refusal path not observable");
        return;
    }
    let dir = std::env::temp_dir().join(format!("rustern-allowother-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_rustern"))
        .args(["-registry", &env().registry, "-o", "allow_other"])
        .arg(&dir)
        .stdin(Stdio::null())
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    let mounted = mountinfo(&dir).is_some();
    if mounted {
        let _ = Command::new("fusermount3").arg("-u").arg(&dir).status();
    }
    std::fs::remove_dir(&dir).unwrap();
    assert!(!mounted, "mounted without allow_other: {stderr}");
    assert_eq!(out.status.code(), Some(1), "{stderr}");
    assert!(stderr.contains("user_allow_other"), "{stderr}");
}

#[test]
#[ignore = "needs cluster: run via scripts/gate.sh"]
fn external_unmount_exits() {
    ref_corpus();
    let mut m = Mount::new(&[]);
    let st = Command::new("fusermount3")
        .arg("-u")
        .arg(&m.path)
        .status()
        .unwrap();
    assert!(st.success());
    let status = m
        .wait_exit(Duration::from_secs(5))
        .expect("rustern still running after fusermount3 -u");
    assert!(status.success(), "{status:?}: {:?}", m.stderr());
    assert!(mountinfo(&m.path).is_none());
}

#[test]
#[ignore = "needs cluster: run via scripts/gate.sh"]
fn signal_parent() {
    ref_corpus();
    let dir = std::env::temp_dir().join(format!("rustern-sigparent-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    // One stream for both, so the order is observable.
    let script =
        r#"trap 'echo got-usr1' USR1; "$0" -registry "$1" -signal-parent "$2" 2>&1 & wait; wait"#;
    let mut sh = Command::new("sh")
        .args(["-c", script, env!("CARGO_BIN_EXE_rustern"), &env().registry])
        .arg(&dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let (tx, rx) = mpsc::channel();
    let out = sh.stdout.take().unwrap();
    std::thread::spawn(move || {
        for line in BufReader::new(out).lines() {
            let Ok(line) = line else { break };
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    let mut lines = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(10);
    while !lines.iter().any(|l: &String| l == "got-usr1") && Instant::now() < deadline {
        if let Ok(l) = rx.recv_timeout(Duration::from_millis(50)) {
            lines.push(l);
        }
    }
    let _ = Command::new("fusermount3").arg("-u").arg(&dir).status();
    let _ = sh.wait();
    let _ = std::fs::remove_dir(&dir);
    let mounted = lines.iter().position(|l| l.contains("mounted at"));
    let got = lines.iter().position(|l| l == "got-usr1");
    match (mounted, got) {
        (Some(a), Some(b)) => assert!(a < b, "{lines:?}"),
        _ => panic!("{lines:?}"),
    }
}

#[test]
#[ignore = "needs cluster: run via scripts/gate.sh"]
fn dir_attr_validity() {
    ref_corpus();
    // DEV-003: directory attributes are cached no longer than entries (250 ms).
    let scratch = ternfuse_root().join("scratch");
    std::fs::create_dir_all(&scratch).unwrap();
    let m = Mount::new(&[]);
    let ours = m.join("scratch");
    let before = std::fs::metadata(&ours).unwrap().modified().unwrap();
    std::thread::sleep(Duration::from_millis(10)); // a distinct mtime

    let local = std::env::temp_dir().join(format!("rustern-dav-{}", std::process::id()));
    std::fs::write(&local, b"x").unwrap();
    let name = format!(
        "f-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let st = Command::new("cp")
        .arg(&local)
        .arg(scratch.join(&name))
        .status()
        .unwrap();
    std::fs::remove_file(&local).unwrap();
    assert!(st.success());

    let start = Instant::now();
    loop {
        let now = std::fs::metadata(&ours).unwrap().modified().unwrap();
        if now != before {
            break;
        }
        assert!(
            start.elapsed() < Duration::from_millis(300),
            "directory mtime unchanged after {:?}",
            start.elapsed()
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    let _ = std::fs::remove_file(scratch.join(&name));
}
