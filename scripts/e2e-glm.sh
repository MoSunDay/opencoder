#!/usr/bin/env bash
# E2E regression against real glm5.2.
#
# Thin wrapper around the Python suite (scripts/e2e_glm.py), which asserts
# actual business contracts — not surface markers. See
# scripts/e2e/{lib,cli_scenarios,web_scenarios}.py for per-contract rationale.
#
# Requires: ZHIPU_API_KEY in env (or loaded from opencoder auth.json).
# Usage:    scripts/e2e-glm.sh [binary] [python-e2e flags...]
set -euo pipefail
exec python3 "$(dirname "$0")/e2e_glm.py" "$@"
