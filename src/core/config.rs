use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default = "default_trash")]
    pub trash: bool,
    #[serde(default = "default_parallel")]
    pub parallel: bool,
    #[serde(default = "default_max_threads")]
    pub max_threads: usize,
    #[serde(default = "default_project_roots")]
    pub project_roots: Vec<String>,
}

fn default_trash() -> bool { true }
fn default_parallel() -> bool { true }
fn default_max_threads() -> usize { 8 }
fn default_project_roots() -> Vec<String> {
    vec![
        "~/Projects".to_string(),
        "~/Code".to_string(),
        "~/Developer".to_string(),
    ]
}

impl Default for Config {
    fn default() -> Self {
        Self {
            trash: true,
            parallel: true,
            max_threads: 8,
            project_roots: default_project_roots(),
        }
    }
}

impl Config {
    pub fn load() -> Self {
        let config_path = dirs::home_dir()
            .map(|p| p.join(".config/scrub/config.toml"))
            .unwrap_or_else(|| PathBuf::from(".config/scrub/config.toml"));

        if let Ok(content) = std::fs::read_to_string(&config_path) {
            if let Ok(config) = toml::from_str(&content) {
                return config;
            }
        } else {
            // Write default config to disk if it doesn't exist
            if let Some(parent) = config_path.parent() {
                let _ = std::fs::create_dir_all(parent);
                if let Ok(default_toml) = toml::to_string_pretty(&Config::default()) {
                    let _ = std::fs::write(&config_path, default_toml);
                }
            }
        }
        Config::default()
    }
}
