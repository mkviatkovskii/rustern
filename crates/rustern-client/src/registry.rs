//! Registry requests: one TCP connection per request (research R4).

use std::time::{Duration, Instant};

use bytes::Bytes;
use rustern_msgs::frame::{
    REGISTRY_RESP_HEADER_LEN, Reply, decode_registry_data, encode_registry_req, registry_resp_len,
};
use rustern_msgs::wire::decode_exact;
use rustern_msgs::{RegistryRequest, RegistryResponse};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::error::{ClientError, Service};
use crate::metrics;

/// Largest registry response accepted; checked before the body is allocated.
pub const MAX_REGISTRY_RESP: usize = 1 << 20;

/// Sends `req` to the registry at `addr` (`host:port`) and waits up to `deadline`.
pub async fn registry_request<R: RegistryRequest>(
    addr: &str,
    req: &R,
    deadline: Duration,
) -> Result<R::Response, ClientError> {
    let start = Instant::now();
    let res = tokio::time::timeout(deadline, exchange(addr, req))
        .await
        .unwrap_or_else(|_| {
            Err(ClientError::Timeout {
                service: Service::Registry,
                elapsed: start.elapsed(),
            })
        });
    metrics::registry_request(match &res {
        Ok(_) => metrics::OK,
        Err(ClientError::Timeout { .. }) => metrics::TIMEOUT,
        Err(_) => metrics::ERROR,
    });
    res
}

async fn exchange<R: RegistryRequest>(addr: &str, req: &R) -> Result<R::Response, ClientError> {
    let frame = encode_registry_req(req)?;
    let mut s = TcpStream::connect(addr)
        .await
        .map_err(|source| ClientError::Connect {
            addr: addr.to_owned(),
            source,
        })?;
    s.write_all(&frame).await?;
    let mut header = [0u8; REGISTRY_RESP_HEADER_LEN];
    s.read_exact(&mut header).await?;
    let len = registry_resp_len(header, MAX_REGISTRY_RESP)?;
    let mut data = vec![0u8; len];
    s.read_exact(&mut data).await?;
    match decode_registry_data(Bytes::from(data))? {
        Reply::Ok { kind, body } if kind == <R::Response as RegistryResponse>::KIND => {
            Ok(decode_exact(body)?)
        }
        Reply::Ok { .. } => Err(ClientError::Malformed {
            what: "unexpected registry response kind",
        }),
        Reply::Err(code) => Err(ClientError::Tern {
            code,
            kind: R::KIND.as_str(),
        }),
    }
}
