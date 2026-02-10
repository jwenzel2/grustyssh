use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::OnceLock;

use crate::error::AppError;

static PROJECT_DIRS: OnceLock<ProjectDirs> = OnceLock::new();

fn project_dirs() -> &'static ProjectDirs {
    PROJECT_DIRS.get_or_init(|| {
        ProjectDirs::from("com", "grustyssh", "grustyssh")
            .expect("Failed to determine project directories")
    })
}

pub fn config_dir() -> PathBuf {
    project_dirs().config_dir().to_path_buf()
}

pub fn data_dir() -> PathBuf {
    project_dirs().data_dir().to_path_buf()
}

pub fn profiles_path() -> PathBuf {
    config_dir().join("profiles.json")
}

pub fn settings_path() -> PathBuf {
    config_dir().join("settings.json")
}

pub fn known_hosts_path() -> PathBuf {
    config_dir().join("known_hosts")
}

pub fn keys_index_path() -> PathBuf {
    data_dir().join("keys.json")
}

pub fn keys_dir() -> PathBuf {
    data_dir().join("keys")
}

pub fn ensure_directories() -> Result<(), AppError> {
    std::fs::create_dir_all(config_dir())?;
    std::fs::create_dir_all(keys_dir())?;
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    pub font_family: String,
    pub font_size: u32,
    pub scrollback_lines: i64,
    pub default_terminal_type: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            font_family: "Monospace".into(),
            font_size: 12,
            scrollback_lines: 10000,
            default_terminal_type: "xterm-256color".into(),
        }
    }
}

impl Settings {
    pub fn load() -> Self {
        let path = settings_path();
        if path.exists() {
            match std::fs::read_to_string(&path) {
                Ok(data) => serde_json::from_str(&data).unwrap_or_default(),
                Err(_) => Self::default(),
            }
        } else {
            Self::default()
        }
    }

    pub fn save(&self) -> Result<(), AppError> {
        let path = settings_path();
        let data = serde_json::to_string_pretty(self)?;
        std::fs::write(path, data)?;
        Ok(())
    }
}
