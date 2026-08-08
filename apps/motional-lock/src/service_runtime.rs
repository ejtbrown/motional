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
use crate::timestamp::event_timestamp;

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
            Ok(event) => log_monitor_event(&config, event),
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

fn log_monitor_event(config: &AppConfig, event: MonitorEvent) {
    eprintln!("{}", format_monitor_event(config, &event));
}

fn format_monitor_event(config: &AppConfig, event: &MonitorEvent) -> String {
    match event {
        MonitorEvent::Status { entry_id, message } => {
            let label = entry_log_label(config, entry_id);
            format!(
                "{}\tmotional-service: {label}: {message}",
                event_timestamp(None)
            )
        }
        MonitorEvent::State { entry_id, state } => {
            let label = entry_log_label(config, entry_id);
            format!(
                "{}\tmotional-service: {label}: {} triggered={}",
                event_timestamp(state.observed_at.as_deref()),
                state.name,
                state
                    .triggered
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "unknown".to_string())
            )
        }
        MonitorEvent::SensorList { entry_id, sensors } => {
            let label = entry_log_label(config, entry_id);
            format!(
                "{}\tmotional-service: {label}: loaded {}",
                event_timestamp(None),
                entry_count_label(sensors.len())
            )
        }
        MonitorEvent::Action {
            entry_id,
            trigger,
            action,
            ok,
            message,
        } => {
            let label = entry_log_label(config, entry_id);
            let outcome = if *ok { "ok" } else { "failed" };
            format!(
                "{}\tmotional-service: {label}: {} action {outcome}: {action}: {message}",
                event_timestamp(None),
                trigger.label()
            )
        }
    }
}

fn entry_log_label<'a>(config: &'a AppConfig, entry_id: &str) -> &'a str {
    config
        .entries
        .iter()
        .find(|entry| entry.id == entry_id)
        .and_then(|entry| {
            [&entry.label, &entry.sensor, &entry.address]
                .into_iter()
                .map(|value| value.trim())
                .find(|value| !value.is_empty())
        })
        .unwrap_or("Motional entry")
}

fn entry_count_label(count: usize) -> String {
    if count == 1 {
        "1 entry".to_string()
    } else {
        format!("{count} entries")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ServerEntry;
    use crate::msp::SensorState;

    #[test]
    fn state_event_log_uses_observation_time_and_label_instead_of_entry_id() {
        let mut entry = ServerEntry::new();
        entry.id = "entry-123456".to_string();
        entry.label = "Office".to_string();
        let config = AppConfig {
            entries: vec![entry],
        };
        let event = MonitorEvent::State {
            entry_id: "entry-123456".to_string(),
            state: SensorState {
                name: "office".to_string(),
                triggered: Some(false),
                status: Some("ok".to_string()),
                last_triggered_at: None,
                seconds_since_triggered: Some(42),
                observed_at: Some("2026-08-08T14:06:01.000Z".to_string()),
                sequence: None,
                raw: None,
            },
        };

        let line = format_monitor_event(&config, &event);
        assert_eq!(
            line,
            "2026-08-08T14:06:01.000Z\tmotional-service: Office: office triggered=false"
        );
        assert!(!line.contains("entry-123456"));
    }
}
