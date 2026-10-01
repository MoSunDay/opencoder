#!/usr/bin/env bash
# Install a native Codex CLI and its shell/git runtime into a prepared rootfs.
# Auth/config are deliberately not copied: the executor mounts the node's
# original Codex home at launch, keeping OAuth refresh on one authoritative copy.
set -euo pipefail

out="${1:?rootfs directory required}"
codex_binary="${2:?native Codex binary required}"
[ -d "$out" ] || { echo "rootfs directory missing" >&2; exit 2; }
out="$(cd "$out" && pwd)"
[ -x "$codex_binary" ] || { echo "Codex binary is missing or not executable" >&2; exit 2; }
python3 - "$codex_binary" <<'PY'
import sys
with open(sys.argv[1], 'rb') as stream:
    if stream.read(4) != b'\x7fELF':
        sys.exit("Pass the native Codex ELF binary, not an npm/Python/shell launcher. Custom launchers and dependencies can instead be provisioned explicitly in the rootfs.")
PY

source "$(dirname "${BASH_SOURCE[0]}")/common.sh"
bash "$(dirname "${BASH_SOURCE[0]}")/install-tools.sh" "$out"
install_binary "$out" "$codex_binary" /usr/bin/codex
echo "==> Codex installed at /usr/bin/codex"
