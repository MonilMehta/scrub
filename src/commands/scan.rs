use anyhow::Result;
use crate::plugins::manager::PluginManager;
use crate::core::scanner::Scanner;
use crate::core::config::Config;
use crate::models::scan::ItemStatus;
use crate::utils::human_size::format_size;
use crate::utils::age::format_age;
use crossterm::style::Stylize;

pub fn execute(json: bool, paths: Vec<String>) -> Result<()> {
    let start_time = std::time::Instant::now();
    if !json {
        println!("Scanning developer caches...\n");
    }

    let config = Config::load();
    let mut plugin_manager = PluginManager::new();
    let current_dir = std::env::current_dir()?;
    let plugins_dir = current_dir.join("plugins");
    let _ = plugin_manager.load_directory(&plugins_dir);

    if !json {
        for p in &plugin_manager.plugins {
            println!("  {} {}", "✓".green(), p.name);
        }
        println!("\n{} Loaded {} plugins\n", "✓".green(), plugin_manager.plugins.len());
    }

    let scan_roots = if !paths.is_empty() { paths } else { config.project_roots.clone() };

    let scanner = Scanner::new(plugin_manager.plugins);
    let report = scanner.scan(&scan_roots);

    if json {
        let j = serde_json::to_string_pretty(&report)?;
        println!("{}", j);
        return Ok(());
    }

    println!("Done in {:.2} s\n", start_time.elapsed().as_secs_f64());
    println!("{}", "─".repeat(52));

    for cat in &report.categories {
        // Header
        let status_sym = if cat.is_detected { "✓".green() } else { "✗".dark_grey() };
        println!("\n{} {}", status_sym, cat.category.as_str().bold());

        if !cat.is_detected {
            println!("  {}", "Not detected".dark_grey());
            continue;
        }

        // Global items
        let found_globals: Vec<_> = cat.global_items.iter()
            .filter(|i| i.status == ItemStatus::Found)
            .collect();

        if !found_globals.is_empty() {
            for item in &found_globals {
                let age = item.last_modified
                    .map(format_age)
                    .unwrap_or_default();
                let safety = if item.safe { "Safe".green() } else { "⚠ Caution".yellow() };
                println!("  {} {:<20} {:>10}   {}   {}",
                    "✓".green(),
                    item.name,
                    format_size(item.size_bytes),
                    safety,
                    age.dark_grey(),
                );
            }
            let global_total: u64 = found_globals.iter().map(|i| i.size_bytes).sum();
            println!("  {}: {}", "Subtotal".dark_grey(), format_size(global_total).bold());
        }

        // Project items
        for proj in &cat.projects {
            if proj.total_size == 0 {
                continue;
            }
            let proj_age = proj.last_modified
                .map(format_age)
                .map(|a| format!("  ({})", a))
                .unwrap_or_default();

            println!("\n  {} {}{}", "→".cyan(), proj.name.as_str().cyan(), proj_age.as_str().dark_grey());

            for item in &proj.items {
                if item.status != ItemStatus::Found || item.size_bytes == 0 {
                    continue;
                }
                let age = item.last_modified
                    .map(format_age)
                    .unwrap_or_default();
                println!("    {} {:<18} {:>10}   {}",
                    "✓".green(),
                    item.name,
                    format_size(item.size_bytes),
                    age.dark_grey(),
                );
            }
            println!("    {}: {}", "Subtotal".dark_grey(), format_size(proj.total_size).bold());
        }
    }

    println!("\n{}", "─".repeat(52));
    println!("Recoverable   {}", format_size(report.total_size).green().bold());

    Ok(())
}
