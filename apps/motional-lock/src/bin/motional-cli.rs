use std::path::PathBuf;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use motional_clients::actions::{
    execute_actions_with_session, install_ctrlc_restore_handler, log_restore_results,
    parse_cli_action, Action, ActionSession, ActionTrigger,
};
use motional_clients::config::{config_path, load_config, save_config, AppConfig};
use motional_clients::msp::{MspConnection, MspEvent, SensorState};
use motional_clients::service_control::{
    install_service, remove_service, restart_service, service_status, start_service, stop_service,
    ServiceInstallOptions,
};

#[derive(Debug, Parser)]
#[command(author, version, about = "CLI client for Motional Service Protocol")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    List(CommonArgs),
    Get(GetArgs),
    Watch(WatchArgs),
    Service(ServiceArgs),
    Config(ConfigArgs),
}

#[derive(Debug, Parser)]
struct CommonArgs {
    #[arg(long, env = "MOTIONAL_SERVER", default_value = "127.0.0.1:7080")]
    server: String,

    #[arg(long, env = "MOTIONAL_TOKEN")]
    token: Option<String>,
}

#[derive(Debug, Parser)]
struct GetArgs {
    #[command(flatten)]
    common: CommonArgs,

    #[arg(long, env = "MOTIONAL_SENSOR")]
    sensor: String,
}

#[derive(Debug, Parser)]
struct WatchArgs {
    #[command(flatten)]
    common: CommonArgs,

    #[arg(long, env = "MOTIONAL_SENSOR")]
    sensor: String,

    #[arg(long, value_enum, default_value_t = WatchMode::Subscribe)]
    mode: WatchMode,

    #[arg(long, default_value_t = 30)]
    poll_interval: u64,

    #[arg(long)]
    fire_initial: bool,

    #[arg(long)]
    dry_run: bool,

    #[arg(long = "on-motion", value_name = "ACTION")]
    on_motion: Vec<String>,

    #[arg(long = "on-absence", value_name = "ACTION")]
    on_absence: Vec<String>,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum WatchMode {
    Subscribe,
    Poll,
}

#[derive(Debug, Parser)]
struct ServiceArgs {
    #[command(subcommand)]
    command: ServiceCommand,
}

#[derive(Debug, Subcommand)]
enum ServiceCommand {
    Install(ServiceInstallArgs),
    Remove,
    Start,
    Stop,
    Restart,
    Status,
}

#[derive(Debug, Parser)]
struct ServiceInstallArgs {
    #[arg(long)]
    service_binary: Option<PathBuf>,

    #[arg(long)]
    start: bool,
}

#[derive(Debug, Parser)]
struct ConfigArgs {
    #[command(subcommand)]
    command: ConfigCommand,
}

#[derive(Debug, Subcommand)]
enum ConfigCommand {
    Path,
    Show,
    Write(ConfigWriteArgs),
}

#[derive(Debug, Parser)]
struct ConfigWriteArgs {
    #[arg(long)]
    file: PathBuf,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::List(args) => list(args),
        Command::Get(args) => get(args),
        Command::Watch(args) => watch(args),
        Command::Service(args) => service(args),
        Command::Config(args) => config(args),
    }
}

fn list(args: CommonArgs) -> Result<()> {
    let mut connection = connect(&args, "motional-cli")?;
    for sensor in connection.list_sensors()? {
        let display_name = sensor.display_name.unwrap_or_else(|| sensor.name.clone());
        let kind = sensor.kind.unwrap_or_else(|| "unknown".to_string());
        println!("{}\t{}\t{}", sensor.name, kind, display_name);
    }
    Ok(())
}

fn get(args: GetArgs) -> Result<()> {
    let mut connection = connect(&args.common, "motional-cli")?;
    let states = connection.get_states(std::slice::from_ref(&args.sensor))?;
    for state in states {
        print_state(&state);
    }
    Ok(())
}

fn watch(args: WatchArgs) -> Result<()> {
    let on_motion = parse_actions(&args.on_motion).context("invalid --on-motion action")?;
    let on_absence = parse_actions(&args.on_absence).context("invalid --on-absence action")?;
    let action_session = Arc::new(ActionSession::new());
    install_ctrlc_restore_handler(Arc::clone(&action_session))?;
    let _restore_guard = RestoreOnDrop {
        session: Arc::clone(&action_session),
    };

    if args.poll_interval == 0 {
        bail!("--poll-interval must be greater than zero");
    }

    match args.mode {
        WatchMode::Subscribe => watch_subscribe(args, on_motion, on_absence, action_session),
        WatchMode::Poll => watch_poll(args, on_motion, on_absence, action_session),
    }
}

struct RestoreOnDrop {
    session: Arc<ActionSession>,
}

impl Drop for RestoreOnDrop {
    fn drop(&mut self) {
        log_restore_results(&self.session.restore_original_settings());
    }
}

fn watch_subscribe(
    args: WatchArgs,
    on_motion: Vec<Action>,
    on_absence: Vec<Action>,
    action_session: Arc<ActionSession>,
) -> Result<()> {
    loop {
        match watch_subscribe_once(&args, &on_motion, &on_absence, &action_session) {
            Ok(()) => return Ok(()),
            Err(error) => {
                eprintln!("motional-cli: {error:#}; reconnecting in 5s");
                thread::sleep(Duration::from_secs(5));
            }
        }
    }
}

fn watch_subscribe_once(
    args: &WatchArgs,
    on_motion: &[Action],
    on_absence: &[Action],
    action_session: &ActionSession,
) -> Result<()> {
    let mut connection = connect(&args.common, "motional-cli")?;
    let subscription_id = connection.subscribe(std::slice::from_ref(&args.sensor), true)?;
    eprintln!("subscribed as {subscription_id}");

    let mut last_triggered: Option<bool> = None;
    loop {
        match connection.read_event()? {
            MspEvent::StateChanged { state, .. } => {
                handle_state(
                    &state,
                    &mut last_triggered,
                    args.fire_initial,
                    on_motion,
                    on_absence,
                    args.dry_run,
                    action_session,
                );
            }
            MspEvent::ResyncRequired { .. } => {
                for state in connection.get_states(std::slice::from_ref(&args.sensor))? {
                    handle_state(
                        &state,
                        &mut last_triggered,
                        args.fire_initial,
                        on_motion,
                        on_absence,
                        args.dry_run,
                        action_session,
                    );
                }
            }
            MspEvent::Other(_) => {}
        }
    }
}

fn watch_poll(
    args: WatchArgs,
    on_motion: Vec<Action>,
    on_absence: Vec<Action>,
    action_session: Arc<ActionSession>,
) -> Result<()> {
    let mut last_triggered: Option<bool> = None;
    loop {
        match connect(&args.common, "motional-cli")
            .and_then(|mut connection| connection.get_states(std::slice::from_ref(&args.sensor)))
        {
            Ok(states) => {
                for state in states {
                    handle_state(
                        &state,
                        &mut last_triggered,
                        args.fire_initial,
                        &on_motion,
                        &on_absence,
                        args.dry_run,
                        &action_session,
                    );
                }
            }
            Err(error) => eprintln!("motional-cli: {error:#}"),
        }
        thread::sleep(Duration::from_secs(args.poll_interval));
    }
}

fn handle_state(
    state: &SensorState,
    last_triggered: &mut Option<bool>,
    fire_initial: bool,
    on_motion: &[Action],
    on_absence: &[Action],
    dry_run: bool,
    action_session: &ActionSession,
) {
    print_state(state);

    let Some(triggered) = state.triggered else {
        return;
    };

    let should_fire = match *last_triggered {
        Some(previous) => previous != triggered,
        None => fire_initial,
    };
    *last_triggered = Some(triggered);

    if !should_fire {
        return;
    }

    let (trigger, actions) = if triggered {
        (ActionTrigger::Motion, on_motion)
    } else {
        (ActionTrigger::Absence, on_absence)
    };

    for result in execute_actions_with_session(actions, dry_run, action_session) {
        if result.ok {
            eprintln!("{} action succeeded: {}", trigger.label(), result.label);
        } else {
            eprintln!(
                "{} action failed: {}: {}",
                trigger.label(),
                result.label,
                result.message
            );
        }
    }
}

fn print_state(state: &SensorState) {
    let triggered = state
        .triggered
        .map(|value| value.to_string())
        .unwrap_or_else(|| "unknown".to_string());
    let since = state
        .seconds_since_triggered
        .map(|value| value.to_string())
        .unwrap_or_else(|| "unknown".to_string());
    let status = state.status.as_deref().unwrap_or("unknown");
    println!(
        "{}\ttriggered={}\tstatus={}\tseconds_since_triggered={}",
        state.name, triggered, status, since
    );
}

fn connect(args: &CommonArgs, client_name: &str) -> Result<MspConnection> {
    MspConnection::connect(&args.server, args.token.as_deref(), client_name)
}

fn parse_actions(specs: &[String]) -> Result<Vec<Action>> {
    specs.iter().map(|spec| parse_cli_action(spec)).collect()
}

fn service(args: ServiceArgs) -> Result<()> {
    match args.command {
        ServiceCommand::Install(args) => {
            let message = install_service(&ServiceInstallOptions {
                service_binary: args.service_binary,
                start: args.start,
            })?;
            println!("{message}");
        }
        ServiceCommand::Remove => println!("{}", remove_service()?),
        ServiceCommand::Start => println!("{}", start_service()?),
        ServiceCommand::Stop => println!("{}", stop_service()?),
        ServiceCommand::Restart => println!("{}", restart_service()?),
        ServiceCommand::Status => {
            let status = service_status()?;
            println!("installed={}", status.installed);
            println!("running={}", status.running);
            println!("{}", status.detail);
        }
    }
    Ok(())
}

fn config(args: ConfigArgs) -> Result<()> {
    let path = config_path();
    match args.command {
        ConfigCommand::Path => println!("{}", path.display()),
        ConfigCommand::Show => {
            let config = load_config(&path)?;
            println!("{}", serde_json::to_string_pretty(&config)?);
        }
        ConfigCommand::Write(args) => {
            let text = std::fs::read_to_string(&args.file)
                .with_context(|| format!("failed to read {}", args.file.display()))?;
            let config: AppConfig = serde_json::from_str(&text)
                .with_context(|| format!("failed to parse {}", args.file.display()))?;
            save_config(&path, &config)?;
            println!("wrote {}", path.display());
        }
    }
    Ok(())
}
