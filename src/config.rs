use crate::types::Conversation;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    pub temperature: f32,
    pub system_prompt: String,
    pub models: Vec<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            base_url: "https://api.openai.com/v1".to_string(),
            api_key: String::new(),
            model: "gpt-4o-mini".to_string(),
            temperature: 0.7,
            system_prompt: String::new(),
            models: vec![
                "gpt-4o-mini".to_string(),
                "gpt-4o".to_string(),
                "gpt-4.1".to_string(),
                "gpt-4.1-mini".to_string(),
            ],
        }
    }
}

pub fn data_dir() -> PathBuf {
    if let Some(appdata) = std::env::var_os("APPDATA") {
        PathBuf::from(appdata).join("msty_studio")
    } else if let Some(home) = std::env::var_os("HOME") {
        PathBuf::from(home).join(".config").join("msty_studio")
    } else {
        PathBuf::from(".")
    }
}

pub fn config_path() -> PathBuf {
    data_dir().join("config.json")
}

pub fn conversations_path() -> PathBuf {
    data_dir().join("conversations.json")
}

pub fn load_settings() -> Settings {
    std::fs::read_to_string(config_path())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save_settings(s: &Settings) -> Result<(), String> {
    let path = config_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let json = serde_json::to_string_pretty(s).map_err(|e| e.to_string())?;
    std::fs::write(path, json).map_err(|e| e.to_string())
}

pub fn load_conversations() -> Vec<Conversation> {
    std::fs::read_to_string(conversations_path())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save_conversations(list: &[Conversation]) -> Result<(), String> {
    let path = conversations_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let json = serde_json::to_string_pretty(list).map_err(|e| e.to_string())?;
    std::fs::write(path, json).map_err(|e| e.to_string())
}
