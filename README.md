# 🧹 Scrub

**Scrub** is a modern terminal application (TUI) that helps developers reclaim disk space by safely deleting rebuildable caches, build artifacts, simulator data, package caches, and temporary files.

Unlike generic disk analyzers such as `ncdu`, `gdu`, or `dua`, **Scrub understands development ecosystems**. It knows what is safe to delete and what isn't, so you can confidently clean up gigabytes of dead weight without fear of losing your source code or documents.

Think: `lazygit` + `CleanMyMac` + `cargo` + `k9s`.

## ✨ Features

- **Safe by Default:** Sends deleted files to your OS Recycle Bin / Trash by default. It never deletes your source code.
- **Ecosystem Aware:** Out of the box, Scrub understands caches for:
  - Xcode & SwiftPM
  - Node.js, Next.js, React Native, Expo
  - Android Studio & Gradle
  - Docker (including smart command execution for `docker system prune`)
  - CocoaPods, Homebrew, Python, Flutter, and more.
- **Cross-Platform:** Works natively on macOS, Linux, and Windows.
- **Dual Views:**
  - **Caches Tab:** View global ecosystems (e.g., all of Android, all of Xcode, all of Docker).
  - **Projects Tab:** View space used by individual projects across your machine, sorted by size.
- **Detailed Explanations:** Explains exactly *why* a folder is safe to delete and how it will be regenerated.

## 🚀 Installation

*Note: Pre-compiled binaries via Homebrew and cURL install scripts are coming soon!*

For now, you can install Scrub via Cargo if you have Rust installed:

```bash
cargo install --path .
```

Or just run it directly from the source code:
```bash
cargo run --release -- dashboard
```

## 🎮 Usage

Launch the interactive dashboard:

```bash
scrub dashboard
```

Or, you can tell Scrub exactly which directories to scan:
```bash
scrub dashboard ~/Code ~/Projects ~/MyWeirdFolder
```

### Controls
- `↑ / ↓` or `j / k`: Navigate the list
- `Space` or `Enter`: Select an item for deletion
- `Tab`: Switch between **Developer Caches** and **Projects** views
- `d`: Open the deletion confirmation dialog
- `c`: Clear selection
- `q`: Quit the application

## ⚙️ Configuration

On first run, Scrub automatically creates a configuration file at `~/.config/scrub/config.toml`. 

By default, Scrub will look for projects in standard developer directories like `~/Projects`, `~/Code`, and `~/Developer`. You can easily add more directories to this list by editing the configuration file.

## 🛠️ Adding New Plugins

Scrub's intelligence is powered by simple YAML plugins. Even though they are bundled into the binary, the structure is incredibly simple to extend. 

If you want to add support for a new language or framework, you just need to define the paths and whether they are safe to delete. 

```yaml
name: Node
description: Node.js and npm caches
detectors:
  - "package.json"
project_items:
  - name: node_modules
    paths:
      - node_modules
    safe: true
    delete: directory
    explanation: |
      Project dependencies. Restored via `npm install` or `yarn install`.
```

## 🛡️ License

MIT License.
