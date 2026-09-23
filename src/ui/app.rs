use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::{Backend, CrosstermBackend},
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Tabs, Wrap},
    Frame, Terminal,
};
use std::collections::HashSet;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, TryRecvError};
use std::time::Duration;

use crate::core::config::Config;
use crate::core::scanner::Scanner;
use crate::models::scan::{ItemStatus, ScanReport};
use crate::plugins::manager::PluginManager;
use crate::utils::age::format_age;
use crate::utils::human_size::format_size;
use crate::utils::paths::expand_tilde;

// ── Row types ─────────────────────────────────────────────────────────────────

#[derive(Clone, Debug)]
#[allow(dead_code)]
enum RowKind {
    EcoHeader { cat_idx: usize },
    GlobalItem { cat_idx: usize, item_idx: usize },
    CommandItem { cat_idx: usize, cmd_idx: usize },
}

#[derive(Clone, Debug, PartialEq)]
enum RowAction {
    Delete,
    RunCommand(String),
    None,
}

#[derive(Clone, Debug)]
struct Row {
    kind: RowKind,
    selectable: bool,
    label: String,
    size_bytes: u64,
    safe: bool,
    last_modified: Option<u64>,
    path: std::path::PathBuf,
    action: RowAction,
    explanation: Option<String>,
    command_desc: Option<String>,
}

// ── Projects tab rows ─────────────────────────────────────────────────────────

#[derive(Clone, Debug)]
#[allow(dead_code)]
enum ProjRowKind { Header { proj_idx: usize }, Item { proj_idx: usize, item_idx: usize } }

#[derive(Clone, Debug)]
struct ProjRow {
    kind: ProjRowKind,
    selectable: bool,
    label: String,
    size_bytes: u64,
    safe: bool,
    last_modified: Option<u64>,
    path: std::path::PathBuf,
    explanation: Option<String>,
}

// ── Dialog state ──────────────────────────────────────────────────────────────

#[derive(Debug, PartialEq)]
enum AppMode {
    Browse,
    Confirm,
    RunCommand { command: String, description: String },
    CommandOutput { output: String, success: bool },
    Deleting { current: String, done: usize, total: usize, permanent: bool },
    Done,
}

#[derive(Debug, PartialEq, Clone, Copy)]
enum ActiveTab { Caches, Projects }

// ── App state ─────────────────────────────────────────────────────────────────

struct App {
    report: ScanReport,
    // Caches tab
    rows: Vec<Row>,
    list_state: ListState,
    selected: HashSet<usize>,
    total_selected_bytes: u64,
    // Projects tab
    proj_rows: Vec<ProjRow>,
    proj_list_state: ListState,
    proj_selected: HashSet<usize>,
    proj_total_selected: u64,
    // Shared
    tab: ActiveTab,
    mode: AppMode,
    delete_log: Vec<String>,
    receipt: Vec<(String, u64, bool)>,
}

impl App {
    fn new(report: ScanReport) -> Self {
        let rows = build_cache_rows(&report);
        let proj_rows = build_proj_rows(&report);

        let mut list_state = ListState::default();
        if !rows.is_empty() { list_state.select(Some(0)); }
        let mut proj_list_state = ListState::default();
        if !proj_rows.is_empty() { proj_list_state.select(Some(0)); }

        Self {
            report, rows, list_state,
            selected: HashSet::new(), total_selected_bytes: 0,
            proj_rows, proj_list_state,
            proj_selected: HashSet::new(), proj_total_selected: 0,
            tab: ActiveTab::Caches,
            mode: AppMode::Browse,
            delete_log: Vec::new(),
            receipt: Vec::new(),
        }
    }

    fn cursor(&self) -> usize {
        match self.tab {
            ActiveTab::Caches => self.list_state.selected().unwrap_or(0),
            ActiveTab::Projects => self.proj_list_state.selected().unwrap_or(0),
        }
    }

    fn list_len(&self) -> usize {
        match self.tab {
            ActiveTab::Caches => self.rows.len(),
            ActiveTab::Projects => self.proj_rows.len(),
        }
    }

    fn move_up(&mut self) {
        let i = self.cursor();
        let n = self.list_len();
        let new = if i == 0 { n.saturating_sub(1) } else { i - 1 };
        match self.tab {
            ActiveTab::Caches => self.list_state.select(Some(new)),
            ActiveTab::Projects => self.proj_list_state.select(Some(new)),
        }
    }

    fn move_down(&mut self) {
        let i = self.cursor();
        let n = self.list_len();
        let new = if i >= n.saturating_sub(1) { 0 } else { i + 1 };
        match self.tab {
            ActiveTab::Caches => self.list_state.select(Some(new)),
            ActiveTab::Projects => self.proj_list_state.select(Some(new)),
        }
    }

    fn toggle(&mut self) {
        let i = self.cursor();
        match self.tab {
            ActiveTab::Caches => {
                if let Some(row) = self.rows.get(i) {
                    if !row.selectable { return; }
                    match &row.action {
                        RowAction::RunCommand(cmd) => {
                            let cmd = cmd.clone();
                            let desc = row.command_desc.clone().unwrap_or_default();
                            self.mode = AppMode::RunCommand { command: cmd, description: desc };
                            return;
                        }
                        _ => {}
                    }
                    if self.selected.contains(&i) { self.selected.remove(&i); }
                    else { self.selected.insert(i); }
                    self.recalc(false);
                }
            }
            ActiveTab::Projects => {
                if let Some(row) = self.proj_rows.get(i) {
                    if !row.selectable { return; }
                    if self.proj_selected.contains(&i) { self.proj_selected.remove(&i); }
                    else { self.proj_selected.insert(i); }
                    self.recalc(true);
                }
            }
        }
    }

    fn recalc(&mut self, proj_tab: bool) {
        if proj_tab {
            self.proj_total_selected = self.proj_selected.iter()
                .filter_map(|&i| self.proj_rows.get(i))
                .map(|r| r.size_bytes).sum();
        } else {
            self.total_selected_bytes = self.selected.iter()
                .filter_map(|&i| self.rows.get(i))
                .map(|r| r.size_bytes).sum();
        }
    }

    fn can_delete(&self) -> bool {
        match self.tab {
            ActiveTab::Caches => !self.selected.is_empty(),
            ActiveTab::Projects => !self.proj_selected.is_empty(),
        }
    }

    fn selected_count(&self) -> usize {
        match self.tab { ActiveTab::Caches => self.selected.len(), ActiveTab::Projects => self.proj_selected.len() }
    }

    fn selected_bytes(&self) -> u64 {
        match self.tab { ActiveTab::Caches => self.total_selected_bytes, ActiveTab::Projects => self.proj_total_selected }
    }

    fn do_delete(&mut self, permanent: bool, mut render: impl FnMut(&mut Self) -> io::Result<()>) -> io::Result<()> {
        let paths: Vec<(std::path::PathBuf, u64)> = match self.tab {
            ActiveTab::Caches => {
                let mut v: Vec<usize> = self.selected.iter().cloned().collect();
                v.sort();
                v.iter().filter_map(|i| self.rows.get(*i)).map(|r| (r.path.clone(), r.size_bytes)).collect()
            }
            ActiveTab::Projects => {
                let mut v: Vec<usize> = self.proj_selected.iter().cloned().collect();
                v.sort();
                v.iter().filter_map(|i| self.proj_rows.get(*i)).map(|r| (r.path.clone(), r.size_bytes)).collect()
            }
        };

        let total = paths.len();
        for (done, (path, size)) in paths.into_iter().enumerate() {
            let label = path.file_name().map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.display().to_string());
            self.mode = AppMode::Deleting { current: label.clone(), done, total, permanent };
            render(self)?;
            let res = if permanent {
                if path.is_dir() { std::fs::remove_dir_all(&path) } else { std::fs::remove_file(&path) }
            } else {
                trash::delete(&path).map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))
            };
            match res {
                Ok(_) => {
                    let action = if permanent { "Permanently deleted" } else { "Moved to Trash" };
                    self.delete_log.push(format!("✓ {}: {}", action, label));
                    self.receipt.push((label.clone(), size, permanent));
                    self.report.total_size = self.report.total_size.saturating_sub(size);

                    for i in 0..self.rows.len() {
                        if self.rows[i].path == path && self.rows[i].size_bytes > 0 {
                            self.rows[i].size_bytes = 0;
                            if let RowKind::GlobalItem { cat_idx, .. } = self.rows[i].kind {
                                for j in 0..self.rows.len() {
                                    if let RowKind::EcoHeader { cat_idx: c } = self.rows[j].kind {
                                        if c == cat_idx { self.rows[j].size_bytes = self.rows[j].size_bytes.saturating_sub(size); }
                                    }
                                }
                            }
                        }
                    }

                    for i in 0..self.proj_rows.len() {
                        if self.proj_rows[i].path == path && self.proj_rows[i].size_bytes > 0 {
                            self.proj_rows[i].size_bytes = 0;
                            if let ProjRowKind::Item { proj_idx, .. } = self.proj_rows[i].kind {
                                for j in 0..self.proj_rows.len() {
                                    if let ProjRowKind::Header { proj_idx: p } = self.proj_rows[j].kind {
                                        if p == proj_idx { self.proj_rows[j].size_bytes = self.proj_rows[j].size_bytes.saturating_sub(size); }
                                    }
                                }
                            }
                        }
                    }
                }
                Err(e) => self.delete_log.push(format!("✗ Failed {}: {}", label, e)),
            }
            self.mode = AppMode::Deleting { current: label, done: done + 1, total, permanent };
            render(self)?;
        }

        match self.tab {
            ActiveTab::Caches => { self.selected.clear(); self.total_selected_bytes = 0; }
            ActiveTab::Projects => { self.proj_selected.clear(); self.proj_total_selected = 0; }
        }
        self.mode = AppMode::Done;
        Ok(())
    }

    fn run_command(&mut self, command: &str) {
        #[cfg(target_os = "windows")]
        let output = std::process::Command::new("cmd").arg("/C").arg(command).output();
        #[cfg(not(target_os = "windows"))]
        let output = std::process::Command::new("sh").arg("-c").arg(command).output();
        
        match output {
            Ok(out) => {
                let stdout = String::from_utf8_lossy(&out.stdout).to_string();
                let stderr = String::from_utf8_lossy(&out.stderr).to_string();
                let combined = if stderr.is_empty() { stdout } else { format!("{}\n{}", stdout, stderr) };
                self.mode = AppMode::CommandOutput {
                    output: combined.trim().to_string(),
                    success: out.status.success(),
                };
            }
            Err(e) => {
                self.mode = AppMode::CommandOutput {
                    output: format!("Failed to run command: {}", e),
                    success: false,
                };
            }
        }
    }

    fn current_detail(&self) -> Option<DetailInfo> {
        match self.tab {
            ActiveTab::Caches => {
                let i = self.cursor();
                let row = self.rows.get(i)?;
                Some(DetailInfo {
                    name: row.label.trim().to_string(),
                    path_str: display_path(&row.path),
                    size_bytes: row.size_bytes,
                    safe: row.safe,
                    last_modified: row.last_modified,
                    explanation: row.explanation.clone(),
                    is_command: matches!(row.action, RowAction::RunCommand(_)),
                    command_desc: row.command_desc.clone(),
                    selectable: row.selectable,
                    selected: self.selected.contains(&i),
                })
            }
            ActiveTab::Projects => {
                let i = self.cursor();
                let row = self.proj_rows.get(i)?;
                Some(DetailInfo {
                    name: row.label.trim().to_string(),
                    path_str: display_path(&row.path),
                    size_bytes: row.size_bytes,
                    safe: row.safe,
                    last_modified: row.last_modified,
                    explanation: row.explanation.clone(),
                    is_command: false,
                    command_desc: None,
                    selectable: row.selectable,
                    selected: self.proj_selected.contains(&i),
                })
            }
        }
    }
}

struct DetailInfo {
    name: String,
    path_str: String,
    size_bytes: u64,
    safe: bool,
    last_modified: Option<u64>,
    explanation: Option<String>,
    is_command: bool,
    command_desc: Option<String>,
    selectable: bool,
    selected: bool,
}

// ── Row builders ──────────────────────────────────────────────────────────────

fn build_cache_rows(report: &ScanReport) -> Vec<Row> {
    let mut rows = Vec::new();
    for (cat_idx, cat) in report.categories.iter().enumerate() {
        let global_size: u64 = cat.global_items.iter()
            .filter(|item| item.status == ItemStatus::Found && item.size_bytes > 0)
            .map(|item| item.size_bytes).sum();
        if global_size == 0 && cat.command_items.is_empty() { continue; }
        rows.push(Row {
            kind: RowKind::EcoHeader { cat_idx },
            selectable: false,
            label: cat.category.clone(),
            size_bytes: global_size,
            safe: true,
            last_modified: None,
            path: std::path::PathBuf::new(),
            action: RowAction::None,
            explanation: None,
            command_desc: None,
        });
        for (item_idx, item) in cat.global_items.iter().enumerate() {
            if item.status != ItemStatus::Found || item.size_bytes == 0 { continue; }
            rows.push(Row {
                kind: RowKind::GlobalItem { cat_idx, item_idx },
                selectable: true,
                label: format!("   {}", item.name),
                size_bytes: item.size_bytes,
                safe: item.safe,
                last_modified: item.last_modified,
                path: item.path.clone(),
                action: RowAction::Delete,
                explanation: item.explanation.clone(),
                command_desc: None,
            });
        }

        for (cmd_idx, cmd) in cat.command_items.iter().enumerate() {
            rows.push(Row {
                kind: RowKind::CommandItem { cat_idx, cmd_idx },
                selectable: true,
                label: format!("   ⚡ {}", cmd.name),
                size_bytes: 0,
                safe: cmd.safe,
                last_modified: None,
                path: std::path::PathBuf::new(),
                action: RowAction::RunCommand(cmd.command.clone()),
                explanation: cmd.explanation.clone(),
                command_desc: Some(cmd.description.clone()),
            });
        }

    }
    rows
}

fn build_proj_rows(report: &ScanReport) -> Vec<ProjRow> {
    let mut rows = Vec::new();
    for (proj_idx, proj) in report.projects.iter().enumerate() {
        if proj.total_size == 0 { continue; }
        let parent = proj.path.parent().and_then(|path| path.file_name())
            .map(|name| name.to_string_lossy()).unwrap_or_default();
        let label = if proj.path.file_name().is_some_and(|name| name == proj.name.as_str()) {
            format!("{} / {}", parent, proj.name)
        } else {
            proj.name.clone()
        };
        rows.push(ProjRow {
            kind: ProjRowKind::Header { proj_idx },
            selectable: false,
            label,
            size_bytes: proj.total_size,
            safe: true,
            last_modified: proj.last_modified,
            path: proj.path.clone(),
            explanation: None,
        });
        for (item_idx, item) in proj.items.iter().enumerate() {
            rows.push(ProjRow {
                kind: ProjRowKind::Item { proj_idx, item_idx },
                selectable: true,
                label: format!("   {}", item.name),
                size_bytes: item.size_bytes,
                safe: item.safe,
                last_modified: item.last_modified,
                path: item.path.clone(),
                explanation: item.explanation.clone(),
            });
        }
    }
    rows
}

// ── Entry point ───────────────────────────────────────────────────────────────

fn discover_scan_roots(home: &Path, cwd: &Path, config: &Config) -> Vec<PathBuf> {
    let mut candidates: Vec<PathBuf> = config.project_roots.iter().map(expand_tilde).collect();
    if let Ok(entries) = std::fs::read_dir(home) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
            if ["code", "projects", "developer", "development", "workspace", "workspaces", "repos", "src", "github"]
                .contains(&name.as_str()) {
                candidates.push(entry.path());
            }
        }
    }
    if ["Cargo.toml", "package.json", "pyproject.toml", ".git"]
        .iter().any(|marker| cwd.join(marker).exists()) {
        if let Some(parent) = cwd.parent() {
            if parent != home { candidates.push(parent.to_path_buf()); }
        }
    }
    let mut seen = HashSet::new();
    candidates.into_iter().filter(|path| path.is_dir())
        .filter(|path| seen.insert(path.canonicalize().unwrap_or_else(|_| path.clone())))
        .collect()
}

fn choose_scan_roots(paths: Vec<String>, config: &Config) -> Result<Vec<PathBuf>> {
    let home = dirs::home_dir().unwrap_or_default();
    let candidates = if paths.is_empty() {
        discover_scan_roots(&home, &std::env::current_dir()?, config)
    } else {
        paths.into_iter().map(expand_tilde).collect()
    };
    if !candidates.is_empty() {
        println!("Project directories to scan:");
        for path in &candidates { println!("  {}", path.display()); }
        loop {
            let prompt = if candidates.len() == 1 { "Is this your project directory? [Y/n]: " }
                else { "Are these your project directories? [Y/n]: " };
            print!("{}", prompt);
            io::stdout().flush()?;
            let mut answer = String::new();
            if io::stdin().read_line(&mut answer)? == 0 { anyhow::bail!("No project directory provided"); }
            match answer.trim().to_ascii_lowercase().as_str() {
                "" | "y" | "yes" => {
                    if candidates.iter().all(|path| path.is_dir()) { return Ok(candidates); }
                    println!("One or more directories do not exist. Enter a directory to scan.");
                    break;
                }
                "n" | "no" => break,
                _ => println!("Please enter y or n."),
            }
        }
    }
    loop {
        print!("Project directory to scan: ");
        io::stdout().flush()?;
        let mut input = String::new();
        if io::stdin().read_line(&mut input)? == 0 { anyhow::bail!("No project directory provided"); }
        let path = expand_tilde(input.trim());
        if path.is_dir() { return Ok(vec![path]); }
        println!("Enter an existing directory.");
    }
}

pub fn run(paths: Vec<String>) -> Result<()> {
    let config = Config::load();
    let scan_roots = choose_scan_roots(paths, &config)?;
    let mut plugin_manager = PluginManager::new();
    let _ = plugin_manager.load_bundled();
    let scanner = Scanner::new(plugin_manager.plugins);

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let (tx, rx) = mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let report = scanner.scan_with_progress(&scan_roots, |path, _| {
            let _ = tx.try_send(ScanMessage::Path(path.to_path_buf()));
        });
        let _ = tx.send(ScanMessage::Done(report));
    });

    let mut receipt = Vec::new();
    let res = match scan_loop(&mut terminal, &rx) {
        Ok(Some(report)) => {
            let mut app = App::new(report);
            let result = event_loop(&mut terminal, &mut app);
            receipt = app.receipt;
            result
        }
        Ok(None) => Ok(()),
        Err(error) => Err(error),
    };

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    res?;

    if !receipt.is_empty() {
        use crossterm::style::Stylize;
        println!("\n🧾 Scrub Receipt");
        println!("{}", "─".repeat(50));
        let mut deleted = 0;
        let mut trashed = 0;
        for (label, size, perm) in receipt {
            let action = if perm { "Permanently deleted".red() } else { "Moved to Trash".yellow() };
            println!("✓ {} {:<20} {}", action, label, format_size(size).bold());
            if perm { deleted += size; } else { trashed += size; }
        }
        println!("{}", "─".repeat(50));
        if deleted > 0 { println!("Estimated space freed: {}", format_size(deleted).green().bold()); }
        if trashed > 0 { println!("Moved to Trash: {} (space freed when Trash is emptied)", format_size(trashed).yellow().bold()); }
        println!();
    }

    Ok(())
}

enum ScanMessage {
    Path(PathBuf),
    Done(ScanReport),
}

fn scan_loop<B: Backend>(terminal: &mut Terminal<B>, rx: &mpsc::Receiver<ScanMessage>) -> io::Result<Option<ScanReport>> {
    let frames = ["◐", "◓", "◑", "◒"];
    let mut current = PathBuf::new();
    let mut tick = 0;
    loop {
        match rx.try_recv() {
            Ok(ScanMessage::Path(path)) => current = path,
            Ok(ScanMessage::Done(report)) => return Ok(Some(report)),
            Err(TryRecvError::Empty) => {},
            Err(TryRecvError::Disconnected) => return Err(io::Error::other("Scan stopped unexpectedly")),
        }
        terminal.draw(|f| draw_scanning(f, frames[tick % frames.len()], &current))?;
        tick += 1;
        if event::poll(Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                if matches!(key.code, KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('Q'))
                    || (key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL)) {
                    return Ok(None);
                }
            }
        }
    }
}

// ── Event loop ────────────────────────────────────────────────────────────────

fn event_loop<B: Backend>(terminal: &mut Terminal<B>, app: &mut App) -> io::Result<()> {
    loop {
        terminal.draw(|f| draw(f, app))?;
        if let Event::Key(key) = event::read()? {
            match &app.mode {
                AppMode::Browse => match key.code {
                    KeyCode::Char('q') | KeyCode::Char('Q') => return Ok(()),
                    KeyCode::Down | KeyCode::Char('j') => app.move_down(),
                    KeyCode::Up | KeyCode::Char('k') => app.move_up(),
                    KeyCode::Char(' ') | KeyCode::Enter => app.toggle(),
                    KeyCode::Tab => {
                        app.tab = if app.tab == ActiveTab::Caches { ActiveTab::Projects } else { ActiveTab::Caches };
                    }
                    KeyCode::Char('d') | KeyCode::Char('D') => {
                        if app.can_delete() { app.mode = AppMode::Confirm; }
                    }
                    KeyCode::Char('c') | KeyCode::Char('C') => {
                        app.selected.clear(); app.proj_selected.clear();
                        app.total_selected_bytes = 0; app.proj_total_selected = 0;
                    }
                    _ => {}
                },
                AppMode::Confirm => match key.code {
                    KeyCode::Char('y') | KeyCode::Char('Y') => app.do_delete(false, |app| terminal.draw(|f| draw(f, app)).map(|_| ()))?,
                    KeyCode::Char('p') | KeyCode::Char('P') => app.do_delete(true, |app| terminal.draw(|f| draw(f, app)).map(|_| ()))?,
                    KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => app.mode = AppMode::Browse,
                    _ => {}
                },
                AppMode::RunCommand { command, .. } => match key.code {
                    KeyCode::Char('y') | KeyCode::Char('Y') => {
                        let cmd = command.clone();
                        app.run_command(&cmd);
                    }
                    KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => app.mode = AppMode::Browse,
                    _ => {}
                },
                AppMode::CommandOutput { .. } | AppMode::Done => match key.code {
                    KeyCode::Enter | KeyCode::Esc | KeyCode::Char('q') => {
                        app.mode = AppMode::Browse;
                        app.delete_log.clear();
                    }
                    _ => {}
                },
                AppMode::Deleting { .. } => {}
            }
        }
    }
}

// ── Drawing ───────────────────────────────────────────────────────────────────

fn draw_scanning(f: &mut Frame, spinner: &str, path: &Path) {
    let area = centered_rect(70, 35, f.area());
    let current = if path.as_os_str().is_empty() {
        "Looking for caches".to_string()
    } else {
        let parent = path.parent().and_then(|p| p.file_name()).unwrap_or_default().to_string_lossy();
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        format!("{} / {}", parent, name)
    };
    let lines = vec![
        Line::from(""),
        Line::from(Span::styled(format!("{}  Scanning caches", spinner), Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))),
        Line::from(""),
        Line::from(current),
        Line::from(""),
        Line::from(Span::styled("q / Esc to cancel", Style::default().fg(Color::DarkGray))),
    ];
    let block = Block::default().title(" Scrub ").borders(Borders::ALL).border_style(Style::default().fg(Color::Cyan));
    f.render_widget(Paragraph::new(lines).alignment(ratatui::layout::Alignment::Center).block(block), area);
}

fn draw(f: &mut Frame, app: &mut App) {
    let area = f.area();
    let layout = Layout::vertical([
        Constraint::Length(3),
        Constraint::Length(2),
        Constraint::Min(0),
        Constraint::Length(3),
    ]).split(area);

    draw_header(f, app, layout[0]);
    draw_tabs(f, app, layout[1]);

    let body = Layout::horizontal([
        Constraint::Percentage(55),
        Constraint::Percentage(45),
    ]).split(layout[2]);

    match app.tab {
        ActiveTab::Caches => draw_cache_list(f, app, body[0]),
        ActiveTab::Projects => draw_proj_list(f, app, body[0]),
    }
    draw_detail(f, app, body[1]);
    draw_footer(f, app, layout[3]);

    // Overlays
    match &app.mode {
        AppMode::Confirm => { let items = get_confirm_items(app); draw_confirm(f, area, &items, app.selected_bytes()); }
        AppMode::RunCommand { command, description } => {
            let cmd = command.clone(); let desc = description.clone();
            draw_run_command(f, area, &cmd, &desc);
        }
        AppMode::CommandOutput { output, success } => {
            let out = output.clone(); let ok = *success;
            draw_command_output(f, area, &out, ok);
        }
        AppMode::Deleting { .. } => draw_deleting(f, area, app),
        AppMode::Done => draw_done(f, app, area),
        _ => {}
    }
}

fn draw_header(f: &mut Frame, app: &App, area: Rect) {
    let sel_info = if app.selected_bytes() > 0 {
        format!("  ·  {} selected", format_size(app.selected_bytes()))
    } else { String::new() };

    let line = Line::from(vec![
        Span::styled(" 🧹 Scrub  ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
        Span::styled(format_size(app.report.total_size), Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
        Span::styled(" found in cache locations", Style::default().fg(Color::DarkGray)),
        Span::styled(sel_info, Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
    ]);
    let block = Block::default().borders(Borders::ALL).border_style(Style::default().fg(Color::Cyan));
    f.render_widget(Paragraph::new(line).block(block), area);
}

fn draw_tabs(f: &mut Frame, app: &App, area: Rect) {
    let titles = vec![
        format!("  Computer caches{}  ", if app.selected.is_empty() { String::new() } else { format!(" ({})", app.selected.len()) }),
        format!("  Project caches{}  ", if app.proj_selected.is_empty() { String::new() } else { format!(" ({})", app.proj_selected.len()) }),
    ];
    let selected = match app.tab { ActiveTab::Caches => 0, ActiveTab::Projects => 1 };
    let tabs = Tabs::new(titles)
        .select(selected)
        .style(Style::default().fg(Color::DarkGray))
        .highlight_style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))
        .divider("|");
    f.render_widget(tabs, area);
}

fn fit_name(name: &str, width: usize) -> String {
    if name.chars().count() > width {
        format!("{}…", name.chars().take(width.saturating_sub(1)).collect::<String>())
    } else {
        format!("{name:<width$}")
    }
}

fn display_path(path: &Path) -> String {
    if let Some(home) = dirs::home_dir() {
        if let Ok(relative) = path.strip_prefix(home) {
            return format!("~/{}", relative.display());
        }
    }
    path.display().to_string()
}

fn draw_cache_list(f: &mut Frame, app: &mut App, area: Rect) {
    let name_width = (area.width as usize).saturating_sub(28).max(1);
    let items: Vec<ListItem> = app.rows.iter().enumerate().map(|(idx, row)| {
        let is_sel = app.selected.contains(&idx);
        match &row.kind {
            RowKind::EcoHeader { .. } => {
                let size_s = if row.size_bytes > 0 { format!("  {}", format_size(row.size_bytes)) } else { String::new() };
                ListItem::new(Line::from(vec![
                    Span::styled("  ▸ ", Style::default().fg(Color::Cyan)),
                    Span::styled(row.label.clone(), Style::default().add_modifier(Modifier::BOLD)),
                    Span::styled(size_s, Style::default().fg(Color::DarkGray)),
                ]))
            }
            RowKind::CommandItem { .. } => {
                let safety = if row.safe { Color::Green } else { Color::Yellow };
                let name = row.label.trim();
                ListItem::new(Line::from(vec![
                    Span::styled("  [run] ", Style::default().fg(Color::Blue)),
                    Span::styled(fit_name(name.trim_start_matches("⚡ "), name_width), Style::default().fg(Color::Blue)),
                    Span::styled(if row.safe { "" } else { " ⚠ caution" }, Style::default().fg(safety)),
                ]))
            }
            _ => {
                let check = if is_sel { "[✓]" } else { "[ ]" };
                let check_color = if is_sel { Color::Yellow } else { Color::DarkGray };
                let name = row.label.trim();
                ListItem::new(Line::from(vec![
                    Span::styled(format!("  {} ", check), Style::default().fg(check_color)),
                    Span::styled(fit_name(name, name_width), Style::default()),
                    Span::styled(format!("{:>10}", format_size(row.size_bytes)), Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
                    Span::styled(if row.safe { "" } else { " ⚠" }, Style::default().fg(Color::Yellow)),
                ]))
            }
        }
    }).collect();

    let block = Block::default()
        .title(" Computer caches ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray));
    let list = List::new(items).block(block)
        .highlight_style(Style::default().bg(Color::from_u32(0x1a2035)).add_modifier(Modifier::BOLD))
        .highlight_symbol("▶ ");
    f.render_stateful_widget(list, area, &mut app.list_state);
}

fn draw_proj_list(f: &mut Frame, app: &mut App, area: Rect) {
    let name_width = (area.width as usize).saturating_sub(29).max(1);
    let items: Vec<ListItem> = app.proj_rows.iter().enumerate().map(|(idx, row)| {
        let is_sel = app.proj_selected.contains(&idx);
        match &row.kind {
            ProjRowKind::Header { .. } => {
                ListItem::new(Line::from(vec![
                    Span::styled(" ◈ ", Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD)),
                    Span::styled(fit_name(row.label.trim(), name_width + 4), Style::default().add_modifier(Modifier::BOLD)),
                    Span::styled(format!("  {}", format_size(row.size_bytes)), Style::default().fg(Color::Yellow)),
                ]))
            }
            ProjRowKind::Item { .. } => {
                let check = if is_sel { "[✓]" } else { "[ ]" };
                let check_color = if is_sel { Color::Yellow } else { Color::DarkGray };
                let name = row.label.trim();
                ListItem::new(Line::from(vec![
                    Span::styled(format!("   {} ", check), Style::default().fg(check_color)),
                    Span::styled(fit_name(name, name_width), Style::default()),
                    Span::styled(format!("{:>10}", format_size(row.size_bytes)), Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
                    Span::styled(if row.safe { "" } else { " ⚠" }, Style::default().fg(Color::Yellow)),
                ]))
            }
        }
    }).collect();

    let project_count = app.report.projects.iter().filter(|p| p.total_size > 0).count();
    let block = Block::default()
        .title(format!(" Project caches ({} {}) ", project_count, if project_count == 1 { "project" } else { "projects" }))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray));

    if items.is_empty() {
        let msg = "\n\nNo project caches found.";
        let para = Paragraph::new(msg)
            .alignment(ratatui::layout::Alignment::Center)
            .style(Style::default().fg(Color::DarkGray))
            .block(block);
        f.render_widget(para, area);
        return;
    }

    let list = List::new(items).block(block)
        .highlight_style(Style::default().bg(Color::from_u32(0x1a2035)).add_modifier(Modifier::BOLD))
        .highlight_symbol("▶ ");
    f.render_stateful_widget(list, area, &mut app.proj_list_state);
}

fn draw_detail(f: &mut Frame, app: &App, area: Rect) {
    let block = Block::default()
        .title(" Details ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let Some(d) = app.current_detail() else { return; };

    let age = d.last_modified.map(format_age).unwrap_or_else(|| "—".to_string());
    let safety_style = if d.safe { Style::default().fg(Color::Green) } else { Style::default().fg(Color::Yellow) };

    let mut lines: Vec<Line> = vec![
        Line::from(Span::styled(d.name.clone(), Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))),
        Line::from(""),
    ];

    if !d.selectable {
        if d.size_bytes > 0 {
            lines.push(Line::from(format!("Measured size  {}", format_size(d.size_bytes))));
        }
        if !d.path_str.is_empty() && d.path_str != "." {
            lines.push(Line::from(""));
            lines.push(Line::from("Project directory"));
            lines.push(Line::from(d.path_str));
        }
        lines.push(Line::from(""));
        lines.push(Line::from("Choose a cache below to see what can be removed."));
        f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), inner);
        return;
    }

    if d.is_command {
        lines.push(Line::from(Span::styled("⚡ Command Action", Style::default().fg(Color::Blue).add_modifier(Modifier::BOLD))));
        if let Some(desc) = &d.command_desc {
            lines.push(Line::from(""));
            for l in desc.lines() {
                lines.push(Line::from(Span::styled(format!("  {}", l), Style::default().fg(Color::Gray))));
            }
        }
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  [Space/Enter] Run this command",
            Style::default().fg(Color::Blue).add_modifier(Modifier::BOLD),
        )));
    } else {
        if d.size_bytes > 0 {
            lines.push(Line::from(vec![
                Span::styled("Size     ", Style::default().fg(Color::DarkGray)),
                Span::styled(format_size(d.size_bytes), Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
            ]));
        }
        lines.push(Line::from(vec![
            Span::styled("Modified ", Style::default().fg(Color::DarkGray)),
            Span::styled(age, Style::default().fg(Color::White)),
        ]));
        lines.push(Line::from(vec![
            Span::styled("Safe     ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                if d.safe { "Yes — safe to delete" } else { "⚠ Caution — review first" },
                safety_style,
            ),
        ]));
        if let Some(expl) = &d.explanation {
            lines.push(Line::from(""));
            let (label, label_color) = if d.safe {
                ("Why it's safe", Color::Green)
            } else {
                ("⚠ Important — read before deleting", Color::Yellow)
            };
            lines.push(Line::from(Span::styled(label, Style::default().fg(label_color))));
            for l in expl.trim().lines() {
                lines.push(Line::from(Span::styled(format!("  {}", l), Style::default().fg(Color::Gray))));
            }
        }
        if d.selectable {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                if d.selected { "  [Space] Deselect" } else { "  [Space] Select for deletion" },
                Style::default().fg(Color::Blue).add_modifier(Modifier::BOLD),
            )));
        }
        if !d.path_str.is_empty() && d.path_str != "." {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled("Path", Style::default().fg(Color::DarkGray))));
            lines.push(Line::from(Span::styled(
                d.path_str.clone(),
                Style::default().fg(Color::DarkGray).add_modifier(Modifier::ITALIC),
            )));
        }
    }

    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), inner);
}

fn draw_footer(f: &mut Frame, app: &App, area: Rect) {
    let hint = if area.width >= 68 {
        " Space Select  d Delete  c Clear  Tab Views  q Quit"
    } else if area.width >= 48 {
        " Space Select  d Delete  Tab Views  q Quit"
    } else {
        " Space Select  d Delete  q Quit"
    };
    let style = if app.selected_count() > 0 { Style::default().fg(Color::Yellow) } else { Style::default().fg(Color::DarkGray) };
    let block = Block::default().borders(Borders::ALL).border_style(Style::default().fg(Color::DarkGray));
    f.render_widget(Paragraph::new(hint).style(style).block(block), area);
}

fn get_confirm_items(app: &App) -> Vec<(String, u64)> {
    match app.tab {
        ActiveTab::Caches => {
            let mut v: Vec<usize> = app.selected.iter().cloned().collect(); v.sort();
            v.iter().filter_map(|i| app.rows.get(*i)).map(|r| (r.label.trim().to_string(), r.size_bytes)).collect()
        }
        ActiveTab::Projects => {
            let mut v: Vec<usize> = app.proj_selected.iter().cloned().collect(); v.sort();
            v.iter().filter_map(|i| app.proj_rows.get(*i)).map(|r| (r.label.trim().to_string(), r.size_bytes)).collect()
        }
    }
}

fn draw_confirm(f: &mut Frame, area: Rect, items: &[(String, u64)], total_bytes: u64) {
    let popup = centered_rect(70, 65, area);
    f.render_widget(Clear, popup);
    let block = Block::default()
        .title(" ⚠  Confirm Deletion ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD));
    let inner = block.inner(popup);
    f.render_widget(block, popup);

    let mut lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            format!("  Remove {} item{} ({})?", items.len(), if items.len() == 1 { "" } else { "s" }, format_size(total_bytes)),
            Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
    ];
    for (name, size) in items.iter().take(3) {
        lines.push(Line::from(vec![
            Span::styled("  • ", Style::default().fg(Color::Yellow)),
            Span::styled(name.clone(), Style::default().fg(Color::Gray)),
            Span::styled(format!("  ({})", format_size(*size)), Style::default().fg(Color::White)),
        ]));
    }
    if items.len() > 3 {
        lines.push(Line::from(Span::styled(format!("  ... and {} more", items.len() - 3), Style::default().fg(Color::DarkGray))));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled("  Trash is recoverable; permanent deletion is not.", Style::default().fg(Color::DarkGray))));
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled("  [Y] Move to Trash   ", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
        Span::styled("[P] Delete Permanently", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)),
    ]));
    lines.push(Line::from(Span::styled("  [N/Esc] Cancel", Style::default().fg(Color::DarkGray).add_modifier(Modifier::BOLD))));
    f.render_widget(Paragraph::new(lines), inner);
}

fn draw_run_command(f: &mut Frame, area: Rect, command: &str, description: &str) {
    let popup = centered_rect(58, 40, area);
    f.render_widget(Clear, popup);
    let block = Block::default()
        .title(" ⚡ Run Command ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Blue).add_modifier(Modifier::BOLD));
    let inner = block.inner(popup);
    f.render_widget(block, popup);

    let lines = vec![
        Line::from(""),
        Line::from(Span::styled(format!("  {}", description), Style::default().fg(Color::White))),
        Line::from(""),
        Line::from(Span::styled("  Command:", Style::default().fg(Color::DarkGray))),
        Line::from(Span::styled(format!("  $ {}", command), Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))),
        Line::from(""),
        Line::from(vec![
            Span::styled("  [Y] Run it   ", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
            Span::styled("[N/Esc] Cancel", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)),
        ]),
    ];
    f.render_widget(Paragraph::new(lines), inner);
}

fn draw_command_output(f: &mut Frame, area: Rect, output: &str, success: bool) {
    let popup = centered_rect(65, 60, area);
    f.render_widget(Clear, popup);
    let color = if success { Color::Green } else { Color::Red };
    let block = Block::default()
        .title(if success { " ✓ Command Output " } else { " ✗ Command Failed " })
        .borders(Borders::ALL)
        .border_style(Style::default().fg(color).add_modifier(Modifier::BOLD));
    let inner = block.inner(popup);
    f.render_widget(block, popup);

    let mut lines: Vec<Line> = vec![Line::from("")];
    for l in output.lines().take(20) {
        lines.push(Line::from(Span::styled(format!("  {}", l), Style::default().fg(Color::Gray))));
    }
    if output.lines().count() > 20 {
        lines.push(Line::from(Span::styled("  ...", Style::default().fg(Color::DarkGray))));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled("  [Enter/Esc] Dismiss", Style::default().fg(Color::DarkGray))));
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), inner);
}

fn draw_deleting(f: &mut Frame, area: Rect, app: &App) {
    let AppMode::Deleting { current, done, total, permanent } = &app.mode else { return; };
    let popup = centered_rect(65, 30, area);
    f.render_widget(Clear, popup);
    let block = Block::default()
        .title(" Deleting... ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));
    let action = if *permanent { "Permanently deleting" } else { "Moving to Trash" };
    f.render_widget(Paragraph::new(format!("\n  {} ({} of {})\n  {}", action, done, total, current)).block(block), popup);
}

fn draw_done(f: &mut Frame, app: &App, area: Rect) {
    let popup = centered_rect(55, 50, area);
    f.render_widget(Clear, popup);
    let block = Block::default()
        .title(" ✓ Done ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Green).add_modifier(Modifier::BOLD));
    let inner = block.inner(popup);
    f.render_widget(block, popup);

    let sections = Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).split(inner);
    let visible = app.delete_log.len().min(sections[0].height.saturating_sub(3) as usize);
    let mut lines = vec![Line::from("  Results:"), Line::from("")];
    for entry in app.delete_log.iter().take(visible) {
        let color = if entry.starts_with('✓') { Color::Green } else { Color::Red };
        lines.push(Line::from(Span::styled(format!("  {}", entry), Style::default().fg(color))));
    }
    if app.delete_log.len() > visible {
        lines.push(Line::from(Span::styled(format!("  ... and {} more", app.delete_log.len() - visible), Style::default().fg(Color::DarkGray))));
    }
    f.render_widget(Paragraph::new(lines), sections[0]);
    f.render_widget(Paragraph::new("  [Enter] Dismiss").style(Style::default().fg(Color::DarkGray)), sections[1]);
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let v = Layout::vertical([
        Constraint::Percentage((100 - percent_y) / 2),
        Constraint::Percentage(percent_y),
        Constraint::Percentage((100 - percent_y) / 2),
    ]).split(r);
    Layout::horizontal([
        Constraint::Percentage((100 - percent_x) / 2),
        Constraint::Percentage(percent_x),
        Constraint::Percentage((100 - percent_x) / 2),
    ]).split(v[1])[1]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::scan::{CacheItem, CategoryReport, ProjectReport, ProjectSummary};
    use ratatui::backend::TestBackend;

    #[test]
    fn scanning_screen_shows_spinner_and_current_folder() {
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal.draw(|f| draw_scanning(f, "◐", Path::new("/code/tal-app/.next"))).unwrap();
        let screen = terminal.backend().to_string();
        assert!(screen.contains("◐  Scanning caches"));
        assert!(screen.contains("tal-app / .next"));
        assert!(!screen.contains("entries"));
    }

    #[test]
    fn deletion_result_keeps_dismiss_visible_when_log_is_long() {
        let report = ScanReport { categories: vec![], total_size: 0, projects: vec![] };
        let mut app = App::new(report);
        app.delete_log = (0..20).map(|i| format!("✓ Deleted cache {i}")).collect();
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal.draw(|f| draw_done(f, &app, f.area())).unwrap();
        let screen = terminal.backend().to_string();
        assert!(screen.contains("[Enter] Dismiss"));
        assert!(screen.contains("... and "));
    }

    #[test]
    fn footer_keeps_quit_hint_at_narrow_width() {
        let report = ScanReport { categories: vec![], total_size: 0, projects: vec![] };
        let app = App::new(report);
        let mut terminal = Terminal::new(TestBackend::new(40, 3)).unwrap();
        terminal.draw(|f| draw_footer(f, &app, f.area())).unwrap();
        assert!(terminal.backend().to_string().contains("q Quit"));
    }

    #[test]
    fn shows_only_nonempty_caches_in_their_respective_tabs() {
        let cache = CacheItem {
            name: "DerivedData".into(), path: "DerivedData".into(), size_bytes: 10,
            file_count: 1, dir_count: 0, safe: true, explanation: None,
            status: ItemStatus::Found, last_modified: None,
        };
        let project_item = CacheItem { name: "Pods".into(), size_bytes: 5, ..cache.clone() };
        let project = ProjectReport {
            name: "tal-app".into(), path: "grapevine/tal-app".into(),
            items: vec![project_item.clone()], total_size: 5, last_modified: None,
        };
        let report = ScanReport {
            categories: vec![CategoryReport {
                category: "Xcode".into(), global_items: vec![cache],
                projects: vec![project], command_items: vec![], total_size: 15,
                is_detected: true,
            }],
            total_size: 15,
            projects: vec![
                ProjectSummary {
                    name: "empty".into(), path: "grapevine/empty".into(),
                    ecosystem: "Xcode".into(), items: vec![], total_size: 0, last_modified: None,
                },
                ProjectSummary {
                    name: "tal-app".into(), path: "grapevine/tal-app".into(),
                    ecosystem: "Xcode".into(), items: vec![project_item.clone()], total_size: 5, last_modified: None,
                },
                ProjectSummary {
                    name: "paperboy (app)".into(), path: "grapevine/paperboy/app".into(),
                    ecosystem: "Expo".into(), items: vec![project_item], total_size: 5, last_modified: None,
                },
            ],
        };

        let caches = build_cache_rows(&report);
        assert_eq!(caches.len(), 2);
        assert_eq!(caches[0].size_bytes, 10);
        assert!(matches!(caches[1].kind, RowKind::GlobalItem { .. }));
        let projects = build_proj_rows(&report);
        assert_eq!(projects.len(), 4);
        assert_eq!(projects[0].label, "grapevine / tal-app");
        assert_eq!(projects[2].label, "paperboy (app)");
    }

    #[test]
    fn discovers_common_and_current_project_roots() {
        let root = std::env::temp_dir().join(format!("scrub-roots-{}", std::process::id()));
        let home = root.join("home");
        let project_root = root.join("grapevine");
        let cwd = project_root.join("scrub");
        std::fs::create_dir_all(home.join("Projects")).unwrap();
        std::fs::create_dir_all(home.join("Documents")).unwrap();
        std::fs::create_dir_all(&cwd).unwrap();
        std::fs::write(cwd.join("Cargo.toml"), "").unwrap();

        let config = Config { project_roots: vec![], ..Config::default() };
        let found = discover_scan_roots(&home, &cwd, &config);
        assert_eq!(found.len(), 2);
        assert!(found.contains(&home.join("Projects")));
        assert!(found.contains(&project_root));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn deletion_reports_progress_before_and_after_the_file_is_removed() {
        let root = std::env::temp_dir().join(format!("scrub-delete-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("cache");
        std::fs::write(&path, "x").unwrap();
        let item = CacheItem {
            name: "cache".into(), path: path.clone(), size_bytes: 1,
            file_count: 1, dir_count: 0, safe: true, explanation: None,
            status: ItemStatus::Found, last_modified: None,
        };
        let report = ScanReport {
            categories: vec![], total_size: 1,
            projects: vec![ProjectSummary {
                name: "project".into(), path: root.clone(), ecosystem: "test".into(),
                items: vec![item], total_size: 1, last_modified: None,
            }],
        };
        let mut app = App::new(report);
        app.tab = ActiveTab::Projects;
        app.proj_selected.insert(1);
        let mut updates = Vec::new();
        app.do_delete(true, |app| {
            if let AppMode::Deleting { done, .. } = &app.mode {
                updates.push((*done, path.exists()));
            }
            Ok(())
        }).unwrap();
        assert_eq!(updates, vec![(0, true), (1, false)]);
        assert_eq!(app.mode, AppMode::Done);
        std::fs::remove_dir_all(root).unwrap();
    }
}
