#!/usr/bin/env bash
# Install the paired native Codex CLI/code-mode host and their shell/git runtime.
# Auth/config are deliberately not copied: the executor mounts the node's
# original Codex home at launch, keeping OAuth refresh on one authoritative copy.
set -euo pipefail

out="${1:?rootfs directory required}"
codex_binary="${2:?native Codex binary required}"
[ -d "$out" ] || { echo "rootfs directory missing" >&2; exit 2; }
out="$(cd "$out" && pwd)"
[ -x "$codex_binary" ] || { echo "Codex binary is missing or not executable" >&2; exit 2; }
codex_host="$(dirname "$(readlink -f "$codex_binary")")/codex-code-mode-host"
[ -x "$codex_host" ] || { echo "Paired codex-code-mode-host is missing or not executable" >&2; exit 2; }
python3 - "$codex_binary" "$codex_host" <<'PY'
import sys
for binary in sys.argv[1:]:
    with open(binary, 'rb') as stream:
        if stream.read(4) != b'\x7fELF':
            sys.exit("Pass the paired native Codex ELF binaries, not npm/Python/shell launchers. Custom launchers and dependencies can instead be provisioned explicitly in the rootfs.")
PY

source "$(dirname "${BASH_SOURCE[0]}")/common.sh"
bash "$(dirname "${BASH_SOURCE[0]}")/install-tools.sh" "$out"
install_binary "$out" "$codex_binary" /usr/bin/codex
install_binary "$out" "$codex_host" /usr/bin/codex-code-mode-host
run_in_rootfs "$out" /usr/bin/codex-code-mode-host --help >/dev/null
echo "==> Codex and its paired code-mode host installed in /usr/bin"
