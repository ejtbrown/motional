use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::actions::Action;

#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
#[cfg(unix)]
use std::{fs::OpenOptions, io::Write};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AppConfig {
    pub entries: Vec<ServerEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerEntry {
    pub id: String,
    pub enabled: bool,
    pub label: String,
    pub address: String,
    pub token: String,
    pub sensor: String,
    #[serde(default)]
    pub on_connected: Vec<Action>,
    #[serde(default)]
    pub on_disconnected: Vec<Action>,
    #[serde(default)]
    pub on_motion: Vec<Action>,
    #[serde(default)]
    pub on_absence: Vec<Action>,
}

impl ServerEntry {
    pub fn new() -> Self {
        Self {
            id: new_entry_id(),
            enabled: true,
            label: "New Motional sensor".to_string(),
            address: "127.0.0.1:7080".to_string(),
            token: String::new(),
            sensor: String::new(),
            on_connected: Vec::new(),
            on_disconnected: Vec::new(),
            on_motion: Vec::new(),
            on_absence: Vec::new(),
        }
    }
}

impl Default for ServerEntry {
    fn default() -> Self {
        Self::new()
    }
}

pub fn config_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("motional")
        .join("motional-gui.json")
}

pub fn load_config(path: &Path) -> Result<AppConfig> {
    if !path.exists() {
        return Ok(AppConfig::default());
    }

    let text = fs::read_to_string(path)
        .with_context(|| format!("failed to read config file {}", path.display()))?;
    serde_json::from_str(&text)
        .with_context(|| format!("failed to parse config file {}", path.display()))
}

pub fn save_config(path: &Path, config: &AppConfig) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create config directory {}", parent.display()))?;
        secure_config_dir(parent)?;
    }

    let text = serde_json::to_string_pretty(config).context("failed to encode config")?;
    write_private_file(path, text.as_bytes())
        .with_context(|| format!("failed to write config file {}", path.display()))
}

#[cfg(unix)]
fn secure_config_dir(path: &Path) -> Result<()> {
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .with_context(|| format!("failed to secure config directory {}", path.display()))
}

#[cfg(not(unix))]
fn secure_config_dir(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(unix)]
fn write_private_file(path: &Path, contents: &[u8]) -> Result<()> {
    let mut file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .mode(0o600)
        .open(path)
        .with_context(|| format!("failed to open config file {}", path.display()))?;
    file.write_all(contents)
        .with_context(|| format!("failed to write config file {}", path.display()))?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
        .with_context(|| format!("failed to secure config file {}", path.display()))
}

#[cfg(not(unix))]
fn write_private_file(path: &Path, contents: &[u8]) -> Result<()> {
    fs::write(path, contents)
        .with_context(|| format!("failed to write config file {}", path.display()))
}

fn new_entry_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    format!("entry-{nanos}")
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn save_config_uses_private_unix_permissions() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("motional-gui.json");

        save_config(&path, &AppConfig::default()).unwrap();

        let dir_mode = fs::metadata(dir.path()).unwrap().permissions().mode() & 0o777;
        let file_mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(dir_mode, 0o700);
        assert_eq!(file_mode, 0o600);
    }
}
