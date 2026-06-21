use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct Project {
    pub path: PathBuf,
    pub frameworks: Vec<String>, // e.g., ["Node", "React Native"]
}
