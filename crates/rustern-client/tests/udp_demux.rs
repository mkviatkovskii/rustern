//! T040: the UDP demultiplexer against an in-process fake shard.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)] // test code

mod common;

use std::time::Duration;

use common::*;
use rustern_client::ClientError;
use rustern_client::udp::Udp;
use rustern_msgs::frame::Reply;
use rustern_msgs::wire::decode_exact;
use rustern_msgs::{ErrorCode, LookupResp, ShardMessageKind};

const MTU: usize = 1472;

fn body_of(reply: Reply<ShardMessageKind>) -> LookupResp {
    match reply {
        Reply::Ok {
            kind: ShardMessageKind::Lookup,
            body,
        } => decode_exact(body).unwrap(),
        other => panic!("unexpected reply {other:?}"),
    }
}

async fn udp() -> Udp {
    Udp::bind_to("127.0.0.1:0".parse().unwrap(), MTU)
        .await
        .unwrap()
}

#[test]
fn out_of_order_replies_reach_their_callers() {
    rt().block_on(async {
        let shard = FakeShard::bind().await;
        let udp = udp().await;
        let mut a = udp.begin();
        let mut b = udp.begin();
        let fa = rustern_msgs::frame::encode_shard_req(a.id(), &lookup("a")).unwrap();
        let fb = rustern_msgs::frame::encode_shard_req(b.id(), &lookup("b")).unwrap();
        udp.send(&fa, shard.addr()).await.unwrap();
        udp.send(&fb, shard.addr()).await.unwrap();
        let (ra, from) = shard.recv().await;
        let (rb, _) = shard.recv().await;
        assert_eq!((ra.request_id, rb.request_id), (a.id(), b.id()));
        // Answer b first, then a.
        shard.reply(rb.request_id, &lookup_resp(0xb), from).await;
        shard.reply(ra.request_id, &lookup_resp(0xa), from).await;
        assert_eq!(body_of(b.reply().await.unwrap()).target_id.0, 0xb);
        assert_eq!(body_of(a.reply().await.unwrap()).target_id.0, 0xa);
    });
}

#[test]
fn request_ids_start_at_one_and_increase() {
    rt().block_on(async {
        let udp = udp().await;
        let ids: Vec<u64> = (0..5).map(|_| udp.begin().id()).collect();
        assert_eq!(ids, [1, 2, 3, 4, 5]);
    });
}

#[test]
fn dropping_a_caller_removes_its_entry() {
    rt().block_on(async {
        let udp = udp().await;
        let a = udp.begin();
        let b = udp.begin();
        assert_eq!(udp.pending_len(), 2);
        drop(a);
        assert_eq!(udp.pending_len(), 1);
        // Cancelling a future that waits for the reply also cleans up.
        let mut c = udp.begin();
        let r = tokio::time::timeout(Duration::from_millis(10), c.reply()).await;
        assert!(r.is_err());
        drop(c);
        drop(b);
        assert_eq!(udp.pending_len(), 0);
    });
}

#[test]
fn duplicate_and_late_replies_are_counted_and_dropped() {
    let ((), metrics) = with_recorder(async {
        let shard = FakeShard::bind().await;
        let udp = udp().await;
        let to = udp.local_addr().unwrap();
        let mut a = udp.begin();
        shard.reply(a.id(), &lookup_resp(1), to).await;
        shard.reply(a.id(), &lookup_resp(2), to).await; // duplicate
        assert_eq!(body_of(a.reply().await.unwrap()).target_id.0, 1);
        let late = udp.begin();
        let late_id = late.id();
        drop(late);
        shard.reply(late_id, &lookup_resp(3), to).await; // late
        shard.reply(999, &lookup_resp(4), to).await; // never asked
        // Nothing reaches `a` again.
        assert!(
            tokio::time::timeout(Duration::from_millis(50), a.reply())
                .await
                .is_err()
        );
        // The delivered reply already removed `a`'s entry.
        assert_eq!(udp.pending_len(), 0);
    });
    assert_eq!(
        sample(&metrics.render(), "rustern_udp_unmatched_total"),
        Some(3.0)
    );
}

#[test]
fn malformed_datagrams_are_counted_and_dropped() {
    let ((), metrics) = with_recorder(async {
        let shard = FakeShard::bind().await;
        let udp = udp().await;
        let to = udp.local_addr().unwrap();
        let mut a = udp.begin();
        let good = rustern_msgs::frame::encode_shard_resp(a.id(), &lookup_resp(1)).unwrap();
        shard.send_raw(&good[..5], to).await; // short
        let mut wrong_protocol = good.clone();
        wrong_protocol[0] ^= 0xff;
        shard.send_raw(&wrong_protocol, to).await;
        let mut unknown_kind = good.clone();
        unknown_kind[12] = 0xfe;
        shard.send_raw(&unknown_kind, to).await;
        shard.send_raw(&good, to).await;
        assert_eq!(body_of(a.reply().await.unwrap()).target_id.0, 1);
    });
    let rendered = metrics.render();
    assert_eq!(sample(&rendered, "rustern_udp_malformed_total"), Some(3.0));
    assert_eq!(sample(&rendered, "rustern_udp_unmatched_total"), None);
}

#[test]
fn frames_over_the_mtu_are_rejected_before_sending() {
    rt().block_on(async {
        let shard = FakeShard::bind().await;
        let udp = Udp::bind_to("127.0.0.1:0".parse().unwrap(), 64)
            .await
            .unwrap();
        let p = udp.begin();
        let name = "x".repeat(100);
        let frame = rustern_msgs::frame::encode_shard_req(p.id(), &lookup(&name)).unwrap();
        let err = udp.send(&frame, shard.addr()).await.unwrap_err();
        assert!(
            matches!(err, ClientError::TooLarge { mtu: 64, .. }),
            "{err:?}"
        );
        assert!(shard.try_recv(Duration::from_millis(50)).await.is_none());
    });
}

#[test]
fn error_reply_is_a_tern_error() {
    rt().block_on(async {
        let shard = FakeShard::bind().await;
        let (client, _registry) = client(Some(shard.addr()), None).await;
        let server = async {
            let (req, from) = shard.recv().await;
            shard
                .reply_err(req.request_id, ErrorCode::NAME_NOT_FOUND, from)
                .await;
        };
        let req = lookup("missing");
        let (res, ()) = tokio::join!(client.shard_request(0, &req), server);
        match res {
            Err(ClientError::Tern { code, kind }) => {
                assert_eq!(code, ErrorCode::NAME_NOT_FOUND);
                assert_eq!(kind, "LOOKUP");
            }
            other => panic!("expected Tern error, got {other:?}"),
        }
        assert_eq!(client.pending_requests(), 0);
    });
}
