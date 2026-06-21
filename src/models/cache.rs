use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct CacheItem {
    pub path: PathBuf,
    pub size: u64,
    pub plugin: String,
    pub is_safe: bool,
}
