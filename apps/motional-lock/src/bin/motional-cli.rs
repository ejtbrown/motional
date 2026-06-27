use std::thread;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use motional_clients::actions::{execute_actions, parse_cli_action, Action, ActionTrigger};
use motional_clients::msp::{MspConnection, MspEvent, SensorState};

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

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::List(args) => list(args),
        Command::Get(args) => get(args),
        Command::Watch(args) => watch(args),
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

    if args.poll_interval == 0 {
        bail!("--poll-interval must be greater than zero");
    }

    match args.mode {
        WatchMode::Subscribe => watch_subscribe(args, on_motion, on_absence),
        WatchMode::Poll => watch_poll(args, on_motion, on_absence),
    }
}

fn watch_subscribe(args: WatchArgs, on_motion: Vec<Action>, on_absence: Vec<Action>) -> Result<()> {
    loop {
        match watch_subscribe_once(&args, &on_motion, &on_absence) {
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
                    );
                }
            }
            MspEvent::Other(_) => {}
        }
    }
}

fn watch_poll(args: WatchArgs, on_motion: Vec<Action>, on_absence: Vec<Action>) -> Result<()> {
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

    for result in execute_actions(actions, dry_run) {
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
