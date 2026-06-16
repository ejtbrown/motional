use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::actions::Action;

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

pub fn load_config(path: &PathBuf) -> Result<AppConfig> {
    if !path.exists() {
        return Ok(AppConfig::default());
    }

    let text = fs::read_to_string(path)
        .with_context(|| format!("failed to read config file {}", path.display()))?;
    serde_json::from_str(&text)
        .with_context(|| format!("failed to parse config file {}", path.display()))
}

pub fn save_config(path: &PathBuf, config: &AppConfig) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create config directory {}", parent.display()))?;
    }

    let text = serde_json::to_string_pretty(config).context("failed to encode config")?;
    fs::write(path, text).with_context(|| format!("failed to write config file {}", path.display()))
}

fn new_entry_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    format!("entry-{nanos}")
}
