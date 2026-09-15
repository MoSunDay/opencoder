#!/usr/bin/env bash
# Roll back ingress and new-task ownership; retain every accepted execution.
set -euo pipefail
script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
exec python3 "$script_dir/rolling_cli.py" --rollback "$@"
