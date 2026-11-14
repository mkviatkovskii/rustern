//! T062: concurrent stats of one inode share one request; nothing is cached (Principle VI).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)] // test code

mod common;

use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::*;
use futures_util::future::join_all;
use rustern_client::{Client, ClientError, ReqTimeouts};
use rustern_core::inode::TernInode;
use rustern_msgs::{
    ErrorCode, InodeId, ShardMessageKind, StatDirectoryResp, StatFileResp, TernTime,
};
use tokio::task::JoinHandle;

const DIR_A: u64 = (1 << 61) | (5 << 8) | 3;
const DIR_B: u64 = (1 << 61) | (6 << 8) | 3;
const FILE_A: u64 = (2 << 61) | (5 << 8) | 3;
const FILE_B: u64 = (2 << 61) | (6 << 8) | 3;

/// A shard that answers each new request after 100 ms, counting distinct request ids.
struct Counting {
    requests: Arc<AtomicU64>,
    fail_with: Arc<Mutex<Option<ErrorCode>>>,
    _task: JoinHandle<()>,
}

impl Counting {
    fn count(&self) -> u64 {
        self.requests.load(Ordering::SeqCst)
    }
}

async fn setup() -> (Client, Counting, JoinHandle<()>) {
    let shard = Arc::new(FakeShard::bind().await);
    let requests = Arc::new(AtomicU64::new(0));
    let fail_with = Arc::new(Mutex::new(None));
    let (r, f, s) = (
        Arc::clone(&requests),
        Arc::clone(&fail_with),
        Arc::clone(&shard),
    );
    let task = tokio::spawn(async move {
        let mut seen = HashSet::new();
        loop {
            let (req, from) = s.recv().await;
            if !seen.insert(req.request_id) {
                continue; // a resend of a request already counted
            }
            let n = r.fetch_add(1, Ordering::SeqCst) + 1;
            let fail = *f.lock().unwrap();
            let s = Arc::clone(&s);
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(100)).await;
                match (fail, req.kind) {
                    (Some(code), _) => s.reply_err(req.request_id, code, from).await,
                    (None, ShardMessageKind::StatFile) => {
                        let resp = StatFileResp {
                            mtime: TernTime(n),
                            atime: TernTime(n),
                            size: n,
                        };
                        s.reply(req.request_id, &resp, from).await;
                    }
                    (None, ShardMessageKind::StatDirectory) => {
                        let resp = StatDirectoryResp {
                            mtime: TernTime(n),
                            owner: InodeId(1 << 61),
                            ..StatDirectoryResp::default()
                        };
                        s.reply(req.request_id, &resp, from).await;
                    }
                    (None, k) => panic!("unexpected {k:?}"),
                }
            });
        }
    });
    // One attempt per request: no resends within the 100 ms answer delay.
    let (client, registry) = client_with(Some(shard.addr()), None, |c| {
        c.shard = ReqTimeouts {
            initial: Duration::from_millis(500),
            max: Duration::from_millis(500),
            overall: Duration::from_secs(2),
        };
    })
    .await;
    let counting = Counting {
        requests,
        fail_with,
        _task: task,
    };
    (client, counting, registry)
}

fn ino(id: u64) -> TernInode {
    TernInode::new(id).unwrap()
}

#[test]
fn concurrent_stat_directory_sends_one_request() {
    rt().block_on(async {
        let (client, shard, _r) = setup().await;
        let all = join_all((0..64).map(|_| client.stat_directory(ino(DIR_A)))).await;
        assert_eq!(shard.count(), 1);
        for r in &all {
            assert_eq!(r.as_ref().unwrap().mtime, TernTime(1));
        }

        // Nothing is cached: the next call is a new request.
        let again = client.stat_directory(ino(DIR_A)).await.unwrap();
        assert_eq!((shard.count(), again.mtime), (2, TernTime(2)));

        // Different inodes are not coalesced.
        let (a, b) = tokio::join!(
            client.stat_directory(ino(DIR_A)),
            client.stat_directory(ino(DIR_B))
        );
        assert_ne!(a.unwrap().mtime, b.unwrap().mtime);
        assert_eq!(shard.count(), 4);
    });
}

#[test]
fn concurrent_stat_file_sends_one_request() {
    rt().block_on(async {
        let (client, shard, _r) = setup().await;
        let all = join_all((0..64).map(|_| client.stat_file(ino(FILE_A)))).await;
        assert_eq!(shard.count(), 1);
        for r in &all {
            assert_eq!(r.as_ref().unwrap().size, 1);
        }
        let again = client.stat_file(ino(FILE_A)).await.unwrap();
        assert_eq!((shard.count(), again.size), (2, 2));
        let (a, b) = tokio::join!(client.stat_file(ino(FILE_A)), client.stat_file(ino(FILE_B)));
        assert_ne!(a.unwrap().size, b.unwrap().size);
        assert_eq!(shard.count(), 4);
    });
}

#[test]
fn a_failure_reaches_every_waiter_and_is_not_cached() {
    rt().block_on(async {
        let (client, shard, _r) = setup().await;
        *shard.fail_with.lock().unwrap() = Some(ErrorCode::DIRECTORY_NOT_FOUND);
        let all = join_all((0..64).map(|_| client.stat_directory(ino(DIR_A)))).await;
        assert_eq!(shard.count(), 1);
        for r in all {
            match r {
                Err(e) => match e.root() {
                    ClientError::Tern { code, kind } => {
                        assert_eq!(
                            (*code, *kind),
                            (ErrorCode::DIRECTORY_NOT_FOUND, "STAT_DIRECTORY")
                        );
                    }
                    other => panic!("{other:?}"),
                },
                Ok(r) => panic!("{r:?}"),
            }
        }
        *shard.fail_with.lock().unwrap() = None;
        client.stat_directory(ino(DIR_A)).await.unwrap();
        assert_eq!(shard.count(), 2);

        *shard.fail_with.lock().unwrap() = Some(ErrorCode::FILE_NOT_FOUND);
        let all = join_all((0..8).map(|_| client.stat_file(ino(FILE_A)))).await;
        assert_eq!(shard.count(), 3);
        assert!(all.iter().all(Result::is_err));
        *shard.fail_with.lock().unwrap() = None;
        client.stat_file(ino(FILE_A)).await.unwrap();
        assert_eq!(shard.count(), 4);
    });
}
