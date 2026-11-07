//! Authenticated, bounded server-identity discovery. No remote credentials are sent.

use crate::{auth, store::Connection, ApiError, App};
use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    Json,
};
use guac_session::{SessionError, Status};
use serde_json::{json, Value};
use std::{
    future::Future,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::OwnedSemaphorePermit;
use tokio_util::sync::CancellationToken;

struct Probe {
    _permit: OwnedSemaphorePermit,
    expires: Instant,
    cancel: CancellationToken,
}

async fn authorize(
    app: &App,
    headers: &HeaderMap,
    id: &str,
) -> Result<(Connection, Probe), ApiError> {
    let (expires, cancel) = app.auth.authorize(headers).await?;
    auth::same_origin(headers)?;
    let connection = app
        .store
        .get(id)
        .await
        .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "找不到連線"))?;
    let permit = app.probes.clone().try_acquire_owned().map_err(|_| {
        ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "主機身分探測忙碌，請稍後再試",
        )
    })?;
    Ok((
        connection,
        Probe {
            _permit: permit,
            expires,
            cancel,
        },
    ))
}

impl Probe {
    async fn run<T>(
        &self,
        app: &App,
        future: impl Future<Output = Result<T, SessionError>>,
    ) -> Result<T, ApiError> {
        tokio::select! {
            biased;
            _ = self.cancel.cancelled() => Err(StatusCode::UNAUTHORIZED.into()),
            _ = app.shutdown.cancelled() => Err(ApiError::new(StatusCode::SERVICE_UNAVAILABLE, "服務正在關閉")),
            _ = tokio::time::sleep_until(tokio::time::Instant::from_std(self.expires)) => Err(StatusCode::UNAUTHORIZED.into()),
            result = tokio::time::timeout(Duration::from_secs(10), future) => match result {
                Err(_) => Err(ApiError::new(StatusCode::GATEWAY_TIMEOUT, "主機身分探測逾時，請確認主機與連接埠")),
                Ok(Err(error)) => {
                    tracing::warn!("server identity probe failed: {error}");
                    Err(ApiError::new(if error.status() == Status::UpstreamTimeout { StatusCode::GATEWAY_TIMEOUT } else { StatusCode::BAD_GATEWAY }, "無法取得主機身分，請確認服務與設定"))
                }
                Ok(Ok(info)) => Ok(info),
            }
        }
    }
}

fn endpoint(connection: &Connection, default_port: u16) -> (&str, u16) {
    let host = connection
        .params
        .get("hostname")
        .map(String::as_str)
        .unwrap_or("");
    let port = connection
        .params
        .get("port")
        .and_then(|p| p.parse().ok())
        .unwrap_or(default_port);
    (host, port)
}

pub async fn certificate(
    State(app): State<Arc<App>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    let (connection, probe) = authorize(&app, &headers, &id).await?;
    if connection.protocol != "rdp" {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "只有 RDP 連線有伺服器憑證",
        ));
    }
    let (host, port) = endpoint(&connection, 3389);
    let info = probe
        .run(&app, guacd::probe_certificate(host, port, 10))
        .await?;
    let pinned = connection
        .params
        .get("server-cert-sha256")
        .and_then(|p| guacd::normalize_fingerprint(p));
    tracing::info!("certificate probe for {id}: sha256 {}", info.sha256);
    Ok(Json(
        json!({ "sha256": info.sha256, "subject": info.subject, "issuer": info.issuer,
        "pinned": pinned, "matches_pin": pinned.as_deref() == Some(info.sha256.as_str()) }),
    ))
}

pub async fn host_key(
    State(app): State<Arc<App>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    let (connection, probe) = authorize(&app, &headers, &id).await?;
    if connection.protocol != "ssh" {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "只有 SSH 連線有主機金鑰",
        ));
    }
    let (host, port) = endpoint(&connection, 22);
    let configured = connection
        .params
        .get("host-key")
        .filter(|s| !s.is_empty())
        .map(String::as_str);
    let info = probe
        .run(&app, guacd::probe_host_key(host, port, configured, 10))
        .await?;
    Ok(Json(
        json!({"public_key": info.public_key, "fingerprint": info.fingerprint,
        "trusted": info.trusted, "matches": info.matches}),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn pending_probe_exits_on_logout_shutdown_and_expiry() {
        let dir = tempfile::tempdir().unwrap();
        let app = App {
            auth: auth::Auth::new("admin".into(), "long-admin-password".into(), false)
                .await
                .unwrap(),
            store: crate::store::Store::open(dir.path().into()).await.unwrap(),
            backend: "127.0.0.1:1".parse().unwrap(),
            tunnels: Arc::new(tokio::sync::Semaphore::new(32)),
            probes: Arc::new(tokio::sync::Semaphore::new(4)),
            trusted_proxies: Vec::new(),
            shutdown: CancellationToken::new(),
        };
        let cancel = CancellationToken::new();
        let draft = crate::store::Draft {
            name: "probe".into(),
            protocol: "rdp".into(),
            params: std::collections::BTreeMap::from([("hostname".into(), "127.0.0.1".into())]),
        };
        let connection = app.store.save(None, draft).await.unwrap().unwrap();
        let token = app
            .auth
            .login(
                "127.0.0.1".parse().unwrap(),
                "admin".into(),
                "long-admin-password".into(),
            )
            .await
            .unwrap();
        let mut headers = HeaderMap::new();
        headers.insert(
            "cookie",
            format!("{}={token}", auth::COOKIE).parse().unwrap(),
        );
        headers.insert("host", "localhost".parse().unwrap());
        headers.insert("origin", "http://localhost".parse().unwrap());
        let mut guards = Vec::new();
        for _ in 0..4 {
            guards.push(authorize(&app, &headers, &connection.id).await.unwrap().1);
        }
        let overflow = authorize(&app, &headers, &connection.id)
            .await
            .err()
            .unwrap();
        assert_eq!(overflow.status, StatusCode::TOO_MANY_REQUESTS);
        app.auth.logout(&headers).await;
        assert_eq!(
            guards[0]
                .run::<()>(&app, std::future::pending())
                .await
                .unwrap_err()
                .status,
            StatusCode::UNAUTHORIZED
        );
        drop(guards);
        assert_eq!(app.probes.available_permits(), 4);
        let mut probe = Probe {
            _permit: app.probes.clone().try_acquire_owned().unwrap(),
            expires: Instant::now() + Duration::from_secs(60),
            cancel: cancel.clone(),
        };
        cancel.cancel();
        assert_eq!(
            probe
                .run::<()>(&app, std::future::pending())
                .await
                .unwrap_err()
                .status,
            StatusCode::UNAUTHORIZED
        );
        probe.cancel = CancellationToken::new();
        probe.expires = Instant::now();
        assert_eq!(
            probe
                .run::<()>(&app, std::future::pending())
                .await
                .unwrap_err()
                .status,
            StatusCode::UNAUTHORIZED
        );
        probe.expires = Instant::now() + Duration::from_secs(60);
        app.shutdown.cancel();
        assert_eq!(
            probe
                .run::<()>(&app, std::future::pending())
                .await
                .unwrap_err()
                .status,
            StatusCode::SERVICE_UNAVAILABLE
        );
    }
}
