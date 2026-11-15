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
        trusted_proxies: Vec::new(),
        backend: backend.into(),
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
async fn static_assets_revalidate_but_dynamic_responses_are_not_cached() {
    let dir = tempfile::tempdir().unwrap();
    let app = app(
        dir.path(),
        "127.0.0.1:1".parse().unwrap(),
        CancellationToken::new(),
    )
    .await;
    let request = |path: &str, tag: Option<&str>| {
        let mut builder = Request::builder().uri(path);
        if let Some(tag) = tag {
            builder = builder.header("if-none-match", tag);
        }
        builder.body(Body::empty()).unwrap()
    };
    let first = app
        .clone()
        .oneshot(request("/assets/js/main.js", None))
        .await
        .unwrap();
    assert_eq!(first.status(), StatusCode::OK);
    assert_eq!(first.headers()["cache-control"], "private, no-cache");
    let tag = first.headers()["etag"].to_str().unwrap().to_owned();
    let conditional = app
        .clone()
        .oneshot(request("/assets/js/main.js", Some(&format!("W/{tag}"))))
        .await
        .unwrap();
    assert_eq!(conditional.status(), StatusCode::NOT_MODIFIED);
    assert_eq!(conditional.headers()["etag"], tag.as_str());
    assert!(to_bytes(conditional.into_body(), 1024)
        .await
        .unwrap()
        .is_empty());
    let other = app
        .clone()
        .oneshot(request("/assets/js/state.js", Some(&tag)))
        .await
        .unwrap();
    assert_eq!(other.status(), StatusCode::OK);
    let index = app.clone().oneshot(request("/", None)).await.unwrap();
    assert_eq!(index.headers()["cache-control"], "no-store");
    let html = String::from_utf8(
        to_bytes(index.into_body(), 1024 * 1024)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap();
    let module = html
        .split_once("src=\"/assets/")
        .unwrap()
        .1
        .split_once("/js/main.js\"")
        .unwrap()
        .0;
    assert_eq!(module.len(), 64);
    assert!(module.bytes().all(|byte| byte.is_ascii_hexdigit()));
    let versioned = format!("/assets/{module}/js/main.js");
    let response = app
        .clone()
        .oneshot(request(&versioned, None))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()["cache-control"],
        "public, max-age=31536000, immutable"
    );
    assert_eq!(response.headers()["etag"], tag.as_str());
    let missing = format!("/assets/{}/js/main.js", "0".repeat(64));
    assert_eq!(
        app.clone()
            .oneshot(request(&missing, None))
            .await
            .unwrap()
            .status(),
        StatusCode::NOT_FOUND
    );
    let api = app.oneshot(request("/api/session", None)).await.unwrap();
    assert_eq!(api.headers()["cache-control"], "no-store");
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
    // Same destination (only name/username change): omitted secrets are kept.
    let rename = json!({"name":"Renamed","protocol":"ssh","params":{"hostname":"Configured.Example ","port":"22","username":"other"}});
    let (status, _, kept) = call(
        &service,
        "PUT",
        &path,
        Some(rename),
        Some(&cookie),
        "http://localhost",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(kept["configured_secrets"].as_array().unwrap().len(), 3);
    let raw: Value =
        serde_json::from_slice(&std::fs::read(dir.path().join("connections.json")).unwrap())
            .unwrap();
    assert_eq!(raw[id]["params"]["password"], "backend-password");
    // Changed hostname or port: saved secrets must NOT follow to the new destination.
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
    for secret in ["password", "private-key", "passphrase"] {
        assert!(
            raw[id]["params"].get(secret).is_none(),
            "{secret} leaked to a changed destination"
        );
    }
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
        json!({"name":"K8s","protocol":"kubernetes","params":{"hostname":"host"}}),
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
async fn web_vnc_connections_keep_their_password_private_and_reject_foreign_parameters() {
    let dir = tempfile::tempdir().unwrap();
    let service = app(
        dir.path(),
        "127.0.0.1:1".parse().unwrap(),
        CancellationToken::new(),
    )
    .await;
    let cookie = login(&service).await;
    let mac = json!({
        "name": "Mac mini",
        "protocol": "vnc",
        "params": {
            "hostname": "mac.local", "port": "5900", "username": "admin",
            "password": "s3cret", "color-depth": "16", "clipboard-encoding": "UTF-8"
        }
    });
    let (status, _, saved) = call(
        &service,
        "POST",
        "/api/connections",
        Some(mac),
        Some(&cookie),
        "http://localhost",
    )
    .await;
    assert!(status.is_success(), "{saved}");
    assert_eq!(saved["protocol"], "vnc");
    assert_eq!(saved["configured_secrets"], json!(["password"]));
    assert!(saved["params"].get("password").is_none());
    assert!(!saved.to_string().contains("s3cret"));

    for foreign in [
        "server-cert-sha256",
        "ignore-cert",
        "host-key",
        "terminal-type",
    ] {
        let bad = json!({"name":"Mac","protocol":"vnc","params":{"hostname":"h", foreign: "x"}});
        assert_eq!(
            call(
                &service,
                "POST",
                "/api/connections",
                Some(bad),
                Some(&cookie),
                "http://localhost"
            )
            .await
            .0,
            StatusCode::BAD_REQUEST,
            "{foreign} must not be accepted for VNC"
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
        trusted_proxies: Vec::new(),
        backend: "127.0.0.1:1".parse::<SocketAddr>().unwrap().into(),
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

#[tokio::test]
async fn web_trusted_proxy_preserves_independent_login_limits() {
    let dir = tempfile::tempdir().unwrap();
    let service = router(WebConfig {
        username: "admin".into(),
        password: PASSWORD.into(),
        data_dir: dir.path().into(),
        secure_cookie: false,
        trusted_proxies: vec!["127.0.0.1".parse().unwrap()],
        backend: "127.0.0.1:1".parse::<SocketAddr>().unwrap().into(),
        shutdown: CancellationToken::new(),
    })
    .await
    .unwrap();
    async fn attempt(service: &Router, ip: &str, password: &str) -> StatusCode {
        service
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/login")
                    .header("host", "localhost")
                    .header("origin", "http://localhost")
                    .header("content-type", "application/json")
                    .header("x-real-ip", ip)
                    .extension(ConnectInfo(
                        "127.0.0.1:12345".parse::<SocketAddr>().unwrap(),
                    ))
                    .body(Body::from(
                        json!({"username":"admin","password":password}).to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap()
            .status()
    }
    for _ in 0..10 {
        assert_eq!(
            attempt(&service, "192.0.2.1", "wrong").await,
            StatusCode::UNAUTHORIZED
        );
    }
    assert_eq!(
        attempt(&service, "192.0.2.1", PASSWORD).await,
        StatusCode::TOO_MANY_REQUESTS
    );
    assert_eq!(
        attempt(&service, "192.0.2.2", PASSWORD).await,
        StatusCode::OK
    );
    assert_eq!(
        attempt(&service, "192.0.2.2,192.0.2.3", PASSWORD).await,
        StatusCode::BAD_REQUEST
    );
}

#[tokio::test]
async fn web_identity_probe_requires_auth_origin_and_matching_protocol() {
    let dir = tempfile::tempdir().unwrap();
    let service = app(
        dir.path(),
        "127.0.0.1:1".parse().unwrap(),
        CancellationToken::new(),
    )
    .await;
    for suffix in ["host-key", "certificate"] {
        assert_eq!(
            call(
                &service,
                "POST",
                &format!("/api/connections/missing/{suffix}"),
                None,
                None,
                "http://localhost"
            )
            .await
            .0,
            StatusCode::UNAUTHORIZED
        );
    }
    let cookie = login(&service).await;
    let probe = json!({"protocol":"ssh", "hostname":"127.0.0.1", "port":"22"});
    assert_eq!(
        call(
            &service,
            "POST",
            "/api/identity-probe",
            Some(probe.clone()),
            None,
            "http://localhost"
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        call(
            &service,
            "POST",
            "/api/identity-probe",
            Some(probe),
            Some(&cookie),
            "https://attacker.invalid"
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    for invalid in [
        json!({"protocol":"telnet", "hostname":"localhost", "port":"23"}),
        json!({"protocol":"ssh", "hostname":"", "port":"22"}),
        json!({"protocol":"ssh", "hostname":"localhost", "port":"65536"}),
        json!({"protocol":"ssh", "hostname":"localhost", "port":"22", "password":"must-not-be-accepted"}),
    ] {
        let status = call(
            &service,
            "POST",
            "/api/identity-probe",
            Some(invalid),
            Some(&cookie),
            "http://localhost",
        )
        .await
        .0;
        assert!(status.is_client_error());
    }
    let (_, _, connection) = call(
        &service,
        "POST",
        "/api/connections",
        Some(draft()),
        Some(&cookie),
        "http://localhost",
    )
    .await;
    let id = connection["id"].as_str().unwrap();
    assert_eq!(
        call(
            &service,
            "POST",
            &format!("/api/connections/{id}/host-key"),
            None,
            Some(&cookie),
            "https://attacker.invalid"
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(
            &service,
            "POST",
            &format!("/api/connections/{id}/certificate"),
            None,
            Some(&cookie),
            "http://localhost"
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
}

#[tokio::test]
async fn web_terminal_encoding_is_validated_and_preserved_without_affecting_rdp() {
    let dir = tempfile::tempdir().unwrap();
    let service = app(
        dir.path(),
        "127.0.0.1:1".parse().unwrap(),
        CancellationToken::new(),
    )
    .await;
    let cookie = login(&service).await;
    for protocol in ["ssh", "telnet"] {
        for encoding in [
            "UTF-8",
            "Big5",
            "GBK",
            "GB18030",
            "Shift_JIS",
            "EUC-JP",
            "EUC-KR",
        ] {
            let body = json!({"name":"多國文字 😀", "protocol":protocol,
                "params":{"hostname":"example.invalid","character-encoding":encoding}});
            let (status, _, value) = call(
                &service,
                "POST",
                "/api/connections",
                Some(body),
                Some(&cookie),
                "http://localhost",
            )
            .await;
            assert_eq!(status, StatusCode::CREATED);
            assert_eq!(value["params"]["character-encoding"], encoding);
        }
    }
    let bad = json!({"name":"bad", "protocol":"ssh", "params":{"hostname":"h","character-encoding":"UTF-16LE"}});
    let (status, _, error) = call(
        &service,
        "POST",
        "/api/connections",
        Some(bad),
        Some(&cookie),
        "http://localhost",
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(error["field"], "character-encoding");
    let rdp = json!({"name":"RDP", "protocol":"rdp", "params":{"hostname":"h","character-encoding":"Big5"}});
    assert_eq!(
        call(
            &service,
            "POST",
            "/api/connections",
            Some(rdp),
            Some(&cookie),
            "http://localhost"
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
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

#[tokio::test]
async fn logs_endpoint_requires_auth_and_returns_logs() {
    let dir = tempfile::tempdir().unwrap();
    let shutdown = CancellationToken::new();
    let service = app(dir.path(), "127.0.0.1:9".parse().unwrap(), shutdown.clone()).await;

    let (status, _, _) = call(&service, "GET", "/api/logs", None, None, "http://localhost").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let cookie = login(&service).await;
    let (status, _, body) = call(
        &service,
        "GET",
        "/api/logs",
        None,
        Some(&cookie),
        "http://localhost",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.get("logs").and_then(|v| v.as_array()).is_some());
    shutdown.cancel();
}

#[tokio::test]
async fn web_protocol_catalogue_requires_login_and_describes_every_protocol() {
    let dir = tempfile::tempdir().unwrap();
    let app = app(
        dir.path(),
        "127.0.0.1:1".parse().unwrap(),
        CancellationToken::new(),
    )
    .await;
    let (status, _, _) = call(
        &app,
        "GET",
        "/api/protocols",
        None,
        None,
        "http://localhost",
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let cookie = login(&app).await;
    let (status, _, body) = call(
        &app,
        "GET",
        "/api/protocols",
        None,
        Some(&cookie),
        "http://localhost",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let rows = body.as_array().unwrap();
    let names: Vec<_> = rows.iter().map(|r| r["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["ssh", "telnet", "rdp", "vnc"]);
    assert_eq!(rows[0]["label"], "SSH");
    assert_eq!(rows[0]["default_port"], 22);
    assert_eq!(rows[0]["identity"], "host-key");
    assert_eq!(rows[2]["identity"], "certificate");
    assert!(rows[1]["identity"].is_null());
    for row in rows {
        let args: Vec<_> = row["args"].as_array().unwrap().iter().collect();
        assert!(args.contains(&&json!("hostname")), "{row}");
    }
    // Probing an identity the protocol does not publish is refused.
    let (status, _, created) = call(
        &app,
        "POST",
        "/api/connections",
        Some(json!({"name":"T","protocol":"telnet","params":{"hostname":"h"}})),
        Some(&cookie),
        "http://localhost",
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let id = created["id"].as_str().unwrap();
    for path in ["host-key", "certificate"] {
        let (status, _, _) = call(
            &app,
            "POST",
            &format!("/api/connections/{id}/{path}"),
            None,
            Some(&cookie),
            "http://localhost",
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{path}");
    }
}

#[tokio::test]
async fn portable_preview_is_authorized_secret_free_and_does_not_save() {
    let dir = tempfile::tempdir().unwrap();
    let app = app(
        dir.path(),
        "127.0.0.1:1".parse().unwrap(),
        CancellationToken::new(),
    )
    .await;
    let cookie = login(&app).await;
    let (_, _, created) = call(
        &app,
        "POST",
        "/api/connections",
        Some(draft()),
        Some(&cookie),
        "http://localhost",
    )
    .await;
    let id = created["id"].as_str().unwrap();
    let request = json!({"ids":[id]});
    assert_eq!(
        call(
            &app,
            "POST",
            "/api/connections/export",
            Some(request.clone()),
            None,
            "http://localhost"
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        call(
            &app,
            "POST",
            "/api/connections/export",
            Some(request.clone()),
            Some(&cookie),
            "https://attacker.invalid"
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let (status, _, exported) = call(
        &app,
        "POST",
        "/api/connections/export",
        Some(request),
        Some(&cookie),
        "http://localhost",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let content = exported["content"].as_str().unwrap();
    assert!(
        !content.contains("backend-password")
            && !content.contains("private-key")
            && !content.contains("passphrase")
    );
    let (status, _, preview) = call(
        &app,
        "POST",
        "/api/connections/import-preview",
        Some(json!({"content":content})),
        Some(&cookie),
        "http://localhost",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(preview["connections"][0]["name"], "Development");
    let (_, _, list) = call(
        &app,
        "GET",
        "/api/connections",
        None,
        Some(&cookie),
        "http://localhost",
    )
    .await;
    assert_eq!(list.as_array().unwrap().len(), 1);
    let malicious = content.replace("\"hostname\"", "\"password\":\"secret\",\"hostname\"");
    assert_eq!(
        call(
            &app,
            "POST",
            "/api/connections/import-preview",
            Some(json!({"content":malicious})),
            Some(&cookie),
            "http://localhost"
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
}

#[tokio::test]
async fn portable_uri_export_and_preview_reject_credentials() {
    let dir = tempfile::tempdir().unwrap();
    let app = app(
        dir.path(),
        "127.0.0.1:1".parse().unwrap(),
        CancellationToken::new(),
    )
    .await;
    let cookie = login(&app).await;
    let (_, _, created) = call(
        &app,
        "POST",
        "/api/connections",
        Some(draft()),
        Some(&cookie),
        "http://localhost",
    )
    .await;
    let id = created["id"].as_str().unwrap();
    let (status, _, exported) = call(
        &app,
        "POST",
        "/api/connections/export",
        Some(json!({"ids":[id],"format":"uri"})),
        Some(&cookie),
        "http://localhost",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let content = exported["content"].as_str().unwrap();
    assert_eq!(content, "ssh://tester@configured.example:22");
    let (status, _, preview) = call(
        &app,
        "POST",
        "/api/connections/import-preview",
        Some(json!({"content":content})),
        Some(&cookie),
        "http://localhost",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        preview["connections"][0]["params"]["hostname"],
        "configured.example"
    );
    assert_eq!(
        call(
            &app,
            "POST",
            "/api/connections/import-preview",
            Some(json!({"content":"ssh://tester:secret@configured.example"})),
            Some(&cookie),
            "http://localhost"
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
}
