//! Shard requests with upstream's retry loop (research R4, `go/client/metadatareq.go`):
//! resend as soon as an attempt times out, alternating Addr1/Addr2, until the overall
//! deadline. A `TIMEOUT` error reply is retried like a lost reply; other codes return.

use std::net::SocketAddr;
use std::time::Instant;

use rustern_msgs::frame::{Reply, encode_shard_req};
use rustern_msgs::wire::decode_exact;
use rustern_msgs::{ErrorCode, ReadOnlyRequest, ShardRequest, ShardResponse};
use tracing::Instrument;

use crate::Client;
use crate::error::{ClientError, Service};
use crate::metrics;
use crate::timeouts::schedule;

impl Client {
    /// Sends a read-only request to `shard` and returns its response.
    ///
    /// Read-only requests compile:
    ///
    /// ```no_run
    /// # async fn f(c: &rustern_client::Client) {
    /// let _ = c.shard_request(0, &rustern_msgs::LookupReq::default()).await;
    /// # }
    /// ```
    ///
    /// Mutating ones do not:
    ///
    /// ```compile_fail,E0277
    /// # async fn f(c: &rustern_client::Client) {
    /// let _ = c.shard_request(0, &rustern_msgs::CreateDirectoryInodeReq::default()).await;
    /// # }
    /// ```
    pub async fn shard_request<Req>(
        &self,
        shard: u8,
        req: &Req,
    ) -> Result<Req::Response, ClientError>
    where
        Req: ShardRequest + ReadOnlyRequest,
    {
        let start = Instant::now();
        let span = tracing::debug_span!(
            "shard_request",
            kind = Req::KIND.as_str(),
            shard,
            attempt = tracing::field::Empty
        );
        let deadline = self.inner.config.shard.overall;
        let res = tokio::time::timeout(deadline, self.attempts(shard, req).instrument(span))
            .await
            .unwrap_or_else(|_| {
                Err(ClientError::Timeout {
                    service: Service::Shard(shard),
                    elapsed: start.elapsed(),
                })
            });
        metrics::shard_request(
            Req::KIND,
            match &res {
                Ok(_) => metrics::OK,
                Err(ClientError::Timeout { .. }) => metrics::TIMEOUT,
                Err(_) => metrics::ERROR,
            },
            start.elapsed(),
        );
        res
    }

    async fn attempts<Req: ShardRequest>(
        &self,
        shard: u8,
        req: &Req,
    ) -> Result<Req::Response, ClientError> {
        if self.shard_addrs(shard).iter().all(Option::is_none) {
            return Err(ClientError::NoAddress(Service::Shard(shard)));
        }
        let mut pending = self.inner.udp.begin();
        let frame = encode_shard_req(pending.id(), req)?;
        let mut which = 0usize;
        for (attempt, timeout) in schedule(&self.inner.config.shard).enumerate() {
            tracing::Span::current().record("attempt", attempt);
            // Re-read each attempt: a refresh may have moved the shard.
            let to = pick(self.shard_addrs(shard), which);
            which = which.wrapping_add(1);
            if let Some(to) = to
                && let Err(e) = self.inner.udp.send(&frame, to).await
            {
                if matches!(e, ClientError::TooLarge { .. }) {
                    return Err(e);
                }
                tracing::debug!(error = %e, %to, "send failed; retrying");
                self.refresh_soon();
            }
            match tokio::time::timeout(timeout, pending.reply()).await {
                Err(_) => {
                    // Attempt timed out: resend, and check whether the shard moved.
                    self.refresh_soon();
                    continue;
                }
                Ok(reply) => match reply? {
                    Reply::Ok { kind, body } if kind == <Req::Response as ShardResponse>::KIND => {
                        return Ok(decode_exact(body)?);
                    }
                    Reply::Ok { .. } => {
                        return Err(ClientError::Malformed {
                            what: "unexpected shard response kind",
                        });
                    }
                    Reply::Err(ErrorCode::TIMEOUT) => pending.rearm(),
                    Reply::Err(code) => {
                        return Err(ClientError::Tern {
                            code,
                            kind: Req::KIND.as_str(),
                        });
                    }
                },
            }
        }
        // The schedule never ends; the overall deadline stops the loop.
        Err(ClientError::Timeout {
            service: Service::Shard(shard),
            elapsed: self.inner.config.shard.overall,
        })
    }
}

/// Address for attempt `which`: alternate, skipping an unknown one.
fn pick(addrs: [Option<SocketAddr>; 2], which: usize) -> Option<SocketAddr> {
    let [a, b] = addrs;
    if which.is_multiple_of(2) {
        a.or(b)
    } else {
        b.or(a)
    }
}
