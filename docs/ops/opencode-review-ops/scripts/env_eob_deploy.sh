#!/bin/sh
# env_eob_deploy.sh - node op `env_eob_deploy`, driven by the env_eob_up
# wasm step (crates/dag-review-tools). Example/stub: replace every
# TODO(real deploy) block with the real bring-up for your fleet.
#
# Contract (see ../README.md):
#   - argv-only launch, NO shell, cwd = the run's context root; the child
#     gets ONLY the env_keys listed in dag.ops (env_clear), so no PATH:
#     use absolute paths or set PATH below.
#   - exit 0 on success; non-zero exit fails the env step (module treats
#     it as "down", fail-closed).
#   - the LAST stdout line MUST be one JSON object:
#       {"endpoint": "http://host:port"}
#     (or {"host": "...", "port": 1234}); env_eob_up then probes
#     <endpoint>/ready until it answers 2xx.
#   - stdout+stderr are captured together, capped at 256 KiB total:
#     print a short progress summary, never full service logs.
#
# Secrets: read ONLY from env vars whitelisted in dag.ops env_keys
# (REVIEW_DEPLOY_TOKEN in node-config.example.json). Never echo values.

set -eu

# env_clear() drops PATH; scripts must self-locate their tools.
PATH="/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"
export PATH

WORKSPACE="${1:-/data00/github/opencoder}"
ENDPOINT_HOST="${ENDPOINT_HOST:-127.0.0.1}"
ENDPOINT_PORT="${ENDPOINT_PORT:-18080}"

echo "[env_eob_deploy] workspace: $WORKSPACE"
echo "[env_eob_deploy] target: ${ENDPOINT_HOST}:${ENDPOINT_PORT}"

if [ -n "${REVIEW_DEPLOY_TOKEN:-}" ]; then
  echo "[env_eob_deploy] REVIEW_DEPLOY_TOKEN present (value never printed)"
else
  echo "[env_eob_deploy] note: REVIEW_DEPLOY_TOKEN not set in node env" >&2
fi

# TODO(real deploy): stop any stale stack, e.g.
#   /usr/bin/systemctl stop eob-stack.service || true
echo "[env_eob_deploy] cleaning previous deployment (stub)"

# TODO(real deploy): the real bring-up. Examples:
#   /usr/bin/systemctl start eob-stack.service
#   /usr/local/bin/docker compose -f /etc/eob/compose.yaml up -d --wait
# The stub only simulates work so the example is runnable everywhere.
sleep 1
echo "[env_eob_deploy] deployed stack (stub)"

# TODO(real deploy): verify locally before handing the endpoint to the
# module's /ready probe, e.g. curl -sf "http://${ENDPOINT_HOST}:${ENDPOINT_PORT}/ready".
# The wasm module re-probes anyway; failing fast here gives a cleaner error.

# Tail JSON: MUST be the last non-empty stdout line, single object, no
# trailing slash on the endpoint.
printf '{"endpoint": "http://%s:%s"}\n' "$ENDPOINT_HOST" "$ENDPOINT_PORT"
