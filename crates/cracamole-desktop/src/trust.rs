//! 對桌面介面提供有限額、可取消的 SSH／RDP 身分探測。
use std::{future::Future, sync::Arc, time::Duration};
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeError {
    Busy,
    Cancelled,
    TimedOut,
    HostNotFound,
    Unavailable,
    InvalidConfig,
    Failed,
}

fn classify(error: &guac_session::SessionError) -> ProbeError {
    match error.status() {
        guac_protocol::Status::UpstreamTimeout | guac_protocol::Status::ClientTimeout => {
            ProbeError::TimedOut
        }
        guac_protocol::Status::UpstreamNotFound | guac_protocol::Status::ResourceNotFound => {
            ProbeError::HostNotFound
        }
        guac_protocol::Status::UpstreamUnavailable => ProbeError::Unavailable,
        guac_protocol::Status::ClientBadRequest => ProbeError::InvalidConfig,
        _ => ProbeError::Failed,
    }
}

/// 同一個應用程式應共用一個實例，以限制所有探測的總併發數。
pub struct Probes {
    limit: Arc<Semaphore>,
    shutdown: CancellationToken,
}

impl Probes {
    pub fn new(shutdown: CancellationToken) -> Self {
        Self {
            limit: Arc::new(Semaphore::new(4)),
            shutdown,
        }
    }

    async fn run<T, E>(
        &self,
        cancel: &CancellationToken,
        future: impl Future<Output = Result<T, E>>,
    ) -> Result<T, ProbeError> {
        if self.shutdown.is_cancelled() || cancel.is_cancelled() {
            return Err(ProbeError::Cancelled);
        }
        let _permit = self
            .limit
            .clone()
            .try_acquire_owned()
            .map_err(|_| ProbeError::Busy)?;
        tokio::select! {
            biased;
            _ = self.shutdown.cancelled() => Err(ProbeError::Cancelled),
            _ = cancel.cancelled() => Err(ProbeError::Cancelled),
            result = tokio::time::timeout(Duration::from_secs(10), future) => match result {
                Err(_) => Err(ProbeError::TimedOut),
                Ok(Err(_)) => Err(ProbeError::Failed),
                Ok(Ok(value)) => Ok(value),
            },
        }
    }

    pub async fn ssh(
        &self,
        host: &str,
        port: u16,
        configured: Option<&str>,
        cancel: &CancellationToken,
    ) -> Result<guacd::HostKeyInfo, ProbeError> {
        self.run(cancel, async {
            Ok::<_, ()>(guacd::probe_host_key(host, port, configured, 10).await)
        })
        .await?
        .map_err(|error| classify(&error))
    }

    pub async fn rdp(
        &self,
        host: &str,
        port: u16,
        cancel: &CancellationToken,
    ) -> Result<guacd::CertInfo, ProbeError> {
        self.run(cancel, async {
            Ok::<_, ()>(guacd::probe_certificate(host, port, 10).await)
        })
        .await?
        .map_err(|error| classify(&error))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncReadExt;
    use tokio::net::TcpListener;
    use tokio::sync::Barrier;

    #[tokio::test]
    async fn refused_ssh_probe_reports_unavailable_without_error_text() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let probes = Probes::new(CancellationToken::new());
        let result = probes
            .ssh("127.0.0.1", port, None, &CancellationToken::new())
            .await;
        assert!(matches!(result, Err(ProbeError::Unavailable)));
    }

    #[tokio::test]
    async fn limits_and_cancels_pending_probes() {
        let shutdown = CancellationToken::new();
        let probes = Arc::new(Probes::new(shutdown.clone()));
        let barrier = Arc::new(Barrier::new(5));
        let mut tasks = Vec::new();
        for _ in 0..4 {
            let probes = probes.clone();
            let barrier = barrier.clone();
            tasks.push(tokio::spawn(async move {
                let cancel = CancellationToken::new();
                probes
                    .run(&cancel, async move {
                        barrier.wait().await;
                        std::future::pending::<Result<(), ()>>().await
                    })
                    .await
            }));
        }
        barrier.wait().await;
        let cancel = CancellationToken::new();
        assert_eq!(
            probes.run(&cancel, async { Ok::<_, ()>(()) }).await,
            Err(ProbeError::Busy)
        );
        shutdown.cancel();
        for task in tasks {
            assert_eq!(task.await.unwrap(), Err(ProbeError::Cancelled));
        }
        assert_eq!(probes.limit.available_permits(), 4);
    }

    #[tokio::test]
    async fn cancelling_a_real_ssh_probe_closes_the_socket() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let shutdown = CancellationToken::new();
        let probes = Arc::new(Probes::new(shutdown));
        let cancel = CancellationToken::new();
        let task = {
            let probes = probes.clone();
            let cancel = cancel.clone();
            tokio::spawn(async move { probes.ssh("127.0.0.1", port, None, &cancel).await })
        };
        let (mut socket, _) = tokio::time::timeout(Duration::from_secs(3), listener.accept())
            .await
            .unwrap()
            .unwrap();
        cancel.cancel();
        assert!(matches!(task.await.unwrap(), Err(ProbeError::Cancelled)));
        let mut buf = [0u8; 1024];
        tokio::time::timeout(Duration::from_secs(3), async {
            while socket.read(&mut buf).await.unwrap() != 0 {}
        })
        .await
        .expect("主機探測取消後 socket 未關閉");
    }
}
