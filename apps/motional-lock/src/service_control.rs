use std::env;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{anyhow, bail, Context, Result};

pub const SERVICE_LABEL: &str = "com.ejtbrown.motional.service";
pub const SERVICE_DISPLAY_NAME: &str = "Motional Service";
const STOP_REQUEST_FILE: &str = "motional-service.stop";

#[derive(Debug, Clone)]
pub struct ServiceStatus {
    pub installed: bool,
    pub running: bool,
    pub detail: String,
}

#[derive(Debug, Clone, Default)]
pub struct ServiceInstallOptions {
    pub service_binary: Option<PathBuf>,
    pub start: bool,
}

pub fn install_service(options: &ServiceInstallOptions) -> Result<String> {
    let service_binary = options
        .service_binary
        .clone()
        .map(Ok)
        .unwrap_or_else(find_service_binary)?;
    ensure_executable_path(&service_binary)?;
    clear_stop_request()?;

    install_platform_service(&service_binary)?;
    if options.start {
        start_service()?;
    }

    Ok(format!(
        "installed {} using {}",
        service_name(),
        service_binary.display()
    ))
}

pub fn remove_service() -> Result<String> {
    let _ = stop_service();
    remove_platform_service()?;
    Ok(format!("removed {}", service_name()))
}

pub fn start_service() -> Result<String> {
    clear_stop_request()?;
    start_platform_service()?;
    Ok(format!("started {}", service_name()))
}

pub fn stop_service() -> Result<String> {
    stop_platform_service()?;
    Ok(format!("stopped {}", service_name()))
}

pub fn restart_service() -> Result<String> {
    let _ = stop_service();
    start_service()
}

pub fn service_status() -> Result<ServiceStatus> {
    platform_service_status()
}

pub(crate) fn service_stop_requested() -> bool {
    stop_request_path().is_file()
}

pub(crate) fn clear_stop_request() -> Result<()> {
    let path = stop_request_path();
    match fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| format!("failed to remove {}", path.display())),
    }
}

pub fn find_service_binary() -> Result<PathBuf> {
    let current_exe = env::current_exe().context("failed to resolve current executable")?;
    let service_name = executable_name("motional-service");

    if current_exe
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.eq_ignore_ascii_case(&service_name))
    {
        return Ok(current_exe);
    }

    for dir in candidate_binary_dirs(&current_exe) {
        let candidate = dir.join(&service_name);
        if candidate.is_file() {
            return Ok(candidate);
        }
    }

    Err(anyhow!(
        "could not find {service_name}; place it beside motional-cli or motional-gui, or pass --service-binary"
    ))
}

fn candidate_binary_dirs(current_exe: &Path) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let mut cursor = current_exe.parent();
    for _ in 0..=4 {
        let Some(dir) = cursor else {
            break;
        };
        dirs.push(dir.to_path_buf());
        cursor = dir.parent();
    }
    dirs
}

fn executable_name(name: &str) -> String {
    if cfg!(target_os = "windows") {
        format!("{name}.exe")
    } else {
        name.to_string()
    }
}

fn ensure_executable_path(path: &Path) -> Result<()> {
    if path.is_file() {
        Ok(())
    } else {
        bail!("service binary does not exist: {}", path.display())
    }
}

fn service_name() -> &'static str {
    if cfg!(target_os = "windows") {
        SERVICE_DISPLAY_NAME
    } else {
        SERVICE_LABEL
    }
}

#[cfg(target_os = "windows")]
fn request_service_stop() -> Result<()> {
    let path = stop_request_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    fs::write(&path, b"stop\n").with_context(|| format!("failed to write {}", path.display()))
}

fn stop_request_path() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(env::temp_dir)
        .join("motional")
        .join(STOP_REQUEST_FILE)
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

fn command_output(program: &str, args: &[&str]) -> Result<String> {
    let output = Command::new(program)
        .args(args)
        .output()
        .with_context(|| format!("failed to spawn {program}"))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if stderr.is_empty() {
            Err(anyhow!("{program} exited with {}", output.status))
        } else {
            Err(anyhow!("{program} exited with {}: {stderr}", output.status))
        }
    }
}

#[cfg(target_os = "linux")]
fn install_platform_service(service_binary: &Path) -> Result<()> {
    let unit_path = linux_unit_path()?;
    if let Some(parent) = unit_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    fs::write(&unit_path, linux_unit_contents(service_binary))
        .with_context(|| format!("failed to write {}", unit_path.display()))?;
    run_command("systemctl", &["--user", "daemon-reload"])?;
    run_command("systemctl", &["--user", "enable", SERVICE_LABEL])
}

#[cfg(target_os = "linux")]
fn remove_platform_service() -> Result<()> {
    let _ = run_command("systemctl", &["--user", "disable", SERVICE_LABEL]);
    let unit_path = linux_unit_path()?;
    if unit_path.exists() {
        fs::remove_file(&unit_path)
            .with_context(|| format!("failed to remove {}", unit_path.display()))?;
    }
    run_command("systemctl", &["--user", "daemon-reload"])
}

#[cfg(target_os = "linux")]
fn start_platform_service() -> Result<()> {
    run_command("systemctl", &["--user", "start", SERVICE_LABEL])
}

#[cfg(target_os = "linux")]
fn stop_platform_service() -> Result<()> {
    run_command("systemctl", &["--user", "stop", SERVICE_LABEL])
}

#[cfg(target_os = "linux")]
fn platform_service_status() -> Result<ServiceStatus> {
    let unit_path = linux_unit_path()?;
    let installed = unit_path.exists();
    let active = Command::new("systemctl")
        .args(["--user", "is-active", "--quiet", SERVICE_LABEL])
        .status()
        .map(|status| status.success())
        .unwrap_or(false);
    let detail = command_output(
        "systemctl",
        &["--user", "status", SERVICE_LABEL, "--no-pager"],
    )
    .unwrap_or_else(|error| format!("{error:#}"));

    Ok(ServiceStatus {
        installed,
        running: active,
        detail,
    })
}

#[cfg(target_os = "linux")]
fn linux_unit_path() -> Result<PathBuf> {
    Ok(dirs::config_dir()
        .ok_or_else(|| anyhow!("could not resolve user config directory"))?
        .join("systemd")
        .join("user")
        .join(SERVICE_LABEL))
}

#[cfg(target_os = "linux")]
pub(crate) fn linux_unit_contents(service_binary: &Path) -> String {
    format!(
        "[Unit]\n\
Description=Motional automation service\n\
After=network-online.target\n\n\
[Service]\n\
ExecStart={}\n\
Restart=always\n\
RestartSec=5\n\n\
[Install]\n\
WantedBy=default.target\n",
        systemd_quote(service_binary)
    )
}

#[cfg(target_os = "linux")]
fn systemd_quote(path: &Path) -> String {
    let value = path.display().to_string();
    if value
        .chars()
        .any(|ch| ch.is_whitespace() || matches!(ch, '"' | '\'' | '\\'))
    {
        format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
    } else {
        value
    }
}

#[cfg(target_os = "macos")]
fn install_platform_service(service_binary: &Path) -> Result<()> {
    let plist_path = macos_plist_path()?;
    if let Some(parent) = plist_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    if let Some(parent) = macos_log_path("out").parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    fs::write(&plist_path, macos_plist_contents(service_binary))
        .with_context(|| format!("failed to write {}", plist_path.display()))?;
    let domain = macos_domain();
    let domain_label = macos_domain_label();
    let _ = run_command("launchctl", &["bootout", &domain_label]);
    let _ = run_command("launchctl", &["enable", &domain_label]);
    run_command(
        "launchctl",
        &["bootstrap", &domain, &plist_path.display().to_string()],
    )?;
    run_command("launchctl", &["enable", &domain_label])
}

#[cfg(target_os = "macos")]
fn remove_platform_service() -> Result<()> {
    let _ = run_command("launchctl", &["disable", &macos_domain_label()]);
    let _ = run_command("launchctl", &["bootout", &macos_domain_label()]);
    let plist_path = macos_plist_path()?;
    if plist_path.exists() {
        fs::remove_file(&plist_path)
            .with_context(|| format!("failed to remove {}", plist_path.display()))?;
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn start_platform_service() -> Result<()> {
    let plist_path = macos_plist_path()?;
    let domain = macos_domain();
    let domain_label = macos_domain_label();
    let _ = run_command("launchctl", &["enable", &domain_label]);
    if plist_path.exists() {
        let _ = run_command(
            "launchctl",
            &["bootstrap", &domain, &plist_path.display().to_string()],
        );
    }
    run_command("launchctl", &["enable", &domain_label])?;
    run_command("launchctl", &["kickstart", "-k", &domain_label])
}

#[cfg(target_os = "macos")]
fn stop_platform_service() -> Result<()> {
    let _ = run_command("launchctl", &["disable", &macos_domain_label()]);
    run_command("launchctl", &["bootout", &macos_domain_label()])
}

#[cfg(target_os = "macos")]
fn platform_service_status() -> Result<ServiceStatus> {
    let plist_path = macos_plist_path()?;
    let installed = plist_path.exists();
    let detail = command_output("launchctl", &["print", &macos_domain_label()])
        .unwrap_or_else(|error| format!("{error:#}"));
    let running = detail.contains("state = running") || detail.contains("state = spawn scheduled");

    Ok(ServiceStatus {
        installed,
        running,
        detail,
    })
}

#[cfg(target_os = "macos")]
fn macos_plist_path() -> Result<PathBuf> {
    Ok(dirs::home_dir()
        .ok_or_else(|| anyhow!("could not resolve home directory"))?
        .join("Library")
        .join("LaunchAgents")
        .join(format!("{SERVICE_LABEL}.plist")))
}

#[cfg(target_os = "macos")]
fn macos_domain() -> String {
    let uid = command_output("/usr/bin/id", &["-u"])
        .unwrap_or_else(|_| "501".to_string())
        .trim()
        .to_string();
    format!("gui/{uid}")
}

#[cfg(target_os = "macos")]
fn macos_domain_label() -> String {
    format!("{}/{}", macos_domain(), SERVICE_LABEL)
}

#[cfg(target_os = "macos")]
pub(crate) fn macos_plist_contents(service_binary: &Path) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\"\n\
  \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
<plist version=\"1.0\">\n\
<dict>\n\
  <key>Label</key>\n\
  <string>{SERVICE_LABEL}</string>\n\
  <key>ProgramArguments</key>\n\
  <array>\n\
    <string>{}</string>\n\
  </array>\n\
  <key>RunAtLoad</key>\n\
  <true/>\n\
  <key>KeepAlive</key>\n\
  <true/>\n\
  <key>StandardOutPath</key>\n\
  <string>{}</string>\n\
  <key>StandardErrorPath</key>\n\
  <string>{}</string>\n\
</dict>\n\
</plist>\n",
        xml_escape(&service_binary.display().to_string()),
        xml_escape(&macos_log_path("out").display().to_string()),
        xml_escape(&macos_log_path("err").display().to_string())
    )
}

#[cfg(target_os = "macos")]
fn macos_log_path(stream: &str) -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(env::temp_dir)
        .join("motional")
        .join(format!("motional-service.{stream}.log"))
}

#[cfg(target_os = "macos")]
fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(target_os = "windows")]
fn install_platform_service(service_binary: &Path) -> Result<()> {
    run_command(
        "schtasks.exe",
        &[
            "/Create",
            "/TN",
            SERVICE_DISPLAY_NAME,
            "/SC",
            "ONLOGON",
            "/TR",
            &windows_task_command(service_binary),
            "/RL",
            "LIMITED",
            "/F",
        ],
    )
}

#[cfg(target_os = "windows")]
fn remove_platform_service() -> Result<()> {
    let _ = stop_platform_service();
    run_command(
        "schtasks.exe",
        &["/Delete", "/TN", SERVICE_DISPLAY_NAME, "/F"],
    )
}

#[cfg(target_os = "windows")]
fn start_platform_service() -> Result<()> {
    run_command("schtasks.exe", &["/Run", "/TN", SERVICE_DISPLAY_NAME])
}

#[cfg(target_os = "windows")]
fn stop_platform_service() -> Result<()> {
    request_service_stop()?;
    if wait_for_windows_task_stop(std::time::Duration::from_secs(10)) {
        return Ok(());
    }
    run_command("schtasks.exe", &["/End", "/TN", SERVICE_DISPLAY_NAME])
}

#[cfg(target_os = "windows")]
fn platform_service_status() -> Result<ServiceStatus> {
    match windows_task_status_detail() {
        Ok(detail) => {
            let running = windows_task_status_is_running(&detail);
            Ok(ServiceStatus {
                installed: true,
                running,
                detail,
            })
        }
        Err(error) => Ok(ServiceStatus {
            installed: false,
            running: false,
            detail: format!("{error:#}"),
        }),
    }
}

#[cfg(target_os = "windows")]
fn wait_for_windows_task_stop(timeout: std::time::Duration) -> bool {
    let deadline = std::time::Instant::now() + timeout;
    while std::time::Instant::now() < deadline {
        match windows_task_status_detail() {
            Ok(detail) if !windows_task_status_is_running(&detail) => return true,
            Err(_) => return true,
            _ => std::thread::sleep(std::time::Duration::from_millis(250)),
        }
    }
    false
}

#[cfg(target_os = "windows")]
fn windows_task_status_detail() -> Result<String> {
    command_output(
        "schtasks.exe",
        &["/Query", "/TN", SERVICE_DISPLAY_NAME, "/FO", "LIST"],
    )
}

#[cfg(target_os = "windows")]
fn windows_task_status_is_running(detail: &str) -> bool {
    detail
        .lines()
        .any(|line| line.contains("Status:") && line.contains("Running"))
}

#[cfg(target_os = "windows")]
pub(crate) fn windows_task_command(service_binary: &Path) -> String {
    format!("\"{}\"", service_binary.display())
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn install_platform_service(_service_binary: &Path) -> Result<()> {
    bail!("service installation is not implemented for this platform")
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn remove_platform_service() -> Result<()> {
    bail!("service removal is not implemented for this platform")
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn start_platform_service() -> Result<()> {
    bail!("service start is not implemented for this platform")
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn stop_platform_service() -> Result<()> {
    bail!("service stop is not implemented for this platform")
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn platform_service_status() -> Result<ServiceStatus> {
    Ok(ServiceStatus {
        installed: false,
        running: false,
        detail: "service status is not implemented for this platform".to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn candidate_binary_dirs_walks_up_from_current_exe() {
        let dirs = candidate_binary_dirs(Path::new("/tmp/Motional.app/Contents/MacOS/Motional"));
        assert!(dirs.contains(&PathBuf::from("/tmp/Motional.app/Contents/MacOS")));
        assert!(dirs.contains(&PathBuf::from("/tmp")));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_unit_points_at_service_binary() {
        let contents = linux_unit_contents(Path::new("/opt/motional/motional-service"));
        assert!(contents.contains("Description=Motional automation service"));
        assert!(contents.contains("ExecStart=/opt/motional/motional-service"));
        assert!(contents.contains("WantedBy=default.target"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_plist_points_at_service_binary() {
        let contents = macos_plist_contents(Path::new("/Applications/Motional Service"));
        assert!(contents.contains(SERVICE_LABEL));
        assert!(contents.contains("/Applications/Motional Service"));
        assert!(contents.contains("<key>KeepAlive</key>"));
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_task_command_quotes_binary_path() {
        assert_eq!(
            windows_task_command(Path::new(r"C:\Program Files\Motional\motional-service.exe")),
            r#""C:\Program Files\Motional\motional-service.exe""#
        );
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_task_status_detects_running_state() {
        assert!(windows_task_status_is_running(
            "TaskName: Motional Service\r\nStatus: Running\r\n"
        ));
        assert!(!windows_task_status_is_running(
            "TaskName: Motional Service\r\nStatus: Ready\r\n"
        ));
    }
}
