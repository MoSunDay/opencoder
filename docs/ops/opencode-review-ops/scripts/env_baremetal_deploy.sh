#!/bin/sh
# env_baremetal_deploy.sh - node op `env_baremetal_deploy`, driven by the
# env_baremetal_up wasm step (crates/dag-review-tools). Example/stub:
# replace every TODO(real deploy) block with the real bring-up.
#
# Contract (see ../README.md): identical to env_eob_deploy.sh except the
# module probes <endpoint>/healthz (not /ready).
#   - argv-only launch, NO shell, cwd = the run's context root; the child
#     gets ONLY the env_keys listed in dag.ops (env_clear), so no PATH:
#     use absolute paths or set PATH below.
#   - exit 0 on success; non-zero exit fails the env step (fail-closed).
#   - the LAST stdout line MUST be one JSON object:
#       {"endpoint": "http://host:port"}
#   - stdout+stderr are captured together, capped at 256 KiB total.
#
# Secrets: read ONLY from env vars whitelisted in dag.ops env_keys
# (REVIEW_DEPLOY_TOKEN in node-config.example.json). Never echo values.

set -eu

# env_clear() drops PATH; scripts must self-locate their tools.
PATH="/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"
export PATH

WORKSPACE="${1:-/data00/github/opencoder}"
ENDPOINT_HOST="${ENDPOINT_HOST:-127.0.0.1}"
ENDPOINT_PORT="${ENDPOINT_PORT:-18090}"

echo "[env_baremetal_deploy] workspace: $WORKSPACE"
echo "[env_baremetal_deploy] target: ${ENDPOINT_HOST}:${ENDPOINT_PORT}"

if [ -n "${REVIEW_DEPLOY_TOKEN:-}" ]; then
  echo "[env_baremetal_deploy] REVIEW_DEPLOY_TOKEN present (value never printed)"
else
  echo "[env_baremetal_deploy] note: REVIEW_DEPLOY_TOKEN not set in node env" >&2
fi

# TODO(real deploy): power/vet the bare-metal host, e.g.
#   /usr/local/bin/bmctl vet --wait-ready 300
echo "[env_baremetal_deploy] vetting bare-metal host (stub)"

# TODO(real deploy): the real bring-up (image flash + boot + agent enroll).
# The stub only simulates work so the example is runnable everywhere.
sleep 1
echo "[env_baremetal_deploy] host provisioned (stub)"

# TODO(real deploy): verify locally before handing the endpoint to the
# module's /healthz probe, e.g.
#   curl -sf "http://${ENDPOINT_HOST}:${ENDPOINT_PORT}/healthz"

# Tail JSON: MUST be the last non-empty stdout line, single object, no
# trailing slash on the endpoint.
printf '{"endpoint": "http://%s:%s"}\n' "$ENDPOINT_HOST" "$ENDPOINT_PORT"
