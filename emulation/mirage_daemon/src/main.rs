use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use clap::Parser;

use mirage_container::{DockerCli, MockContainerRuntime};
use mirage_daemon::MirageDaemon;
use mirage_schema::daemon::MirageDaemonServer;
use mirage_schema::paths;

#[derive(Debug, Parser)]
#[command(name = "mirage_daemon")]
struct Cli {
    /// Path to the Unix-domain control socket.
    #[arg(long, default_value_os_t = paths::socket_path())]
    socket: PathBuf,

    /// Address for the HTTP REST + dashboard server.
    ///
    /// Set to an empty string to disable the HTTP server.
    #[arg(long, default_value = "127.0.0.1:8080")]
    http_addr: String,

    /// Directory with built dashboard static assets to serve at `/`.
    ///
    /// When unset, the dashboard bundle embedded in the daemon binary is
    /// served instead.
    #[arg(long)]
    dashboard_dir: Option<PathBuf>,

    /// Use an in-memory mock container runtime instead of Docker.
    ///
    /// Intended for e2e/dashboard tests where Docker is unavailable.
    #[arg(long)]
    mock: bool,

    /// Override the daemon configuration root (profiles, workloads).
    #[arg(long)]
    config_root: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let cli = Cli::parse();
    let runtime: Arc<dyn mirage_container::ContainerRuntime> = if cli.mock {
        Arc::new(MockContainerRuntime::default())
    } else {
        Arc::new(DockerCli::new())
    };
    let mut daemon_inner = MirageDaemon::with_container_runtime(runtime);
    if let Some(root) = cli.config_root.clone() {
        daemon_inner.set_config_root(root);
    }
    let daemon = Arc::new(daemon_inner);

    // Unix-socket control server.
    let unix_server = MirageDaemonServer::from_arc(cli.socket.clone(), daemon.clone());
    let unix_task = tokio::spawn(async move { unix_server.serve().await });

    // HTTP server for the dashboard REST API + static assets.
    let http_task: Option<tokio::task::JoinHandle<std::io::Result<()>>> =
        if cli.http_addr.is_empty() {
            None
        } else {
            let addr: SocketAddr = cli
                .http_addr
                .parse()
                .map_err(|e| format!("invalid --http-addr '{}': {e}", cli.http_addr))?;

            let api_router = mirage_schema::ctl::daemon::axum_router(daemon.clone());
            let reset_daemon = daemon.clone();
            let api_router = api_router.route(
                "/__reset",
                axum::routing::post(move || {
                    let daemon = reset_daemon.clone();
                    async move {
                        daemon.reset_for_testing().await;
                        axum::Json(serde_json::json!({"ok": true}))
                    }
                }),
            );
            let mut app = axum::Router::new().nest("/api", api_router);

            if let Some(dir) = cli.dashboard_dir.clone() {
                use tower_http::services::{ServeDir, ServeFile};
                let index = dir.join("index.html");
                let serve_dir = ServeDir::new(&dir).fallback(ServeFile::new(index));
                app = app.fallback_service(serve_dir);
                tracing::info!(?dir, "serving dashboard static assets from disk");
            } else {
                app = app.fallback(axum::routing::any(mirage_daemon::dashboard::handler));
                tracing::info!("serving embedded dashboard assets");
            }

            app = app.layer(tower_http::cors::CorsLayer::permissive());

            tracing::info!(%addr, "starting mirage http server");
            let listener = tokio::net::TcpListener::bind(addr).await?;
            Some(tokio::spawn(async move {
                axum::serve(listener, app.into_make_service()).await
            }))
        };

    // Wait on whichever finishes first; both running forever is the norm.
    type BoxErr = Box<dyn std::error::Error + Send + Sync>;
    if let Some(http_task) = http_task {
        tokio::select! {
            result = unix_task => {
                match result {
                    Ok(Ok(())) => {}
                    Ok(Err(e)) => return Err(Box::new(e) as BoxErr),
                    Err(e) => return Err(Box::new(e) as BoxErr),
                }
            }
            result = http_task => {
                match result {
                    Ok(Ok(())) => {}
                    Ok(Err(e)) => return Err(Box::new(e) as BoxErr),
                    Err(e) => return Err(Box::new(e) as BoxErr),
                }
            }
        }
    } else {
        match unix_task.await {
            Ok(Ok(())) => {}
            Ok(Err(e)) => return Err(Box::new(e) as BoxErr),
            Err(e) => return Err(Box::new(e) as BoxErr),
        }
    }

    Ok(())
}
