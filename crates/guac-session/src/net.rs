use crate::error::SessionError;
use guac_protocol::Status;
use std::io::ErrorKind;
use std::time::Duration;
use tokio::net::TcpStream;

/// Connect over TCP, trying every resolved address. The timeout applies to each attempt.
///
/// Failure mapping (the C plugins did not check or distinguish connect failures; see DEVIATIONS):
/// unresolvable host -> UPSTREAM_NOT_FOUND, timeout -> UPSTREAM_TIMEOUT, anything else -> UPSTREAM_UNAVAILABLE.
pub async fn connect_tcp(
    host: &str,
    port: u16,
    timeout_secs: u64,
) -> Result<TcpStream, SessionError> {
    let addrs: Vec<_> = tokio::net::lookup_host((host, port))
        .await
        .map_err(|e| {
            SessionError::plugin(
                Status::UpstreamNotFound,
                format!("Unable to resolve host \"{host}\": {e}"),
            )
        })?
        .collect();
    if addrs.is_empty() {
        return Err(SessionError::plugin(
            Status::UpstreamNotFound,
            format!("No addresses found for \"{host}\"."),
        ));
    }
    let mut last = SessionError::plugin(Status::UpstreamUnavailable, "Unable to connect.");
    for addr in addrs {
        match tokio::time::timeout(
            Duration::from_secs(timeout_secs.max(1)),
            TcpStream::connect(addr),
        )
        .await
        {
            Ok(Ok(s)) => {
                let _ = s.set_nodelay(true);
                return Ok(s);
            }
            Ok(Err(e)) => {
                let status = if e.kind() == ErrorKind::TimedOut {
                    Status::UpstreamTimeout
                } else {
                    Status::UpstreamUnavailable
                };
                last = SessionError::plugin(
                    status,
                    format!("Unable to connect to \"{host}:{port}\": {e}"),
                );
            }
            Err(_) => {
                last = SessionError::plugin(
                    Status::UpstreamTimeout,
                    format!("Timed out connecting to \"{host}:{port}\"."),
                );
            }
        }
    }
    Err(last)
}
