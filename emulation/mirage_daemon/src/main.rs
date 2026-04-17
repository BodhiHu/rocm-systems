use std::path::PathBuf;

use clap::Parser;

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
    let cli = Cli::parse();
    let daemon = InMemoryMirageDaemon::new();
    let server = MirageDaemonServer::new(cli.socket, daemon);
    server.serve().await?;
    Ok(())
}
