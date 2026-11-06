use cracamole_web::WebConfig;
use std::{future::IntoFuture, net::SocketAddr, path::PathBuf, process::ExitCode, sync::Arc};
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

#[tokio::main]
async fn main() -> ExitCode {
    let level = std::env::var("LOG_LEVEL").unwrap_or_else(|_| "info".into());
    let Some(level) = guacd::config::LogLevel::parse(&level) else {
        eprintln!("Invalid LOG_LEVEL");
        return ExitCode::FAILURE;
    };
    guacd::logging::init(level);
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!("{error}");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<(), String> {
    let password = std::env::var("CRACAMOLE_ADMIN_PASSWORD")
        .map_err(|_| "請設定 CRACAMOLE_ADMIN_PASSWORD（至少 12 個字元）".to_string())?;
    let address: SocketAddr = std::env::var("CRACAMOLE_HTTP_BIND")
        .unwrap_or_else(|_| "127.0.0.1:8080".into())
        .parse()
        .map_err(|_| "CRACAMOLE_HTTP_BIND 無效".to_string())?;
    let shutdown = CancellationToken::new();
    let daemon = TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|e| e.to_string())?;
    let backend = daemon.local_addr().map_err(|e| e.to_string())?;
    let app = cracamole_web::router(WebConfig {
        username: std::env::var("CRACAMOLE_ADMIN_USERNAME").unwrap_or_else(|_| "admin".into()),
        password,
        data_dir: PathBuf::from(
            std::env::var_os("CRACAMOLE_DATA_DIR").unwrap_or_else(|| "data".into()),
        ),
        secure_cookie: std::env::var("CRACAMOLE_SECURE_COOKIE").is_ok_and(|s| s == "true"),
        backend,
        shutdown: shutdown.clone(),
    })
    .await?;
    let listener = TcpListener::bind(address)
        .await
        .map_err(|e| e.to_string())?;
    tracing::info!(
        "cracamole web listening on {}",
        listener.local_addr().map_err(|e| e.to_string())?
    );
    tokio::task::spawn_blocking(guac_terminal::FontSet::warm_up);
    let mut backend_task = tokio::spawn(guacd::serve(
        daemon,
        Arc::new(guacd::Registry::builtin()),
        guacd::ServerConfig::default(),
        shutdown.clone(),
    ));
    let signal = shutdown.clone();
    tokio::spawn(async move {
        #[cfg(unix)]
        {
            let mut term =
                tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                    .expect("SIGTERM handler");
            tokio::select! {_=term.recv()=>{},_=tokio::signal::ctrl_c()=>{}}
        }
        #[cfg(not(unix))]
        {
            let _ = tokio::signal::ctrl_c().await;
        }
        signal.cancel();
    });
    let server = axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown({
        let shutdown = shutdown.clone();
        async move { shutdown.cancelled().await }
    })
    .into_future();
    tokio::pin!(server);
    tokio::select! {
        result=&mut server => {shutdown.cancel();result.map_err(|e|e.to_string())?;}
        result=&mut backend_task => {
            let normal=shutdown.is_cancelled();shutdown.cancel();
            (&mut server).await.map_err(|e|e.to_string())?;
            return if normal {Ok(())} else {Err(format!("private daemon stopped: {result:?}"))};
        }
    }
    let _ = tokio::time::timeout(std::time::Duration::from_secs(8), backend_task).await;
    Ok(())
}
