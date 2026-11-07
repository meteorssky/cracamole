#![forbid(unsafe_code)]

mod auth;
mod store;
mod tunnel;

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

pub struct WebConfig {
    pub username: String,
    pub password: String,
    pub data_dir: PathBuf,
    pub secure_cookie: bool,
    pub backend: SocketAddr,
    pub shutdown: CancellationToken,
}

struct App {
    auth: auth::Auth,
    store: store::Store,
    backend: SocketAddr,
    tunnels: Arc<Semaphore>,
    shutdown: CancellationToken,
}

pub struct ApiError {
    status: StatusCode,
    message: &'static str,
}
impl ApiError {
    fn new(status: StatusCode, message: &'static str) -> Self {
        Self { status, message }
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
        (self.status, Json(json!({"error":self.message}))).into_response()
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
        shutdown: config.shutdown,
    });
    Ok(Router::new()
        .route(
            "/",
            get(|| async { Html(include_str!("../../../web/index.html")) }),
        )
        .route(
            "/assets/app.js",
            get(|| async {
                (
                    [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
                    include_str!("../../../web/app.js"),
                )
            }),
        )
        .route(
            "/assets/style.css",
            get(|| async {
                (
                    [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
                    include_str!("../../../web/style.css"),
                )
            }),
        )
        .route(
            "/assets/guacamole.js",
            get(|| async {
                (
                    [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
                    include_str!("../../../web/vendor/guacamole.js"),
                )
            }),
        )
        .route("/health", get(|| async { Json(json!({"status":"ok"})) }))
        .route(
            "/assets/NOTICE",
            get(|| async {
                (
                    [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
                    include_str!("../../../web/vendor/NOTICE"),
                )
            }),
        )
        .route(
            "/assets/LICENSE",
            get(|| async {
                (
                    [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
                    include_str!("../../../web/vendor/LICENSE"),
                )
            }),
        )
        .route("/api/login", post(login))
        .route("/api/logout", post(logout))
        .route("/api/session", get(session))
        .route("/api/connections", get(list).post(create))
        .route(
            "/api/connections/{id}",
            get(detail).put(update).delete(remove),
        )
        .route("/api/connections/{id}/tunnel", get(tunnel::upgrade))
        .layer(DefaultBodyLimit::max(128 * 1024))
        .layer(middleware::from_fn(security_headers))
        .with_state(app))
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
    let token = app
        .auth
        .login(peer.ip(), login.username, login.password)
        .await
        .map_err(|status| {
            tracing::warn!(
                "web login failed from {}: HTTP {}",
                peer.ip(),
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
    tracing::info!("web login succeeded from {}", peer.ip());
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
    store::validate(&draft).map_err(|e| ApiError::new(StatusCode::BAD_REQUEST, e))?;
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
    store::validate(&draft).map_err(|e| ApiError::new(StatusCode::BAD_REQUEST, e))?;
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
