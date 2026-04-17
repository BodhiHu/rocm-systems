use std::process::ExitCode;

use clap::Parser;

use mirage_schema::daemon::{MirageDaemonCli, MirageDaemonClient};

#[tokio::main]
async fn main() -> ExitCode {
    let cli = MirageDaemonCli::parse();
    let client = MirageDaemonClient::new(cli.socket.clone());

    match cli.run_with(&client).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::from(1)
        }
    }
}
