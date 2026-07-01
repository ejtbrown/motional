use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};

use crate::actions::{log_restore_results, ActionSession};
use crate::config::{config_path, load_config, AppConfig};
use crate::monitor::{spawn_entry_monitor, MonitorEvent, MonitorHandle};
use crate::service_control::{clear_stop_request, service_stop_requested};

#[derive(Debug, Clone)]
pub struct ServiceRunOptions {
    pub config_path: PathBuf,
    pub dry_run: bool,
}

impl Default for ServiceRunOptions {
    fn default() -> Self {
        Self {
            config_path: config_path(),
            dry_run: false,
        }
    }
}

pub fn run_service(options: ServiceRunOptions) -> Result<()> {
    clear_stop_request()?;

    let config = load_config(&options.config_path)
        .with_context(|| format!("failed to load {}", options.config_path.display()))?;
    eprintln!(
        "motional-service: loaded {} from {}",
        entry_count_label(config.entries.len()),
        options.config_path.display()
    );

    let stop = Arc::new(AtomicBool::new(false));
    install_stop_handler(Arc::clone(&stop))?;

    let action_session = Arc::new(ActionSession::new());
    let (tx, rx) = mpsc::channel();
    let mut monitors = spawn_monitors(&config, &tx, options.dry_run, Arc::clone(&action_session));
    if monitors.is_empty() {
        eprintln!("motional-service: no enabled entries with configured sensors");
    }

    while !stop.load(Ordering::Relaxed) {
        match rx.recv_timeout(Duration::from_millis(500)) {
            Ok(event) => log_monitor_event(event),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
        if service_stop_requested() {
            break;
        }
    }

    eprintln!("motional-service: stopping");
    for monitor in monitors.drain(..) {
        monitor.stop();
    }
    log_restore_results(&action_session.restore_original_settings());
    let _ = clear_stop_request();
    eprintln!("motional-service: stopped");
    Ok(())
}

fn spawn_monitors(
    config: &AppConfig,
    tx: &mpsc::Sender<MonitorEvent>,
    dry_run: bool,
    action_session: Arc<ActionSession>,
) -> Vec<MonitorHandle> {
    config
        .entries
        .iter()
        .filter(|entry| {
            entry.enabled && !entry.address.trim().is_empty() && !entry.sensor.trim().is_empty()
        })
        .map(|entry| {
            spawn_entry_monitor(
                entry.clone(),
                tx.clone(),
                dry_run,
                Arc::clone(&action_session),
            )
        })
        .collect()
}

fn install_stop_handler(stop: Arc<AtomicBool>) -> Result<()> {
    ctrlc::set_handler(move || {
        stop.store(true, Ordering::Relaxed);
    })
    .context("failed to install service shutdown handler")
}

fn log_monitor_event(event: MonitorEvent) {
    match event {
        MonitorEvent::Status { entry_id, message } => {
            eprintln!("motional-service: {entry_id}: {message}");
        }
        MonitorEvent::State { entry_id, state } => {
            eprintln!(
                "motional-service: {entry_id}: {} triggered={}",
                state.name,
                state
                    .triggered
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "unknown".to_string())
            );
        }
        MonitorEvent::SensorList { entry_id, sensors } => {
            eprintln!(
                "motional-service: {entry_id}: loaded {}",
                entry_count_label(sensors.len())
            );
        }
        MonitorEvent::Action {
            entry_id,
            trigger,
            action,
            ok,
            message,
        } => {
            let outcome = if ok { "ok" } else { "failed" };
            eprintln!(
                "motional-service: {entry_id}: {} action {outcome}: {action}: {message}",
                trigger.label()
            );
        }
    }
}

fn entry_count_label(count: usize) -> String {
    if count == 1 {
        "1 entry".to_string()
    } else {
        format!("{count} entries")
    }
}
