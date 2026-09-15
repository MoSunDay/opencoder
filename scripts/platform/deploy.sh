#!/usr/bin/env bash
# Compatible releases never restart the active Server or any execution Runtime.
set -euo pipefail
script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
exec python3 "$script_dir/rolling_cli.py" "$@"
