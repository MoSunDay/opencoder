#!/usr/bin/env bash
# Two-process distributed-nodes smoke: one real `opencode server` plus one
# real `opencode node` worker. Exercises the Phase-1..3 surface over plain
# curl (no test harness, no LLM round-trip assumed):
#   ✅ 1  worker registers and reports idle
#   ✅ 2  dispatch accepts a task (task_id issued)
#   ✅ 3  task reaches a terminal state (done OR error — error counts too,
#         since the smoke runs with whatever LLM config this machine has)
# Injection points: OPENCODER_SMOKE_BIN (prebuilt binary path — the cargo
# wrapper test injects the debug binary to skip the release build) and
# OPENCODER_SMOKE_PORT (listen port, avoids clashing with parallel tests).
# Requires: cargo, curl, python3 (no jq). Keep assertions python3-only.
set -euo pipefail

PORT="${OPENCODER_SMOKE_PORT:-18733}"
TOKEN="local-smoke-token"
BASE="http://127.0.0.1:${PORT}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BIN="${OPENCODER_SMOKE_BIN:-${ROOT}/target/release/opencoder}"

if [ -z "${OPENCODER_SMOKE_BIN:-}" ]; then
  echo "== building release binary =="
  # NOTE: the `opencoder` binary is the ROOT package target (src/main.rs);
  # building -p opencoder-cli only produces that crate's library.
  cargo build --release --bin opencoder
fi

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

echo "== starting server on :${PORT} =="
"${BIN}" server --port "${PORT}" --token "${TOKEN}" >"${TMP}/server.log" 2>&1 &
SRV_PID=$!

for _ in $(seq 1 60); do
  if curl -sf -H "Authorization: Bearer ${TOKEN}" "${BASE}/api/nodes" >/dev/null 2>&1; then
    break
  fi
  sleep 0.5
done

echo "== starting worker node 'smoke-node' =="
"${BIN}" node --name smoke-node --remote "${BASE}" --token "${TOKEN}" \
  --workdir "${TMP}/work" >"${TMP}/node.log" 2>&1 &
NODE_PID=$!
mkdir -p "${TMP}/work"

# ✅ checkpoint 1: registry lists smoke-node as idle.
CK1=""
for _ in $(seq 1 60); do
  OUT="$(curl -s -H "Authorization: Bearer ${TOKEN}" "${BASE}/api/nodes")"
  CK1="$(printf '%s' "${OUT}" | python3 -c '
import json,sys
v=json.load(sys.stdin)
ns=[n for n in v.get("nodes",[]) if n.get("name")=="smoke-node"]
print(ns[0]["id"] if ns and ns[0].get("status")=="idle" else "")
' 2>/dev/null || true)"
  [ -n "${CK1}" ] && break
  sleep 0.5
done
if [ -z "${CK1}" ]; then
  echo "❌ checkpoint 1 FAILED: smoke-node never registered idle"
  echo "--- server.log ---"; tail -20 "${TMP}/server.log"
  echo "--- node.log ---"; tail -20 "${TMP}/node.log"
  exit 1
fi
echo "✅ checkpoint 1: smoke-node registered idle (id=${CK1})"

# ✅ checkpoint 2: dispatch accepted with a task_id.
OUT="$(curl -s -X POST -H "Authorization: Bearer ${TOKEN}" \
  -H 'Content-Type: application/json' \
  -d '{"prompt":"reply with exactly: ok"}' \
  "${BASE}/api/nodes/${CK1}/tasks")"
TID="$(printf '%s' "${OUT}" | python3 -c '
import json,sys
try: print(json.load(sys.stdin).get("task_id",""))
except Exception: print("")' 2>/dev/null || true)"
if [ -z "${TID}" ]; then
  echo "❌ checkpoint 2 FAILED: dispatch did not return a task_id: ${OUT}"
  exit 1
fi
echo "✅ checkpoint 2: task dispatched (task_id=${TID})"

# ✅ checkpoint 3: terminal status (done or error both pass).
FINAL=""
for _ in $(seq 1 120); do
  TASKS="$(curl -s -H "Authorization: Bearer ${TOKEN}" "${BASE}/api/nodes/${CK1}/tasks")"
  FINAL="$(printf '%s' "${TASKS}" | python3 -c '
import json,sys
v=json.load(sys.stdin)
t=[t for t in v.get("tasks",[]) if t.get("id")==sys.argv[1]]
print(t[0]["status"] if t and t[0].get("status") in ("done","error","cancelled") else "")' "${TID}" 2>/dev/null || true)"
  [ -n "${FINAL}" ] && break
  sleep 0.5
done
if [ -z "${FINAL}" ]; then
  echo "❌ checkpoint 3 FAILED: task never reached a terminal state"
  echo "--- server.log ---"; tail -20 "${TMP}/server.log"
  echo "--- node.log ---"; tail -20 "${TMP}/node.log"
  exit 1
fi
echo "✅ checkpoint 3: task terminal (status=${FINAL}; error also passes — no LLM asserted)"

echo "== cleanup =="
echo "SMOKE NODES PASSED"
