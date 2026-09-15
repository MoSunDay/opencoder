#!/usr/bin/env bash
# Roll one verified platform bundle through the local systemd fleet.
#
# The server intentionally persists a Frozen admission during graceful
# shutdown. A release must therefore reopen admission after the replacement
# Agent has reconnected; doing that here avoids manual node re-registration.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
bundle=""
dest_dir="/usr/local/bin"
server_unit="opencoder-server.service"
agent_unit="opencoder-agent.service"
server_url="http://127.0.0.1:18081"
server_token_file="/etc/opencoder/server.token"
agent_token_file="/etc/opencoder/agent.token"
wait_seconds=90

usage() {
  cat <<'USAGE'
Usage: scripts/platform/deploy.sh --bundle DIR [options]

Install a verified platform bundle, restart server then agent, wait for the
existing node IDs to reconnect, and reopen admission. The server and agent
token files must contain the same credential.

Options:
  --bundle DIR          Bundle produced by scripts/platform/release/build.sh
  --dest-dir DIR        Binary destination (default: /usr/local/bin)
  --server-unit NAME    systemd server unit
  --agent-unit NAME     systemd agent unit
  --server-url URL      Local server URL (default: http://127.0.0.1:18081)
  --server-token FILE   Server bearer token file
  --agent-token FILE    Agent bearer token file
  --wait-seconds N      Reconnect/ready timeout (default: 90)
USAGE
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --bundle) bundle="${2:?--bundle requires a directory}"; shift 2 ;;
    --dest-dir) dest_dir="${2:?--dest-dir requires a directory}"; shift 2 ;;
    --server-unit) server_unit="${2:?--server-unit requires a unit}"; shift 2 ;;
    --agent-unit) agent_unit="${2:?--agent-unit requires a unit}"; shift 2 ;;
    --server-url) server_url="${2:?--server-url requires a URL}"; shift 2 ;;
    --server-token) server_token_file="${2:?--server-token requires a file}"; shift 2 ;;
    --agent-token) agent_token_file="${2:?--agent-token requires a file}"; shift 2 ;;
    --wait-seconds) wait_seconds="${2:?--wait-seconds requires a number}"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "unknown argument: $1" >&2; usage >&2; exit 2 ;;
  esac
done

[[ -n "$bundle" ]] || { echo "--bundle is required" >&2; usage >&2; exit 2; }
[[ -d "$bundle" ]] || { echo "bundle does not exist: $bundle" >&2; exit 2; }
[[ "$wait_seconds" =~ ^[1-9][0-9]*$ ]] || { echo "--wait-seconds must be positive" >&2; exit 2; }
[[ -r "$server_token_file" && -r "$agent_token_file" ]] || {
  echo "server and agent token files must be readable" >&2
  exit 3
}
cmp -s "$server_token_file" "$agent_token_file" || {
  echo "server and agent token files differ; refusing deployment" >&2
  exit 3
}

tmp_dir="$(mktemp -d)"
auth_config="$tmp_dir/curl.conf"
before_nodes="$tmp_dir/before-nodes.json"
after_nodes="$tmp_dir/after-nodes.json"
reopen="$tmp_dir/reopen.json"
cleanup() { rm -rf "$tmp_dir"; }
trap cleanup EXIT

python3 - "$server_token_file" "$auth_config" <<'PY'
import pathlib
import sys

token = pathlib.Path(sys.argv[1]).read_text(encoding="utf-8").strip()
escaped = token.replace("\\", "\\\\").replace('"', '\\"')
pathlib.Path(sys.argv[2]).write_text(
    f'header = "Authorization: Bearer {escaped}"\n', encoding="utf-8"
)
PY
chmod 600 "$auth_config"

curl_json() {
  local method="$1" path="$2" output="$3"
  if [[ "$method" == GET ]]; then
    /usr/bin/curl --silent --show-error --fail --noproxy '*' \
      --config "$auth_config" --max-time 10 "$server_url$path" >"$output"
  else
    /usr/bin/curl --silent --show-error --fail --noproxy '*' \
      --config "$auth_config" --max-time 10 -X "$method" "$server_url$path" >"$output"
  fi
}

curl_json GET /api/nodes "$before_nodes"
python3 - "$before_nodes" <<'PY'
import json
import sys

nodes = json.load(open(sys.argv[1], encoding="utf-8")).get("nodes", [])
print(f"existing node IDs: {', '.join(n['id'] for n in nodes) or '(none)'}")
PY

python3 "$repo_root/scripts/platform/install_bundle.py" \
  --bundle "$bundle" --dest-dir "$dest_dir" --backup
systemctl restart "$server_unit"
systemctl is-active --quiet "$server_unit"
curl_json GET /api/health "$tmp_dir/health.json"
systemctl restart "$agent_unit"
systemctl is-active --quiet "$agent_unit"

deadline=$((SECONDS + wait_seconds))
while :; do
  curl_json GET /api/nodes "$after_nodes"
  if python3 - "$before_nodes" "$after_nodes" <<'PY'
import json
import sys

before = {n["id"] for n in json.load(open(sys.argv[1], encoding="utf-8")).get("nodes", [])}
nodes = json.load(open(sys.argv[2], encoding="utf-8")).get("nodes", [])
after = {n["id"] for n in nodes}
if not before.issubset(after):
    raise SystemExit(1)
if before and not all(n.get("online") for n in nodes if n["id"] in before):
    raise SystemExit(1)
if before and not all(n.get("protocol_version") == 9 for n in nodes if n["id"] in before):
    raise SystemExit(1)
PY
  then
    break
  fi
  (( SECONDS < deadline )) || { echo "timed out waiting for existing nodes to reconnect Ready" >&2; exit 4; }
  sleep 1
done

curl_json DELETE /api/admin/drain "$reopen"
python3 - "$reopen" <<'PY'
import json
import sys

body = json.load(open(sys.argv[1], encoding="utf-8"))
if body.get("server", {}).get("mode") != "open":
    raise SystemExit(f"admission reopen failed: {body}")
for node in body.get("nodes", []):
    if not 200 <= int(node.get("status", 0)) < 300 or node.get("body", {}).get("mode") != "open":
        raise SystemExit(f"node admission reopen failed: {node}")
print("admission reopened for all online nodes")
PY

deadline=$((SECONDS + wait_seconds))
while :; do
  curl_json GET /api/ready "$tmp_dir/ready.json" && break || true
  (( SECONDS < deadline )) || { echo "timed out waiting for admission Ready" >&2; exit 4; }
  sleep 1
done
python3 - "$after_nodes" "$tmp_dir/ready.json" <<'PY'
import json
import sys

nodes = json.load(open(sys.argv[1], encoding="utf-8")).get("nodes", [])
ready = json.load(open(sys.argv[2], encoding="utf-8"))
if ready.get("ready_nodes", 0) < 1 or ready.get("mode") != "open":
    raise SystemExit(f"cluster is not ready: {ready}")
print(f"ready node IDs: {', '.join(n['id'] for n in nodes if n.get('online') and n.get('snapshot', {}).get('ready'))}")
PY
echo "deployment complete: $bundle"
