use anyhow::Result;
use std::path::Path;
use std::fs;
use crate::models::plugin::PluginDefinition;
use crate::plugins::parser::load_plugin;

pub struct PluginManager {
    pub plugins: Vec<PluginDefinition>,
}

impl PluginManager {
    pub fn new() -> Self {
        Self { plugins: Vec::new() }
    }

    pub fn load_directory<P: AsRef<Path>>(&mut self, dir: P) -> Result<()> {
        let dir = dir.as_ref();
        if !dir.exists() || !dir.is_dir() {
            return Ok(());
        }

        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_file() && path.extension().unwrap_or_default() == "yaml" {
                match load_plugin(&path) {
                    Ok(plugin) => self.plugins.push(plugin),
                    Err(e) => log::warn!("Failed to load plugin {:?}: {}", path, e),
                }
            }
        }
        Ok(())
    }

    pub fn load_bundled(&mut self) -> Result<()> {
        let bundled = vec![
            include_str!("../../plugins/android.yaml"),
            include_str!("../../plugins/cocoapods.yaml"),
            include_str!("../../plugins/docker.yaml"),
            include_str!("../../plugins/expo.yaml"),
            include_str!("../../plugins/homebrew.yaml"),
            include_str!("../../plugins/ios-simulator.yaml"),
            include_str!("../../plugins/next.yaml"),
            include_str!("../../plugins/node.yaml"),
            include_str!("../../plugins/python.yaml"),
            include_str!("../../plugins/react-native.yaml"),
            include_str!("../../plugins/xcode.yaml"),
            include_str!("../../plugins/yarn.yaml"),
        ];

        for content in bundled {
            if let Ok(plugin) = crate::plugins::parser::load_plugin_from_str(content) {
                self.plugins.push(plugin);
            }
        }
        Ok(())
    }
}
