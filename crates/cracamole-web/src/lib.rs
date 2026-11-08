#![forbid(unsafe_code)]

mod assets;
mod auth;
mod protocols;
mod proxy;
mod store;
mod trust;
mod tunnel;
pub use proxy::parse_trusted_proxies;

use axum::{
    extract::{ConnectInfo, DefaultBodyLimit, Path, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    middleware::{self, Next},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use serde_json::json;
use std::{net::SocketAddr, path::PathBuf, sync::Arc};
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;

/// How the web layer reaches the Guacamole daemon.
#[derive(Clone)]
pub enum BackendLink {
    /// In-memory pipes to a daemon in the same process: no socket exists for anything else to use.
    Memory(tokio::sync::mpsc::Sender<(tokio::io::DuplexStream, String)>),
    /// A TCP daemon. Only for tests: that socket carries no authentication.
    Tcp(SocketAddr),
}

impl From<SocketAddr> for BackendLink {
    fn from(address: SocketAddr) -> Self {
        Self::Tcp(address)
    }
}

pub struct WebConfig {
    pub username: String,
    pub password: String,
    pub data_dir: PathBuf,
    pub secure_cookie: bool,
    pub trusted_proxies: Vec<std::net::IpAddr>,
    pub backend: BackendLink,
    pub shutdown: CancellationToken,
}

struct App {
    auth: auth::Auth,
    store: store::Store,
    backend: BackendLink,
    tunnels: Arc<Semaphore>,
    probes: Arc<Semaphore>,
    trusted_proxies: Vec<std::net::IpAddr>,
    shutdown: CancellationToken,
}

#[derive(Debug)]
pub struct ApiError {
    status: StatusCode,
    message: &'static str,
    field: Option<&'static str>,
}
impl ApiError {
    fn new(status: StatusCode, message: &'static str) -> Self {
        Self {
            status,
            message,
            field: None,
        }
    }
}
impl From<store::Invalid> for ApiError {
    fn from(invalid: store::Invalid) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: invalid.message,
            field: Some(invalid.field),
        }
    }
}
impl From<StatusCode> for ApiError {
    fn from(status: StatusCode) -> Self {
        Self::new(
            status,
            match status {
                StatusCode::UNAUTHORIZED => "請先登入",
                StatusCode::FORBIDDEN => "來源驗證失敗",
                _ => "請求失敗",
            },
        )
    }
}
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = match self.field {
            Some(field) => json!({"error":self.message,"field":field}),
            None => json!({"error":self.message}),
        };
        (self.status, Json(body)).into_response()
    }
}
fn storage_error(error: std::io::Error) -> ApiError {
    tracing::error!("connection storage failed: {error}");
    ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "無法保存連線設定")
}

pub async fn router(config: WebConfig) -> Result<Router, String> {
    let app = Arc::new(App {
        auth: auth::Auth::new(config.username, config.password, config.secure_cookie).await?,
        store: store::Store::open(config.data_dir)
            .await
            .map_err(|e| e.to_string())?,
        backend: config.backend,
        tunnels: Arc::new(Semaphore::new(32)),
        probes: Arc::new(Semaphore::new(4)),
        trusted_proxies: config.trusted_proxies,
        shutdown: config.shutdown,
    });
    Ok(Router::new()
        .route("/", get(|| async { Html(assets::INDEX) }))
        .route("/assets/{*path}", get(asset))
        .route("/health", get(|| async { Json(json!({"status":"ok"})) }))
        .route("/api/login", post(login))
        .route("/api/logout", post(logout))
        .route("/api/session", get(session))
        .route("/api/protocols", get(protocols::list))
        .route("/api/connections", get(list).post(create))
        .route(
            "/api/connections/{id}",
            get(detail).put(update).delete(remove),
        )
        .route("/api/connections/{id}/tunnel", get(tunnel::upgrade))
        .route(
            "/api/connections/{id}/certificate",
            post(trust::certificate),
        )
        .route("/api/connections/{id}/host-key", post(trust::host_key))
        .route("/api/logs", get(get_logs))
        .layer(DefaultBodyLimit::max(128 * 1024))
        .layer(middleware::from_fn(security_headers))
        .with_state(app))
}

async fn asset(Path(path): Path<String>) -> Result<Response, ApiError> {
    let asset =
        assets::find(&path).ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "找不到檔案"))?;
    Ok(([(header::CONTENT_TYPE, asset.mime)], asset.body).into_response())
}

async fn security_headers(request: axum::extract::Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    for (name,value) in [
        ("cache-control","no-store"), ("x-content-type-options","nosniff"), ("referrer-policy","no-referrer"),
        ("content-security-policy","default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; connect-src 'self'; object-src 'none'; frame-ancestors 'none'; base-uri 'none'"),
    ] {response.headers_mut().insert(name,HeaderValue::from_static(value));}
    response
}

#[derive(Deserialize)]
struct Login {
    username: String,
    password: String,
}
async fn login(
    State(app): State<Arc<App>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(login): Json<Login>,
) -> Result<Response, ApiError> {
    auth::same_origin(&headers)?;
    let source_ip = proxy::client_ip(&headers, peer.ip(), &app.trusted_proxies)?;
    let token = app
        .auth
        .login(source_ip, login.username, login.password)
        .await
        .map_err(|status| {
            tracing::warn!(
                "web login failed from {}: HTTP {}",
                source_ip,
                status.as_u16()
            );
            ApiError::new(
                status,
                if status == StatusCode::TOO_MANY_REQUESTS {
                    "登入嘗試過於頻繁，請稍後重試"
                } else {
                    "帳號或密碼不正確"
                },
            )
        })?;
    tracing::info!("web login succeeded from {}", source_ip);
    Ok((
        [(header::SET_COOKIE, app.auth.cookie(&token))],
        Json(json!({"authenticated":true})),
    )
        .into_response())
}
async fn logout(State(app): State<Arc<App>>, headers: HeaderMap) -> Result<Response, ApiError> {
    auth::same_origin(&headers)?;
    app.auth.logout(&headers).await;
    Ok((
        [(
            header::SET_COOKIE,
            format!(
                "{}=; HttpOnly; SameSite=Strict; Path=/; Max-Age=0",
                auth::COOKIE
            ),
        )],
        StatusCode::NO_CONTENT,
    )
        .into_response())
}
async fn session(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, ApiError> {
    app.auth.authorize(&headers).await?;
    Ok(Json(json!({"authenticated":true})))
}
async fn list(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
) -> Result<Json<Vec<store::PublicConnection>>, ApiError> {
    app.auth.authorize(&headers).await?;
    Ok(Json(app.store.list().await))
}
async fn detail(
    State(app): State<Arc<App>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<store::PublicConnection>, ApiError> {
    app.auth.authorize(&headers).await?;
    app.store
        .get(&id)
        .await
        .map(|c| Json(c.public()))
        .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "找不到連線"))
}
async fn create(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    Json(draft): Json<store::Draft>,
) -> Result<Response, ApiError> {
    app.auth.authorize(&headers).await?;
    auth::same_origin(&headers)?;
    store::validate(&draft)?;
    let connection = app
        .store
        .save(None, draft)
        .await
        .map_err(storage_error)?
        .unwrap();
    tracing::info!("connection created: {}", connection.id);
    Ok((StatusCode::CREATED, Json(connection)).into_response())
}
async fn update(
    State(app): State<Arc<App>>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(draft): Json<store::Draft>,
) -> Result<Json<store::PublicConnection>, ApiError> {
    app.auth.authorize(&headers).await?;
    auth::same_origin(&headers)?;
    store::validate(&draft)?;
    app.store
        .save(Some(&id), draft)
        .await
        .map_err(storage_error)?
        .map(Json)
        .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "找不到連線"))
}
async fn remove(
    State(app): State<Arc<App>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<StatusCode, ApiError> {
    app.auth.authorize(&headers).await?;
    auth::same_origin(&headers)?;
    if app.store.delete(&id).await.map_err(storage_error)? {
        tracing::info!("connection deleted: {id}");
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::new(StatusCode::NOT_FOUND, "找不到連線"))
    }
}

#[derive(Deserialize)]
struct LogQuery {
    limit: Option<usize>,
    level: Option<String>,
}

async fn get_logs(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    axum::extract::Query(query): axum::extract::Query<LogQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    app.auth.authorize(&headers).await?;
    let logs = guacd::logging::recent_logs(query.limit, query.level.as_deref());
    Ok(Json(json!({ "logs": logs })))
}
