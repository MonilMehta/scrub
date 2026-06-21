use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginDefinition {
    pub name: String,
    pub description: String,
    /// ALL of these must exist in the directory
    #[serde(default)]
    pub detectors: Vec<String>,
    /// At least ONE of these must exist (combined with detectors above)
    #[serde(default)]
    pub detectors_any: Vec<String>,
    #[serde(default)]
    pub global_items: Vec<PluginItem>,
    #[serde(default)]
    pub project_items: Vec<PluginItem>,
    #[serde(default)]
    pub command_items: Vec<CommandItem>,
}

impl PluginDefinition {
    /// Check if a directory (represented by its children filenames) matches this plugin
    pub fn matches_directory(&self, children: &[String]) -> bool {
        if self.detectors.is_empty() && self.detectors_any.is_empty() {
            return false;
        }

        // ALL required detectors must match
        for detector in &self.detectors {
            if !matches_detector(detector, children) {
                return false;
            }
        }

        // If detectors_any is non-empty, at least ONE must match
        if !self.detectors_any.is_empty() {
            let any_match = self.detectors_any.iter().any(|d| matches_detector(d, children));
            if !any_match {
                return false;
            }
        }

        true
    }
}

fn matches_detector(detector: &str, children: &[String]) -> bool {
    let det = detector.trim_end_matches('/');
    if det.starts_with("*.") {
        let ext = det.trim_start_matches("*.");
        children.iter().any(|n| n.ends_with(ext))
    } else if det.ends_with('.') {
        children.iter().any(|n| n.starts_with(det))
    } else {
        children.contains(&det.to_string())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginItem {
    pub name: String,
    pub paths: Vec<String>,
    #[serde(default)]
    pub safe: bool,
    #[serde(default = "default_delete_strategy")]
    pub delete: String,
    pub explanation: Option<String>,
    /// If true, each immediate child of the path is listed as a separate item
    #[serde(default)]
    pub scan_children: bool,
}

/// A command-type item that runs a shell command instead of deleting a path.
/// Shown as a separate action in the TUI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandItem {
    pub name: String,
    pub command: String,
    pub description: String,
    pub safe: bool,
    pub explanation: Option<String>,
}

fn default_delete_strategy() -> String {
    "directory".to_string()
}
