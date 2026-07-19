#!/usr/bin/env bash
# Atomic installer for the opencoder release binary.
#
# Resolves the freshly-built `opencoder` release binary (honouring
# .cargo/config.toml's target-dir via `cargo metadata`) and atomically installs
# it as a hard copy at a canonical destination (/usr/local/bin/opencoder by
# default).
#
# Why a hard copy and not a symlink: /usr/local/bin must keep working even if the
# build cache under /data is wiped, and absolute-path callers (systemd units,
# cron) must not follow a symlink that can dangle. The cargo-managed
# ~/.cargo/bin/opencoder symlink remains for interactive dev use and tracks
# rebuilds automatically; this script keeps the FHS system copy fresh.
#
# Atomicity: we copy to <dest>.new.<pid>, chmod, fsync, then rename() over the
# destination. rename() on Linux atomically swaps the inode; a process already
# running the old binary keeps its mapping untouched (no ETXTBSY, no torn
# writes). Safe to run while opencoder is executing.
#
# Usage:
#   scripts/install.sh                       # build + install to /usr/local/bin
#   scripts/install.sh --no-build            # install existing build only
#   scripts/install.sh --dest /opt/bin/opencoder
#   scripts/install.sh --source path/to/opencoder --no-build
#   OPENCODER_INSTALL_DEST=/x scripts/install.sh
#
# Exit codes: 0 success | 1 usage | 2 build failed | 3 source missing
#             | 4 install failed | 5 self-check failed

set -euo pipefail

PROGNAME="$(basename "$0")"

DEST="${OPENCODER_INSTALL_DEST:-/usr/local/bin/opencoder}"
SOURCE=""
NO_BUILD=0

usage() {
  cat <<USAGE
Usage: $PROGNAME [--dest PATH] [--source PATH] [--no-build] [-h|--help]

Atomically install the opencoder release binary to a canonical path.

Options:
  --dest PATH     Install destination (default: $DEST
                  or \$OPENCODER_INSTALL_DEST if set).
  --source PATH   Use this binary instead of auto-resolving via cargo metadata.
  --no-build      Skip \`cargo build --release\` (use the existing build).
  -h, --help      Show this help.

Exit codes: 0 ok | 1 usage | 2 build | 3 source | 4 install | 5 self-check
USAGE
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --dest)     DEST="$2"; shift 2;;
    --source)   SOURCE="$2"; shift 2;;
    --no-build) NO_BUILD=1; shift;;
    -h|--help)  usage; exit 0;;
    *) echo "$PROGNAME: unknown argument: $1" >&2; usage >&2; exit 1;;
  esac
done

# --- repo root (where Cargo.toml + .cargo/config.toml live) -----------------
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
if [[ ! -f "$REPO_ROOT/Cargo.toml" ]]; then
  echo "$PROGNAME: cannot locate repo root above scripts/ (no Cargo.toml at $REPO_ROOT)" >&2
  exit 1
fi

# --- resolve source binary -------------------------------------------------
if [[ -n "$SOURCE" ]]; then
  SRC="$SOURCE"
else
  TARGET_DIR="$(cd "$REPO_ROOT" && cargo metadata --no-deps --format-version 1 \
    | python3 -c 'import sys,json;print(json.load(sys.stdin)["target_directory"])')"
  SRC="$TARGET_DIR/release/opencoder"
fi

# --- optional build --------------------------------------------------------
if [[ "$NO_BUILD" -eq 0 ]]; then
  echo "$PROGNAME: building release (cargo build --release)..."
  if ! (cd "$REPO_ROOT" && cargo build --release); then
    echo "$PROGNAME: cargo build --release failed" >&2
    exit 2
  fi
fi

if [[ ! -x "$SRC" ]]; then
  echo "$PROGNAME: source binary not found or not executable: $SRC" >&2
  echo "$PROGNAME: (run without --no-build, or pass --source PATH)" >&2
  exit 3
fi

SRC_VERSION="$("$SRC" --version 2>/dev/null || echo "")"

# --- atomic install --------------------------------------------------------
dest_dir="$(dirname "$DEST")"
if [[ ! -d "$dest_dir" ]]; then
  echo "$PROGNAME: destination directory does not exist: $dest_dir" >&2
  exit 4
fi

tmp="$DEST.new.$$"
cleanup() { rm -f "$tmp"; }
trap cleanup EXIT

echo "$PROGNAME: installing $SRC -> $DEST"
if ! cp -f "$SRC" "$tmp"; then
  echo "$PROGNAME: copy to $tmp failed" >&2
  exit 4
fi
chmod 0755 "$tmp"
# fsync the staged file so its bytes are durable before the rename exposes it.
sync "$tmp" 2>/dev/null || sync

if ! mv -f "$tmp" "$DEST"; then
  echo "$PROGNAME: rename $tmp -> $DEST failed" >&2
  exit 4
fi
trap - EXIT

# --- self-check ------------------------------------------------------------
if ! installed_version="$("$DEST" --version 2>/dev/null)"; then
  echo "$PROGNAME: self-check failed: '$DEST --version' exited non-zero" >&2
  exit 5
fi

echo "$PROGNAME: installed: $DEST"
echo "$PROGNAME: version:   $installed_version"

if [[ -n "$SRC_VERSION" && "$installed_version" != "$SRC_VERSION" ]]; then
  echo "$PROGNAME: ERROR version mismatch after install (source='$SRC_VERSION' dest='$installed_version')" >&2
  exit 5
fi

echo "$PROGNAME: done."
