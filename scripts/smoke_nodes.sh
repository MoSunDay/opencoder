#!/usr/bin/env bash
# Two-process distributed-nodes smoke: one real `opencoder-server`
# plus one real `opencoder-agent` worker. Exercises the fleet
# surface over plain curl (no test harness, no LLM round-trip assumed):
#   ✅ 1  worker registers and reports idle
#   ✅ 2  dispatch durably accepts an execution with its five-field index
#   ✅ 3  execution reaches an interactive/terminal state (idle/error/cancelled;
#         the worker runs against a seeded loopback LLM stub, so the outcome
#         is deterministic and needs zero credentials)
#   ✅ 4  ID-routed Node detail/events and fleet list fields
# Auth is the shared `Authorization: Bearer <token>` scheme. `/`,
# `/static/*`, and `/api/time` stay unauthenticated for bootstrap/readiness.
# Injection points: OPENCODER_SMOKE_SERVER_BIN / OPENCODER_SMOKE_AGENT_BIN
# (prebuilt binary paths — the cargo wrapper test injects the debug binaries
# to skip the release build) and
# OPENCODER_SMOKE_PORT (listen port, avoids clashing with parallel tests).
# Requires: cargo, curl, python3 (no jq). Keep assertions
# python3-only.
set -euo pipefail

PORT="${OPENCODER_SMOKE_PORT:-18733}"
TOKEN="local-smoke-token"
BASE="http://127.0.0.1:${PORT}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
# Unique per run: ck1 selects by name, so a fixed name could bind to a stale
# registry entry left behind by an earlier run.
NODE_NAME="smoke-node-$$"

if [ -z "${OPENCODER_SMOKE_SERVER_BIN:-}" ] || [ -z "${OPENCODER_SMOKE_AGENT_BIN:-}" ]; then
  echo "== building release fleet binaries =="
  # NOTE: the fleet pair lives in its own crates (crates/server,
  # crates/agent); anchor on the packages, not the bin names below.
  cargo build --release -p opencoder-server -p opencoder-agent
fi
# The fleet binaries carry the package spelling (`opencoder-server` /
# `opencoder-agent`, matching the `opencoder daemon` migration hint) while
# some docs spell them without the `r`. Probe both release paths, package
# spelling first, so the script tracks whichever way the naming settles.
# Explicit injection always wins.
probe_release_bin() {
  local name="$1" fallback="$2" path
  for path in "${ROOT}/target/release/${name}" "${ROOT}/target/release/${fallback}"; do
    [ -x "${path}" ] && { printf '%s' "${path}"; return 0; }
  done
  printf '%s' "${ROOT}/target/release/${name}"
}
SERVER_BIN="${OPENCODER_SMOKE_SERVER_BIN:-$(probe_release_bin opencoder-server opencoder-server)}"
AGENT_BIN="${OPENCODER_SMOKE_AGENT_BIN:-$(probe_release_bin opencoder-agent opencoder-agent)}"

TMP="$(mktemp -d /tmp/opencoder-smoke-nodes.XXXXXX)"
SRV_PID=""
NODE_PID=""
cleanup() {
  [ -n "${NODE_PID}" ] && kill "${NODE_PID}" 2>/dev/null || true
  [ -n "${SRV_PID}" ] && kill "${SRV_PID}" 2>/dev/null || true
  wait 2>/dev/null || true
  rm -rf "${TMP}"
}
trap cleanup EXIT

# ---------------------------------------------------------------------------
# Bearer-authenticated request helper.
# req METHOD PATH_AND_QUERY [JSON_BODY]   → response body on stdout.
# ---------------------------------------------------------------------------
req() {
  local method="$1" pq="$2" body="${3:-}"
  if [ -n "${body}" ]; then
    curl --noproxy '*' -s -X "${method}" \
      -H 'Content-Type: application/json' \
      -H "Authorization: Bearer ${TOKEN}" \
      -d "${body}" \
      "${BASE}${pq}"
  else
    curl --noproxy '*' -s -X "${method}" \
      -H "Authorization: Bearer ${TOKEN}" \
      "${BASE}${pq}"
  fi
}

echo "== starting server (opencoder-server) on :${PORT} =="
# A per-run workdir pins the server store inside ${TMP}: data_dir_for(workdir)
# is <XDG_DATA_HOME>/opencoder/<digest(workdir)>, so pointing XDG_DATA_HOME at
# ${TMP} keeps the node registry out of the shared persistent DB entirely and
# lets cleanup's `rm -rf ${TMP}` reclaim it — no cross-run entry can poison ck1.
XDG_DATA_HOME="${TMP}/xdg" "${SERVER_BIN}" --workdir "${TMP}/srv" --host 127.0.0.1 --port "${PORT}" --token "${TOKEN}" >"${TMP}/server.log" 2>&1 &
SRV_PID=$!

# Readiness probe over the unauthenticated compatibility clock endpoint.
SERVER_UP=""
# 90s budget: a cold start of the debug binary can blow past 30s when a
# parallel `cargo test --workspace` compile storm is hammering the box.
for _ in $(seq 1 180); do
  if curl --noproxy '*' -sf "${BASE}/api/time" >/dev/null 2>&1; then
    SERVER_UP=1
    break
  fi
  sleep 0.5
done
if [ -z "${SERVER_UP}" ]; then
  echo "❌ server never became reachable on ${BASE}"
  echo "--- server.log ---"; tail -20 "${TMP}/server.log"
  exit 1
fi

echo "== starting worker node '${NODE_NAME}' (opencoder-agent) =="
# Seed a deterministic loopback LLM config so worker startup never depends on
# this machine's global config: checkpoint 3 accepts `error` as terminal, so
# the unreachable loopback provider keeps the run deterministic with zero
# credentials.
mkdir -p "${TMP}/work/.opencoder"
cat > "${TMP}/work/.opencoder/config.json" <<'EOF'
{"model":"stub/m1","providers":{"stub":{"base_url":"http://127.0.0.1:9/v1","api_key":"smoke-dummy-key","model":"m1"}}}
EOF
# same XDG redirect: the worker's local task store dies with ${TMP} too.
XDG_DATA_HOME="${TMP}/xdg" "${AGENT_BIN}" --workdir "${TMP}/work" --remote "${BASE}" --token "${TOKEN}" --name "${NODE_NAME}" --workflow-root "${TMP}/workflow" >"${TMP}/node.log" 2>&1 &
NODE_PID=$!

# ✅ checkpoint 1: registration — node visible AND idle. NOTE: every probing
# curl runs under `|| true`; with `set -e` a failed connection inside a poll
# assignment would otherwise kill the whole script instead of retrying.
CK1=""
# same 90s rationale as the readiness probe above
for _ in $(seq 1 180); do
  OUT="$(req GET /api/nodes || true)"
  CK1="$(printf '%s' "${OUT}" | python3 -c '
import json,sys
v=json.load(sys.stdin)
ns=[n for n in v.get("nodes",[]) if n.get("name")==sys.argv[1]]
print(ns[0]["id"] if ns and ns[0].get("status")=="idle" else "")
' "${NODE_NAME}" 2>/dev/null || true)"
  [ -n "${CK1}" ] && break
  sleep 0.5
done
if [ -z "${CK1}" ]; then
  echo "❌ checkpoint 1 FAILED: ${NODE_NAME} never registered idle"
  echo "--- server.log ---"; tail -20 "${TMP}/server.log"
  echo "--- node.log ---"; tail -20 "${TMP}/node.log"
  exit 1
fi
echo "✅ checkpoint 1: ${NODE_NAME} registered idle (id=${CK1})"

# Durable acceptance and ID-based routing on the v2 control plane.
OUT="$(req POST /api/executions "{\"id\":\"agent-smoke\",\"kind\":\"agent\",\"node_id\":\"${CK1}\",\"input\":{\"prompt\":\"reply with exactly: ok\"}}")"
python3 -c 'import json,sys; v=json.loads(sys.argv[1]); assert set(v)=={"id","created_at","kind","node_id","status"} and v["kind"]=="agent" and v["node_id"]==sys.argv[2],v' "$OUT" "$CK1"
echo "✅ checkpoint 2: node durably accepted execution"
FINAL=""
for _ in $(seq 1 120); do
  OUT="$(req GET /api/executions/agent-smoke)"
  FINAL="$(python3 -c 'import json,sys; v=json.loads(sys.argv[1]); s=v.get("execution",{}).get("status"); print(s if s in ("idle","error","cancelled") else "")' "$OUT")"
  [ -n "$FINAL" ] && break
  sleep 0.5
done
[ -n "$FINAL" ] || { echo "execution did not finish: $OUT"; exit 1; }
python3 -c 'import json,sys; v=json.loads(sys.argv[1]); assert v["request"]["input"]["prompt"]=="reply with exactly: ok" and "session" in v,v' "$OUT"
echo "✅ checkpoint 3: node-owned detail and terminal state verified ($FINAL)"
OUT="$(req GET "/api/executions?node_id=${CK1}")"
python3 -c 'import json,sys; rows=json.loads(sys.argv[1])["executions"]; assert any(r["id"]=="agent-smoke" for r in rows); assert all(set(r)=={"id","created_at","kind","node_id","status"} for r in rows)' "$OUT"
EVENTS="$(req GET /api/executions/agent-smoke/events)"
[[ "$EVENTS" == *"id:"* ]] || { echo "node events missing"; exit 1; }
echo "✅ checkpoint 4: five-field index and node event replay verified"
echo "SMOKE NODES PASSED"
