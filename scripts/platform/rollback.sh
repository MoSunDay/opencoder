#!/usr/bin/env bash
# Atomically switch all three platform launchers to a verified rollback bundle.
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "$script_dir/../.." && pwd)"
bundle=""
dest_dir="/usr/local/bin"

usage() {
  cat <<'USAGE'
Usage: rollback.sh --bundle DIR [--dest-dir DIR]

Verifies a previously saved platform bundle and atomically switches the CLI,
Server and Agent to that bundle as one protocol-compatible generation. The
currently active generation is retained as a new rollback bundle.
USAGE
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --bundle) bundle="${2:?--bundle requires a directory}"; shift 2 ;;
    --dest-dir) dest_dir="${2:?--dest-dir requires a directory}"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "rollback.sh: unknown argument: $1" >&2; usage >&2; exit 2 ;;
  esac
done

[[ -n "$bundle" ]] || { usage >&2; exit 2; }
exec "$repo_root/scripts/install.sh" \
  --bundle "$bundle" --dest-dir "$dest_dir" --backup --no-build
