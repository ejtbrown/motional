use std::collections::HashMap;
use std::process::Command;
use std::sync::{Arc, Mutex};

use anyhow::{anyhow, bail, Context, Result};
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Action {
    LockScreen,
    UnlockScreen,
    PowerOffDisplay,
    PowerOnDisplay,
    ShutDownSystem,
    KeyPress {
        keystroke: String,
    },
    RestApiCall {
        method: String,
        url: String,
        body: String,
    },
    LogoutLocalTerminalUsers,
    DisableTimedScreenLock,
    EnableTimedScreenLock,
    DisableTimedSleep,
    EnableTimedSleep,
}

#[derive(Debug, Clone)]
pub struct ActionResult {
    pub label: String,
    pub ok: bool,
    pub message: String,
}

#[derive(Debug, Default)]
pub struct ActionSession {
    original_settings: Mutex<OriginalSettings>,
}

#[derive(Debug, Default)]
struct OriginalSettings {
    order: Vec<String>,
    values: HashMap<String, OriginalSetting>,
}

#[derive(Debug, Clone)]
struct OriginalSetting {
    label: String,
    restore_commands: Vec<CommandSpec>,
}

impl ActionSession {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn restore_original_settings(&self) -> Vec<ActionResult> {
        let originals = {
            let mut settings = self.original_settings.lock().unwrap();
            let order = std::mem::take(&mut settings.order);
            let values = std::mem::take(&mut settings.values);
            order
                .into_iter()
                .rev()
                .filter_map(|key| values.get(&key).cloned())
                .collect::<Vec<_>>()
        };

        originals
            .into_iter()
            .map(|setting| {
                let result = run_sequence(&setting.restore_commands);
                match result {
                    Ok(()) => ActionResult {
                        label: setting.label,
                        ok: true,
                        message: "restored original setting".to_string(),
                    },
                    Err(error) => ActionResult {
                        label: setting.label,
                        ok: false,
                        message: format!("{error:#}"),
                    },
                }
            })
            .collect()
    }

    fn with_original_settings<T>(
        &self,
        f: impl FnOnce(&mut OriginalSettings) -> Result<T>,
    ) -> Result<T> {
        let mut settings = self.original_settings.lock().unwrap();
        f(&mut settings)
    }
}

impl OriginalSettings {
    fn remember(
        &mut self,
        key: String,
        label: String,
        capture: impl FnOnce() -> Result<Vec<CommandSpec>>,
    ) -> Result<()> {
        if self.values.contains_key(&key) {
            return Ok(());
        }

        let restore_commands = capture()?;
        self.order.push(key.clone());
        self.values.insert(
            key,
            OriginalSetting {
                label,
                restore_commands,
            },
        );
        Ok(())
    }

    #[cfg(any(target_os = "macos", target_os = "windows"))]
    fn forget(&mut self, key: &str) {
        self.values.remove(key);
        self.order.retain(|stored_key| stored_key != key);
    }
}

pub fn install_ctrlc_restore_handler(session: Arc<ActionSession>) -> Result<()> {
    ctrlc::set_handler(move || {
        log_restore_results(&session.restore_original_settings());
        std::process::exit(130);
    })
    .context("failed to install Ctrl-C restore handler")
}

pub fn log_restore_results(results: &[ActionResult]) {
    for result in results {
        let status = if result.ok {
            "restored"
        } else {
            "restore failed"
        };
        eprintln!("motional: {status}: {}: {}", result.label, result.message);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionTrigger {
    Connected,
    Disconnected,
    Motion,
    Absence,
}

impl ActionTrigger {
    pub fn label(self) -> &'static str {
        match self {
            Self::Connected => "connected",
            Self::Disconnected => "disconnected",
            Self::Motion => "motion",
            Self::Absence => "absence",
        }
    }
}

impl Action {
    pub fn label(&self) -> String {
        match self {
            Self::LockScreen => "Lock Screen".to_string(),
            Self::UnlockScreen => "Unlock Screen".to_string(),
            Self::PowerOffDisplay => "Power Off Display".to_string(),
            Self::PowerOnDisplay => "Power On Display".to_string(),
            Self::ShutDownSystem => "Shut Down System".to_string(),
            Self::KeyPress { keystroke } => {
                if keystroke.is_empty() {
                    "Key Press".to_string()
                } else {
                    format!("Key Press ({keystroke})")
                }
            }
            Self::RestApiCall { method, url, .. } => {
                if url.is_empty() {
                    "REST API Call".to_string()
                } else {
                    format!("REST API Call ({method} {url})")
                }
            }
            Self::LogoutLocalTerminalUsers => "Logout Local Terminal Users".to_string(),
            Self::DisableTimedScreenLock => "Disable Timed Screen Lock".to_string(),
            Self::EnableTimedScreenLock => "Enable Timed Screen Lock".to_string(),
            Self::DisableTimedSleep => "Disable Timed Sleep".to_string(),
            Self::EnableTimedSleep => "Enable Timed Sleep".to_string(),
        }
    }

    pub fn kind_label(&self) -> &'static str {
        match self {
            Self::LockScreen => "Lock Screen",
            Self::UnlockScreen => "Unlock Screen",
            Self::PowerOffDisplay => "Power Off Display",
            Self::PowerOnDisplay => "Power On Display",
            Self::ShutDownSystem => "Shut Down System",
            Self::KeyPress { .. } => "Key Press",
            Self::RestApiCall { .. } => "REST API Call",
            Self::LogoutLocalTerminalUsers => "Logout Local Terminal Users",
            Self::DisableTimedScreenLock => "Disable Timed Screen Lock",
            Self::EnableTimedScreenLock => "Enable Timed Screen Lock",
            Self::DisableTimedSleep => "Disable Timed Sleep",
            Self::EnableTimedSleep => "Enable Timed Sleep",
        }
    }
}

pub fn execute_actions(actions: &[Action], dry_run: bool) -> Vec<ActionResult> {
    let session = ActionSession::new();
    execute_actions_with_session(actions, dry_run, &session)
}

pub fn execute_actions_with_session(
    actions: &[Action],
    dry_run: bool,
    session: &ActionSession,
) -> Vec<ActionResult> {
    actions
        .iter()
        .map(|action| {
            let label = action.label();
            match execute_action_with_session(action, dry_run, session) {
                Ok(message) => ActionResult {
                    label,
                    ok: true,
                    message,
                },
                Err(error) => ActionResult {
                    label,
                    ok: false,
                    message: format!("{error:#}"),
                },
            }
        })
        .collect()
}

pub fn execute_action(action: &Action, dry_run: bool) -> Result<String> {
    let session = ActionSession::new();
    execute_action_with_session(action, dry_run, &session)
}

pub fn execute_action_with_session(
    action: &Action,
    dry_run: bool,
    session: &ActionSession,
) -> Result<String> {
    if dry_run {
        return Ok("dry run".to_string());
    }

    match action {
        Action::LockScreen => {
            lock_screen()?;
            Ok("screen lock requested".to_string())
        }
        Action::UnlockScreen => {
            unlock_screen()?;
            Ok("screen unlock requested".to_string())
        }
        Action::PowerOffDisplay => {
            power_off_display()?;
            Ok("display power-off requested".to_string())
        }
        Action::PowerOnDisplay => {
            power_on_display()?;
            Ok("display power-on requested".to_string())
        }
        Action::ShutDownSystem => {
            shut_down_system()?;
            Ok("system shutdown requested".to_string())
        }
        Action::KeyPress { keystroke } => {
            press_key(keystroke)?;
            Ok(format!("key press requested: {keystroke}"))
        }
        Action::RestApiCall { method, url, body } => {
            rest_api_call(method, url, body)?;
            Ok(format!("REST API call completed: {method} {url}"))
        }
        Action::LogoutLocalTerminalUsers => {
            logout_local_terminal_users()?;
            Ok("local terminal users logout requested".to_string())
        }
        Action::DisableTimedScreenLock => {
            disable_timed_screen_lock(session)?;
            Ok("timed screen lock disabled".to_string())
        }
        Action::EnableTimedScreenLock => {
            enable_timed_screen_lock(session)?;
            Ok("timed screen lock enabled".to_string())
        }
        Action::DisableTimedSleep => {
            disable_timed_sleep(session)?;
            Ok("timed sleep disabled".to_string())
        }
        Action::EnableTimedSleep => {
            enable_timed_sleep(session)?;
            Ok("timed sleep enabled".to_string())
        }
    }
}

pub fn parse_cli_action(spec: &str) -> Result<Action> {
    match spec {
        "logout-local-terminal-users" => Ok(Action::LogoutLocalTerminalUsers),
        "power-off-display" => Ok(Action::PowerOffDisplay),
        "power-on-display" => Ok(Action::PowerOnDisplay),
        "shut-down-system" => Ok(Action::ShutDownSystem),
        "disable-timed-screen-lock" => Ok(Action::DisableTimedScreenLock),
        "enable-timed-screen-lock" => Ok(Action::EnableTimedScreenLock),
        "disable-timed-sleep" => Ok(Action::DisableTimedSleep),
        "enable-timed-sleep" => Ok(Action::EnableTimedSleep),
        _ if spec.starts_with("rest|") => parse_rest_action(spec),
        _ => bail!(
            "unknown action {spec}; use logout-local-terminal-users, power-off-display, power-on-display, shut-down-system, disable-timed-screen-lock, enable-timed-screen-lock, disable-timed-sleep, enable-timed-sleep, or rest|METHOD|URL|BODY"
        ),
    }
}

fn parse_rest_action(spec: &str) -> Result<Action> {
    let mut parts = spec.splitn(4, '|');
    let _prefix = parts.next();
    let method = parts
        .next()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| anyhow!("REST action missing method"))?;
    let url = parts
        .next()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| anyhow!("REST action missing URL"))?;
    let body = parts.next().unwrap_or_default();
    let body = if let Some(path) = body.strip_prefix('@') {
        std::fs::read_to_string(path)
            .with_context(|| format!("failed to read REST body file {path}"))?
    } else {
        body.to_string()
    };

    Ok(Action::RestApiCall {
        method: method.to_string(),
        url: url.to_string(),
        body,
    })
}

fn rest_api_call(method: &str, url: &str, body: &str) -> Result<()> {
    if method.trim().is_empty() {
        bail!("REST API method is required");
    }
    if url.trim().is_empty() {
        bail!("REST API URL is required");
    }

    let method = method
        .parse()
        .with_context(|| format!("invalid REST API method {method}"))?;
    let client = Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .context("failed to build REST API client")?;
    let mut request = client.request(method, url);
    if !body.is_empty() {
        request = request.body(body.to_string());
    }

    let response = request.send().context("REST API request failed")?;
    if response.status().is_success() {
        Ok(())
    } else {
        let status = response.status();
        bail!("{}", rest_api_error_message(status))
    }
}

fn rest_api_error_message(status: reqwest::StatusCode) -> String {
    format!("REST API returned {status}")
}

#[cfg(target_os = "linux")]
fn lock_screen() -> Result<()> {
    run_candidates(&[
        command("loginctl", &["lock-session"]),
        command("xdg-screensaver", &["lock"]),
        command("gnome-screensaver-command", &["-l"]),
        command(
            "qdbus",
            &["org.freedesktop.ScreenSaver", "/ScreenSaver", "Lock"],
        ),
        command(
            "dbus-send",
            &[
                "--session",
                "--dest=org.freedesktop.ScreenSaver",
                "--type=method_call",
                "/ScreenSaver",
                "org.freedesktop.ScreenSaver.Lock",
            ],
        ),
    ])
}

#[cfg(target_os = "macos")]
fn lock_screen() -> Result<()> {
    run_candidates(&[
        command(
            "/System/Library/CoreServices/Menu Extras/User.menu/Contents/Resources/CGSession",
            &["-suspend"],
        ),
        command("/usr/bin/pmset", &["displaysleepnow"]),
    ])
}

#[cfg(target_os = "windows")]
fn lock_screen() -> Result<()> {
    run_command("rundll32.exe", &["user32.dll,LockWorkStation"])
}

#[cfg(target_os = "linux")]
fn unlock_screen() -> Result<()> {
    run_command("loginctl", &["unlock-session"])
}

#[cfg(target_os = "macos")]
fn unlock_screen() -> Result<()> {
    bail!("macOS does not provide a general safe screen-unlock command")
}

#[cfg(target_os = "windows")]
fn unlock_screen() -> Result<()> {
    bail!("Windows does not provide a general safe screen-unlock command")
}

#[cfg(target_os = "linux")]
fn power_off_display() -> Result<()> {
    run_candidates(&[
        command("xset", &["dpms", "force", "off"]),
        shell("dbus-send --session --dest=org.gnome.ScreenSaver --type=method_call /org/gnome/ScreenSaver org.gnome.ScreenSaver.SetActive boolean:true"),
    ])
}

#[cfg(target_os = "macos")]
fn power_off_display() -> Result<()> {
    run_command("/usr/bin/pmset", &["displaysleepnow"])
}

#[cfg(target_os = "windows")]
fn power_off_display() -> Result<()> {
    run_powershell(
        r#"Add-Type -TypeDefinition 'using System; using System.Runtime.InteropServices; public class Native { [DllImport("user32.dll")] public static extern IntPtr SendMessage(IntPtr hWnd, int Msg, IntPtr wParam, IntPtr lParam); }'; [Native]::SendMessage([intptr]0xffff, 0x0112, [intptr]0xF170, [intptr]2)"#,
    )
}

#[cfg(target_os = "linux")]
fn power_on_display() -> Result<()> {
    run_candidates(&[
        command("xset", &["dpms", "force", "on"]),
        command("ydotool", &["mousemove", "--", "0", "0"]),
    ])
}

#[cfg(target_os = "macos")]
fn power_on_display() -> Result<()> {
    run_command("/usr/bin/caffeinate", &["-u", "-t", "2"])
}

#[cfg(target_os = "windows")]
fn power_on_display() -> Result<()> {
    run_powershell(
        r#"Add-Type -TypeDefinition 'using System; using System.Runtime.InteropServices; public class Native { [DllImport("user32.dll")] public static extern IntPtr SendMessage(IntPtr hWnd, int Msg, IntPtr wParam, IntPtr lParam); }'; [Native]::SendMessage([intptr]0xffff, 0x0112, [intptr]0xF170, [intptr]-1)"#,
    )
}

#[cfg(target_os = "linux")]
fn shut_down_system() -> Result<()> {
    run_command("systemctl", &["poweroff"])
}

#[cfg(target_os = "macos")]
fn shut_down_system() -> Result<()> {
    run_command(
        "/usr/bin/osascript",
        &["-e", r#"tell application "System Events" to shut down"#],
    )
}

#[cfg(target_os = "windows")]
fn shut_down_system() -> Result<()> {
    run_command("shutdown.exe", &["/s", "/t", "0"])
}

#[cfg(target_os = "linux")]
fn logout_local_terminal_users() -> Result<()> {
    run_shell(r#"who | awk '$2 ~ /^(tty|pts\/)/ {print $2}' | xargs -r -I{} pkill -KILL -t {}"#)
}

#[cfg(not(target_os = "linux"))]
fn logout_local_terminal_users() -> Result<()> {
    bail!("Logout Local Terminal Users is only implemented for Linux")
}

#[cfg(target_os = "linux")]
fn disable_timed_screen_lock(session: &ActionSession) -> Result<()> {
    set_linux_screen_lock(session, "false")
}

#[cfg(target_os = "linux")]
fn enable_timed_screen_lock(session: &ActionSession) -> Result<()> {
    set_linux_screen_lock(session, "true")
}

#[cfg(target_os = "linux")]
fn disable_timed_sleep(session: &ActionSession) -> Result<()> {
    set_linux_sleep(session, "nothing")
}

#[cfg(target_os = "linux")]
fn enable_timed_sleep(session: &ActionSession) -> Result<()> {
    set_linux_sleep(session, "suspend")
}

#[cfg(target_os = "linux")]
fn set_linux_screen_lock(session: &ActionSession, value: &str) -> Result<()> {
    let mut errors = Vec::new();

    match set_gsettings_tracked(
        session,
        "org.gnome.desktop.screensaver",
        "lock-enabled",
        value,
    ) {
        Ok(()) => return Ok(()),
        Err(error) => errors.push(format!("{error:#}")),
    }
    match set_kde_config_tracked(
        session,
        "kreadconfig6",
        "kwriteconfig6",
        "kscreenlockerrc",
        "Daemon",
        "Autolock",
        value,
    ) {
        Ok(()) => return Ok(()),
        Err(error) => errors.push(format!("{error:#}")),
    }
    match set_kde_config_tracked(
        session,
        "kreadconfig5",
        "kwriteconfig5",
        "kscreenlockerrc",
        "Daemon",
        "Autolock",
        value,
    ) {
        Ok(()) => return Ok(()),
        Err(error) => errors.push(format!("{error:#}")),
    }

    bail!("no action command succeeded: {}", errors.join("; "))
}

#[cfg(target_os = "linux")]
fn set_linux_sleep(session: &ActionSession, value: &str) -> Result<()> {
    session.with_original_settings(|settings| {
        set_gsettings_tracked_locked(
            settings,
            "org.gnome.settings-daemon.plugins.power",
            "sleep-inactive-ac-type",
            value,
        )?;
        set_gsettings_tracked_locked(
            settings,
            "org.gnome.settings-daemon.plugins.power",
            "sleep-inactive-battery-type",
            value,
        )
    })
}

#[cfg(target_os = "linux")]
fn set_gsettings_tracked(
    session: &ActionSession,
    schema: &str,
    key: &str,
    value: &str,
) -> Result<()> {
    session.with_original_settings(|settings| {
        set_gsettings_tracked_locked(settings, schema, key, value)
    })
}

#[cfg(target_os = "linux")]
fn set_gsettings_tracked_locked(
    settings: &mut OriginalSettings,
    schema: &str,
    key: &str,
    value: &str,
) -> Result<()> {
    let setting_key = format!("linux:gsettings:{schema}:{key}");
    let label = format!("gsettings {schema} {key}");
    settings.remember(setting_key, label, || {
        let original = run_command_output("gsettings", &["get", schema, key])?;
        Ok(vec![command(
            "gsettings",
            &["set", schema, key, original.trim()],
        )])
    })?;
    run_command("gsettings", &["set", schema, key, value])
}

#[cfg(target_os = "linux")]
fn set_kde_config_tracked(
    session: &ActionSession,
    read_program: &str,
    write_program: &str,
    file: &str,
    group: &str,
    key: &str,
    value: &str,
) -> Result<()> {
    session.with_original_settings(|settings| {
        let setting_key = format!("linux:kde:{write_program}:{file}:{group}:{key}");
        let label = format!("{file} {group}.{key}");
        settings.remember(setting_key, label, || {
            let sentinel = "__MOTIONAL_UNSET__";
            let original = run_command_output(
                read_program,
                &[
                    "--file",
                    file,
                    "--group",
                    group,
                    "--key",
                    key,
                    "--default",
                    sentinel,
                ],
            )?;
            let original = original.trim();
            if original == sentinel {
                Ok(vec![command(
                    write_program,
                    &["--file", file, "--group", group, "--key", key, "--delete"],
                )])
            } else {
                Ok(vec![command(
                    write_program,
                    &["--file", file, "--group", group, "--key", key, original],
                )])
            }
        })?;
        run_command(
            write_program,
            &["--file", file, "--group", group, "--key", key, value],
        )
    })
}

#[cfg(target_os = "macos")]
fn disable_timed_screen_lock(session: &ActionSession) -> Result<()> {
    set_macos_screen_lock(session, "0", "0")
}

#[cfg(target_os = "macos")]
fn enable_timed_screen_lock(session: &ActionSession) -> Result<()> {
    set_macos_screen_lock(session, "300", "1")
}

#[cfg(target_os = "macos")]
fn disable_timed_sleep(session: &ActionSession) -> Result<()> {
    set_macos_pmset_sleep(session, "0")
}

#[cfg(target_os = "macos")]
fn enable_timed_sleep(session: &ActionSession) -> Result<()> {
    set_macos_pmset_sleep(session, "30")
}

#[cfg(target_os = "macos")]
fn set_macos_screen_lock(
    session: &ActionSession,
    idle_time: &str,
    ask_for_password: &str,
) -> Result<()> {
    session.with_original_settings(|settings| {
        set_macos_defaults_int_tracked(
            settings,
            true,
            "com.apple.screensaver",
            "idleTime",
            idle_time,
        )?;
        set_macos_defaults_int_tracked(
            settings,
            false,
            "com.apple.screensaver",
            "askForPassword",
            ask_for_password,
        )
    })
}

#[cfg(target_os = "macos")]
fn set_macos_defaults_int_tracked(
    settings: &mut OriginalSettings,
    current_host: bool,
    domain: &str,
    key: &str,
    value: &str,
) -> Result<()> {
    let scope = if current_host {
        "current-host"
    } else {
        "global"
    };
    let setting_key = format!("macos:defaults:{scope}:{domain}:{key}");
    let label = format!("defaults {scope} {domain} {key}");
    settings.remember(setting_key, label, || {
        let read_args = if current_host {
            vec![
                "-currentHost".to_string(),
                "read".to_string(),
                domain.to_string(),
                key.to_string(),
            ]
        } else {
            vec!["read".to_string(), domain.to_string(), key.to_string()]
        };

        match run_command_output_owned("/usr/bin/defaults", &read_args) {
            Ok(original) => Ok(vec![macos_defaults_write_command(
                current_host,
                domain,
                key,
                original.trim(),
            )]),
            Err(_) => Ok(vec![macos_defaults_delete_command(
                current_host,
                domain,
                key,
            )]),
        }
    })?;
    run_command_spec(&macos_defaults_write_command(
        current_host,
        domain,
        key,
        value,
    ))
}

#[cfg(target_os = "macos")]
fn macos_defaults_write_command(
    current_host: bool,
    domain: &str,
    key: &str,
    value: &str,
) -> CommandSpec {
    let mut args = Vec::new();
    if current_host {
        args.push("-currentHost".to_string());
    }
    args.extend([
        "write".to_string(),
        domain.to_string(),
        key.to_string(),
        "-int".to_string(),
        value.to_string(),
    ]);
    program_command("/usr/bin/defaults", args)
}

#[cfg(target_os = "macos")]
fn macos_defaults_delete_command(current_host: bool, domain: &str, key: &str) -> CommandSpec {
    let mut args = Vec::new();
    if current_host {
        args.push("-currentHost".to_string());
    }
    args.extend(["delete".to_string(), domain.to_string(), key.to_string()]);
    program_command("/usr/bin/defaults", args)
}

#[cfg(target_os = "macos")]
fn set_macos_pmset_sleep(session: &ActionSession, value: &str) -> Result<()> {
    session.with_original_settings(|settings| {
        let setting_key = "macos:pmset:sleep";
        settings.remember(setting_key.to_string(), "pmset sleep".to_string(), || {
            let output = run_command_output("/usr/bin/pmset", &["-g", "custom"])
                .or_else(|_| run_command_output("/usr/bin/pmset", &["-g"]))?;
            macos_pmset_sleep_restore_commands(&output)
        })?;
        match run_command("/usr/bin/pmset", &["-a", "sleep", value]) {
            Ok(()) => Ok(()),
            Err(error) => {
                settings.forget(setting_key);
                Err(error)
            }
        }
    })
}

#[cfg(target_os = "macos")]
fn macos_pmset_sleep_restore_commands(output: &str) -> Result<Vec<CommandSpec>> {
    let mut current_profile = None;
    let mut commands = Vec::new();

    for line in output.lines() {
        let trimmed = line.trim();
        current_profile = match trimmed {
            "Battery Power:" => Some("-b"),
            "AC Power:" => Some("-c"),
            "UPS Power:" => Some("-u"),
            _ => current_profile,
        };

        let parts = trimmed.split_whitespace().collect::<Vec<_>>();
        if parts.first() == Some(&"sleep") {
            let value = parts
                .get(1)
                .ok_or_else(|| anyhow!("pmset sleep line missing value"))?;
            commands.push(command(
                "/usr/bin/pmset",
                &[current_profile.unwrap_or("-a"), "sleep", value],
            ));
        }
    }

    if commands.is_empty() {
        bail!("pmset output did not include sleep settings");
    }

    Ok(commands)
}

#[cfg(target_os = "windows")]
fn disable_timed_screen_lock(session: &ActionSession) -> Result<()> {
    set_windows_screen_lock(session, "0", "0")
}

#[cfg(target_os = "windows")]
fn enable_timed_screen_lock(session: &ActionSession) -> Result<()> {
    set_windows_screen_lock(session, "1", "1")
}

#[cfg(target_os = "windows")]
fn disable_timed_sleep(session: &ActionSession) -> Result<()> {
    set_windows_sleep(session, "0", "0")
}

#[cfg(target_os = "windows")]
fn enable_timed_sleep(session: &ActionSession) -> Result<()> {
    set_windows_sleep(session, "30", "15")
}

#[cfg(target_os = "windows")]
fn set_windows_screen_lock(
    session: &ActionSession,
    screen_save_active: &str,
    screen_saver_is_secure: &str,
) -> Result<()> {
    session.with_original_settings(|settings| {
        set_windows_registry_tracked(
            settings,
            r#"HKCU\Control Panel\Desktop"#,
            "ScreenSaveActive",
            "REG_SZ",
            screen_save_active,
        )?;
        set_windows_registry_tracked(
            settings,
            r#"HKCU\Control Panel\Desktop"#,
            "ScreenSaverIsSecure",
            "REG_SZ",
            screen_saver_is_secure,
        )
    })
}

#[cfg(target_os = "windows")]
fn set_windows_registry_tracked(
    settings: &mut OriginalSettings,
    path: &str,
    value_name: &str,
    value_type: &str,
    value: &str,
) -> Result<()> {
    let setting_key = format!("windows:registry:{path}:{value_name}");
    let label = format!("registry {path}\\{value_name}");
    settings.remember(setting_key, label, || {
        windows_registry_restore_commands(path, value_name)
    })?;
    run_command(
        "reg.exe",
        &[
            "add", path, "/v", value_name, "/t", value_type, "/d", value, "/f",
        ],
    )
}

#[cfg(target_os = "windows")]
fn windows_registry_restore_commands(path: &str, value_name: &str) -> Result<Vec<CommandSpec>> {
    match run_command_output("reg.exe", &["query", path, "/v", value_name]) {
        Ok(output) => {
            let (value_type, data) = parse_windows_registry_value(&output, value_name)
                .ok_or_else(|| anyhow!("reg query output missing {value_name}"))?;
            Ok(vec![command(
                "reg.exe",
                &[
                    "add",
                    path,
                    "/v",
                    value_name,
                    "/t",
                    &value_type,
                    "/d",
                    &data,
                    "/f",
                ],
            )])
        }
        Err(_) => Ok(vec![command(
            "reg.exe",
            &["delete", path, "/v", value_name, "/f"],
        )]),
    }
}

#[cfg(target_os = "windows")]
fn parse_windows_registry_value(output: &str, value_name: &str) -> Option<(String, String)> {
    for line in output.lines() {
        let parts = line.split_whitespace().collect::<Vec<_>>();
        if parts.len() >= 3 && parts[0].eq_ignore_ascii_case(value_name) {
            return Some((parts[1].to_string(), parts[2..].join(" ")));
        }
    }
    None
}

#[cfg(target_os = "windows")]
fn set_windows_sleep(session: &ActionSession, ac_minutes: &str, dc_minutes: &str) -> Result<()> {
    session.with_original_settings(|settings| {
        let setting_key = "windows:powercfg:standbyidle";
        settings.remember(
            setting_key.to_string(),
            "powercfg standby idle".to_string(),
            windows_powercfg_sleep_restore_commands,
        )?;
        let ac_command = command(
            "powercfg.exe",
            &["/change", "standby-timeout-ac", ac_minutes],
        );
        if let Err(error) = run_command_spec(&ac_command) {
            settings.forget(setting_key);
            return Err(error);
        }

        run_command(
            "powercfg.exe",
            &["/change", "standby-timeout-dc", dc_minutes],
        )
    })
}

#[cfg(target_os = "windows")]
fn windows_powercfg_sleep_restore_commands() -> Result<Vec<CommandSpec>> {
    let output = run_command_output(
        "powercfg.exe",
        &["/query", "SCHEME_CURRENT", "SUB_SLEEP", "STANDBYIDLE"],
    )?;
    let ac_seconds = parse_windows_powercfg_index(&output, "Current AC Power Setting Index")
        .ok_or_else(|| anyhow!("powercfg output missing AC sleep setting"))?;
    let dc_seconds = parse_windows_powercfg_index(&output, "Current DC Power Setting Index")
        .ok_or_else(|| anyhow!("powercfg output missing DC sleep setting"))?;

    Ok(vec![
        command(
            "powercfg.exe",
            &[
                "/setacvalueindex",
                "SCHEME_CURRENT",
                "SUB_SLEEP",
                "STANDBYIDLE",
                &ac_seconds.to_string(),
            ],
        ),
        command(
            "powercfg.exe",
            &[
                "/setdcvalueindex",
                "SCHEME_CURRENT",
                "SUB_SLEEP",
                "STANDBYIDLE",
                &dc_seconds.to_string(),
            ],
        ),
        command("powercfg.exe", &["/setactive", "SCHEME_CURRENT"]),
    ])
}

#[cfg(target_os = "windows")]
fn parse_windows_powercfg_index(output: &str, label: &str) -> Option<u64> {
    output.lines().find_map(|line| {
        let trimmed = line.trim();
        let (_, value) = trimmed.split_once(':')?;
        if !trimmed.starts_with(label) {
            return None;
        }

        let value = value.trim();
        if let Some(hex) = value
            .strip_prefix("0x")
            .or_else(|| value.strip_prefix("0X"))
        {
            u64::from_str_radix(hex, 16).ok()
        } else {
            value.parse().ok()
        }
    })
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn disable_timed_screen_lock(_session: &ActionSession) -> Result<()> {
    bail!("Disable Timed Screen Lock is not implemented for this platform")
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn enable_timed_screen_lock(_session: &ActionSession) -> Result<()> {
    bail!("Enable Timed Screen Lock is not implemented for this platform")
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn disable_timed_sleep(_session: &ActionSession) -> Result<()> {
    bail!("Disable Timed Sleep is not implemented for this platform")
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn enable_timed_sleep(_session: &ActionSession) -> Result<()> {
    bail!("Enable Timed Sleep is not implemented for this platform")
}

#[cfg(target_os = "linux")]
fn press_key(keystroke: &str) -> Result<()> {
    if keystroke.trim().is_empty() {
        bail!("key press action has no captured key");
    }
    run_command("xdotool", &["key", keystroke])
}

#[cfg(target_os = "macos")]
fn press_key(keystroke: &str) -> Result<()> {
    if keystroke.trim().is_empty() {
        bail!("key press action has no captured key");
    }

    let script = macos_key_script(keystroke)?;
    run_command("/usr/bin/osascript", &["-e", &script])
}

#[cfg(target_os = "windows")]
fn press_key(keystroke: &str) -> Result<()> {
    if keystroke.trim().is_empty() {
        bail!("key press action has no captured key");
    }

    let send_keys = windows_send_keys(keystroke);
    run_powershell(&format!(
        "$wshell = New-Object -ComObject WScript.Shell; $wshell.SendKeys('{}')",
        send_keys.replace('\'', "''")
    ))
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn lock_screen() -> Result<()> {
    bail!("Lock Screen is not implemented for this platform")
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn unlock_screen() -> Result<()> {
    bail!("Unlock Screen is not implemented for this platform")
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn power_off_display() -> Result<()> {
    bail!("Power Off Display is not implemented for this platform")
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn power_on_display() -> Result<()> {
    bail!("Power On Display is not implemented for this platform")
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn shut_down_system() -> Result<()> {
    bail!("Shut Down System is not implemented for this platform")
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn press_key(_keystroke: &str) -> Result<()> {
    bail!("Key Press is not implemented for this platform")
}

#[derive(Debug, Clone)]
enum CommandSpec {
    Program {
        program: String,
        args: Vec<String>,
    },
    #[cfg(target_os = "linux")]
    Shell(String),
}

fn command(program: &str, args: &[&str]) -> CommandSpec {
    CommandSpec::Program {
        program: program.to_string(),
        args: args.iter().map(|arg| arg.to_string()).collect(),
    }
}

#[cfg(target_os = "macos")]
fn program_command(program: &str, args: Vec<String>) -> CommandSpec {
    CommandSpec::Program {
        program: program.to_string(),
        args,
    }
}

#[cfg(target_os = "linux")]
fn shell(command: &str) -> CommandSpec {
    CommandSpec::Shell(command.to_string())
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn run_candidates(candidates: &[CommandSpec]) -> Result<()> {
    let mut errors = Vec::new();
    for candidate in candidates {
        match run_command_spec(candidate) {
            Ok(()) => return Ok(()),
            Err(error) => errors.push(format!("{error:#}")),
        }
    }

    bail!("no action command succeeded: {}", errors.join("; "))
}

fn run_sequence(commands: &[CommandSpec]) -> Result<()> {
    for command in commands {
        run_command_spec(command)?;
    }
    Ok(())
}

fn run_command_spec(spec: &CommandSpec) -> Result<()> {
    match spec {
        CommandSpec::Program { program, args } => run_command(
            program,
            &args.iter().map(String::as_str).collect::<Vec<_>>(),
        ),
        #[cfg(target_os = "linux")]
        CommandSpec::Shell(command) => run_shell(command),
    }
}

fn run_command(program: &str, args: &[&str]) -> Result<()> {
    let status = Command::new(program)
        .args(args)
        .status()
        .with_context(|| format!("failed to spawn {program}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(anyhow!("{program} exited with {status}"))
    }
}

fn run_command_output(program: &str, args: &[&str]) -> Result<String> {
    let owned_args = args.iter().map(|arg| arg.to_string()).collect::<Vec<_>>();
    run_command_output_owned(program, &owned_args)
}

fn run_command_output_owned(program: &str, args: &[String]) -> Result<String> {
    let output = Command::new(program)
        .args(args)
        .output()
        .with_context(|| format!("failed to spawn {program}"))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let detail = if stderr.is_empty() {
            output.status.to_string()
        } else {
            format!("{}: {stderr}", output.status)
        };
        Err(anyhow!("{program} exited with {detail}"))
    }
}

#[cfg(target_os = "linux")]
fn run_shell(command: &str) -> Result<()> {
    let status = if cfg!(target_os = "windows") {
        Command::new("cmd").args(["/C", command]).status()
    } else {
        Command::new("sh").args(["-c", command]).status()
    }
    .with_context(|| format!("failed to spawn shell command {command}"))?;

    if status.success() {
        Ok(())
    } else {
        Err(anyhow!("shell command exited with {status}: {command}"))
    }
}

#[cfg(target_os = "windows")]
fn run_powershell(command: &str) -> Result<()> {
    run_command(
        "powershell.exe",
        &[
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            command,
        ],
    )
}

#[cfg(target_os = "macos")]
fn macos_key_script(keystroke: &str) -> Result<String> {
    let (modifiers, key) = split_keystroke(keystroke);
    let mut modifier_parts = Vec::new();
    for modifier in modifiers {
        match modifier.as_str() {
            "CTRL" | "CONTROL" => modifier_parts.push("control down"),
            "ALT" | "OPTION" => modifier_parts.push("option down"),
            "SHIFT" => modifier_parts.push("shift down"),
            "CMD" | "COMMAND" | "META" => modifier_parts.push("command down"),
            _ => {}
        }
    }
    let using = if modifier_parts.is_empty() {
        String::new()
    } else {
        format!(" using {{{}}}", modifier_parts.join(", "))
    };

    if key.chars().count() == 1 {
        Ok(format!(
            "tell application \"System Events\" to keystroke \"{}\"{}",
            key.replace('"', "\\\"").to_lowercase(),
            using
        ))
    } else if let Some(code) = macos_key_code(&key) {
        Ok(format!(
            "tell application \"System Events\" to key code {code}{using}"
        ))
    } else {
        bail!("unsupported macOS key name {key}")
    }
}

#[cfg(target_os = "macos")]
fn macos_key_code(key: &str) -> Option<u16> {
    match key {
        "RETURN" | "ENTER" => Some(36),
        "ESCAPE" => Some(53),
        "SPACE" => Some(49),
        "TAB" => Some(48),
        "BACKSPACE" => Some(51),
        "DELETE" => Some(117),
        "LEFT" => Some(123),
        "RIGHT" => Some(124),
        "DOWN" => Some(125),
        "UP" => Some(126),
        "F1" => Some(122),
        "F2" => Some(120),
        "F3" => Some(99),
        "F4" => Some(118),
        "F5" => Some(96),
        "F6" => Some(97),
        "F7" => Some(98),
        "F8" => Some(100),
        "F9" => Some(101),
        "F10" => Some(109),
        "F11" => Some(103),
        "F12" => Some(111),
        _ => None,
    }
}

#[cfg(target_os = "windows")]
fn windows_send_keys(keystroke: &str) -> String {
    let (modifiers, key) = split_keystroke(keystroke);
    let mut out = String::new();
    for modifier in modifiers {
        match modifier.as_str() {
            "CTRL" | "CONTROL" => out.push('^'),
            "ALT" | "OPTION" => out.push('%'),
            "SHIFT" => out.push('+'),
            _ => {}
        }
    }

    if key.chars().count() == 1 {
        out.push_str(&key.to_lowercase());
    } else {
        out.push('{');
        out.push_str(&key);
        out.push('}');
    }
    out
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn split_keystroke(keystroke: &str) -> (Vec<String>, String) {
    let mut parts = keystroke
        .split('+')
        .map(|part| part.trim().to_uppercase())
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    let key = parts.pop().unwrap_or_default();
    (parts, key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_rest_cli_action() {
        let action = parse_cli_action("rest|POST|http://example.local/hook|{\"ok\":true}").unwrap();
        assert_eq!(
            action,
            Action::RestApiCall {
                method: "POST".to_string(),
                url: "http://example.local/hook".to_string(),
                body: "{\"ok\":true}".to_string(),
            }
        );
    }

    #[test]
    fn rest_api_error_message_omits_response_body() {
        let message = rest_api_error_message(reqwest::StatusCode::BAD_REQUEST);
        assert_eq!(message, "REST API returned 400 Bad Request");
    }

    #[test]
    fn remembers_original_setting_once() {
        let mut settings = OriginalSettings::default();
        let mut captures = 0;

        settings
            .remember("setting".to_string(), "Setting".to_string(), || {
                captures += 1;
                Ok(vec![command("restore", &["first"])])
            })
            .unwrap();
        settings
            .remember("setting".to_string(), "Setting".to_string(), || {
                captures += 1;
                Ok(vec![command("restore", &["second"])])
            })
            .unwrap();

        assert_eq!(captures, 1);
        assert_eq!(settings.order, vec!["setting"]);
        assert_eq!(settings.values["setting"].restore_commands.len(), 1);
    }

    #[test]
    fn restore_without_captured_settings_is_empty() {
        let session = ActionSession::new();
        assert!(session.restore_original_settings().is_empty());
    }

    #[test]
    #[ignore = "modifies timed lock and sleep OS settings before restoring them"]
    fn real_os_config_actions_restore_original_settings() {
        struct RestoreOnPanic<'a>(&'a ActionSession);

        impl Drop for RestoreOnPanic<'_> {
            fn drop(&mut self) {
                let _ = self.0.restore_original_settings();
            }
        }

        let session = ActionSession::new();
        let _restore_on_panic = RestoreOnPanic(&session);
        let mut succeeded = Vec::new();
        let mut failed = Vec::new();

        for action in [
            Action::DisableTimedScreenLock,
            Action::EnableTimedScreenLock,
            Action::DisableTimedSleep,
            Action::EnableTimedSleep,
        ] {
            match execute_action_with_session(&action, false, &session) {
                Ok(_) => succeeded.push(action.label()),
                Err(error) => failed.push(format!("{}: {error:#}", action.label())),
            }
        }

        let restore_results = session.restore_original_settings();
        assert!(
            restore_results.iter().all(|result| result.ok),
            "restore failed: {restore_results:?}"
        );
        assert!(
            !succeeded.is_empty(),
            "no OS config action succeeded: {}",
            failed.join("; ")
        );
        if !failed.is_empty() {
            eprintln!(
                "OS config actions that did not succeed: {}",
                failed.join("; ")
            );
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn parses_macos_pmset_sleep_profiles() {
        let output = r#"
Battery Power:
 sleep                12
AC Power:
 sleep                34
"#;

        let commands = macos_pmset_sleep_restore_commands(output).unwrap();

        assert_eq!(command_args(&commands[0]), &["-b", "sleep", "12"]);
        assert_eq!(command_args(&commands[1]), &["-c", "sleep", "34"]);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn parses_windows_registry_query_value() {
        let output = r#"
HKEY_CURRENT_USER\Control Panel\Desktop
    ScreenSaveActive    REG_SZ    1
"#;

        assert_eq!(
            parse_windows_registry_value(output, "ScreenSaveActive"),
            Some(("REG_SZ".to_string(), "1".to_string()))
        );
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn parses_windows_powercfg_hex_indexes() {
        let output = r#"
    Current AC Power Setting Index: 0x0000000000000708
    Current DC Power Setting Index: 0x0000000000000258
"#;

        assert_eq!(
            parse_windows_powercfg_index(output, "Current AC Power Setting Index"),
            Some(1800)
        );
        assert_eq!(
            parse_windows_powercfg_index(output, "Current DC Power Setting Index"),
            Some(600)
        );
    }

    #[cfg(target_os = "macos")]
    fn command_args(command: &CommandSpec) -> Vec<&str> {
        match command {
            CommandSpec::Program { args, .. } => args.iter().map(String::as_str).collect(),
            #[cfg(target_os = "linux")]
            CommandSpec::Shell(_) => unreachable!(),
        }
    }
}
