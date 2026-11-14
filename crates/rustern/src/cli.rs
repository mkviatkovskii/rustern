//! Command line (contracts/cli.md): ternfuse-compatible flags parsed with clap.

use std::ffi::OsString;
use std::path::PathBuf;
use std::time::Duration;

use bytesize::ByteSize;
use clap::{CommandFactory, Parser};
use fuser::{MountOption, SessionACL};
use rustern_client::{ClientConfig, ReqTimeouts};

/// Most FUSE worker threads by default.
const MAX_DEFAULT_THREADS: usize = 16;

/// Raw flags; a repeated flag overrides the earlier one, as in Go. Go-style `-name` is rewritten to `--name` by [`go_style`] before clap parses.
#[derive(Debug, Parser)]
#[command(
    name = "rustern",
    about = "Read-only FUSE client for TernFS",
    args_override_self = true
)]
struct Args {
    /// Registry address (host:port).
    #[arg(long)]
    registry: String,
    /// Comma-separated mount options (may be repeated).
    #[arg(short = 'o')]
    options: Vec<String>,
    /// Log to this file instead of stderr.
    #[arg(long)]
    log_file: Option<PathBuf>,
    /// Debug logging.
    #[arg(long, num_args = 0..=1, require_equals = true, default_missing_value = "true",
          default_value = "false", value_parser = clap::builder::BoolishValueParser::new())]
    verbose: bool,
    /// Trace logging.
    #[arg(long, num_args = 0..=1, require_equals = true, default_missing_value = "true",
          default_value = "false", value_parser = clap::builder::BoolishValueParser::new())]
    trace: bool,
    /// Largest UDP datagram to shards.
    #[arg(long, default_value_t = 1472)]
    mtu: u16,
    /// Attribute TTL (directories are capped at the entry TTL, DEV-003).
    #[arg(long, default_value = "1h", value_parser = go_duration)]
    attr_cache_time: Duration,
    /// Directory entry TTL (positive and negative).
    #[arg(long, default_value = "250ms", value_parser = go_duration)]
    dir_entry_cache_time: Duration,
    /// Entries fetched and stat-ed per readdir batch.
    #[arg(long, default_value_t = 1000)]
    readdir_batch_size: u32,
    /// Report the daemon's effective uid/gid as every file's owner.
    #[arg(long, num_args = 0..=1, require_equals = true, default_missing_value = "true",
          default_value = "false", value_parser = clap::builder::BoolishValueParser::new())]
    set_uid: bool,
    /// Send SIGUSR1 to the parent once mounted.
    #[arg(long, num_args = 0..=1, require_equals = true, default_missing_value = "true",
          default_value = "false", value_parser = clap::builder::BoolishValueParser::new())]
    signal_parent: bool,
    #[arg(long, default_value = "100ms", value_parser = go_duration)]
    initial_shard_timeout: Duration,
    #[arg(long, default_value = "2s", value_parser = go_duration)]
    max_shard_timeout: Duration,
    #[arg(long, default_value = "10s", value_parser = go_duration)]
    overall_shard_timeout: Duration,
    #[arg(long = "initial-cdc-timeout", default_value = "1s", value_parser = go_duration)]
    initial_cdc_timeout: Duration,
    #[arg(long = "max-cdc-timeout", default_value = "10s", value_parser = go_duration)]
    max_cdc_timeout: Duration,
    #[arg(long = "overall-cdc-timeout", default_value = "60s", value_parser = go_duration)]
    overall_cdc_timeout: Duration,
    #[arg(long, default_value = "1s", value_parser = go_duration)]
    initial_block_timeout: Duration,
    #[arg(long, default_value = "10s", value_parser = go_duration)]
    max_block_timeout: Duration,
    /// DEV-006: 60 s, not upstream's 5 min.
    #[arg(long, default_value = "60s", value_parser = go_duration)]
    overall_block_timeout: Duration,
    /// Bytes of read buffers in total (e.g. `1GiB`).
    #[arg(long, default_value = "1GiB")]
    memory_limit: ByteSize,
    /// Bytes of read buffers per file (e.g. `32MiB`).
    #[arg(long, default_value = "32MiB")]
    file_memory_limit: ByteSize,
    /// FUSE worker threads (default: CPUs, at most 16).
    #[arg(long)]
    threads: Option<usize>,

    // Accepted for ternfuse compatibility and ignored with a warning.
    #[arg(long, hide = true)]
    close_tracker_object: Option<String>,
    #[arg(long, hide = true, num_args = 0..=1, require_equals = true,
          default_missing_value = "true", value_parser = clap::builder::BoolishValueParser::new())]
    direct_mount: Option<bool>,
    #[arg(long, hide = true, num_args = 0..=1, require_equals = true,
          default_missing_value = "true", value_parser = clap::builder::BoolishValueParser::new())]
    direct_mount_strict: Option<bool>,
    #[arg(long, hide = true)]
    profile_file: Option<String>,
    #[arg(long, hide = true, num_args = 0..=1, require_equals = true,
          default_missing_value = "true", value_parser = clap::builder::BoolishValueParser::new())]
    syslog: Option<bool>,
    #[arg(long, hide = true, num_args = 0..=1, require_equals = true,
          default_missing_value = "true", value_parser = clap::builder::BoolishValueParser::new())]
    print_quiet_alerts: Option<bool>,
    #[arg(long, hide = true)]
    xmon: Option<String>,

    /// Mount point.
    mountpoint: PathBuf,
}

/// Mount options and access control for fuser.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MountOpts {
    /// `ro`, `nosuid`, `nodev` first, then the user's options.
    pub options: Vec<MountOption>,
    /// Who may use the mount (`allow_other`, `allow_root`).
    pub acl: SessionACL,
}

/// Everything the daemon needs from its command line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    /// Client settings, validated.
    pub client: ClientConfig,
    /// Mount options.
    pub mount: MountOpts,
    /// Mount point.
    pub mountpoint: PathBuf,
    /// Log file; `None` logs to stderr.
    pub log_file: Option<PathBuf>,
    /// `-verbose`.
    pub verbose: bool,
    /// `-trace`.
    pub trace: bool,
    /// `-set-uid`.
    pub set_uid: bool,
    /// `-signal-parent`.
    pub signal_parent: bool,
    /// FUSE worker threads.
    pub threads: usize,
    /// Warnings to log once logging is set up (ignored flags, unusual values).
    pub warnings: Vec<String>,
}

/// An invalid command line (exit code 2) or `--help` (exit code 0).
#[derive(Debug, thiserror::Error)]
pub enum CliError {
    /// Rejected by clap, or `--help`.
    #[error(transparent)]
    Clap(#[from] clap::Error),
    /// Parsed but invalid.
    #[error("{0}")]
    Invalid(String),
}

impl CliError {
    /// Process exit code.
    pub fn exit_code(&self) -> i32 {
        match self {
            Self::Clap(e) => e.exit_code(),
            Self::Invalid(_) => 2,
        }
    }
}

/// Parses `argv` (including the program name).
pub fn parse<I, T>(argv: I) -> Result<Settings, CliError>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString>,
{
    let a = Args::try_parse_from(go_style(argv.into_iter().map(Into::into).collect()))?;
    let mut warnings: Vec<String> = [
        ("close-tracker-object", a.close_tracker_object.is_some()),
        ("direct-mount", a.direct_mount.is_some()),
        ("direct-mount-strict", a.direct_mount_strict.is_some()),
        ("profile-file", a.profile_file.is_some()),
        ("syslog", a.syslog.is_some()),
        ("print-quiet-alerts", a.print_quiet_alerts.is_some()),
        ("xmon", a.xmon.is_some()),
    ]
    .into_iter()
    .filter(|(_, given)| *given)
    .map(|(flag, _)| format!("-{flag} is not supported by rustern and is ignored"))
    .collect();

    let bytes = |name: &str, b: ByteSize| {
        usize::try_from(b.as_u64())
            .map_err(|_| CliError::Invalid(format!("{name} {b} is too large")))
    };
    let client = ClientConfig {
        shard: ReqTimeouts {
            initial: a.initial_shard_timeout,
            max: a.max_shard_timeout,
            overall: a.overall_shard_timeout,
        },
        cdc: ReqTimeouts {
            initial: a.initial_cdc_timeout,
            max: a.max_cdc_timeout,
            overall: a.overall_cdc_timeout,
        },
        block: ReqTimeouts {
            initial: a.initial_block_timeout,
            max: a.max_block_timeout,
            overall: a.overall_block_timeout,
        },
        mtu: a.mtu,
        readdir_batch: a.readdir_batch_size,
        memory_limit: bytes("--memory-limit", a.memory_limit)?,
        file_memory_limit: bytes("--file-memory-limit", a.file_memory_limit)?,
        attr_ttl: a.attr_cache_time,
        entry_ttl: a.dir_entry_cache_time,
        ..ClientConfig::new(a.registry)
    };
    warnings.extend(
        client
            .validate()
            .map_err(|e| CliError::Invalid(e.to_string()))?,
    );
    let threads = match a.threads {
        Some(0) => return Err(CliError::Invalid("--threads must be positive".into())),
        Some(n) => n,
        None => std::thread::available_parallelism()
            .map_or(1, |n| n.get())
            .min(MAX_DEFAULT_THREADS),
    };
    Ok(Settings {
        client,
        mount: mount_opts(&a.options)?,
        mountpoint: a.mountpoint,
        log_file: a.log_file,
        verbose: a.verbose,
        trace: a.trace,
        set_uid: a.set_uid,
        signal_parent: a.signal_parent,
        threads,
        warnings,
    })
}

/// `ro,nosuid,nodev` plus the user's `-o` options; `allow_other` adds
/// `default_permissions` (FR-003).
fn mount_opts(given: &[String]) -> Result<MountOpts, CliError> {
    let mut options = vec![MountOption::RO, MountOption::NoSuid, MountOption::NoDev];
    let mut acl = SessionACL::Owner;
    let push = |o: MountOption, options: &mut Vec<MountOption>| {
        if !options.contains(&o) {
            options.push(o);
        }
    };
    for opt in given.iter().flat_map(|o| o.split(',')).map(str::trim) {
        let o = match opt {
            "" | "ro" | "nosuid" | "nodev" => continue,
            "rw" | "suid" | "dev" => {
                return Err(CliError::Invalid(format!(
                    "mount option {opt} is not allowed: rustern always mounts ro,nosuid,nodev"
                )));
            }
            "allow_other" => {
                acl = SessionACL::All;
                MountOption::DefaultPermissions
            }
            "allow_root" => {
                acl = SessionACL::RootAndOwner;
                continue;
            }
            "default_permissions" => MountOption::DefaultPermissions,
            "auto_unmount" => MountOption::AutoUnmount,
            "exec" => MountOption::Exec,
            "noexec" => MountOption::NoExec,
            "atime" => MountOption::Atime,
            "noatime" => MountOption::NoAtime,
            "dirsync" => MountOption::DirSync,
            "sync" => MountOption::Sync,
            "async" => MountOption::Async,
            other => MountOption::CUSTOM(other.to_owned()),
        };
        push(o, &mut options);
    }
    Ok(MountOpts { options, acl })
}

/// Rewrites Go-style `-name[=v]` to `--name[=v]` for the long flags of [`Args`], as Go's
/// `flag` package accepts both (FR-002). Stops at `--`; other arguments pass unchanged.
fn go_style(argv: Vec<OsString>) -> Vec<OsString> {
    let cmd = Args::command();
    let longs: Vec<&str> = cmd.get_arguments().filter_map(|a| a.get_long()).collect();
    let mut out = Vec::with_capacity(argv.len());
    let mut flags = true;
    for arg in argv {
        let rewritten = match arg.to_str() {
            Some("--") => {
                flags = false;
                None
            }
            Some(s) if flags && !s.starts_with("--") => s
                .strip_prefix('-')
                .filter(|rest| longs.contains(&rest.split('=').next().unwrap_or_default()))
                .map(|rest| OsString::from(format!("--{rest}"))),
            _ => None,
        };
        out.push(rewritten.unwrap_or(arg));
    }
    out
}

/// Parses a Go `time.Duration` string (`250ms`, `1m30s`, `1.5s`, `0`); negative durations
/// are rejected.
pub fn go_duration(s: &str) -> Result<Duration, String> {
    const UNITS: [(&str, u128); 8] = [
        ("ns", 1),
        ("us", 1_000),
        ("µs", 1_000),
        ("μs", 1_000),
        ("ms", 1_000_000),
        ("s", 1_000_000_000),
        ("m", 60_000_000_000),
        ("h", 3_600_000_000_000),
    ];
    let bad = || format!("invalid duration {s:?} (Go syntax, e.g. 250ms, 10s, 1m30s)");
    let rest = s.strip_prefix('+').unwrap_or(s);
    if rest == "0" {
        return Ok(Duration::ZERO);
    }
    if rest.is_empty() {
        return Err(bad());
    }
    let mut total: u128 = 0;
    let mut rest = rest;
    while !rest.is_empty() {
        let int_len = rest
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(rest.len());
        let (int, after) = rest.split_at(int_len);
        let (frac, after) = match after.strip_prefix('.') {
            Some(f) => f.split_at(f.find(|c: char| !c.is_ascii_digit()).unwrap_or(f.len())),
            None => ("", after),
        };
        if int.is_empty() && frac.is_empty() {
            return Err(bad());
        }
        let unit_len = after
            .find(|c: char| c.is_ascii_digit() || c == '.')
            .unwrap_or(after.len());
        let (unit, after) = after.split_at(unit_len);
        let scale = UNITS
            .iter()
            .find(|(u, _)| *u == unit)
            .map(|(_, n)| *n)
            .ok_or_else(bad)?;
        let whole = if int.is_empty() {
            0
        } else {
            int.parse::<u128>().map_err(|_| bad())?
        };
        let mut part = whole.checked_mul(scale).ok_or_else(bad)?;
        // Fractional digits beyond nanosecond precision are dropped, as in Go.
        let mut place = scale;
        for d in frac.chars().filter_map(|c| c.to_digit(10)) {
            place /= 10;
            if place == 0 {
                break;
            }
            part = u128::from(d)
                .checked_mul(place)
                .and_then(|x| part.checked_add(x))
                .ok_or_else(bad)?;
        }
        total = total.checked_add(part).ok_or_else(bad)?;
        rest = after;
    }
    u64::try_from(total)
        .map(Duration::from_nanos)
        .map_err(|_| bad())
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)] // test code
mod tests {
    use super::*;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    fn try_args(args: &[&str]) -> Result<Settings, CliError> {
        parse(std::iter::once("rustern").chain(args.iter().copied()))
    }

    fn args(args: &[&str]) -> Settings {
        try_args(args).unwrap()
    }

    fn base() -> Settings {
        args(&["-registry", "r:1", "/mnt"])
    }

    #[test]
    fn go_durations() {
        for (s, d) in [
            ("250ms", ms(250)),
            ("10s", ms(10_000)),
            ("1h", ms(3_600_000)),
            ("1m30s", ms(90_000)),
            ("1.5s", ms(1500)),
            ("500us", Duration::from_micros(500)),
            ("500µs", Duration::from_micros(500)),
            ("7ns", Duration::from_nanos(7)),
            ("1h2m3.5s", ms(3_723_500)),
            ("0", Duration::ZERO),
            ("+2s", ms(2000)),
            (".5s", ms(500)),
            // Digits past nanosecond precision are dropped, as in Go.
            ("1.0000000019s", Duration::from_nanos(1_000_000_001)),
        ] {
            assert_eq!(go_duration(s), Ok(d), "{s}");
        }
        for s in [
            "10",
            "1x",
            "",
            "s",
            "-1s",
            "1.s5",
            "1..5s",
            "99999999999999999999h",
        ] {
            assert!(go_duration(s).is_err(), "{s}");
        }
    }

    #[test]
    fn defaults_equal_the_table() {
        let s = base();
        assert_eq!(s.client, ClientConfig::new("r:1"));
        assert_eq!(s.mountpoint, PathBuf::from("/mnt"));
        assert_eq!(s.log_file, None);
        assert!(!s.verbose && !s.trace && !s.set_uid && !s.signal_parent);
        let cpus = std::thread::available_parallelism().map_or(1, |n| n.get());
        assert_eq!(s.threads, cpus.min(16));
        assert_eq!(s.warnings, Vec::<String>::new());
        assert_eq!(
            s.mount.options,
            [MountOption::RO, MountOption::NoSuid, MountOption::NoDev]
        );
        assert_eq!(s.mount.acl, SessionACL::Owner);
    }

    /// Each valued flag in its four spellings, with the effect checked.
    #[test]
    fn every_flag_in_every_form() {
        type Check = fn(&Settings) -> bool;
        let flags: &[(&str, &str, Check)] = &[
            ("registry", "h:9", |s| s.client.registry == "h:9"),
            ("log-file", "/tmp/l", |s| {
                s.log_file.as_deref() == Some("/tmp/l".as_ref())
            }),
            ("mtu", "9000", |s| s.client.mtu == 9000),
            ("attr-cache-time", "5s", |s| s.client.attr_ttl == ms(5000)),
            ("dir-entry-cache-time", "1s", |s| {
                s.client.entry_ttl == ms(1000)
            }),
            ("readdir-batch-size", "7", |s| s.client.readdir_batch == 7),
            ("initial-shard-timeout", "50ms", |s| {
                s.client.shard.initial == ms(50)
            }),
            ("max-shard-timeout", "3s", |s| {
                s.client.shard.max == ms(3000)
            }),
            ("overall-shard-timeout", "20s", |s| {
                s.client.shard.overall == ms(20_000)
            }),
            ("initial-cdc-timeout", "2s", |s| {
                s.client.cdc.initial == ms(2000)
            }),
            ("max-cdc-timeout", "20s", |s| s.client.cdc.max == ms(20_000)),
            ("overall-cdc-timeout", "30s", |s| {
                s.client.cdc.overall == ms(30_000)
            }),
            ("initial-block-timeout", "2s", |s| {
                s.client.block.initial == ms(2000)
            }),
            ("max-block-timeout", "20s", |s| {
                s.client.block.max == ms(20_000)
            }),
            ("overall-block-timeout", "30s", |s| {
                s.client.block.overall == ms(30_000)
            }),
            ("memory-limit", "2GiB", |s| s.client.memory_limit == 2 << 30),
            ("file-memory-limit", "64MiB", |s| {
                s.client.file_memory_limit == 64 << 20
            }),
            ("threads", "3", |s| s.threads == 3),
        ];
        for (name, value, check) in flags {
            let forms = [
                vec![format!("-{name}"), (*value).to_owned()],
                vec![format!("-{name}={value}")],
                vec![format!("--{name}"), (*value).to_owned()],
                vec![format!("--{name}={value}")],
            ];
            for form in forms {
                let mut argv = vec!["-registry", "r:1"];
                argv.extend(form.iter().map(String::as_str));
                argv.push("/mnt");
                let s = try_args(&argv).unwrap_or_else(|e| panic!("{form:?}: {e}"));
                assert!(check(&s), "{form:?}: {s:?}");
            }
        }
        type BoolCheck = fn(&Settings) -> bool;
        let bools: &[(&str, BoolCheck)] = &[
            ("verbose", |s| s.verbose),
            ("trace", |s| s.trace),
            ("set-uid", |s| s.set_uid),
            ("signal-parent", |s| s.signal_parent),
        ];
        for (name, check) in bools {
            for form in [
                format!("-{name}"),
                format!("--{name}"),
                format!("-{name}=true"),
                format!("--{name}=true"),
            ] {
                let s = args(&["-registry", "r:1", &form, "/mnt"]);
                assert!(check(&s), "{form}");
            }
            let s = args(&["-registry", "r:1", &format!("-{name}=false"), "/mnt"]);
            assert!(!check(&s), "-{name}=false");
        }
    }

    #[test]
    fn mount_options() {
        let names = |s: &Settings| s.mount.options.clone();
        let mandatory = [MountOption::RO, MountOption::NoSuid, MountOption::NoDev];

        // `-o` stays a short flag, in all its spellings.
        for form in [&["-o", "noatime"][..], &["-o=noatime"], &["-onoatime"]] {
            let mut argv = vec!["-registry", "r:1"];
            argv.extend(form);
            argv.push("/mnt");
            let s = args(&argv);
            assert_eq!(names(&s)[..3], mandatory, "{form:?}");
            assert!(names(&s).contains(&MountOption::NoAtime), "{form:?}");
        }

        // ro,nosuid,nodev are always present, once each.
        let s = args(&["-registry", "r:1", "-o", "ro,nodev", "/mnt"]);
        assert_eq!(names(&s), mandatory);

        for bad in ["rw", "ro,rw", "suid", "dev"] {
            let e = try_args(&["-registry", "r:1", "-o", bad, "/mnt"]).unwrap_err();
            assert_eq!(e.exit_code(), 2, "{bad}");
        }

        // allow_other implies default_permissions (FR-003).
        let s = args(&["-registry", "r:1", "-o", "allow_other", "/mnt"]);
        assert_eq!(s.mount.acl, SessionACL::All);
        assert!(names(&s).contains(&MountOption::DefaultPermissions));
        let s = args(&["-registry", "r:1", "-o", "noexec", "/mnt"]);
        assert_eq!(s.mount.acl, SessionACL::Owner);
        assert!(!names(&s).contains(&MountOption::DefaultPermissions));
        let s = args(&["-registry", "r:1", "-o", "allow_root", "/mnt"]);
        assert_eq!(s.mount.acl, SessionACL::RootAndOwner);
        assert_eq!(names(&s), mandatory);

        // Every other option passes through, unknown ones verbatim.
        for (opt, want) in [
            ("default_permissions", MountOption::DefaultPermissions),
            ("auto_unmount", MountOption::AutoUnmount),
            ("exec", MountOption::Exec),
            ("atime", MountOption::Atime),
            ("dirsync", MountOption::DirSync),
            ("sync", MountOption::Sync),
            ("async", MountOption::Async),
            ("max_read=4096", MountOption::CUSTOM("max_read=4096".into())),
        ] {
            let s = args(&["-registry", "r:1", "-o", opt, "/mnt"]);
            assert_eq!(names(&s)[3..], [want], "{opt}");
        }
    }

    #[test]
    fn double_dash_ends_flags() {
        let s = args(&["-registry", "r:1", "--", "-mnt"]);
        assert_eq!(s.mountpoint, std::path::Path::new("-mnt"));
    }

    #[test]
    fn ignored_flags_warn() {
        for argv in [
            &["-close-tracker-object", "x.o"][..],
            &["-direct-mount"],
            &["-direct-mount-strict"],
            &["-profile-file", "p"],
            &["-syslog"],
            &["-print-quiet-alerts"],
            &["-xmon", "prod"],
        ] {
            let mut a = vec!["-registry", "r:1"];
            a.extend(argv);
            a.push("/mnt");
            let s = args(&a);
            let flag = argv[0].trim_start_matches('-');
            assert_eq!(s.warnings.len(), 1, "{argv:?}: {:?}", s.warnings);
            assert!(s.warnings[0].contains(flag), "{:?}", s.warnings);
            assert_eq!(s.mountpoint, PathBuf::from("/mnt"));
        }
    }

    #[test]
    fn invalid_command_lines_exit_2() {
        for argv in [
            &["-registry", "r:1", "-bogus", "/mnt"][..],
            &["-registry", "r:1"],
            &["/mnt"],
            &["-registry", "r:1", "-mtu", "big", "/mnt"],
            &["-registry", "r:1", "-attr-cache-time", "10", "/mnt"],
            &["-registry", "r:1", "-file-memory-limit", "1MiB", "/mnt"],
            &["-registry", "r:1", "-initial-shard-timeout", "5s", "/mnt"],
            &["-registry", "r:1", "-threads", "0", "/mnt"],
            &["-registry", "r:1", "-mtu", "100", "/mnt"],
        ] {
            let e = try_args(argv).unwrap_err();
            assert_eq!(e.exit_code(), 2, "{argv:?}: {e}");
        }
    }

    #[test]
    fn non_utf8_mount_point() {
        use std::os::unix::ffi::OsStringExt;
        let mp = OsString::from_vec(b"/mnt/\xff".to_vec());
        let s = parse([
            OsString::from("rustern"),
            "-registry".into(),
            "r:1".into(),
            mp.clone(),
        ])
        .unwrap();
        assert_eq!(s.mountpoint, PathBuf::from(mp));
    }
}
