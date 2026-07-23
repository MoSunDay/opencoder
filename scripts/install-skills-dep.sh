#!/usr/bin/env bash
# install-skills-dep.sh — install optional skill dependencies (tmux + chromium).
#
# After running this script:
#   - tmux is available (enables the `ts` persistent TUI command and the
#     ssh-pty skill's session backend).
#   - chromium/chrome is available (enables the chrome-headless skill).
#   - the `~/.opencoder/skills/.skills-deps` sentinel is created so opencode
#     seeds the ssh-pty and chrome-headless skills on next startup.
#
# Safe to re-run; skips packages already installed.
set -euo pipefail

OP_DIR="${HOME}/.opencoder"
SENTINEL="${OP_DIR}/skills/.skills-deps"

echo "=== opencode optional skill dependencies ==="
echo ""

# --- detect package manager ---
install_pkgs() {
    if command -v apt-get &>/dev/null; then
        sudo apt-get update -qq && sudo apt-get install -y "$@"
    elif command -v dnf &>/dev/null; then
        sudo dnf install -y "$@"
    elif command -v pacman &>/dev/null; then
        sudo pacman -Sy --noconfirm "$@"
    elif command -v zypper &>/dev/null; then
        sudo zypper install -y "$@"
    elif command -v brew &>/dev/null; then
        brew install "$@"
    else
        echo "ERROR: no supported package manager found (apt/dnf/pacman/zypper/brew)."
        echo "Install tmux and chromium manually, then re-run this script."
        return 1
    fi
}

# --- tmux ---
if command -v tmux &>/dev/null; then
    echo "[ok] tmux already installed."
else
    echo "[..] installing tmux..."
    install_pkgs tmux || echo "[warn] tmux install failed; install manually."
fi

# --- chromium ---
CHROME_NAMES=("google-chrome" "google-chrome-stable" "chromium-browser" "chromium")
CHROME_FOUND=""
for name in "${CHROME_NAMES[@]}"; do
    if command -v "$name" &>/dev/null; then
        CHROME_FOUND="$name"
        break
    fi
done
if [ -n "$CHROME_FOUND" ]; then
    echo "[ok] chrome/chromium already installed ($CHROME_FOUND)."
else
    echo "[..] installing chromium..."
    if command -v apt-get &>/dev/null; then
        install_pkgs chromium-browser || install_pkgs chromium || echo "[warn] chromium install failed."
    elif command -v dnf &>/dev/null; then
        install_pkgs chromium || echo "[warn] chromium install failed."
    elif command -v pacman &>/dev/null; then
        install_pkgs chromium || echo "[warn] chromium install failed."
    elif command -v brew &>/dev/null; then
        install_pkgs chromium || install_pkgs google-chrome || echo "[warn] chromium install failed."
    else
        echo "[warn] could not detect package manager for chromium."
        echo "       Install Chrome or Chromium manually."
    fi
fi

# --- create sentinel ---
mkdir -p "${OP_DIR}/skills"
touch "$SENTINEL"
echo ""
echo "=== Done ==="
echo "Sentinel written: $SENTINEL"
echo ""
echo "Next steps:"
echo "  1. Restart opencode (or run 'opencode tui')."
echo "  2. Press \$ in the TUI — ssh-pty and chrome-headless skills now appear."
echo "  3. Type {\$ssh-pty} or {\$chrome-headless} to activate a skill."
