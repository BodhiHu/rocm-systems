use std::path::PathBuf;
use std::sync::Arc;

use clap::Parser;

use mirage_container::DockerCli;
use mirage_daemon::InMemoryMirageDaemon;
use mirage_schema::daemon::MirageDaemonServer;
use mirage_schema::paths;

#[derive(Debug, Parser)]
#[command(name = "mirage_daemon")]
struct Cli {
    #[arg(long, default_value_os_t = paths::socket_path())]
    socket: PathBuf,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let cli = Cli::parse();
    let docker = Arc::new(DockerCli::new());
    let daemon = InMemoryMirageDaemon::with_container_runtime(docker);
    let server = MirageDaemonServer::new(cli.socket, daemon);
    server.serve().await?;
    Ok(())
}
