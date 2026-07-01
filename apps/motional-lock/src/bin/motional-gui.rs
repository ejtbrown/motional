use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Duration;

use chrono::Local;
use eframe::egui;
use motional_clients::actions::{Action, ActionTrigger};
use motional_clients::config::{config_path, load_config, save_config, AppConfig, ServerEntry};
use motional_clients::monitor::MonitorEvent;
use motional_clients::msp::{MspConnection, SensorDescription, SensorState};
use motional_clients::service_control::{
    install_service, remove_service, restart_service, service_status, start_service, stop_service,
    ServiceInstallOptions,
};

const APP_ID: &str = "com.ejtbrown.motional";
const APP_NAME: &str = "Motional";

fn main() -> eframe::Result {
    let mut viewport = egui::ViewportBuilder::default()
        .with_app_id(APP_ID)
        .with_title(APP_NAME)
        .with_inner_size([1040.0, 760.0]);
    if let Some(icon) = app_icon() {
        viewport = viewport.with_icon(icon);
    }

    let options = eframe::NativeOptions {
        viewport,
        renderer: native_renderer(),
        ..Default::default()
    };

    eframe::run_native(
        APP_NAME,
        options,
        Box::new(|_cc| Ok(Box::new(MotionalGuiApp::new()))),
    )
}

fn app_icon() -> Option<egui::IconData> {
    eframe::icon_data::from_png_bytes(include_bytes!("../../assets/motional-icon.png")).ok()
}

#[cfg(target_os = "windows")]
fn native_renderer() -> eframe::Renderer {
    eframe::Renderer::Wgpu
}

#[cfg(not(target_os = "windows"))]
fn native_renderer() -> eframe::Renderer {
    eframe::Renderer::Glow
}

struct MotionalGuiApp {
    config: AppConfig,
    config_path: PathBuf,
    tx: Sender<MonitorEvent>,
    rx: Receiver<MonitorEvent>,
    sensors: HashMap<String, Vec<SensorDescription>>,
    states: HashMap<String, SensorState>,
    statuses: HashMap<String, String>,
    logs: VecDeque<String>,
    rest_editor: Option<RestEditor>,
    key_capture: Option<ActionTarget>,
    dirty: bool,
}

#[derive(Debug, Clone)]
struct RestEditor {
    target: ActionTarget,
    method: String,
    url: String,
    body: String,
}

#[derive(Debug, Clone, Copy)]
struct ActionTarget {
    entry_index: usize,
    trigger: ActionTrigger,
    action_index: usize,
}

#[derive(Debug, Clone, Copy)]
struct ActionTargetBase {
    entry_index: usize,
    trigger: ActionTrigger,
}

struct ActionSectionState<'a> {
    rest_editor: &'a mut Option<RestEditor>,
    key_capture: &'a mut Option<ActionTarget>,
    dirty: &'a mut bool,
}

impl MotionalGuiApp {
    fn new() -> Self {
        let path = config_path();
        let (tx, rx) = mpsc::channel();
        Self {
            config: load_config(&path).unwrap_or_default(),
            config_path: path,
            tx,
            rx,
            sensors: HashMap::new(),
            states: HashMap::new(),
            statuses: HashMap::new(),
            logs: VecDeque::new(),
            rest_editor: None,
            key_capture: None,
            dirty: false,
        }
    }

    fn save_and_restart_service(&mut self) {
        match save_config(&self.config_path, &self.config) {
            Ok(()) => {
                self.dirty = false;
                self.push_log(format!("saved {}", self.config_path.display()));
                match restart_service() {
                    Ok(message) => self.push_log(message),
                    Err(error) => self.push_log(format!("service restart failed: {error:#}")),
                }
            }
            Err(error) => self.push_log(format!("save failed: {error:#}")),
        }
    }

    fn push_log(&mut self, message: String) {
        let timestamp = Local::now().format("%Y-%m-%d %H:%M:%S");
        self.logs.push_back(format!("{timestamp}  {message}"));
        while self.logs.len() > 500 {
            self.logs.pop_front();
        }
    }

    fn entry_log_label(&self, entry_id: &str) -> String {
        self.config
            .entries
            .iter()
            .find(|entry| entry.id == entry_id)
            .and_then(|entry| {
                let label = entry.label.trim();
                (!label.is_empty()).then(|| label.to_string())
            })
            .unwrap_or_else(|| entry_id.to_string())
    }

    fn process_monitor_events(&mut self) {
        while let Ok(event) = self.rx.try_recv() {
            match event {
                MonitorEvent::Status { entry_id, message } => {
                    let label = self.entry_log_label(&entry_id);
                    self.statuses.insert(entry_id.clone(), message.clone());
                    self.push_log(format!("{label}: {message}"));
                }
                MonitorEvent::State { entry_id, state } => {
                    let label = self.entry_log_label(&entry_id);
                    self.states.insert(entry_id.clone(), state.clone());
                    self.push_log(format!(
                        "{}: {} triggered={}",
                        label,
                        state.name,
                        state
                            .triggered
                            .map(|value| value.to_string())
                            .unwrap_or_else(|| "unknown".to_string())
                    ));
                }
                MonitorEvent::SensorList { entry_id, sensors } => {
                    let label = self.entry_log_label(&entry_id);
                    let count = sensors.len();
                    self.sensors.insert(entry_id.clone(), sensors);
                    self.statuses
                        .insert(entry_id.clone(), format!("loaded {count} sensors"));
                    self.push_log(format!("{label}: loaded {count} sensors"));
                }
                MonitorEvent::Action {
                    entry_id,
                    trigger,
                    action,
                    ok,
                    message,
                } => {
                    let label = self.entry_log_label(&entry_id);
                    let outcome = if ok { "ok" } else { "failed" };
                    self.push_log(format!(
                        "{label}: {} action {outcome}: {action}: {message}",
                        trigger.label()
                    ));
                }
            }
        }
    }

    fn refresh_sensors(&mut self, entry_index: usize) {
        let Some(entry) = self.config.entries.get(entry_index) else {
            return;
        };

        let entry_id = entry.id.clone();
        let label = entry.label.clone();
        let address = entry.address.clone();
        let token = entry.token.clone();
        let tx = self.tx.clone();

        self.statuses
            .insert(entry_id.clone(), "refreshing sensor list".to_string());
        self.push_log(format!("{label}: refreshing sensor list"));

        thread::spawn(move || {
            let result = MspConnection::connect(&address, token_option(&token), "motional-gui")
                .and_then(|mut connection| connection.list_sensors());

            match result {
                Ok(sensors) => {
                    let _ = tx.send(MonitorEvent::SensorList { entry_id, sensors });
                }
                Err(error) => {
                    let _ = tx.send(MonitorEvent::Status {
                        entry_id,
                        message: format!("sensor refresh failed: {error:#}"),
                    });
                }
            }
        });
    }

    fn action_mut(&mut self, target: ActionTarget) -> Option<&mut Action> {
        let entry = self.config.entries.get_mut(target.entry_index)?;
        let actions = match target.trigger {
            ActionTrigger::Connected => &mut entry.on_connected,
            ActionTrigger::Disconnected => &mut entry.on_disconnected,
            ActionTrigger::Motion => &mut entry.on_motion,
            ActionTrigger::Absence => &mut entry.on_absence,
        };
        actions.get_mut(target.action_index)
    }

    fn run_service_command(
        &mut self,
        label: &str,
        command: impl FnOnce() -> anyhow::Result<String>,
    ) {
        match command() {
            Ok(message) => self.push_log(message),
            Err(error) => self.push_log(format!("{label} failed: {error:#}")),
        }
    }
}

impl eframe::App for MotionalGuiApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.process_monitor_events();

        egui::TopBottomPanel::top("top").show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                if ui.button("Add Server Entry").clicked() {
                    self.config.entries.push(ServerEntry::new());
                    self.dirty = true;
                }
                if ui.button("Save and Restart Service").clicked() {
                    self.save_and_restart_service();
                }
                if ui.button("Install Service").clicked() {
                    self.run_service_command("install service", || {
                        install_service(&ServiceInstallOptions {
                            service_binary: None,
                            start: false,
                        })
                    });
                }
                if ui.button("Remove Service").clicked() {
                    self.run_service_command("remove service", remove_service);
                }
                if ui.button("Start Service").clicked() {
                    self.run_service_command("start service", start_service);
                }
                if ui.button("Stop Service").clicked() {
                    self.run_service_command("stop service", stop_service);
                }
                if ui.button("Service Status").clicked() {
                    match service_status() {
                        Ok(status) => self.push_log(format!(
                            "service installed={} running={}",
                            status.installed, status.running
                        )),
                        Err(error) => self.push_log(format!("service status failed: {error:#}")),
                    }
                }
                if self.dirty {
                    ui.label("Unsaved changes");
                }
            });
        });

        egui::TopBottomPanel::bottom("log")
            .resizable(true)
            .default_height(220.0)
            .height_range(120.0..=520.0)
            .show(ctx, |ui| {
                ui.heading("Activity");
                ui.separator();
                egui::ScrollArea::vertical()
                    .id_salt("activity-log")
                    .stick_to_bottom(true)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        for log in &self.logs {
                            ui.label(log);
                        }
                    });
            });

        egui::CentralPanel::default().show(ctx, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                let mut remove_entry = None;
                for entry_index in 0..self.config.entries.len() {
                    ui.separator();
                    if self.render_entry(ui, entry_index) {
                        remove_entry = Some(entry_index);
                    }
                }
                if let Some(index) = remove_entry {
                    let entry = self.config.entries.remove(index);
                    self.sensors.remove(&entry.id);
                    self.states.remove(&entry.id);
                    self.statuses.remove(&entry.id);
                    self.dirty = true;
                }
            });
        });

        self.render_rest_editor(ctx);
        self.render_key_capture(ctx);

        ctx.request_repaint_after(Duration::from_millis(500));
    }
}

impl MotionalGuiApp {
    fn render_entry(&mut self, ui: &mut egui::Ui, entry_index: usize) -> bool {
        let mut remove = false;
        let mut refresh_sensor_list = false;
        let entry_id = self.config.entries[entry_index].id.clone();
        let title = self.config.entries[entry_index].label.clone();

        egui::CollapsingHeader::new(title)
            .id_salt(entry_id.clone())
            .default_open(true)
            .show(ui, |ui| {
                let entry = &mut self.config.entries[entry_index];

                ui.horizontal(|ui| {
                    if ui.checkbox(&mut entry.enabled, "Enabled").changed() {
                        self.dirty = true;
                    }
                    if ui.button("Remove Entry").clicked() {
                        remove = true;
                    }
                });

                egui::Grid::new(format!("entry-grid-{entry_id}"))
                    .num_columns(2)
                    .spacing([12.0, 8.0])
                    .show(ui, |ui| {
                        ui.label("Label");
                        if ui.text_edit_singleline(&mut entry.label).changed() {
                            self.dirty = true;
                        }
                        ui.end_row();

                        ui.label("Server");
                        if ui.text_edit_singleline(&mut entry.address).changed() {
                            self.dirty = true;
                        }
                        ui.end_row();

                        ui.label("Token");
                        if ui
                            .add(egui::TextEdit::singleline(&mut entry.token).password(true))
                            .changed()
                        {
                            self.dirty = true;
                        }
                        ui.end_row();
                    });

                ui.horizontal(|ui| {
                    ui.label("Sensor");
                    let sensors = self.sensors.get(&entry_id).cloned().unwrap_or_default();
                    egui::ComboBox::from_id_salt(format!("sensor-{entry_id}"))
                        .selected_text(if entry.sensor.is_empty() {
                            "Select sensor".to_string()
                        } else {
                            entry.sensor.clone()
                        })
                        .show_ui(ui, |ui| {
                            for sensor in sensors {
                                let display = sensor
                                    .display_name
                                    .clone()
                                    .unwrap_or_else(|| sensor.name.clone());
                                if ui
                                    .selectable_label(entry.sensor == sensor.name, display)
                                    .clicked()
                                {
                                    entry.sensor = sensor.name;
                                    self.dirty = true;
                                }
                            }
                        });
                    if ui.text_edit_singleline(&mut entry.sensor).changed() {
                        self.dirty = true;
                    }
                    if ui.button("List Sensors").clicked() {
                        refresh_sensor_list = true;
                    }
                });

                if let Some(status) = self.statuses.get(&entry_id) {
                    ui.label(format!("Status: {status}"));
                }
                if let Some(state) = self.states.get(&entry_id) {
                    ui.label(format!(
                        "State: triggered={}, status={}, seconds_since_triggered={}",
                        state
                            .triggered
                            .map(|value| value.to_string())
                            .unwrap_or_else(|| "unknown".to_string()),
                        state.status.as_deref().unwrap_or("unknown"),
                        state
                            .seconds_since_triggered
                            .map(|value| value.to_string())
                            .unwrap_or_else(|| "unknown".to_string())
                    ));
                }

                ui.columns(2, |columns| {
                    render_action_section(
                        &mut columns[0],
                        "On Server Connected",
                        ActionTargetBase {
                            entry_index,
                            trigger: ActionTrigger::Connected,
                        },
                        &mut self.config.entries[entry_index].on_connected,
                        &connection_action_templates(),
                        Action::DisableTimedScreenLock,
                        &mut ActionSectionState {
                            rest_editor: &mut self.rest_editor,
                            key_capture: &mut self.key_capture,
                            dirty: &mut self.dirty,
                        },
                    );
                    render_action_section(
                        &mut columns[1],
                        "On Server Disconnected",
                        ActionTargetBase {
                            entry_index,
                            trigger: ActionTrigger::Disconnected,
                        },
                        &mut self.config.entries[entry_index].on_disconnected,
                        &connection_action_templates(),
                        Action::EnableTimedScreenLock,
                        &mut ActionSectionState {
                            rest_editor: &mut self.rest_editor,
                            key_capture: &mut self.key_capture,
                            dirty: &mut self.dirty,
                        },
                    );
                });

                ui.columns(2, |columns| {
                    render_action_section(
                        &mut columns[0],
                        "On Motion",
                        ActionTargetBase {
                            entry_index,
                            trigger: ActionTrigger::Motion,
                        },
                        &mut self.config.entries[entry_index].on_motion,
                        &gui_action_templates(),
                        Action::LockScreen,
                        &mut ActionSectionState {
                            rest_editor: &mut self.rest_editor,
                            key_capture: &mut self.key_capture,
                            dirty: &mut self.dirty,
                        },
                    );
                    render_action_section(
                        &mut columns[1],
                        "On Absence",
                        ActionTargetBase {
                            entry_index,
                            trigger: ActionTrigger::Absence,
                        },
                        &mut self.config.entries[entry_index].on_absence,
                        &gui_action_templates(),
                        Action::LockScreen,
                        &mut ActionSectionState {
                            rest_editor: &mut self.rest_editor,
                            key_capture: &mut self.key_capture,
                            dirty: &mut self.dirty,
                        },
                    );
                });
            });

        if refresh_sensor_list {
            self.refresh_sensors(entry_index);
        }

        remove
    }

    fn render_rest_editor(&mut self, ctx: &egui::Context) {
        let Some(editor) = self.rest_editor.as_mut() else {
            return;
        };

        let mut close = false;
        let mut save = false;
        egui::Window::new("REST API Call")
            .collapsible(false)
            .resizable(true)
            .show(ctx, |ui| {
                ui.label("Method");
                ui.text_edit_singleline(&mut editor.method);
                ui.label("URL");
                ui.text_edit_singleline(&mut editor.url);
                ui.label("Body");
                ui.add(
                    egui::TextEdit::multiline(&mut editor.body)
                        .desired_rows(8)
                        .code_editor(),
                );
                ui.horizontal(|ui| {
                    if ui.button("Save").clicked() {
                        save = true;
                    }
                    if ui.button("Cancel").clicked() {
                        close = true;
                    }
                });
            });

        if save {
            let editor = self.rest_editor.take().unwrap();
            if let Some(action) = self.action_mut(editor.target) {
                *action = Action::RestApiCall {
                    method: editor.method,
                    url: editor.url,
                    body: editor.body,
                };
                self.dirty = true;
            }
        } else if close {
            self.rest_editor = None;
        }
    }

    fn render_key_capture(&mut self, ctx: &egui::Context) {
        let Some(target) = self.key_capture else {
            return;
        };

        let captured = ctx.input(|input| {
            input.events.iter().find_map(|event| match event {
                egui::Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } => Some(format_key(*key, *modifiers)),
                _ => None,
            })
        });

        let mut cancel = false;
        egui::Window::new("Capture Key Press")
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.label("Press the key or key combination to store for this action.");
                if ui.button("Cancel").clicked() {
                    cancel = true;
                }
            });

        if let Some(keystroke) = captured {
            if let Some(action) = self.action_mut(target) {
                *action = Action::KeyPress { keystroke };
                self.dirty = true;
            }
            self.key_capture = None;
        } else if cancel {
            self.key_capture = None;
        }
    }
}

fn render_action_section(
    ui: &mut egui::Ui,
    title: &str,
    target_base: ActionTargetBase,
    actions: &mut Vec<Action>,
    templates: &[(&'static str, Action)],
    default_action: Action,
    state: &mut ActionSectionState<'_>,
) {
    ui.heading(title);

    let mut remove_action = None;
    for (action_index, action) in actions.iter_mut().enumerate() {
        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt(format!(
                "action-kind-{entry_index}-{}-{action_index}",
                target_base.trigger.label(),
                entry_index = target_base.entry_index
            ))
            .selected_text(action.kind_label())
            .show_ui(ui, |ui| {
                for (label, replacement) in templates {
                    if ui
                        .selectable_label(action.kind_label() == *label, *label)
                        .clicked()
                    {
                        *action = replacement.clone();
                        *state.dirty = true;
                    }
                }
            });

            match action {
                Action::KeyPress { keystroke } => {
                    let text = if keystroke.is_empty() {
                        "Capture".to_string()
                    } else {
                        keystroke.clone()
                    };
                    if ui.button(text).clicked() {
                        *state.key_capture = Some(ActionTarget {
                            entry_index: target_base.entry_index,
                            trigger: target_base.trigger,
                            action_index,
                        });
                    }
                }
                Action::RestApiCall { method, url, .. } => {
                    let text = if url.is_empty() {
                        "Edit".to_string()
                    } else {
                        format!("{method} {url}")
                    };
                    if ui.button(text).clicked() {
                        if let Action::RestApiCall { method, url, body } = action.clone() {
                            *state.rest_editor = Some(RestEditor {
                                target: ActionTarget {
                                    entry_index: target_base.entry_index,
                                    trigger: target_base.trigger,
                                    action_index,
                                },
                                method,
                                url,
                                body,
                            });
                        }
                    }
                }
                _ => {
                    ui.label(action.label());
                }
            }

            if ui.button("Remove").clicked() {
                remove_action = Some(action_index);
            }
        });
    }

    if let Some(index) = remove_action {
        actions.remove(index);
        *state.dirty = true;
    }

    if ui.button("Add Action").clicked() {
        actions.push(default_action);
        *state.dirty = true;
    }
}

fn gui_action_templates() -> Vec<(&'static str, Action)> {
    vec![
        ("Lock Screen", Action::LockScreen),
        ("Unlock Screen", Action::UnlockScreen),
        ("Power Off Display", Action::PowerOffDisplay),
        ("Power On Display", Action::PowerOnDisplay),
        ("Shut Down System", Action::ShutDownSystem),
        (
            "Key Press",
            Action::KeyPress {
                keystroke: String::new(),
            },
        ),
        (
            "REST API Call",
            Action::RestApiCall {
                method: "POST".to_string(),
                url: String::new(),
                body: String::new(),
            },
        ),
    ]
}

fn connection_action_templates() -> Vec<(&'static str, Action)> {
    vec![
        ("Disable Timed Screen Lock", Action::DisableTimedScreenLock),
        ("Enable Timed Screen Lock", Action::EnableTimedScreenLock),
        ("Disable Timed Sleep", Action::DisableTimedSleep),
        ("Enable Timed Sleep", Action::EnableTimedSleep),
    ]
}

fn token_option(token: &str) -> Option<&str> {
    let token = token.trim();
    if token.is_empty() {
        None
    } else {
        Some(token)
    }
}

fn format_key(key: egui::Key, modifiers: egui::Modifiers) -> String {
    let mut parts = Vec::new();
    if modifiers.ctrl {
        parts.push("CTRL".to_string());
    }
    if modifiers.alt {
        parts.push("ALT".to_string());
    }
    if modifiers.shift {
        parts.push("SHIFT".to_string());
    }
    if modifiers.command {
        parts.push("CMD".to_string());
    }
    parts.push(key_name(key));
    parts.join("+")
}

fn key_name(key: egui::Key) -> String {
    match key {
        egui::Key::ArrowDown => "DOWN".to_string(),
        egui::Key::ArrowLeft => "LEFT".to_string(),
        egui::Key::ArrowRight => "RIGHT".to_string(),
        egui::Key::ArrowUp => "UP".to_string(),
        egui::Key::Escape => "ESCAPE".to_string(),
        egui::Key::Tab => "TAB".to_string(),
        egui::Key::Backspace => "BACKSPACE".to_string(),
        egui::Key::Enter => "ENTER".to_string(),
        egui::Key::Space => "SPACE".to_string(),
        _ => format!("{key:?}").to_uppercase(),
    }
}
