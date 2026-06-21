use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use crate::core::size::calculate_size;
use crate::models::plugin::PluginDefinition;
use crate::models::scan::{
    CacheItem, CategoryReport, CommandCacheItem, ItemStatus, ProjectReport, ProjectSummary, ScanReport,
};
use crate::utils::paths::expand_tilde;

const SKIP_DESCENT: &[&str] = &[
    "node_modules", ".git", "Pods", "build", ".next", "dist",
    "target", ".gradle", ".expo", "__pycache__", "venv", ".venv",
    "DerivedData", ".build", "out", ".dart_tool",
];

const MAX_WALK_DEPTH: usize = 5;

pub struct Scanner {
    pub plugins: Vec<PluginDefinition>,
}

impl Scanner {
    pub fn new(plugins: Vec<PluginDefinition>) -> Self {
        Self { plugins }
    }

    pub fn scan<P: AsRef<Path>>(&self, roots: &[P]) -> ScanReport {
        // ── Phase 1: Global items ─────────────────────────────────────────────
        let mut cat_map: HashMap<String, CategoryReport> = HashMap::new();

        for plugin in &self.plugins {
            let mut cat = CategoryReport {
                category: plugin.name.clone(),
                global_items: Vec::new(),
                projects: Vec::new(),
                command_items: Vec::new(),
                total_size: 0,
                is_detected: false,
            };

            for item in &plugin.global_items {
                for raw_path in &item.paths {
                    let full_path = expand_tilde(raw_path);

                    if item.scan_children && full_path.exists() {
                        // List each child directory separately
                        cat.is_detected = true;
                        if let Ok(entries) = std::fs::read_dir(&full_path) {
                            for entry in entries.flatten() {
                                let child_path = entry.path();
                                if !child_path.is_dir() { continue; }
                                let child_name = child_path.file_name()
                                    .unwrap_or_default()
                                    .to_string_lossy()
                                    .into_owned();
                                // Skip .ini files etc.
                                if child_name.ends_with(".ini") || child_name.ends_with(".pub") { continue; }

                                let (status, size_bytes, file_count, dir_count, last_modified) =
                                    measure_path(&child_path);

                                if status == ItemStatus::Found {
                                    cat.total_size += size_bytes;
                                }
                                cat.global_items.push(CacheItem {
                                    name: format!("{} / {}", item.name, child_name),
                                    path: child_path,
                                    size_bytes,
                                    file_count,
                                    dir_count,
                                    safe: item.safe,
                                    explanation: item.explanation.clone(),
                                    status,
                                    last_modified,
                                });
                            }
                        }
                    } else {
                        let (status, size_bytes, file_count, dir_count, last_modified) =
                            measure_path(&full_path);

                        if status == ItemStatus::Found {
                            cat.is_detected = true;
                            cat.total_size += size_bytes;
                        }

                        cat.global_items.push(CacheItem {
                            name: item.name.clone(),
                            path: full_path,
                            size_bytes,
                            file_count,
                            dir_count,
                            safe: item.safe,
                            explanation: item.explanation.clone(),
                            status,
                            last_modified,
                        });
                    }
                }
            }

            // Command items
            for cmd in &plugin.command_items {
                cat.is_detected = true;
                cat.command_items.push(CommandCacheItem {
                    name: cmd.name.clone(),
                    command: cmd.command.clone(),
                    description: cmd.description.clone(),
                    safe: cmd.safe,
                    explanation: cmd.explanation.clone(),
                });
            }

            cat_map.insert(plugin.name.clone(), cat);
        }

        // ── Phase 2: Project walk ─────────────────────────────────────────────
        for root in roots {
            let root_path = expand_tilde(root.as_ref().to_string_lossy().as_ref());
            if !root_path.exists() { continue; }
            self.walk_for_projects(&root_path, 0, &mut cat_map);
        }

        // ── Phase 3: Build cross-ecosystem project summaries ──────────────────
        let mut project_map: HashMap<PathBuf, ProjectSummary> = HashMap::new();
        for cat in cat_map.values() {
            for proj in &cat.projects {
                let entry = project_map.entry(proj.path.clone()).or_insert_with(|| ProjectSummary {
                    name: proj.name.clone(),
                    path: proj.path.clone(),
                    ecosystem: cat.category.clone(),
                    items: Vec::new(),
                    total_size: 0,
                    last_modified: proj.last_modified,
                });
                for item in &proj.items {
                    if item.status == ItemStatus::Found && item.size_bytes > 0 {
                        entry.total_size += item.size_bytes;
                        entry.items.push(item.clone());
                    }
                }
            }
        }
        let mut projects: Vec<ProjectSummary> = project_map.into_values().collect();
        projects.sort_by(|a, b| b.total_size.cmp(&a.total_size));

        // ── Phase 4: Assemble & sort categories ───────────────────────────────
        let mut categories: Vec<CategoryReport> = cat_map.into_values().collect();
        categories.sort_by(|a, b| b.total_size.cmp(&a.total_size));

        let total_size = categories.iter().map(|c| c.total_size).sum();

        ScanReport { categories, total_size, projects }
    }

    fn walk_for_projects(
        &self,
        dir: &Path,
        depth: usize,
        cat_map: &mut HashMap<String, CategoryReport>,
    ) {
        if depth > MAX_WALK_DEPTH { return; }

        let entries: Vec<(String, PathBuf, bool)> = match std::fs::read_dir(dir) {
            Ok(rd) => rd.flatten().map(|e| {
                let name = e.file_name().to_string_lossy().into_owned();
                let path = e.path();
                let is_dir = path.is_dir();
                (name, path, is_dir)
            }).collect(),
            Err(_) => return,
        };

        let child_names: Vec<String> = entries.iter().map(|(n, _, _)| n.clone()).collect();

        // Find all matching plugins for this directory
        let matching_plugins: Vec<&PluginDefinition> = self.plugins.iter()
            .filter(|p| p.matches_directory(&child_names))
            .collect();

        if matching_plugins.is_empty() {
            // No match — just recurse
            for (name, path, is_dir) in &entries {
                if !is_dir { continue; }
                if depth == 0 && name.starts_with('.') { continue; }
                if SKIP_DESCENT.contains(&name.as_str()) { continue; }
                self.walk_for_projects(path, depth + 1, cat_map);
            }
            return;
        }

        // Use a SINGLE shared measured_paths set across ALL matching plugins
        // so node_modules is only counted once even if Node + Next.js + Expo all match
        let mut measured_paths: HashSet<PathBuf> = HashSet::new();
        let project_name = canonical_name(dir);
        let last_modified = dir_last_modified(dir);

        for plugin in &matching_plugins {
            let mut proj = ProjectReport {
                name: project_name.clone(),
                path: dir.to_path_buf(),
                items: Vec::new(),
                total_size: 0,
                last_modified,
            };

            for item in &plugin.project_items {
                for rel in &item.paths {
                    let full_path = dir.join(rel);
                    if measured_paths.contains(&full_path) { continue; }

                    let (status, size_bytes, file_count, dir_count, last_modified) =
                        measure_path(&full_path);

                    if status == ItemStatus::Found {
                        measured_paths.insert(full_path.clone());
                        proj.total_size += size_bytes;
                    }

                    proj.items.push(CacheItem {
                        name: item.name.clone(),
                        path: full_path,
                        size_bytes,
                        file_count,
                        dir_count,
                        safe: item.safe,
                        explanation: item.explanation.clone(),
                        status,
                        last_modified,
                    });
                }
            }

            if let Some(cat) = cat_map.get_mut(&plugin.name) {
                cat.is_detected = true;
                cat.total_size += proj.total_size;
                cat.projects.push(proj);
            }
        }

        for (name, path, is_dir) in entries {
            if !is_dir { continue; }
            if depth == 0 && name.starts_with('.') { continue; }
            if SKIP_DESCENT.contains(&name.as_str()) { continue; }
            self.walk_for_projects(&path, depth + 1, cat_map);
        }
    }
}

fn measure_path(path: &Path) -> (ItemStatus, u64, usize, usize, Option<u64>) {
    if !path.exists() {
        return (ItemStatus::Missing, 0, 0, 0, None);
    }
    let (size_bytes, file_count, dir_count) = calculate_size(path);
    let last_modified = path.metadata().ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs());
    (ItemStatus::Found, size_bytes, file_count, dir_count, last_modified)
}

fn canonical_name(dir: &Path) -> String {
    dir.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| {
            std::fs::canonicalize(dir).ok()
                .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
                .unwrap_or_else(|| dir.display().to_string())
        })
}

fn dir_last_modified(dir: &Path) -> Option<u64> {
    dir.metadata().ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
}
