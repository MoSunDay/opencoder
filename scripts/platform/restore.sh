#!/usr/bin/env bash
# Verify a platform backup and restore it only into a new isolated directory.
set -euo pipefail
script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
exec python3 "$script_dir/data_archive.py" restore "$@"
