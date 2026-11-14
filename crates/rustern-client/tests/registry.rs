//! T044: registry requests, the shard table and the refresh alert against a fake TCP
//! registry.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)] // test code

mod common;

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use common::*;
use rustern_client::{Client, ClientError, ReqTimeouts, Service};
use tracing::field::{Field, Visit};
use tracing_subscriber::Layer;
use tracing_subscriber::layer::{Context, SubscriberExt};

#[test]
fn shard_table_comes_from_the_registry() {
    rt().block_on(async {
        let addr = |ip: [u8; 4], port: u16| Some(SocketAddr::from((ip, port)));
        let resp = shards_resp_with(|i| {
            (
                addr([10, 0, 0, i], 1000 + u16::from(i)),
                addr([10, 0, 1, i], 2000 + u16::from(i)),
            )
        });
        let (registry, _r) = fake_registry(Registry::Answer(resp)).await;
        let client = Client::connect(config(registry)).await.unwrap();
        for i in [0u8, 1, 77, 255] {
            assert_eq!(
                client.shard_addrs(i),
                [
                    addr([10, 0, 0, i], 1000 + u16::from(i)),
                    addr([10, 0, 1, i], 2000 + u16::from(i))
                ]
            );
        }
    });
}

#[test]
fn port_zero_means_no_address() {
    rt().block_on(async {
        let a = SocketAddr::from(([10, 0, 0, 1], 5));
        let (client, _r) = client(Some(a), None).await;
        assert_eq!(client.shard_addrs(4), [Some(a), None]);
    });
}

#[test]
fn table_without_256_shards_is_rejected() {
    rt().block_on(async {
        let mut resp = shards_resp(None, None);
        resp.shards.pop();
        let (registry, _r) = fake_registry(Registry::Answer(resp)).await;
        let err = Client::connect(config(registry)).await.unwrap_err();
        assert!(matches!(err, ClientError::Malformed { .. }), "{err:?}");
    });
}

#[test]
fn oversized_length_is_rejected_before_reading() {
    rt().block_on(async {
        let (registry, _r) = fake_registry(Registry::Length(2 << 20)).await;
        let start = Instant::now();
        let err = Client::connect(config(registry)).await.unwrap_err();
        // Fails at once on the header; the fake never sends the 2 MiB body.
        assert!(start.elapsed() < Duration::from_millis(500), "{err:?}");
        assert!(matches!(err, ClientError::Malformed { .. }), "{err:?}");
    });
}

#[test]
fn error_and_wrong_kind_replies_fail() {
    use rustern_msgs::frame::Reply;
    use rustern_msgs::{ErrorCode, RegistryMessageKind};
    rt().block_on(async {
        let (registry, _r) = fake_registry(Registry::Reply(Reply::Err(ErrorCode(52)))).await;
        let err = Client::connect(config(registry)).await.unwrap_err();
        assert!(
            matches!(
                err,
                ClientError::Tern {
                    code: ErrorCode(52),
                    ..
                }
            ),
            "{err:?}"
        );

        let kind = *RegistryMessageKind::ALL
            .iter()
            .find(|k| k.as_str() != "LOCAL_SHARDS")
            .unwrap();
        let reply = Reply::Ok {
            kind,
            body: bytes::Bytes::new(),
        };
        let (registry, _r) = fake_registry(Registry::Reply(reply)).await;
        let err = Client::connect(config(registry)).await.unwrap_err();
        assert!(matches!(err, ClientError::Malformed { .. }), "{err:?}");
    });
}

#[test]
fn refused_connection_names_the_address() {
    rt().block_on(async {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap().to_string();
        drop(l);
        let err = Client::connect(config(addr.clone())).await.unwrap_err();
        assert!(
            matches!(&err, ClientError::Connect { addr: a, .. } if *a == addr),
            "{err:?}"
        );
        assert!(err.to_string().contains(&addr), "{err}");
    });
}

#[test]
fn silent_registry_times_out_within_the_shard_deadline() {
    rt().block_on(async {
        let (registry, _r) = fake_registry(Registry::Silent).await;
        let mut c = config(registry);
        let overall = Duration::from_millis(200);
        c.shard = ReqTimeouts { overall, ..c.shard };
        let start = Instant::now();
        let err = Client::connect(c).await.unwrap_err();
        assert!(
            matches!(
                err,
                ClientError::Timeout {
                    service: Service::Registry,
                    ..
                }
            ),
            "{err:?}"
        );
        assert!(start.elapsed() <= overall + Duration::from_millis(50));
    });
}

/// Events with `alert = true`: their `registry` field.
#[derive(Clone, Default)]
struct Alerts(Arc<Mutex<Vec<String>>>);

#[derive(Default)]
struct Fields {
    alert: bool,
    registry: Option<String>,
}

impl Visit for Fields {
    fn record_bool(&mut self, field: &Field, value: bool) {
        if field.name() == "alert" {
            self.alert = value;
        }
    }
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "registry" {
            self.registry = Some(format!("{value:?}"));
        }
    }
}

impl<S: tracing::Subscriber> Layer<S> for Alerts {
    fn on_event(&self, event: &tracing::Event<'_>, _: Context<'_, S>) {
        let mut f = Fields::default();
        event.record(&mut f);
        if f.alert && *event.metadata().level() == tracing::Level::ERROR {
            self.0.lock().unwrap().push(f.registry.unwrap_or_default());
        }
    }
}

#[test]
fn one_alert_after_failures_last_longer_than_alert_after() {
    let alerts = Alerts::default();
    let subscriber = tracing_subscriber::registry().with(alerts.clone());
    tracing::subscriber::with_default(subscriber, || {
        rt().block_on(async {
            let (registry, server) = fake_registry(Registry::Answer(shards_resp(None, None))).await;
            let mut c = config(registry.clone());
            c.registry_refresh = Duration::from_millis(50);
            c.registry_alert_after = Duration::from_millis(200);
            let client = Client::connect(c).await.unwrap();

            // Answering: refreshes succeed, no alert.
            tokio::time::sleep(Duration::from_millis(300)).await;
            assert!(alerts.0.lock().unwrap().is_empty());

            // Closed: one alert once failures have lasted more than 200 ms, not one per tick.
            server.abort();
            tokio::time::sleep(Duration::from_millis(150)).await;
            assert!(
                alerts.0.lock().unwrap().is_empty(),
                "alert before 200 ms of failures"
            );
            tokio::time::sleep(Duration::from_millis(450)).await;
            let got = alerts.0.lock().unwrap().clone();
            assert_eq!(got.len(), 1, "{got:?}");
            assert!(got[0].contains(&registry), "{got:?}");
            drop(client);
        });
    });
}
