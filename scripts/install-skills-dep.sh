#!/usr/bin/env bash
# install-skills-dep.sh — install optional skill dependencies (tmux).
#
# After running this script:
#   - tmux is available (enables the `ts` persistent TUI command and the
#     ssh-pty skill's session backend).
#   - the `~/.opencoder/skills/.skills-deps` sentinel is created so opencode
#     seeds the ssh-pty skill on next startup.
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
        echo "Install tmux manually, then re-run this script."
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

# --- create sentinel ---
mkdir -p "${OP_DIR}/skills"
touch "$SENTINEL"
echo ""
echo "=== Done ==="
echo "Sentinel written: $SENTINEL"
echo ""
echo "Next steps:"
echo "  1. Restart opencode (or run 'opencode tui')."
echo "  2. Press \$ in the TUI — ssh-pty skill now appears."
echo "  3. Type {\$ssh-pty} to activate it."
