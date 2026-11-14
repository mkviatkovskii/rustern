//! T101 (edge cases "shard leader change" and "Registry unreachable after start-up"): a
//! shard that stops answering makes the client refresh its shard table, at most once per
//! second, and the retry loop picks up the new address.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)] // test code

mod common;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use common::*;
use rustern_client::{Client, ClientError, Service};

/// Answers every request it receives with `lookup_resp(target)`; counts them.
fn answering(shard: FakeShard, target: u64) -> Arc<AtomicUsize> {
    let n = Arc::new(AtomicUsize::new(0));
    let count = Arc::clone(&n);
    tokio::spawn(async move {
        loop {
            let (req, from) = shard.recv().await;
            count.fetch_add(1, Ordering::SeqCst);
            shard
                .reply(req.request_id, &lookup_resp(target), from)
                .await;
        }
    });
    n
}

#[test]
fn moved_shard_is_found_after_a_refresh() {
    rt().block_on(async {
        let (old, new) = (FakeShard::bind().await, FakeShard::bind().await);
        let table = Arc::new(Mutex::new(shards_resp(Some(old.addr()), None)));
        let asked = Arc::new(AtomicUsize::new(0));
        let (registry, _r) =
            fake_registry(Registry::Shared(Arc::clone(&table), Arc::clone(&asked))).await;
        let client = Client::connect(config(registry)).await.unwrap();
        assert_eq!(asked.load(Ordering::SeqCst), 1);
        // The leader moves; `old` stays bound but silent.
        let new_addr = new.addr();
        *table.lock().unwrap() = shards_resp(Some(new_addr), None);
        let served = answering(new, 42);

        let start = Instant::now();
        let r = client.shard_request(3, &lookup("f")).await.unwrap();
        assert_eq!(r.target_id.0, 42);
        assert!(start.elapsed() < client.config().shard.overall);
        assert_eq!(served.load(Ordering::SeqCst), 1);
        assert_eq!(client.shard_addrs(3)[0], Some(new_addr));
        drop(old);
    });
}

#[test]
fn refresh_is_rate_limited_to_once_per_second() {
    rt().block_on(async {
        let silent = FakeShard::bind().await;
        let table = Arc::new(Mutex::new(shards_resp(Some(silent.addr()), None)));
        let asked = Arc::new(AtomicUsize::new(0));
        let (registry, _r) =
            fake_registry(Registry::Shared(Arc::clone(&table), Arc::clone(&asked))).await;
        let client = Client::connect(config(registry)).await.unwrap();
        // Ten concurrent requests to a silent shard for the whole 1 s deadline.
        let calls = (0..10u8).map(|i| {
            let c = client.clone();
            async move { c.shard_request(i, &lookup("f")).await }
        });
        let results = futures_util::future::join_all(calls).await;
        assert!(results.iter().all(|r| matches!(
            r,
            Err(ClientError::Timeout {
                service: Service::Shard(_),
                ..
            })
        )));
        // connect + at most one refresh per started second of failures.
        let n = asked.load(Ordering::SeqCst);
        assert!((2..=3).contains(&n), "{n} registry requests");
        drop(silent);
    });
}

#[test]
fn registry_down_after_start() {
    rt().block_on(async {
        let (silent, other, moved) = (
            FakeShard::bind().await,
            FakeShard::bind().await,
            FakeShard::bind().await,
        );
        let (silent_addr, other_addr) = (silent.addr(), other.addr());
        let table = Arc::new(Mutex::new(shards_resp_with(|s| {
            if s == 3 {
                (Some(silent_addr), None)
            } else {
                (Some(other_addr), None)
            }
        })));
        let asked = Arc::new(AtomicUsize::new(0));
        let (registry, task) =
            fake_registry(Registry::Shared(Arc::clone(&table), Arc::clone(&asked))).await;
        let client = Client::connect(config(registry)).await.unwrap();
        task.abort();
        let _ = task.await;
        // Shard 3 moves, but nobody can tell the client.
        *table.lock().unwrap() = shards_resp(Some(moved.addr()), None);
        let served = answering(other, 7);

        let start = Instant::now();
        let err = client.shard_request(3, &lookup("f")).await.unwrap_err();
        let took = start.elapsed();
        assert!(
            matches!(
                err,
                ClientError::Timeout {
                    service: Service::Shard(3),
                    ..
                }
            ),
            "{err:?}"
        );
        let overall = client.config().shard.overall;
        assert!(
            took >= overall && took <= overall + Duration::from_millis(100),
            "{took:?}"
        );
        assert_eq!(
            client
                .shard_request(4, &lookup("f"))
                .await
                .unwrap()
                .target_id
                .0,
            7
        );
        assert_eq!(served.load(Ordering::SeqCst), 1);
        drop((silent, moved));
    });
}
