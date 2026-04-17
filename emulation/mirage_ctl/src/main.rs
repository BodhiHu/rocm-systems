// mod format;
// pub(crate) mod traits;

use std::error::Error;
use std::fmt;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand, ValueEnum};

use mirage_schema::common::{ExecArgs, HealthStatus, ProfileDef, SessionDef, SimulatorMode, Time};
use mirage_schema::daemon::{MirageDaemon, MirageDaemonClient, MirageDaemonError};
use mirage_schema::paths;
use mirage_schema::socket::{
    BootSessionReply, BootSessionRequest, CreateProfileReply, CreateProfileRequest,
    DashboardCreateSessionReply, DashboardCreateSessionRequest, DashboardDeleteSessionReply,
    DashboardDeleteSessionRequest, DeleteProfileReply, DeleteProfileRequest, ExecInSessionRequest,
    GetOverviewRequest, GetSessionDetailRequest, GetSimulatorRequest, HealthRequest,
    ListProfilesRequest, ListSessionsRequest, ListSimulatorsRequest, ShutdownSessionReply,
    ShutdownSessionRequest, TimeRequest,
};

#[derive(Debug)]
struct CliError(String);

impl CliError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Error for CliError {}

impl From<MirageDaemonError> for CliError {
    fn from(value: MirageDaemonError) -> Self {
        Self(value.to_string())
    }
}

impl From<serde_json::Error> for CliError {
    fn from(value: serde_json::Error) -> Self {
        Self(value.to_string())
    }
}

type CliResult<T = ()> = Result<T, CliError>;

#[derive(Debug, Clone, Copy, ValueEnum)]
enum CliSimulatorMode {
    Functional,
    Clocked,
    CycleAccurate,
}

impl From<CliSimulatorMode> for SimulatorMode {
    fn from(value: CliSimulatorMode) -> Self {
        match value {
            CliSimulatorMode::Functional => SimulatorMode::Functional,
            CliSimulatorMode::Clocked => SimulatorMode::Clocked,
            CliSimulatorMode::CycleAccurate => SimulatorMode::CycleAccurate,
        }
    }
}

#[derive(Debug, Parser)]
#[command(name = "mirage-ctl")]
struct Cli {
    #[arg(long, global = true, default_value_os_t = paths::socket_path())]
    socket: PathBuf,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Overview {
        #[arg(long)]
        json: bool,
    },
    Health {
        #[arg(long)]
        session: Option<String>,
        #[arg(long)]
        json: bool,
    },
    Time {
        #[arg(long)]
        session: Option<String>,
        #[arg(long)]
        json: bool,
    },
    Simulators {
        #[command(subcommand)]
        command: SimulatorCommand,
    },
    Profile {
        #[command(subcommand)]
        command: ProfileCommand,
    },
    Session {
        #[command(subcommand)]
        command: SessionCommand,
    },
    /// Boot a new session from a profile and container image.
    Boot(BootArgs),
    /// Run a command inside a booted session.
    Exec(ExecCommandArgs),
    /// Shut down a booted session and release its resources.
    Shutdown(ShutdownArgs),
}

#[derive(Debug, Subcommand)]
enum SimulatorCommand {
    List {
        #[arg(long)]
        json: bool,
    },
    Show {
        simulator: String,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Debug, Subcommand)]
enum ProfileCommand {
    List {
        #[arg(long)]
        simulator: Option<String>,
        #[arg(long)]
        json: bool,
    },
    Create(ProfileCreateArgs),
    Delete {
        name: String,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Debug, Args)]
struct ProfileCreateArgs {
    #[arg(long)]
    name: String,
    #[arg(long)]
    simulator: String,
    #[arg(long)]
    gpu: String,
    #[arg(long, value_enum)]
    mode: CliSimulatorMode,
    #[arg(long = "gpus-per-node")]
    gpus_per_node: u32,
    #[arg(long)]
    nodes: u32,
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Subcommand)]
enum SessionCommand {
    List {
        #[arg(long)]
        profile: Option<String>,
        #[arg(long)]
        json: bool,
    },
    Create(SessionCreateArgs),
    Delete {
        name: String,
        #[arg(long)]
        json: bool,
    },
    Show {
        name: String,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Debug, Args)]
struct SessionCreateArgs {
    #[arg(long)]
    name: String,
    #[arg(long)]
    profile: String,
    #[arg(long)]
    image: String,
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Args)]
struct BootArgs {
    #[arg(long)]
    name: String,
    #[arg(long)]
    profile: String,
    #[arg(long)]
    image: String,
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Args)]
struct ExecCommandArgs {
    #[arg(long)]
    name: String,
    /// The command and arguments to run, separated by `--`.
    #[arg(last = true, required = true)]
    command: Vec<String>,
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Args)]
struct ShutdownArgs {
    #[arg(long)]
    name: String,
    #[arg(long)]
    json: bool,
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    let client = MirageDaemonClient::new(cli.socket);

    match run_command(&client, cli.command).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::from(1)
        }
    }
}
async fn run_command(daemon: &dyn MirageDaemon, command: Command) -> CliResult {
    match command {
        Command::Overview { json } => {
            let reply = daemon.get_overview(GetOverviewRequest::default()).await?;
            if json {
                print_json(&reply)?;
            } else {
                println!("simulators: {}", reply.simulator_count);
                println!("profiles: {}", reply.profile_count);
                println!("sessions: {}", reply.session_count);
            }
        }
        Command::Health { session, json } => {
            let reply = daemon
                .health(HealthRequest {
                    session_id: session,
                })
                .await?;
            if json {
                print_json(&reply)?;
            } else {
                println!("healthy: {}", reply.healthy);
                println!("status: {}", health_name(reply.status));
            }
        }
        Command::Time { session, json } => {
            let reply = daemon
                .time(TimeRequest {
                    session_id: session,
                })
                .await?;
            if json {
                print_json(&reply)?;
            } else {
                println!(
                    "time: {} s + {} ps",
                    reply.time.seconds, reply.time.picoseconds
                );
            }
        }
        Command::Simulators { command } => match command {
            SimulatorCommand::List { json } => {
                let reply = daemon
                    .list_simulators(ListSimulatorsRequest::default())
                    .await?;
                if json {
                    print_json(&reply.simulators)?;
                } else if reply.simulators.is_empty() {
                    println!("no simulators registered");
                } else {
                    for simulator in reply.simulators {
                        println!(
                            "{} {} | gpus: {} | modes: {} | custom gpu: {} | active sessions: {}",
                            simulator.name.unwrap_or_default(),
                            simulator.version.unwrap_or_default(),
                            format_gpus(&simulator.supported_gpus),
                            format_modes(&simulator.supported_modes),
                            simulator.supports_custom_gpus,
                            simulator.active_session_count,
                        );
                    }
                }
            }
            SimulatorCommand::Show { simulator, json } => {
                let reply = daemon
                    .get_simulator(GetSimulatorRequest {
                        name: simulator.clone(),
                    })
                    .await?;
                let simulator = reply
                    .simulator
                    .ok_or_else(|| CliError::new(format!("simulator '{}' not found", simulator)))?;
                if json {
                    print_json(&simulator)?;
                } else {
                    println!(
                        "{} {}",
                        simulator.name.unwrap_or_default(),
                        simulator.version.unwrap_or_default()
                    );
                    if let Some(description) = simulator.description {
                        println!("description: {description}");
                    }
                    println!("gpus: {}", format_gpus(&simulator.supported_gpus));
                    println!("modes: {}", format_modes(&simulator.supported_modes));
                    println!("custom gpu: {}", simulator.supports_custom_gpus);
                    println!("active sessions: {}", simulator.active_session_count);
                }
            }
        },
        Command::Profile { command } => match command {
            ProfileCommand::List { simulator, json } => {
                let reply = daemon
                    .list_profiles(ListProfilesRequest {
                        simulator_filter: simulator,
                    })
                    .await?;
                if json {
                    print_json(&reply.profiles)?;
                } else if reply.profiles.is_empty() {
                    println!("no profiles found");
                } else {
                    for profile in reply.profiles {
                        println!(
                            "{} | simulator: {} | gpu: {} | mode: {} | nodes: {} | gpus/node: {}",
                            profile.name,
                            profile.simulator,
                            profile.gpu,
                            mode_name(profile.mode),
                            profile.num_nodes,
                            profile.num_gpus,
                        );
                    }
                }
            }
            ProfileCommand::Create(args) => {
                let reply = daemon
                    .create_profile(CreateProfileRequest {
                        profile: ProfileDef {
                            name: args.name.clone(),
                            simulator: args.simulator,
                            mode: args.mode.into(),
                            gpu: args.gpu,
                            num_gpus: args.gpus_per_node,
                            num_nodes: args.nodes,
                        },
                    })
                    .await?;
                handle_profile_reply(
                    reply,
                    args.json,
                    &format!("profile '{}' created", args.name),
                )?;
            }
            ProfileCommand::Delete { name, json } => {
                let reply = daemon
                    .delete_profile(DeleteProfileRequest { name: name.clone() })
                    .await?;
                handle_delete_profile_reply(reply, json, &format!("profile '{}' deleted", name))?;
            }
        },
        Command::Session { command } => match command {
            SessionCommand::List { profile, json } => {
                let reply = daemon
                    .list_sessions(ListSessionsRequest {
                        profile_filter: profile,
                    })
                    .await?;
                if json {
                    print_json(&reply.sessions)?;
                } else if reply.sessions.is_empty() {
                    println!("no sessions found");
                } else {
                    for session in reply.sessions {
                        println!(
                            "{} | profile: {} | simulator: {} | image: {} | health: {}",
                            session.name.unwrap_or_default(),
                            session.profile.unwrap_or_default(),
                            session.simulator.unwrap_or_default(),
                            session.image.unwrap_or_default(),
                            health_name(session.health_status),
                        );
                    }
                }
            }
            SessionCommand::Create(args) => {
                let reply = daemon
                    .create_session(DashboardCreateSessionRequest {
                        session: SessionDef {
                            name: args.name.clone(),
                            profile: args.profile,
                            image: args.image,
                        },
                    })
                    .await?;
                handle_create_session_reply(
                    reply,
                    args.json,
                    &format!("session '{}' created", args.name),
                )?;
            }
            SessionCommand::Delete { name, json } => {
                let reply = daemon
                    .delete_session(DashboardDeleteSessionRequest { name: name.clone() })
                    .await?;
                handle_delete_session_reply(reply, json, &format!("session '{}' deleted", name))?;
            }
            SessionCommand::Show { name, json } => {
                let reply = daemon
                    .get_session_detail(GetSessionDetailRequest { name: name.clone() })
                    .await?;
                if json {
                    print_json(&reply)?;
                } else {
                    println!("name: {}", reply.name.unwrap_or(name));
                    if let Some(profile) = reply.profile {
                        println!("profile: {}", profile.name);
                        println!("simulator: {}", profile.simulator);
                        println!("gpu: {}", profile.gpu);
                        println!("mode: {}", mode_name(profile.mode));
                    }
                    println!("image: {}", reply.image.unwrap_or_default());
                    println!("health: {}", health_name(reply.health));
                    println!("ticks: {}", reply.ticks);
                    println!("ipc: {:.3}", reply.ipc);
                    println!("simulation speed: {:.3}", reply.simulation_speed);
                    println!("active contexts: {}", reply.active_contexts);
                    if let Some(uptime) = reply.uptime {
                        println!("uptime: {} s + {} ps", uptime.seconds, uptime.picoseconds);
                    }
                    if let Some(error_message) = reply.error_message {
                        println!("error: {error_message}");
                    }
                }
            }
        },
        Command::Boot(args) => {
            let reply = daemon
                .boot_session(BootSessionRequest {
                    session: SessionDef {
                        name: args.name.clone(),
                        profile: args.profile,
                        image: args.image,
                    },
                })
                .await?;
            handle_boot_reply(reply, args.json, &args.name)?;
        }
        Command::Exec(args) => {
            if args.command.is_empty() {
                return Err(CliError::new("no command provided after --"));
            }
            let (program, program_args) = args.command.split_first().unwrap();
            let reply = daemon
                .exec_in_session(ExecInSessionRequest {
                    session_name: args.name.clone(),
                    exec: ExecArgs {
                        command: program.clone(),
                        args: program_args.to_vec(),
                        env: vec![],
                    },
                })
                .await?;
            if args.json {
                print_json(&serde_json::json!({
                    "exit_code": reply.exit_code,
                    "stdout": String::from_utf8_lossy(&reply.stdout),
                    "stderr": String::from_utf8_lossy(&reply.stderr),
                }))?;
            } else {
                if !reply.stdout.is_empty() {
                    print!("{}", String::from_utf8_lossy(&reply.stdout));
                }
                if !reply.stderr.is_empty() {
                    eprint!("{}", String::from_utf8_lossy(&reply.stderr));
                }
                if reply.exit_code != 0 {
                    return Err(CliError::new(format!(
                        "exec exited with code {}",
                        reply.exit_code
                    )));
                }
            }
        }
        Command::Shutdown(args) => {
            let reply = daemon
                .shutdown_session(ShutdownSessionRequest {
                    name: args.name.clone(),
                })
                .await?;
            handle_shutdown_reply(
                reply,
                args.json,
                &format!("session '{}' shut down", args.name),
            )?;
        }
    }

    Ok(())
}

fn handle_profile_reply(reply: CreateProfileReply, json: bool, message: &str) -> CliResult {
    handle_ok_reply(reply.ok, reply.error, json, message)
}

fn handle_delete_profile_reply(reply: DeleteProfileReply, json: bool, message: &str) -> CliResult {
    handle_ok_reply(reply.ok, reply.error, json, message)
}

fn handle_create_session_reply(
    reply: DashboardCreateSessionReply,
    json: bool,
    message: &str,
) -> CliResult {
    handle_ok_reply(reply.ok, reply.error, json, message)
}

fn handle_delete_session_reply(
    reply: DashboardDeleteSessionReply,
    json: bool,
    message: &str,
) -> CliResult {
    handle_ok_reply(reply.ok, reply.error, json, message)
}

fn handle_boot_reply(reply: BootSessionReply, json: bool, name: &str) -> CliResult {
    if !reply.ok {
        return Err(CliError::new(
            reply.error.unwrap_or_else(|| "boot failed".to_string()),
        ));
    }
    if json {
        print_json(&serde_json::json!({
            "ok": true,
            "container_id": reply.container_id,
            "container_ids": reply.container_ids,
        }))?;
    } else {
        println!("session '{}' booted", name);
        if reply.container_ids.len() > 1 {
            for (i, id) in reply.container_ids.iter().enumerate() {
                println!("node{i}: {id}");
            }
        } else if let Some(id) = reply.container_id {
            println!("container: {id}");
        }
    }
    Ok(())
}

fn handle_shutdown_reply(reply: ShutdownSessionReply, json: bool, message: &str) -> CliResult {
    handle_ok_reply(reply.ok, reply.error, json, message)
}

fn handle_ok_reply(ok: bool, error: Option<String>, json: bool, message: &str) -> CliResult {
    if !ok {
        return Err(CliError::new(
            error.unwrap_or_else(|| "request failed".to_string()),
        ));
    }

    if json {
        print_json(&serde_json::json!({ "ok": true }))?;
    } else {
        println!("{message}");
    }
    Ok(())
}

fn print_json<T>(value: &T) -> CliResult
where
    T: serde::Serialize,
{
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

fn format_modes(modes: &[SimulatorMode]) -> String {
    if modes.is_empty() {
        return "all".to_string();
    }
    modes
        .iter()
        .map(|mode| mode_name(*mode))
        .collect::<Vec<_>>()
        .join(", ")
}

fn format_gpus(gpus: &[mirage_schema::common::GpuDef]) -> String {
    if gpus.is_empty() {
        return "none".to_string();
    }
    gpus.iter()
        .map(|gpu| gpu.name.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

fn mode_name(mode: SimulatorMode) -> &'static str {
    match mode {
        SimulatorMode::Functional => "functional",
        SimulatorMode::Clocked => "clocked",
        SimulatorMode::CycleAccurate => "cycle-accurate",
    }
}

fn health_name(status: HealthStatus) -> &'static str {
    match status {
        HealthStatus::Unknown => "unknown",
        HealthStatus::Healthy => "healthy",
        HealthStatus::Unhealthy => "unhealthy",
    }
}

#[allow(dead_code)]
fn format_time(time: Time) -> String {
    format!("{} s + {} ps", time.seconds, time.picoseconds)
}
