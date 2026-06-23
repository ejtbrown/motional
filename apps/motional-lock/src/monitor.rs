use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crate::actions::{execute_actions, ActionTrigger};
use crate::config::ServerEntry;
use crate::msp::{MspConnection, MspEvent, SensorDescription, SensorState};

const RECONNECT_INTERVAL: Duration = Duration::from_secs(15);

#[derive(Debug, Clone)]
pub enum MonitorEvent {
    Status {
        entry_id: String,
        message: String,
    },
    State {
        entry_id: String,
        state: SensorState,
    },
    SensorList {
        entry_id: String,
        sensors: Vec<SensorDescription>,
    },
    Action {
        entry_id: String,
        trigger: ActionTrigger,
        action: String,
        ok: bool,
        message: String,
    },
}

pub struct MonitorHandle {
    stop: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
}

impl MonitorHandle {
    pub fn stop(mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = self.join.take();
    }
}

impl Drop for MonitorHandle {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

pub fn spawn_entry_monitor(
    entry: ServerEntry,
    tx: Sender<MonitorEvent>,
    dry_run: bool,
) -> MonitorHandle {
    let stop = Arc::new(AtomicBool::new(false));
    let thread_stop = Arc::clone(&stop);
    let join = thread::spawn(move || monitor_loop(entry, tx, dry_run, thread_stop));

    MonitorHandle {
        stop,
        join: Some(join),
    }
}

fn monitor_loop(
    entry: ServerEntry,
    tx: Sender<MonitorEvent>,
    dry_run: bool,
    stop: Arc<AtomicBool>,
) {
    let mut availability = ServerAvailability::Unknown;

    while !stop.load(Ordering::Relaxed) {
        let result = monitor_once(&entry, &tx, dry_run, &stop, &mut availability);
        if let Err(error) = result {
            let _ = tx.send(MonitorEvent::Status {
                entry_id: entry.id.clone(),
                message: format!("{error:#}; reconnecting in 15s"),
            });
            if availability != ServerAvailability::Disconnected {
                emit_action_results(
                    &entry,
                    &tx,
                    ActionTrigger::Disconnected,
                    &entry.on_disconnected,
                    dry_run,
                );
                availability = ServerAvailability::Disconnected;
            }
            sleep_interruptible(RECONNECT_INTERVAL, &stop);
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ServerAvailability {
    Unknown,
    Connected,
    Disconnected,
}

fn monitor_once(
    entry: &ServerEntry,
    tx: &Sender<MonitorEvent>,
    dry_run: bool,
    stop: &AtomicBool,
    availability: &mut ServerAvailability,
) -> anyhow::Result<()> {
    let _ = tx.send(MonitorEvent::Status {
        entry_id: entry.id.clone(),
        message: format!("connecting to {}", entry.address),
    });

    let mut connection = MspConnection::connect(
        &entry.address,
        token_option(&entry.token),
        "motional-gui-monitor",
        entry.allow_insecure_msp,
    )?;
    let subscription_id = connection.subscribe(std::slice::from_ref(&entry.sensor), true)?;

    let _ = tx.send(MonitorEvent::Status {
        entry_id: entry.id.clone(),
        message: format!("subscribed as {subscription_id}"),
    });
    if *availability != ServerAvailability::Connected {
        emit_action_results(
            entry,
            tx,
            ActionTrigger::Connected,
            &entry.on_connected,
            dry_run,
        );
        *availability = ServerAvailability::Connected;
    }

    let mut last_triggered: Option<bool> = None;
    while !stop.load(Ordering::Relaxed) {
        match connection.read_event()? {
            MspEvent::StateChanged { state, .. } => {
                let triggered = state.triggered;
                let _ = tx.send(MonitorEvent::State {
                    entry_id: entry.id.clone(),
                    state,
                });

                if let Some(triggered) = triggered {
                    if let Some(previous) = last_triggered {
                        if previous != triggered {
                            let (trigger, actions) = if triggered {
                                (ActionTrigger::Motion, &entry.on_motion)
                            } else {
                                (ActionTrigger::Absence, &entry.on_absence)
                            };
                            emit_action_results(entry, tx, trigger, actions, dry_run);
                        }
                    }
                    last_triggered = Some(triggered);
                }
            }
            MspEvent::ResyncRequired { .. } => {
                let states = connection.get_states(std::slice::from_ref(&entry.sensor))?;
                for state in states {
                    let _ = tx.send(MonitorEvent::State {
                        entry_id: entry.id.clone(),
                        state,
                    });
                }
            }
            MspEvent::Other(_) => {}
        }
    }

    Ok(())
}

fn emit_action_results(
    entry: &ServerEntry,
    tx: &Sender<MonitorEvent>,
    trigger: ActionTrigger,
    actions: &[crate::actions::Action],
    dry_run: bool,
) {
    for result in execute_actions(actions, dry_run) {
        let _ = tx.send(MonitorEvent::Action {
            entry_id: entry.id.clone(),
            trigger,
            action: result.label,
            ok: result.ok,
            message: result.message,
        });
    }
}

fn token_option(token: &str) -> Option<&str> {
    let token = token.trim();
    if token.is_empty() {
        None
    } else {
        Some(token)
    }
}

fn sleep_interruptible(duration: Duration, stop: &AtomicBool) {
    let mut elapsed = Duration::ZERO;
    while elapsed < duration && !stop.load(Ordering::Relaxed) {
        let step = Duration::from_millis(100);
        thread::sleep(step);
        elapsed += step;
    }
}
