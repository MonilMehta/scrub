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
  - **Computer Caches:** View caches stored outside individual projects (e.g., Xcode, Android, Docker).
  - **Project Caches:** View nonempty caches inside individual projects, sorted by size.
- **Detailed Explanations:** Explains exactly *why* a folder is safe to delete and how it will be regenerated.

## 🚀 Installation

### Option 1: Homebrew (macOS / Linux)
The easiest way to install Scrub is via our Homebrew tap:

```bash
brew install MonilMehta/scrub/scrub
```

### Option 2: cURL Install Script (macOS / Linux / Windows)
You can download and install the pre-compiled binary directly:

```bash
curl -fsSL https://raw.githubusercontent.com/MonilMehta/scrub/master/install.sh | bash
```

### Option 3: Cargo (Rust Developers)
If you already have Rust installed, you can build from source:

```bash
cargo install --git https://github.com/MonilMehta/scrub
```

Or just run it directly from the source code:
```bash
cargo run --release -- dashboard
```

### Updating

After a new release is published, update using the same installation method:

```bash
brew update && brew upgrade MonilMehta/scrub/scrub
```

Or rerun the install script, which downloads the latest release:

```bash
curl -fsSL https://raw.githubusercontent.com/MonilMehta/scrub/master/install.sh | bash
```

For Cargo installs, run:

```bash
cargo install --git https://github.com/MonilMehta/scrub --force
```

Homebrew updates require the tap formula to be updated to the new release.

## 🎮 Usage

Launch the interactive dashboard:

```bash
scrub dashboard
```

With no paths, the dashboard suggests existing project directories. It always asks you to confirm the directories or enter a different one before scanning. A spinner appears during the scan; press `q` or `Esc` to cancel.

Or, you can provide directories for Scrub to confirm:
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
