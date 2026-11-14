//! T100: requests through a lossy, duplicating, reordering and damaging relay in front of
//! one real shard. Every request returns what it returns without the relay, or a typed
//! error within the deadline; nothing panics; damaged datagrams are counted.
//!
//! Random bit flips inside otherwise valid bodies are not tested: the protocol has no
//! message checksum, so a flipped field is undetectable by any client.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)] // test code

mod common;
#[path = "support/relay.rs"]
mod relay;

/// Serialised with the other fault tests (`.config/nextest.toml`).
mod cluster_serial {
    use std::time::{Duration, Instant};

    use bytes::Bytes;
    use rustern_client::metrics::{UDP_MALFORMED, UDP_UNMATCHED};
    use rustern_client::{Client, ClientConfig, ClientError};
    use rustern_msgs::{InodeId, LookupReq, NameHash, ReadDirReq};

    use super::common::{config, sample, with_recorder};
    use super::relay::{Faults, relay};

    const ROOT: InodeId = InodeId(0x2000_0000_0000_0000);
    const NAMES: [&str; 3] = ["corpus", "scratch", "no-such-name"];

    fn registry() -> String {
        std::env::var("RUSTERN_TEST_REGISTRY").unwrap_or_else(|_| "127.0.0.1:10001".into())
    }

    fn lookup(name: &str) -> LookupReq {
        LookupReq {
            dir_id: ROOT,
            name: Bytes::copy_from_slice(name.as_bytes()),
        }
    }

    fn readdir() -> ReadDirReq {
        ReadDirReq {
            dir_id: ROOT,
            start_hash: NameHash(0),
            mtu: 1472,
        }
    }

    /// Request `i`'s outcome as a comparable string: the response, or the error code.
    async fn ask(c: &Client, i: usize) -> Result<String, ClientError> {
        let r = if i % 4 == 3 {
            c.shard_request(0, &readdir())
                .await
                .map(|r| format!("{r:?}"))
        } else {
            c.shard_request(0, &lookup(NAMES[i % 4]))
                .await
                .map(|r| format!("{:?}", r.target_id))
        };
        match r {
            Err(ClientError::Tern { code, .. }) => Ok(format!("tern error {code:?}")),
            other => other,
        }
    }

    #[test]
    #[ignore = "needs cluster: run via scripts/gate.sh"]
    fn requests_through_a_faulty_relay() {
        let ((), rendered) = with_recorder(async {
            let direct = Client::connect(ClientConfig::new(registry()))
                .await
                .unwrap();
            let mut want = Vec::new();
            for i in 0..4 {
                want.push(ask(&direct, i).await.unwrap());
            }
            let shard = direct.shard_addrs(0)[0].expect("shard 0 address");
            let faults = Faults {
                loss: 0.3,
                dup: 0.3,
                max_delay: Duration::from_millis(50),
                damage: 0.2,
            };
            let r = relay(shard, faults, 0x5eed).await;
            let mut c = config(registry());
            c.shard_overrides.insert(0, [r.addr, r.addr]);
            let client = Client::connect(c).await.unwrap();
            let overall = client.config().shard.overall;
            let (mut ok, mut failed) = (0, 0);
            for i in 0..200 {
                let start = Instant::now();
                let got = ask(&client, i).await;
                let took = start.elapsed();
                assert!(
                    took <= overall + Duration::from_millis(100),
                    "request {i} took {took:?}"
                );
                match got {
                    Ok(v) => {
                        assert_eq!(v, want[i % 4], "request {i}");
                        ok += 1;
                    }
                    Err(e) => {
                        assert!(
                            matches!(
                                e,
                                ClientError::Timeout { .. }
                                    | ClientError::Decode(_)
                                    | ClientError::Malformed { .. }
                            ),
                            "request {i}: {e:?}"
                        );
                        failed += 1;
                    }
                }
            }
            eprintln!("{ok} matched, {failed} typed errors");
            assert!(ok > 100, "only {ok} of 200 succeeded");
            assert_eq!(client.pending_requests(), 0);
        });
        let rendered = rendered.render();
        for name in [UDP_UNMATCHED, UDP_MALFORMED] {
            let n = sample(&rendered, name).unwrap_or(0.0);
            assert!(n > 0.0, "{name} = {n}\n{rendered}");
        }
    }
}
