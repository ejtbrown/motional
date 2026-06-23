use std::process::Command;

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
    actions
        .iter()
        .map(|action| {
            let label = action.label();
            match execute_action(action, dry_run) {
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
            disable_timed_screen_lock()?;
            Ok("timed screen lock disabled".to_string())
        }
        Action::EnableTimedScreenLock => {
            enable_timed_screen_lock()?;
            Ok("timed screen lock enabled".to_string())
        }
        Action::DisableTimedSleep => {
            disable_timed_sleep()?;
            Ok("timed sleep disabled".to_string())
        }
        Action::EnableTimedSleep => {
            enable_timed_sleep()?;
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
fn disable_timed_screen_lock() -> Result<()> {
    run_candidates(&[
        command(
            "gsettings",
            &[
                "set",
                "org.gnome.desktop.screensaver",
                "lock-enabled",
                "false",
            ],
        ),
        command(
            "kwriteconfig6",
            &[
                "--file",
                "kscreenlockerrc",
                "--group",
                "Daemon",
                "--key",
                "Autolock",
                "false",
            ],
        ),
        command(
            "kwriteconfig5",
            &[
                "--file",
                "kscreenlockerrc",
                "--group",
                "Daemon",
                "--key",
                "Autolock",
                "false",
            ],
        ),
    ])
}

#[cfg(target_os = "linux")]
fn enable_timed_screen_lock() -> Result<()> {
    run_candidates(&[
        command(
            "gsettings",
            &[
                "set",
                "org.gnome.desktop.screensaver",
                "lock-enabled",
                "true",
            ],
        ),
        command(
            "kwriteconfig6",
            &[
                "--file",
                "kscreenlockerrc",
                "--group",
                "Daemon",
                "--key",
                "Autolock",
                "true",
            ],
        ),
        command(
            "kwriteconfig5",
            &[
                "--file",
                "kscreenlockerrc",
                "--group",
                "Daemon",
                "--key",
                "Autolock",
                "true",
            ],
        ),
    ])
}

#[cfg(target_os = "linux")]
fn disable_timed_sleep() -> Result<()> {
    run_sequence(&[
        command(
            "gsettings",
            &[
                "set",
                "org.gnome.settings-daemon.plugins.power",
                "sleep-inactive-ac-type",
                "nothing",
            ],
        ),
        command(
            "gsettings",
            &[
                "set",
                "org.gnome.settings-daemon.plugins.power",
                "sleep-inactive-battery-type",
                "nothing",
            ],
        ),
    ])
}

#[cfg(target_os = "linux")]
fn enable_timed_sleep() -> Result<()> {
    run_sequence(&[
        command(
            "gsettings",
            &[
                "set",
                "org.gnome.settings-daemon.plugins.power",
                "sleep-inactive-ac-type",
                "suspend",
            ],
        ),
        command(
            "gsettings",
            &[
                "set",
                "org.gnome.settings-daemon.plugins.power",
                "sleep-inactive-battery-type",
                "suspend",
            ],
        ),
    ])
}

#[cfg(target_os = "macos")]
fn disable_timed_screen_lock() -> Result<()> {
    run_sequence(&[
        command(
            "/usr/bin/defaults",
            &[
                "-currentHost",
                "write",
                "com.apple.screensaver",
                "idleTime",
                "-int",
                "0",
            ],
        ),
        command(
            "/usr/bin/defaults",
            &[
                "write",
                "com.apple.screensaver",
                "askForPassword",
                "-int",
                "0",
            ],
        ),
    ])
}

#[cfg(target_os = "macos")]
fn enable_timed_screen_lock() -> Result<()> {
    run_sequence(&[
        command(
            "/usr/bin/defaults",
            &[
                "-currentHost",
                "write",
                "com.apple.screensaver",
                "idleTime",
                "-int",
                "300",
            ],
        ),
        command(
            "/usr/bin/defaults",
            &[
                "write",
                "com.apple.screensaver",
                "askForPassword",
                "-int",
                "1",
            ],
        ),
    ])
}

#[cfg(target_os = "macos")]
fn disable_timed_sleep() -> Result<()> {
    run_command("/usr/bin/pmset", &["-a", "sleep", "0"])
}

#[cfg(target_os = "macos")]
fn enable_timed_sleep() -> Result<()> {
    run_command("/usr/bin/pmset", &["-a", "sleep", "30"])
}

#[cfg(target_os = "windows")]
fn disable_timed_screen_lock() -> Result<()> {
    run_sequence(&[
        command(
            "reg.exe",
            &[
                "add",
                r#"HKCU\Control Panel\Desktop"#,
                "/v",
                "ScreenSaveActive",
                "/t",
                "REG_SZ",
                "/d",
                "0",
                "/f",
            ],
        ),
        command(
            "reg.exe",
            &[
                "add",
                r#"HKCU\Control Panel\Desktop"#,
                "/v",
                "ScreenSaverIsSecure",
                "/t",
                "REG_SZ",
                "/d",
                "0",
                "/f",
            ],
        ),
    ])
}

#[cfg(target_os = "windows")]
fn enable_timed_screen_lock() -> Result<()> {
    run_sequence(&[
        command(
            "reg.exe",
            &[
                "add",
                r#"HKCU\Control Panel\Desktop"#,
                "/v",
                "ScreenSaveActive",
                "/t",
                "REG_SZ",
                "/d",
                "1",
                "/f",
            ],
        ),
        command(
            "reg.exe",
            &[
                "add",
                r#"HKCU\Control Panel\Desktop"#,
                "/v",
                "ScreenSaverIsSecure",
                "/t",
                "REG_SZ",
                "/d",
                "1",
                "/f",
            ],
        ),
    ])
}

#[cfg(target_os = "windows")]
fn disable_timed_sleep() -> Result<()> {
    run_sequence(&[
        command("powercfg.exe", &["/change", "standby-timeout-ac", "0"]),
        command("powercfg.exe", &["/change", "standby-timeout-dc", "0"]),
    ])
}

#[cfg(target_os = "windows")]
fn enable_timed_sleep() -> Result<()> {
    run_sequence(&[
        command("powercfg.exe", &["/change", "standby-timeout-ac", "30"]),
        command("powercfg.exe", &["/change", "standby-timeout-dc", "15"]),
    ])
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn disable_timed_screen_lock() -> Result<()> {
    bail!("Disable Timed Screen Lock is not implemented for this platform")
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn enable_timed_screen_lock() -> Result<()> {
    bail!("Enable Timed Screen Lock is not implemented for this platform")
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn disable_timed_sleep() -> Result<()> {
    bail!("Disable Timed Sleep is not implemented for this platform")
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn enable_timed_sleep() -> Result<()> {
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

#[derive(Clone)]
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
}
