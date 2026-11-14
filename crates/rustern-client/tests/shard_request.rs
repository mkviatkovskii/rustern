//! T042: the shard retry loop against two fake shard sockets.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)] // test code

mod common;

use std::net::SocketAddr;
use std::time::{Duration, Instant};

use common::*;
use rustern_client::{ClientError, ReqTimeouts, Service};
use rustern_msgs::ErrorCode;
use tokio::sync::mpsc;

/// A datagram seen by one of the two fakes.
struct Seen {
    which: usize,
    id: u64,
    from: SocketAddr,
    at: Instant,
}

/// Forwards every request either fake receives to one channel.
fn watch(
    a: FakeShard,
    b: FakeShard,
) -> (
    mpsc::UnboundedReceiver<Seen>,
    [std::sync::Arc<FakeShard>; 2],
) {
    let (tx, rx) = mpsc::unbounded_channel();
    let shards = [std::sync::Arc::new(a), std::sync::Arc::new(b)];
    for (which, s) in shards.iter().enumerate() {
        let (s, tx) = (std::sync::Arc::clone(s), tx.clone());
        tokio::spawn(async move {
            loop {
                let (req, from) = s.recv().await;
                let _ = tx.send(Seen {
                    which,
                    id: req.request_id,
                    from,
                    at: Instant::now(),
                });
            }
        });
    }
    (rx, shards)
}

#[test]
fn lost_replies_are_retried_alternating_addresses() {
    rt().block_on(async {
        let (a, b) = (FakeShard::bind().await, FakeShard::bind().await);
        let (client, _r) = client(Some(a.addr()), Some(b.addr())).await;
        let (mut seen, shards) = watch(a, b);
        let req = lookup("f");
        let call = client.shard_request(3, &req);
        let server = async {
            let mut log = Vec::new();
            for _ in 0..4 {
                log.push(seen.recv().await.unwrap());
            }
            let last = log.last().unwrap();
            shards[last.which]
                .reply(last.id, &lookup_resp(42), last.from)
                .await;
            log
        };
        let (res, log) = tokio::join!(call, server);
        assert_eq!(res.unwrap().target_id.0, 42);
        // One request id for every attempt, alternating Addr1, Addr2, Addr1, Addr2.
        assert!(log.iter().all(|s| s.id == log[0].id));
        assert_eq!(
            log.iter().map(|s| s.which).collect::<Vec<_>>(),
            [0, 1, 0, 1]
        );
        // Gaps follow the schedule (20, 30, 45 ms ±5%) with no extra sleep.
        for (pair, expected) in log.windows(2).zip([20u64, 30, 45]) {
            let gap = pair[1].at - pair[0].at;
            let expected = Duration::from_millis(expected);
            assert!(gap >= expected.mul_f64(0.9), "gap {gap:?} < {expected:?}");
            assert!(
                gap <= expected + Duration::from_millis(40),
                "gap {gap:?} ≫ {expected:?}"
            );
        }
        assert_eq!(client.pending_requests(), 0);
    });
}

#[test]
fn unknown_second_address_uses_the_first() {
    rt().block_on(async {
        let a = FakeShard::bind().await;
        let (client, _r) = client(Some(a.addr()), None).await;
        let req = lookup("f");
        let server = async {
            let (r1, _) = a.recv().await;
            let (r2, from) = a.recv().await;
            assert_eq!(r1.request_id, r2.request_id);
            a.reply(r2.request_id, &lookup_resp(1), from).await;
        };
        let (res, ()) = tokio::join!(client.shard_request(0, &req), server);
        assert_eq!(res.unwrap().target_id.0, 1);
    });
}

#[test]
fn no_address_is_an_error() {
    rt().block_on(async {
        let (client, _r) = client(None, None).await;
        let err = client.shard_request(9, &lookup("f")).await.unwrap_err();
        assert!(
            matches!(err, ClientError::NoAddress(Service::Shard(9))),
            "{err:?}"
        );
    });
}

#[test]
fn silence_times_out_at_the_overall_deadline() {
    rt().block_on(async {
        let (a, b) = (FakeShard::bind().await, FakeShard::bind().await);
        let overall = Duration::from_millis(300);
        let (client, _r) = client_with(Some(a.addr()), Some(b.addr()), |c| {
            c.shard = ReqTimeouts { overall, ..c.shard };
        })
        .await;
        let start = Instant::now();
        let err = client.shard_request(1, &lookup("f")).await.unwrap_err();
        let elapsed = start.elapsed();
        assert!(
            matches!(
                err,
                ClientError::Timeout {
                    service: Service::Shard(1),
                    ..
                }
            ),
            "{err:?}"
        );
        assert!(elapsed >= overall, "{elapsed:?}");
        assert!(
            elapsed <= overall + Duration::from_millis(50),
            "{elapsed:?}"
        );
        assert_eq!(client.pending_requests(), 0);
        drop((a, b));
    });
}

#[test]
fn timeout_error_is_retried_other_errors_are_not() {
    rt().block_on(async {
        let a = FakeShard::bind().await;
        let (client, _r) = client(Some(a.addr()), None).await;

        // TIMEOUT, then success: retried like a lost reply.
        let req = lookup("f");
        let server = async {
            let (r, from) = a.recv().await;
            a.reply_err(r.request_id, ErrorCode::TIMEOUT, from).await;
            let (r2, from) = a.recv().await;
            assert_eq!(r.request_id, r2.request_id);
            a.reply(r2.request_id, &lookup_resp(5), from).await;
        };
        let (res, ()) = tokio::join!(client.shard_request(0, &req), server);
        assert_eq!(res.unwrap().target_id.0, 5);

        // Any other code returns at once, without resending.
        let server = async {
            let (r, from) = a.recv().await;
            a.reply_err(r.request_id, ErrorCode::DIRECTORY_NOT_FOUND, from)
                .await;
        };
        let start = Instant::now();
        let (res, ()) = tokio::join!(client.shard_request(0, &req), server);
        assert!(start.elapsed() < Duration::from_millis(20));
        match res {
            Err(ClientError::Tern { code, .. }) => assert_eq!(code, ErrorCode::DIRECTORY_NOT_FOUND),
            other => panic!("{other:?}"),
        }
        assert!(a.try_recv(Duration::from_millis(100)).await.is_none());
    });
}
