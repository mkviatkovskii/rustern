//! `rustern`: read-only FUSE client for TernFS (contracts/cli.md).

mod cli;

use std::fs::OpenOptions;
use std::io::{self, IsTerminal};
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use fuser::{BackgroundSession, MountOption};
use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};
use rustern_client::{Client, ClientError};
use rustern_fuse::attr::AttrConfig;
use rustern_fuse::bridge::Bridge;
use rustern_fuse::fs::RusternFs;
use tokio::signal::unix::{SignalKind, signal};
use tracing::Level;

use crate::cli::{CliError, Settings};

/// How long in-flight requests may finish after the unmount.
const DRAIN: Duration = Duration::from_secs(5);

/// How often histogram samples are folded into the metrics' summaries (the exporter's own
/// default when it runs the upkeep itself).
const UPKEEP: Duration = Duration::from_secs(5);

/// Why the daemon did not start, or stopped.
#[derive(Debug, thiserror::Error)]
enum StartupError {
    /// Bad command line (exit 2), or `--help` (exit 0).
    #[error(transparent)]
    InvalidArgs(#[from] CliError),
    /// The registry did not answer.
    #[error("cannot fetch shard addresses from registry {addr}: {source}")]
    Registry {
        /// `-registry`.
        addr: String,
        /// Cause.
        source: ClientError,
    },
    /// The mount failed.
    #[error("cannot mount {}: {source}", path.display())]
    Mount {
        /// Mount point.
        path: PathBuf,
        /// Cause.
        source: io::Error,
    },
    /// Logging, metrics, signals or the runtime could not be set up.
    #[error("{what}: {source}")]
    Setup {
        /// What failed.
        what: &'static str,
        /// Cause.
        source: io::Error,
    },
    /// The FUSE session failed after mounting.
    #[error("FUSE session failed: {0}")]
    Session(io::Error),
}

impl StartupError {
    fn exit_code(&self) -> u8 {
        match self {
            Self::InvalidArgs(e) => u8::try_from(e.exit_code()).unwrap_or(2),
            Self::Registry { .. } | Self::Mount { .. } | Self::Setup { .. } | Self::Session(_) => 1,
        }
    }

    fn setup(what: &'static str) -> impl FnOnce(io::Error) -> Self {
        move |source| Self::Setup { what, source }
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(StartupError::InvalidArgs(CliError::Clap(e))) => {
            let _ = e.print(); // usage errors to stderr, --help to stdout
            ExitCode::from(u8::try_from(e.exit_code()).unwrap_or(2))
        }
        Err(e) => {
            tracing::error!("{e}");
            eprintln!("rustern: {e}");
            ExitCode::from(e.exit_code())
        }
    }
}

fn run() -> Result<(), StartupError> {
    let settings = cli::parse(std::env::args_os())?;
    init_logging(&settings)?;
    for w in &settings.warnings {
        tracing::warn!("{w}");
    }
    let recorder = PrometheusBuilder::new().build_recorder();
    let metrics = recorder.handle();
    metrics::set_global_recorder(recorder)
        .map_err(|e| StartupError::setup("cannot install metrics")(io::Error::other(e)))?;

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(StartupError::setup("cannot start the runtime"))?;
    let client = rt
        .block_on(Client::connect(settings.client.clone()))
        .map_err(|source| StartupError::Registry {
            addr: settings.client.registry.clone(),
            source,
        })?;

    let attrs = AttrConfig::new(
        settings.client.attr_ttl,
        settings.client.entry_ttl,
        settings.set_uid,
    );
    let fs = RusternFs::new(client, attrs);
    let mut config = fuser::Config::default();
    config.mount_options = settings.mount.options.clone();
    config.mount_options.extend([
        MountOption::FSName("ternfs".into()),
        MountOption::Subtype("rustern".into()),
    ]);
    config.acl = settings.mount.acl;
    config.n_threads = Some(settings.threads);
    config.clone_fd = true;
    let session = fuser::spawn_mount(
        Bridge::new(Arc::new(fs), rt.handle().clone()),
        &settings.mountpoint,
        &config,
    )
    .map_err(|source| StartupError::Mount {
        path: settings.mountpoint.clone(),
        source,
    })?;
    tracing::info!("mounted at {}", settings.mountpoint.display());
    if settings.signal_parent {
        signal_parent();
    }

    let result = rt.block_on(serve(session, metrics));
    rt.shutdown_timeout(DRAIN);
    result
}

/// Logs to stderr or `-log-file`; `-trace` > `-verbose` > info.
fn init_logging(s: &Settings) -> Result<(), StartupError> {
    let level = if s.trace {
        Level::TRACE
    } else if s.verbose {
        Level::DEBUG
    } else {
        Level::INFO
    };
    let builder = tracing_subscriber::fmt().with_max_level(level);
    match &s.log_file {
        Some(path) => {
            let file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .map_err(StartupError::setup("cannot open the log file"))?;
            builder
                .with_ansi(false)
                .with_writer(Mutex::new(file))
                .init();
        }
        // Colour only on a terminal: piped logs are parsed by tools and tests.
        None => builder
            .with_ansi(io::stderr().is_terminal())
            .with_writer(io::stderr)
            .init(),
    }
    Ok(())
}

/// `-signal-parent`: SIGUSR1 to the parent once mounted (as ternfuse).
fn signal_parent() {
    let Some(ppid) = rustix::process::getppid() else {
        tracing::warn!("-signal-parent: no parent process");
        return;
    };
    if let Err(e) = rustix::process::kill_process(ppid, rustix::process::Signal::USR1) {
        tracing::warn!(error = %e, "-signal-parent: cannot signal the parent");
    }
}

/// Serves until a signal (unmount, exit 0), an external unmount (exit 0) or a session
/// failure (exit 1). SIGUSR1 dumps the metrics to the log; histogram samples are folded
/// into summaries every [`UPKEEP`].
async fn serve(session: BackgroundSession, metrics: PrometheusHandle) -> Result<(), StartupError> {
    let sig = |k| signal(k).map_err(StartupError::setup("cannot install signal handlers"));
    let (mut int, mut term, mut hup, mut usr1) = (
        sig(SignalKind::interrupt())?,
        sig(SignalKind::terminate())?,
        sig(SignalKind::hangup())?,
        sig(SignalKind::user_defined1())?,
    );
    // `build_recorder` starts no upkeep task: without one, every histogram sample (several
    // per block fetch) stays buffered until the next render, so memory grows with traffic.
    let upkeep = metrics.clone();
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(UPKEEP);
        loop {
            tick.tick().await;
            upkeep.run_upkeep();
        }
    });
    tokio::spawn(async move {
        while usr1.recv().await.is_some() {
            tracing::info!("metrics:\n{}", metrics.render());
        }
    });
    let mut poll = tokio::time::interval(Duration::from_millis(100));
    let signal = loop {
        tokio::select! {
            _ = int.recv() => break "SIGINT",
            _ = term.recv() => break "SIGTERM",
            _ = hup.recv() => break "SIGHUP",
            _ = poll.tick() => if session.guard.is_finished() {
                // Unmounted from outside (`fusermount3 -u`, `umount`), or the session died.
                return match tokio::task::spawn_blocking(|| session.join()).await {
                    Ok(Ok(())) => {
                        tracing::info!("unmounted externally; exiting");
                        Ok(())
                    }
                    Ok(Err(e)) => Err(StartupError::Session(e)),
                    Err(e) => Err(StartupError::Session(io::Error::other(e))),
                };
            },
        }
    };
    tracing::info!("{signal}: unmounting");
    match tokio::task::spawn_blocking(|| session.umount_and_join()).await {
        Ok(Ok(())) => {
            tracing::info!("unmounted");
            Ok(())
        }
        Ok(Err(e)) => Err(StartupError::Session(e)),
        Err(e) => Err(StartupError::Session(io::Error::other(e))),
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use rustern_client::Service;

    use super::*;

    #[test]
    fn exit_codes() {
        let e = StartupError::InvalidArgs(CliError::Invalid("bad".into()));
        assert_eq!(e.exit_code(), 2);

        let e = StartupError::Registry {
            addr: "10.1.2.3:10001".into(),
            source: ClientError::Timeout {
                service: Service::Registry,
                elapsed: Duration::from_secs(10),
            },
        };
        assert_eq!(e.exit_code(), 1);
        assert!(e.to_string().contains("10.1.2.3:10001"), "{e}");

        let e = StartupError::Mount {
            path: "/no/such".into(),
            source: io::Error::from(io::ErrorKind::NotFound),
        };
        assert_eq!(e.exit_code(), 1);
        assert!(e.to_string().contains("/no/such"), "{e}");
    }
}
