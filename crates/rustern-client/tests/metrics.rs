//! T047: metrics recorded by the client, read back from a local Prometheus recorder.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)] // test code

mod common;

use common::*;
use rustern_client::{Client, metrics as m};

#[test]
fn lookup_duplicate_and_refused_registry_are_recorded() {
    let ((), handle) = with_recorder(async {
        let shard = FakeShard::bind().await;
        let (client, _r) = client(Some(shard.addr()), None).await;
        let req = lookup("f");
        let server = async {
            let (r, from) = shard.recv().await;
            shard.reply(r.request_id, &lookup_resp(1), from).await;
            shard.reply(r.request_id, &lookup_resp(1), from).await; // duplicate
        };
        let (res, ()) = tokio::join!(client.shard_request(0, &req), server);
        res.unwrap();

        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let refused = l.local_addr().unwrap().to_string();
        drop(l);
        assert!(Client::connect(config(refused)).await.is_err());

        // Give the receive task time to count the duplicate.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    });
    let text = handle.render();
    assert_eq!(
        sample(
            &text,
            r#"rustern_shard_requests_total{kind="LOOKUP",result="ok"}"#
        ),
        Some(1.0),
        "{text}"
    );
    assert_eq!(
        sample(
            &text,
            r#"rustern_shard_request_seconds_count{kind="LOOKUP"}"#
        ),
        Some(1.0),
        "{text}"
    );
    assert_eq!(
        sample(&text, "rustern_udp_unmatched_total"),
        Some(1.0),
        "{text}"
    );
    assert_eq!(
        sample(&text, r#"rustern_registry_requests_total{result="ok"}"#),
        Some(1.0),
        "{text}"
    );
    assert_eq!(
        sample(&text, r#"rustern_registry_requests_total{result="error"}"#),
        Some(1.0),
        "{text}"
    );
    assert_labels_from_constants(&text);
}

/// Every `kind` and `result` label value is one of the exported constants, and no other
/// label names exist (FR-017: no inode, block or address values).
fn assert_labels_from_constants(text: &str) {
    let kinds: Vec<&str> = m::shard_kinds().collect();
    for line in text
        .lines()
        .filter(|l| !l.starts_with('#') && l.contains('{'))
    {
        let labels = &line[line.find('{').unwrap() + 1..line.find('}').unwrap()];
        for pair in labels.split(',') {
            let (k, v) = pair.split_once('=').unwrap();
            let v = v.trim_matches('"');
            match k {
                "kind" => assert!(kinds.contains(&v), "{line}"),
                "result" => assert!(m::RESULTS.contains(&v), "{line}"),
                "quantile" => {}
                other => panic!("unexpected label {other} in {line}"),
            }
        }
    }
}
