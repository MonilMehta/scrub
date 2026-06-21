use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode},
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
use std::io;
use std::process::Command as SysCommand;

use crate::core::config::Config;
use crate::core::scanner::Scanner;
use crate::models::scan::{ItemStatus, ScanReport};
use crate::plugins::manager::PluginManager;
use crate::utils::age::format_age;
use crate::utils::human_size::format_size;

// ── Row types ─────────────────────────────────────────────────────────────────

#[derive(Clone, Debug)]
#[allow(dead_code)]
enum RowKind {
    EcoHeader { cat_idx: usize },
    GlobalItem { cat_idx: usize, item_idx: usize },
    CommandItem { cat_idx: usize, cmd_idx: usize },
    ProjectHeader { cat_idx: usize, proj_idx: usize },
    ProjectItem { cat_idx: usize, proj_idx: usize, item_idx: usize },
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
    ecosystem: String,
    explanation: Option<String>,
}

// ── Dialog state ──────────────────────────────────────────────────────────────

#[derive(Debug, PartialEq)]
enum AppMode {
    Browse,
    Confirm,
    RunCommand { command: String, description: String },
    CommandOutput { output: String, success: bool },
    Deleting,
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

    fn do_delete(&mut self, permanent: bool) {
        self.mode = AppMode::Deleting;
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

        for (path, size) in paths {
            let label = path.file_name().map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.display().to_string());
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
                            if let RowKind::ProjectItem { cat_idx, proj_idx, .. } = self.rows[i].kind {
                                for j in 0..self.rows.len() {
                                    if let RowKind::ProjectHeader { cat_idx: c, proj_idx: p } = self.rows[j].kind {
                                        if c == cat_idx && p == proj_idx { self.rows[j].size_bytes = self.rows[j].size_bytes.saturating_sub(size); }
                                    }
                                    if let RowKind::EcoHeader { cat_idx: c } = self.rows[j].kind {
                                        if c == cat_idx { self.rows[j].size_bytes = self.rows[j].size_bytes.saturating_sub(size); }
                                    }
                                }
                            } else if let RowKind::GlobalItem { cat_idx, .. } = self.rows[i].kind {
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
        }

        match self.tab {
            ActiveTab::Caches => { self.selected.clear(); self.total_selected_bytes = 0; }
            ActiveTab::Projects => { self.proj_selected.clear(); self.proj_total_selected = 0; }
        }
        self.mode = AppMode::Done;
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
                    path_str: row.path.display().to_string(),
                    size_bytes: row.size_bytes,
                    total_size: self.report.total_size,
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
                    path_str: row.path.display().to_string(),
                    size_bytes: row.size_bytes,
                    total_size: self.report.total_size,
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
    total_size: u64,
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
        rows.push(Row {
            kind: RowKind::EcoHeader { cat_idx },
            selectable: false,
            label: format!(" {} {}", if cat.is_detected { "✓" } else { "✗" }, cat.category),
            size_bytes: cat.total_size,
            safe: true,
            last_modified: None,
            path: std::path::PathBuf::new(),
            action: RowAction::None,
            explanation: None,
            command_desc: None,
        });
        if !cat.is_detected { continue; }

        for (item_idx, item) in cat.global_items.iter().enumerate() {
            if item.status != ItemStatus::Found { continue; }
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

        for (proj_idx, proj) in cat.projects.iter().enumerate() {
            rows.push(Row {
                kind: RowKind::ProjectHeader { cat_idx, proj_idx },
                selectable: false,
                label: format!("   → {}", proj.name),
                size_bytes: proj.total_size,
                safe: true,
                last_modified: proj.last_modified,
                path: proj.path.clone(),
                action: RowAction::None,
                explanation: None,
                command_desc: None,
            });
            for (item_idx, item) in proj.items.iter().enumerate() {
                if item.status != ItemStatus::Found || item.size_bytes == 0 { continue; }
                rows.push(Row {
                    kind: RowKind::ProjectItem { cat_idx, proj_idx, item_idx },
                    selectable: true,
                    label: format!("      {}", item.name),
                    size_bytes: item.size_bytes,
                    safe: item.safe,
                    last_modified: item.last_modified,
                    path: item.path.clone(),
                    action: RowAction::Delete,
                    explanation: item.explanation.clone(),
                    command_desc: None,
                });
            }
        }
    }
    rows
}

fn build_proj_rows(report: &ScanReport) -> Vec<ProjRow> {
    let mut rows = Vec::new();
    for (proj_idx, proj) in report.projects.iter().enumerate() {
        let _age = proj.last_modified.map(format_age).unwrap_or_default();
        rows.push(ProjRow {
            kind: ProjRowKind::Header { proj_idx },
            selectable: false,
            label: format!(" {} ", proj.name),
            size_bytes: proj.total_size,
            safe: true,
            last_modified: proj.last_modified,
            path: proj.path.clone(),
            ecosystem: proj.ecosystem.clone(),
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
                ecosystem: String::new(),
                explanation: item.explanation.clone(),
            });
        }
    }
    rows
}

// ── Entry point ───────────────────────────────────────────────────────────────

pub fn run(paths: Vec<String>) -> Result<()> {
    println!("Scanning developer caches...");
    let config = Config::load();
    let mut plugin_manager = PluginManager::new();
    let _ = plugin_manager.load_bundled();

    let scan_roots = if !paths.is_empty() { paths } else { config.project_roots.clone() };
    let scanner = Scanner::new(plugin_manager.plugins);
    let report = scanner.scan(&scan_roots);

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new(report);
    let res = event_loop(&mut terminal, &mut app);

    let receipt = app.receipt.clone();

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    if let Err(e) = res { println!("{:?}", e); }

    if !receipt.is_empty() {
        use crossterm::style::Stylize;
        println!("\n🧾 Scrub Receipt");
        println!("{}", "─".repeat(50));
        let mut total = 0;
        for (label, size, perm) in receipt {
            let action = if perm { "Permanently deleted".red() } else { "Moved to Trash".yellow() };
            println!("✓ {} {:<20} {}", action, label, format_size(size).bold());
            total += size;
        }
        println!("{}", "─".repeat(50));
        println!("Total reclaimed: {}\n", format_size(total).green().bold());
    }

    Ok(())
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
                    KeyCode::Char('y') | KeyCode::Char('Y') => app.do_delete(false),
                    KeyCode::Char('p') | KeyCode::Char('P') => app.do_delete(true),
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
                AppMode::Deleting => {}
            }
        }
    }
}

// ── Drawing ───────────────────────────────────────────────────────────────────

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
        Constraint::Percentage(52),
        Constraint::Percentage(48),
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
        AppMode::Deleting => draw_deleting(f, area),
        AppMode::Done => draw_done(f, app, area),
        _ => {}
    }
}

fn draw_header(f: &mut Frame, app: &App, area: Rect) {
    let sel_info = if app.selected_bytes() > 0 {
        format!("  ·  {} selected for deletion", format_size(app.selected_bytes()))
    } else { String::new() };

    let line = Line::from(vec![
        Span::styled(" 🧹 Scrub  ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
        Span::styled(format_size(app.report.total_size), Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
        Span::styled(" reclaimable", Style::default().fg(Color::DarkGray)),
        Span::styled(sel_info, Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
    ]);
    let block = Block::default().borders(Borders::ALL).border_style(Style::default().fg(Color::Cyan));
    f.render_widget(Paragraph::new(line).block(block), area);
}

fn draw_tabs(f: &mut Frame, app: &App, area: Rect) {
    let titles = vec!["  Caches  ", "  Projects  "];
    let selected = match app.tab { ActiveTab::Caches => 0, ActiveTab::Projects => 1 };
    let tabs = Tabs::new(titles)
        .select(selected)
        .style(Style::default().fg(Color::DarkGray))
        .highlight_style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))
        .divider("|");
    f.render_widget(tabs, area);
}

fn draw_cache_list(f: &mut Frame, app: &mut App, area: Rect) {
    let total = app.report.total_size.max(1);
    let items: Vec<ListItem> = app.rows.iter().enumerate().map(|(idx, row)| {
        let is_sel = app.selected.contains(&idx);
        match &row.kind {
            RowKind::EcoHeader { .. } => {
                let sym = if row.label.contains('✓') { "✓" } else { "✗" };
                let color = if row.label.contains('✓') { Color::Green } else { Color::DarkGray };
                let name = row.label.trim().trim_start_matches("✓ ").trim_start_matches("✗ ");
                let size_s = if row.size_bytes > 0 { format!("  {}", format_size(row.size_bytes)) } else { String::new() };
                ListItem::new(Line::from(vec![
                    Span::styled(format!(" {} ", sym), Style::default().fg(color).add_modifier(Modifier::BOLD)),
                    Span::styled(name.to_string(), Style::default().add_modifier(Modifier::BOLD)),
                    Span::styled(size_s, Style::default().fg(Color::DarkGray)),
                ]))
            }
            RowKind::CommandItem { .. } => {
                let safety = if row.safe { Color::Green } else { Color::Yellow };
                let name = row.label.trim();
                ListItem::new(Line::from(vec![
                    Span::styled("  [⚡] ", Style::default().fg(Color::Blue)),
                    Span::styled(format!("{:<24}", name.trim_start_matches("⚡ ")), Style::default().fg(Color::Blue)),
                    Span::styled(if row.safe { "  ✓ safe" } else { "  ⚠ caution" }, Style::default().fg(safety)),
                ]))
            }
            RowKind::ProjectHeader { .. } => {
                let age = row.last_modified.map(format_age).map(|a| format!("  {}", a)).unwrap_or_default();
                let name = row.label.trim().trim_start_matches("→ ");
                ListItem::new(Line::from(vec![
                    Span::raw("   "),
                    Span::styled("→ ", Style::default().fg(Color::Cyan)),
                    Span::styled(name.to_string(), Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
                    Span::styled(format!("  {}", format_size(row.size_bytes)), Style::default().fg(Color::Yellow)),
                    Span::styled(age, Style::default().fg(Color::DarkGray)),
                ]))
            }
            _ => {
                let check = if is_sel { "[✓]" } else { "[ ]" };
                let check_color = if is_sel { Color::Yellow } else { Color::DarkGray };
                let pct = if row.size_bytes > 0 {
                    format!(" {:>4.1}%", (row.size_bytes as f64 / total as f64) * 100.0)
                } else { String::new() };
                let safety = if row.safe { Color::Green } else { Color::Yellow };
                let name = row.label.trim();
                ListItem::new(Line::from(vec![
                    Span::styled(format!("  {} ", check), Style::default().fg(check_color)),
                    Span::styled(format!("{:<22}", name), Style::default()),
                    Span::styled(format!("{:>10}", format_size(row.size_bytes)), Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
                    Span::styled(format!("{:<6}", pct), Style::default().fg(Color::DarkGray)),
                    Span::styled(if row.safe { "  ✓ safe" } else { "  ⚠" }, Style::default().fg(safety)),
                ]))
            }
        }
    }).collect();

    let block = Block::default()
        .title(" Developer Caches ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray));
    let list = List::new(items).block(block)
        .highlight_style(Style::default().bg(Color::from_u32(0x1a2035)).add_modifier(Modifier::BOLD))
        .highlight_symbol("▶ ");
    f.render_stateful_widget(list, area, &mut app.list_state);
}

fn draw_proj_list(f: &mut Frame, app: &mut App, area: Rect) {
    let total = app.report.total_size.max(1);
    let items: Vec<ListItem> = app.proj_rows.iter().enumerate().map(|(idx, row)| {
        let is_sel = app.proj_selected.contains(&idx);
        match &row.kind {
            ProjRowKind::Header { .. } => {
                let age = row.last_modified.map(format_age).map(|a| format!("  {}", a)).unwrap_or_default();
                let pct = if row.size_bytes > 0 {
                    format!("  {:.1}%", (row.size_bytes as f64 / total as f64) * 100.0)
                } else { String::new() };
                ListItem::new(Line::from(vec![
                    Span::styled(" ◈ ", Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD)),
                    Span::styled(row.label.trim().to_string(), Style::default().add_modifier(Modifier::BOLD)),
                    Span::styled(format!("  {}", format_size(row.size_bytes)), Style::default().fg(Color::Yellow)),
                    Span::styled(pct, Style::default().fg(Color::DarkGray)),
                    Span::styled(age, Style::default().fg(Color::DarkGray)),
                    Span::styled(format!("  [{}]", row.ecosystem), Style::default().fg(Color::Blue)),
                ]))
            }
            ProjRowKind::Item { .. } => {
                let check = if is_sel { "[✓]" } else { "[ ]" };
                let check_color = if is_sel { Color::Yellow } else { Color::DarkGray };
                let safety = if row.safe { Color::Green } else { Color::Yellow };
                let pct = if row.size_bytes > 0 {
                    format!(" {:>4.1}%", (row.size_bytes as f64 / total as f64) * 100.0)
                } else { String::new() };
                let name = row.label.trim();
                ListItem::new(Line::from(vec![
                    Span::styled(format!("   {} ", check), Style::default().fg(check_color)),
                    Span::styled(format!("{:<20}", name), Style::default()),
                    Span::styled(format!("{:>10}", format_size(row.size_bytes)), Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
                    Span::styled(format!("{:<6}", pct), Style::default().fg(Color::DarkGray)),
                    Span::styled(if row.safe { "  ✓" } else { "  ⚠" }, Style::default().fg(safety)),
                ]))
            }
        }
    }).collect();

    let block = Block::default()
        .title(format!(" Projects ({} found) ", app.report.projects.len()))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray));

    if items.is_empty() {
        let msg = "\n\n\nNo projects found.\n\nEdit ~/.config/scrub/config.toml to add your code directories,\nor run `scrub dashboard /path/to/projects`";
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

    let pct = (d.size_bytes as f64 / d.total_size.max(1) as f64) * 100.0;
    let age = d.last_modified.map(format_age).unwrap_or_else(|| "—".to_string());
    let safety_style = if d.safe { Style::default().fg(Color::Green) } else { Style::default().fg(Color::Yellow) };

    let mut lines: Vec<Line> = vec![
        Line::from(Span::styled(d.name.clone(), Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))),
        Line::from(""),
    ];

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
            lines.push(Line::from(vec![
                Span::styled("Share    ", Style::default().fg(Color::DarkGray)),
                Span::styled(format!("{:.2}% of total", pct), Style::default().fg(Color::Yellow)),
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
        if !d.path_str.is_empty() && d.path_str != "." {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled("Path", Style::default().fg(Color::DarkGray))));
            lines.push(Line::from(Span::styled(
                d.path_str.clone(),
                Style::default().fg(Color::DarkGray).add_modifier(Modifier::ITALIC),
            )));
        }
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
    }

    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), inner);
}

fn draw_footer(f: &mut Frame, app: &App, area: Rect) {
    let n = app.selected_count();
    let hint = if n > 0 {
        format!(" [↑/↓] Navigate  [Space] Toggle  [d] Delete {} items ({})  [c] Clear  [Tab] Switch tab  [q] Quit",
            n, format_size(app.selected_bytes()))
    } else {
        " [↑/↓] Navigate  [Space/Enter] Select  [d] Delete selected  [Tab] Switch tab  [q] Quit".to_string()
    };
    let style = if n > 0 { Style::default().fg(Color::Yellow) } else { Style::default().fg(Color::DarkGray) };
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
    let popup = centered_rect(60, 55, area);
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
            format!("  Move {} items ({}) to Trash?", items.len(), format_size(total_bytes)),
            Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
    ];
    for (name, size) in items.iter().take(10) {
        lines.push(Line::from(vec![
            Span::styled("  • ", Style::default().fg(Color::Yellow)),
            Span::styled(name.clone(), Style::default().fg(Color::Gray)),
            Span::styled(format!("  ({})", format_size(*size)), Style::default().fg(Color::White)),
        ]));
    }
    if items.len() > 10 {
        lines.push(Line::from(Span::styled(format!("  ... and {} more", items.len() - 10), Style::default().fg(Color::DarkGray))));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled("  Items go to Trash — easily recoverable.", Style::default().fg(Color::DarkGray))));
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled("  [Y] Move to Trash   ", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
        Span::styled("[P] Delete Permanently   ", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)),
        Span::styled("[N/Esc] Cancel", Style::default().fg(Color::DarkGray).add_modifier(Modifier::BOLD)),
    ]));
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

fn draw_deleting(f: &mut Frame, area: Rect) {
    let popup = centered_rect(40, 20, area);
    f.render_widget(Clear, popup);
    let block = Block::default()
        .title(" Deleting... ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));
    f.render_widget(Paragraph::new("\n  Moving to Trash...").block(block), popup);
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

    let mut lines = vec![Line::from(""), Line::from("  Results:"), Line::from("")];
    for entry in &app.delete_log {
        let color = if entry.starts_with('✓') { Color::Green } else { Color::Red };
        lines.push(Line::from(Span::styled(format!("  {}", entry), Style::default().fg(color))));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled("  [Enter] Dismiss", Style::default().fg(Color::DarkGray))));
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
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
