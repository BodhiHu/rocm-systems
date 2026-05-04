#![forbid(unsafe_code)]
use std::io::{self, Write};
use std::process::ExitCode;

use clap::Parser;

use mirage_schema::ctl::daemon::ImplAttach as MirageDaemonAttach;
use mirage_schema::daemon::{
    AttachInput, AttachOutput, AttachReply, AttachRequest, ExecReply, ExecRequest, MirageDaemonCli,
    MirageDaemonClient, MirageDaemonCommand, MirageDaemonExec,
};

#[tokio::main]
async fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let cli = MirageDaemonCli::parse();
    let client = MirageDaemonClient::new(cli.socket.clone());

    // Intercept the `exec` subcommand to support auto-attach (default) vs
    // `--detach` (print exec_id and exit immediately).
    if let MirageDaemonCommand::Exec(ref args) = cli.command {
        let args = args.clone();
        let detach = args.detach;
        let exec_request = ExecRequest {
            session: args.session,
            node_index: args.node_index,
            command: args.command,
        };

        match client.exec(exec_request).await {
            Err(e) => {
                eprintln!("{e}");
                return ExitCode::from(1);
            }
            Ok(ExecReply { exec_id }) => {
                if detach {
                    println!("{exec_id}");
                    return ExitCode::SUCCESS;
                }
                // Auto-attach: stream output to stdout/stderr.
                let exit_code = attach_and_stream(&client, exec_id).await;
                return ExitCode::from(if exit_code == 0 { 0u8 } else { 1u8 });
            }
        }
    }

    match cli.run_with(&client).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::from(1)
        }
    }
}

/// Open a WebSocket (or Unix-socket) attach to `exec_id`, stream stdout to
/// stdout and stderr to stderr, and return the final exit code.
async fn attach_and_stream(client: &MirageDaemonClient, exec_id: String) -> i32 {
    use tokio::sync::mpsc;

    let (input_tx, input_rx) = mpsc::channel::<AttachInput>(1);
    let (output_tx, mut output_rx) = mpsc::channel::<AttachOutput>(64);

    // We don't forward stdin from the terminal here; drop the sender
    // immediately so the daemon sees EOF on stdin.
    drop(input_tx);

    let attach_req = AttachRequest { exec_id };
    let attach_fut = client.attach(attach_req, input_rx, output_tx);

    let print_fut = async {
        while let Some(chunk) = output_rx.recv().await {
            if chunk.is_stdout {
                let _ = io::stdout().write_all(&chunk.output);
            } else {
                let _ = io::stderr().write_all(&chunk.output);
            }
        }
    };

    let (reply_res, _) = tokio::join!(attach_fut, print_fut);
    match reply_res {
        Ok(AttachReply { exit_code }) => exit_code,
        Err(e) => {
            eprintln!("attach error: {e}");
            -1
        }
    }
}
