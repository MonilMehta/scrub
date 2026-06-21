use anyhow::Result;
use std::fs;
use std::path::Path;
use crate::models::plugin::PluginDefinition;

pub fn load_plugin<P: AsRef<Path>>(path: P) -> Result<PluginDefinition> {
    let content = fs::read_to_string(path)?;
    load_plugin_from_str(&content)
}

pub fn load_plugin_from_str(content: &str) -> Result<PluginDefinition> {
    let plugin: PluginDefinition = serde_yaml::from_str(content)?;
    Ok(plugin)
}
