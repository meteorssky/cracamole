use axum::{
    body::{to_bytes, Body},
    extract::ConnectInfo,
    http::{Request, StatusCode},
    Router,
};
use cracamole_web::{router, WebConfig};
use futures_util::{SinkExt, StreamExt};
use guac_protocol::{Instruction, Parser};
use guac_session::{BoxFuture, InputEvent, Params, ProtocolPlugin, SessionCtx, SessionError};
use serde_json::{json, Value};
use std::{net::SocketAddr, sync::Arc, time::Duration};
use tokio::net::TcpListener;
use tokio_tungstenite::{
    connect_async,
    tungstenite::{client::IntoClientRequest, Message},
};
use tokio_util::sync::CancellationToken;
use tower::ServiceExt;

const PASSWORD: &str = "correct-admin-password";

async fn app(
    directory: &std::path::Path,
    backend: SocketAddr,
    shutdown: CancellationToken,
) -> Router {
    router(WebConfig {
        username: "admin".into(),
        password: PASSWORD.into(),
        data_dir: directory.into(),
        secure_cookie: false,
        backend,
        shutdown,
    })
    .await
    .unwrap()
}
async fn call(
    app: &Router,
    method: &str,
    path: &str,
    body: Option<Value>,
    cookie: Option<&str>,
    origin: &str,
) -> (StatusCode, axum::http::HeaderMap, Value) {
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .header("host", "localhost")
        .header("origin", origin)
        .header("content-type", "application/json")
        .extension(ConnectInfo(
            "127.0.0.1:12345".parse::<SocketAddr>().unwrap(),
        ));
    if let Some(cookie) = cookie {
        request = request.header("cookie", cookie);
    }
    let response = app
        .clone()
        .oneshot(
            request
                .body(body.map_or_else(Body::empty, |v| Body::from(v.to_string())))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = to_bytes(response.into_body(), 2 * 1024 * 1024)
        .await
        .unwrap();
    (
        status,
        headers,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}
async fn login(app: &Router) -> String {
    let (status, headers, _) = call(
        app,
        "POST",
        "/api/login",
        Some(json!({"username":"admin","password":PASSWORD})),
        None,
        "http://localhost",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let cookie = headers["set-cookie"].to_str().unwrap();
    assert!(cookie.contains("HttpOnly") && cookie.contains("SameSite=Strict"));
    cookie.split(';').next().unwrap().into()
}
fn draft() -> Value {
    json!({"name":"Development","protocol":"ssh","params":{"hostname":"configured.example","port":"22","username":"tester","password":"backend-password","private-key":"test-private-key","passphrase":"test-passphrase"}})
}

#[tokio::test]
async fn web_auth_rejects_bad_credentials_and_cross_origin_and_revokes_logout() {
    let dir = tempfile::tempdir().unwrap();
    let app = app(
        dir.path(),
        "127.0.0.1:1".parse().unwrap(),
        CancellationToken::new(),
    )
    .await;
    assert_eq!(
        call(
            &app,
            "GET",
            "/api/connections",
            None,
            None,
            "http://localhost"
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    let credentials = json!({"username":"admin","password":PASSWORD});
    assert_eq!(
        call(
            &app,
            "POST",
            "/api/login",
            Some(credentials),
            None,
            "https://attacker.invalid"
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(
            &app,
            "POST",
            "/api/login",
            Some(json!({"username":"admin","password":"wrong"})),
            None,
            "http://localhost"
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    let cookie = login(&app).await;
    assert_eq!(
        call(
            &app,
            "GET",
            "/api/session",
            None,
            Some(&cookie),
            "http://localhost"
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        call(
            &app,
            "POST",
            "/api/logout",
            None,
            Some(&cookie),
            "http://attacker.invalid"
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(
            &app,
            "POST",
            "/api/logout",
            None,
            Some(&cookie),
            "http://localhost"
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        call(
            &app,
            "GET",
            "/api/session",
            None,
            Some(&cookie),
            "http://localhost"
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn web_connection_crud_persists_across_restart_without_returning_credentials() {
    let dir = tempfile::tempdir().unwrap();
    let service = app(
        dir.path(),
        "127.0.0.1:1".parse().unwrap(),
        CancellationToken::new(),
    )
    .await;
    let cookie = login(&service).await;
    assert_eq!(
        call(
            &service,
            "POST",
            "/api/connections",
            Some(draft()),
            Some(&cookie),
            "http://foreign.invalid"
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let (status, _, connection) = call(
        &service,
        "POST",
        "/api/connections",
        Some(draft()),
        Some(&cookie),
        "http://localhost",
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let id = connection["id"].as_str().unwrap();
    let path = format!("/api/connections/{id}");
    for secret in ["backend-password", "test-private-key", "test-passphrase"] {
        assert!(!connection.to_string().contains(secret));
    }
    assert_eq!(
        connection["configured_secrets"].as_array().unwrap().len(),
        3
    );
    assert!(connection["params"].get("password").is_none());
    let update = json!({"name":"Renamed","protocol":"ssh","params":{"hostname":"changed.example","port":"2222"}});
    assert_eq!(
        call(
            &service,
            "PUT",
            &path,
            Some(update),
            Some(&cookie),
            "http://localhost"
        )
        .await
        .0,
        StatusCode::OK
    );
    let raw: Value =
        serde_json::from_slice(&std::fs::read(dir.path().join("connections.json")).unwrap())
            .unwrap();
    assert_eq!(raw[id]["params"]["password"], "backend-password");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(dir.path().join("connections.json"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    drop(service);
    let restarted = app(
        dir.path(),
        "127.0.0.1:1".parse().unwrap(),
        CancellationToken::new(),
    )
    .await;
    let cookie = login(&restarted).await;
    let (_, _, loaded) = call(
        &restarted,
        "GET",
        &path,
        None,
        Some(&cookie),
        "http://localhost",
    )
    .await;
    assert_eq!(loaded["name"], "Renamed");
    assert_eq!(loaded["params"]["hostname"], "changed.example");
    let clear = json!({"name":"Renamed","protocol":"ssh","params":{"hostname":"changed.example","password":"","private-key":"","passphrase":""}});
    let (_, _, cleared) = call(
        &restarted,
        "PUT",
        &path,
        Some(clear),
        Some(&cookie),
        "http://localhost",
    )
    .await;
    assert_eq!(cleared["configured_secrets"], json!([]));
    assert_eq!(
        call(
            &restarted,
            "DELETE",
            &path,
            None,
            Some(&cookie),
            "http://localhost"
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        call(
            &restarted,
            "GET",
            &path,
            None,
            Some(&cookie),
            "http://localhost"
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    let (_, _, list) = call(
        &restarted,
        "GET",
        "/api/connections",
        None,
        Some(&cookie),
        "http://localhost",
    )
    .await;
    assert_eq!(list, json!([]));
}

#[tokio::test]
async fn web_connection_validation_rejects_unsupported_protocols_and_oversized_values() {
    let dir = tempfile::tempdir().unwrap();
    let app = app(
        dir.path(),
        "127.0.0.1:1".parse().unwrap(),
        CancellationToken::new(),
    )
    .await;
    let cookie = login(&app).await;
    let invalids = [
        json!({"name":"VNC","protocol":"vnc","params":{"hostname":"host"}}),
        json!({"name":"Blank","protocol":"ssh","params":{"hostname":""}}),
        json!({"name":"Port","protocol":"ssh","params":{"hostname":"host","port":"65536"}}),
        json!({"name":"Long","protocol":"ssh","params":{"hostname":"host","password":"x".repeat(8193)}}),
        json!({"name":"Unknown","protocol":"ssh","params":{"hostname":"host","unrecognized-setting":"x"}}),
    ];
    for value in invalids {
        assert_eq!(
            call(
                &app,
                "POST",
                "/api/connections",
                Some(value),
                Some(&cookie),
                "http://localhost"
            )
            .await
            .0,
            StatusCode::BAD_REQUEST
        );
    }
}

#[tokio::test]
async fn web_login_rate_limit_and_corrupt_storage_fail_explicitly() {
    let dir = tempfile::tempdir().unwrap();
    let service = app(
        dir.path(),
        "127.0.0.1:1".parse().unwrap(),
        CancellationToken::new(),
    )
    .await;
    for _ in 0..10 {
        assert_eq!(
            call(
                &service,
                "POST",
                "/api/login",
                Some(json!({"username":"admin","password":"wrong"})),
                None,
                "http://localhost"
            )
            .await
            .0,
            StatusCode::UNAUTHORIZED
        );
    }
    assert_eq!(
        call(
            &service,
            "POST",
            "/api/login",
            Some(json!({"username":"admin","password":PASSWORD})),
            None,
            "http://localhost"
        )
        .await
        .0,
        StatusCode::TOO_MANY_REQUESTS
    );
    std::fs::write(
        dir.path().join("connections.json"),
        b"corrupt, do not overwrite",
    )
    .unwrap();
    let result = router(WebConfig {
        username: "admin".into(),
        password: PASSWORD.into(),
        data_dir: dir.path().into(),
        secure_cookie: false,
        backend: "127.0.0.1:1".parse().unwrap(),
        shutdown: CancellationToken::new(),
    })
    .await;
    assert!(result.is_err());
    assert_eq!(
        std::fs::read(dir.path().join("connections.json")).unwrap(),
        b"corrupt, do not overwrite"
    );
}

struct Spy {
    captured: tokio::sync::mpsc::Sender<Params>,
}
impl ProtocolPlugin for Spy {
    fn name(&self) -> &'static str {
        "ssh"
    }
    fn arg_names(&self) -> &'static [&'static str] {
        &[
            "hostname",
            "port",
            "username",
            "password",
            "private-key",
            "passphrase",
        ]
    }
    fn run(&self, mut ctx: SessionCtx, params: Params) -> BoxFuture<Result<(), SessionError>> {
        let captured = self.captured.clone();
        Box::pin(async move {
            captured.send(params).await.unwrap();
            ctx.tx
                .send(Instruction::new("size", ["0", "320", "160"]))
                .await?;
            ctx.tx.sync().await?;
            while let Some(event) = ctx.events.recv().await {
                if let InputEvent::Key { keysym, .. } = event {
                    ctx.tx
                        .send(Instruction::new("log", [keysym.to_string()]))
                        .await?;
                }
            }
            Ok(())
        })
    }
}

#[tokio::test]
async fn web_tunnel_auth_origin_server_owned_parameters_ping_and_logout() {
    let dir = tempfile::tempdir().unwrap();
    let shutdown = CancellationToken::new();
    let backend = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = backend.local_addr().unwrap();
    let (captured, mut capture) = tokio::sync::mpsc::channel(2);
    tokio::spawn(guacd::serve(
        backend,
        Arc::new(guacd::Registry::new().with(Spy { captured })),
        guacd::ServerConfig::default(),
        shutdown.clone(),
    ));
    let service = app(dir.path(), address, shutdown.clone()).await;
    let cookie = login(&service).await;
    let (_, _, connection) = call(
        &service,
        "POST",
        "/api/connections",
        Some(draft()),
        Some(&cookie),
        "http://localhost",
    )
    .await;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let web = listener.local_addr().unwrap();
    let serving = service.clone();
    let stop = shutdown.clone();
    tokio::spawn(async move {
        axum::serve(
            listener,
            serving.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .with_graceful_shutdown(async move { stop.cancelled().await })
        .await
        .unwrap();
    });
    let url = format!(
        "ws://{web}/api/connections/{}/tunnel?hostname=attacker.example&password=attacker-value",
        connection["id"].as_str().unwrap()
    );
    let request = |origin: &str, cookie: &str| {
        let mut r = url.as_str().into_client_request().unwrap();
        r.headers_mut().insert("origin", origin.parse().unwrap());
        r.headers_mut().insert("cookie", cookie.parse().unwrap());
        r.headers_mut()
            .insert("sec-websocket-protocol", "guacamole".parse().unwrap());
        r
    };
    assert!(connect_async(request("http://attacker.invalid", &cookie))
        .await
        .is_err());
    assert!(connect_async(request(
        &format!("http://{web}"),
        "cracamole_session=invalid"
    ))
    .await
    .is_err());
    let (mut socket, response) = connect_async(request(&format!("http://{web}"), &cookie))
        .await
        .unwrap();
    assert_eq!(response.headers()["sec-websocket-protocol"], "guacamole");
    let actual = tokio::time::timeout(Duration::from_secs(2), capture.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(actual.get("hostname"), Some("configured.example"));
    assert_eq!(actual.get("password"), Some("backend-password"));
    let mut parser = Parser::new();
    let mut seen_uuid = false;
    let mut seen_size = false;
    while !seen_size {
        let message = tokio::time::timeout(Duration::from_secs(2), socket.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        if let Message::Text(text) = message {
            parser.feed(text.as_bytes()).unwrap();
            while let Some(i) = parser.next_instruction() {
                if i.opcode.is_empty() {
                    seen_uuid = true
                }
                if i.opcode == "size" {
                    seen_size = true
                }
            }
        }
    }
    assert!(seen_uuid);
    socket
        .send(Message::Text("0.,4.ping,3.123;".into()))
        .await
        .unwrap();
    loop {
        let message = tokio::time::timeout(Duration::from_secs(2), socket.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        if let Message::Text(text) = message {
            parser.feed(text.as_bytes()).unwrap();
            let mut pong = false;
            while let Some(i) = parser.next_instruction() {
                pong |= i.opcode.is_empty() && i.args == ["ping", "123"];
            }
            if pong {
                break;
            }
        }
    }
    call(
        &service,
        "POST",
        "/api/logout",
        None,
        Some(&cookie),
        "http://localhost",
    )
    .await;
    let closed = tokio::time::timeout(Duration::from_secs(2), socket.next())
        .await
        .unwrap();
    let Some(Ok(Message::Close(Some(frame)))) = closed else {
        panic!("logout must send an explicit normal WebSocket close: {closed:?}");
    };
    assert_eq!(u16::from(frame.code), 1000);
    shutdown.cancel();
}
