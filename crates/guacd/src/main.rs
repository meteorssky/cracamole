use guacd::config::{usage, Config, CONF_FILE, VERSION_LINE};
use guacd::{logging, serve, Registry, ServerConfig};
use std::process::ExitCode;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

fn main() -> ExitCode {
    let mut cfg = Config::default();

    // Like the C daemon: a missing config file is fine, a broken one is fatal.
    if let Ok(text) = std::fs::read_to_string(CONF_FILE) {
        if let Err(e) = cfg.apply_conf(&text) {
            eprintln!("Unable to parse {CONF_FILE}: {e}");
            return ExitCode::FAILURE;
        }
    }
    let mut args = std::env::args();
    let prog = args.next().unwrap_or_else(|| "guacd".into());
    if let Err(e) = cfg.apply_args(args) {
        eprintln!("{e}");
        eprintln!("{}", usage(&prog));
        return ExitCode::FAILURE;
    }

    if cfg.print_version {
        println!("{VERSION_LINE}");
        return ExitCode::SUCCESS;
    }

    logging::init(cfg.log_level);
    tracing::info!("{VERSION_LINE} started");
    if !cfg.foreground {
        tracing::warn!(
            "daemonization is not supported; running in the foreground (use -f to silence this)"
        );
    }
    if let Some(path) = &cfg.pid_file {
        if let Err(e) = std::fs::write(path, format!("{}\n", std::process::id())) {
            tracing::error!("Unable to write PID file \"{path}\": {e}");
            return ExitCode::FAILURE;
        }
    }

    let rt = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            tracing::error!("Unable to start runtime: {e}");
            return ExitCode::FAILURE;
        }
    };
    let code = rt.block_on(run(cfg.clone()));
    if let Some(path) = &cfg.pid_file {
        let _ = std::fs::remove_file(path);
    }
    code
}

async fn run(cfg: Config) -> ExitCode {
    let port: u16 = match cfg.bind_port.parse() {
        Ok(p) => p,
        Err(_) => {
            tracing::error!("Invalid port \"{}\"", cfg.bind_port);
            return ExitCode::FAILURE;
        }
    };
    let listener = match TcpListener::bind((cfg.bind_host.as_str(), port)).await {
        Ok(l) => l,
        Err(e) => {
            tracing::error!("Unable to bind socket to any addresses: {e}");
            return ExitCode::FAILURE;
        }
    };
    let local = listener
        .local_addr()
        .map(|a| a.ip().to_string())
        .unwrap_or_else(|_| cfg.bind_host.clone());
    tracing::info!("Listening on host {local}, port {port}");

    // Fonts (incl. large CJK ones) take seconds to parse; do it in the background now.
    tokio::task::spawn_blocking(guac_terminal::FontSet::warm_up);

    let shutdown = CancellationToken::new();
    tokio::spawn(wait_for_signal(shutdown.clone()));

    let registry = Arc::new(Registry::builtin());
    tracing::debug!("protocols: {:?}", registry.names());
    match serve(listener, registry, ServerConfig::default(), shutdown).await {
        Ok(()) => {
            tracing::info!("Exiting");
            ExitCode::SUCCESS
        }
        Err(e) => {
            tracing::error!("server error: {e}");
            ExitCode::FAILURE
        }
    }
}

async fn wait_for_signal(shutdown: CancellationToken) {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        if let Ok(mut term) = signal(SignalKind::terminate()) {
            tokio::select! {
                _ = term.recv() => {}
                _ = tokio::signal::ctrl_c() => {}
            }
        } else {
            let _ = tokio::signal::ctrl_c().await;
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
    tracing::info!("Received shutdown signal");
    shutdown.cancel();
}
