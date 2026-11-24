# LiteCode

An ultra-lightweight, native **Arch Linux** code editor inspired by **Code - OSS / VS Code**, stripped down to just the essentials:
1. **Minimal VS Code Dark Editor** — Explorer folder sidebar with inline `+` file/folder creation and right-click delete on files and folders, tabbed editing, syntax highlighting, line numbers, smart Enter/Tab auto-indentation, comment toggling, and instant Find.
2. **Smart Auto-Formatter** — Built-in universal indentation formatter (C, C++, Rust, Python, JS/TS, HTML, XML, CSS, JSON, Go, Shell/`PKGBUILD`, Lua) plus seamless automatic use of system CLI formatters (`rustfmt`, `clang-format`, `prettier`, `ruff`/`black`, `gofmt`, `shfmt`, `xmllint`) when installed.
3. **Right-Side Built-in PTY Terminal** — Antigravity-style collapsible right sidebar (`Ctrl+\``) powered by a zero-dependency pure-Rust Linux PTY & 24-bit TrueColor VT/ANSI engine (supports `fish`, `zsh`, `bash`, Nerd Font Powerline prompts, lazy-spawned with 0 MB idle overhead when closed).

## Resource Comparison (Arch Linux)

| Metric | Code - OSS (Electron) | LiteCode (Rust + GTK4) |
| :--- | :--- | :--- |
| **Binary Size** | ~100 MB (+ Electron) | **611 KB (0.60 MB)** |
| **Private RAM** | ~250–380 MB | **~17.5 MB** (~35 MB PSS) |
| **Cold Startup** | ~600–1200 ms | **< 80 ms** |

## Keyboard Shortcuts

| Shortcut | Action |
| :--- | :--- |
| `Ctrl+Shift+I` / `Alt+Shift+F` | **Auto-Format Code & Indentation** |
| ``Ctrl+` `` | **Toggle Right-Side Terminal Sidebar** (`Ctrl+Shift+C` / `Ctrl+Shift+V` copy/paste in terminal) |
| ``Ctrl+Shift+` `` | Open a new terminal tab in the right sidebar |
| `Ctrl+PageUp` / `Ctrl+PageDown` | Switch between open terminal tabs |
| `Ctrl+S` | Save current file (runs auto-format first if **Format on Save** is checked) |
| `Ctrl+Shift+S` | Save As |
| `Ctrl+N` | New untitled tab |
| `Ctrl+O` | Open file |
| `Ctrl+Shift+O` | Open workspace folder in Explorer |
| `Ctrl+W` | Close active tab |
| `Ctrl+B` | Toggle Explorer sidebar |
| `Ctrl+F` | Find in current file (`Enter` next, `Shift+Enter` previous, `Esc` close) |
| `Ctrl+/` | Toggle line comment |
| `Tab` / `Shift+Tab` | Indent / Outdent line or multi-line selection |
| `Ctrl+Z` / `Ctrl+Y` | Undo / Redo (including undoing an entire Auto-Format in 1 step) |

## Quick Run & Build

```bash
# Run directly in release mode
cargo run --release

# Open a folder or specific file
./target/release/litecode .
./target/release/litecode src/main.rs
```

## Install as an Arch Linux Application

### Option A: User-local install (no `sudo` required)
```bash
chmod +x install-local.sh
./install-local.sh
```

### Option B: Native Arch `pacman` package (`PKGBUILD`)
```bash
cd packaging
makepkg -si
```
