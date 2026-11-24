#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

echo "==> Building release binary (optimized + stripped)..."
cargo build --release

mkdir -p "$HOME/.local/bin"
mkdir -p "$HOME/.local/share/applications"
mkdir -p "$HOME/.local/share/icons/hicolor/scalable/apps"

install -Dm755 "target/release/litecode" "$HOME/.local/bin/litecode"
install -Dm644 "packaging/litecode.desktop" "$HOME/.local/share/applications/litecode.desktop"
install -Dm644 "assets/litecode.svg" "$HOME/.local/share/icons/hicolor/scalable/apps/litecode.svg"

echo "==> Installed LiteCode to ~/.local/bin/litecode"
echo "    Launch from your app menu or run: ~/.local/bin/litecode ."
