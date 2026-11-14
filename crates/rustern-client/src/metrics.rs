//! Metric names and recording helpers. Label values come only from the constant lists
//! below, never from inodes, blocks or addresses (FR-017). Every call is a no-op when no
//! `metrics` recorder is installed.

use std::time::Duration;

use rustern_msgs::ShardMessageKind;

/// `rustern_shard_requests_total{kind,result}`.
pub const SHARD_REQUESTS: &str = "rustern_shard_requests_total";
/// `rustern_shard_request_seconds{kind}`.
pub const SHARD_REQUEST_SECONDS: &str = "rustern_shard_request_seconds";
/// `rustern_registry_requests_total{result}`.
pub const REGISTRY_REQUESTS: &str = "rustern_registry_requests_total";
/// `rustern_block_fetches_total{result}`.
pub const BLOCK_FETCHES: &str = "rustern_block_fetches_total";
/// `rustern_block_fetch_seconds`.
pub const BLOCK_FETCH_SECONDS: &str = "rustern_block_fetch_seconds";
/// `rustern_block_bytes_total`.
pub const BLOCK_BYTES: &str = "rustern_block_bytes_total";
/// `rustern_block_connect_seconds`: TCP connect time of a new block-service connection
/// (its count against `rustern_block_fetches_total` shows connection reuse).
pub const BLOCK_CONNECT_SECONDS: &str = "rustern_block_connect_seconds";
/// `rustern_block_header_seconds`: request sent to response header received.
pub const BLOCK_HEADER_SECONDS: &str = "rustern_block_header_seconds";
/// `rustern_udp_unmatched_total`: responses with no waiting request.
pub const UDP_UNMATCHED: &str = "rustern_udp_unmatched_total";
/// `rustern_udp_malformed_total`: datagrams that are not valid response frames.
pub const UDP_MALFORMED: &str = "rustern_udp_malformed_total";

/// Request succeeded.
pub const OK: &str = "ok";
/// Service returned an error code, or the response was invalid.
pub const ERROR: &str = "error";
/// Overall deadline passed.
pub const TIMEOUT: &str = "timeout";
/// Every `result` label value.
pub const RESULTS: [&str; 3] = [OK, ERROR, TIMEOUT];

/// Every `kind` label value: upstream shard message names.
pub fn shard_kinds() -> impl Iterator<Item = &'static str> {
    ShardMessageKind::ALL
        .into_iter()
        .map(ShardMessageKind::as_str)
}

/// Records one finished shard request.
pub fn shard_request(kind: ShardMessageKind, result: &'static str, elapsed: Duration) {
    metrics::counter!(SHARD_REQUESTS, "kind" => kind.as_str(), "result" => result).increment(1);
    metrics::histogram!(SHARD_REQUEST_SECONDS, "kind" => kind.as_str())
        .record(elapsed.as_secs_f64());
}

/// Records one registry request.
pub fn registry_request(result: &'static str) {
    metrics::counter!(REGISTRY_REQUESTS, "result" => result).increment(1);
}

/// Records one block fetch.
pub fn block_fetch(result: &'static str, elapsed: Duration, bytes: u64) {
    metrics::counter!(BLOCK_FETCHES, "result" => result).increment(1);
    metrics::histogram!(BLOCK_FETCH_SECONDS).record(elapsed.as_secs_f64());
    metrics::counter!(BLOCK_BYTES).increment(bytes);
}

/// Records one new block-service connection.
pub fn block_connect(elapsed: Duration) {
    metrics::histogram!(BLOCK_CONNECT_SECONDS).record(elapsed.as_secs_f64());
}

/// Records the wait from sending a block request to its response header.
pub fn block_header(elapsed: Duration) {
    metrics::histogram!(BLOCK_HEADER_SECONDS).record(elapsed.as_secs_f64());
}

/// Records a response nobody was waiting for.
pub fn udp_unmatched() {
    metrics::counter!(UDP_UNMATCHED).increment(1);
}

/// Records a datagram that is not a valid response frame.
pub fn udp_malformed() {
    metrics::counter!(UDP_MALFORMED).increment(1);
}
