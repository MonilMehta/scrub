use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanReport {
    pub categories: Vec<CategoryReport>,
    pub total_size: u64,
    pub projects: Vec<ProjectSummary>,
}

/// A flattened cross-ecosystem view of a single project directory
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectSummary {
    pub name: String,
    pub path: PathBuf,
    pub ecosystem: String,
    pub items: Vec<CacheItem>,
    pub total_size: u64,
    pub last_modified: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CategoryReport {
    pub category: String,
    pub global_items: Vec<CacheItem>,
    pub projects: Vec<ProjectReport>,
    pub command_items: Vec<CommandCacheItem>,
    pub total_size: u64,
    pub is_detected: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectReport {
    pub name: String,
    pub path: PathBuf,
    pub items: Vec<CacheItem>,
    pub total_size: u64,
    pub last_modified: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ItemStatus {
    Found,
    Missing,
    RequiresAction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheItem {
    pub name: String,
    pub path: PathBuf,
    pub size_bytes: u64,
    pub file_count: usize,
    pub dir_count: usize,
    pub safe: bool,
    pub explanation: Option<String>,
    pub status: ItemStatus,
    pub last_modified: Option<u64>,
}

/// An item that runs a shell command rather than deleting a path
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandCacheItem {
    pub name: String,
    pub command: String,
    pub description: String,
    pub safe: bool,
    pub explanation: Option<String>,
}
