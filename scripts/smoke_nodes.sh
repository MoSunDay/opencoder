#!/usr/bin/env bash
# Two-process distributed-nodes smoke: one real `opencode daemon --server`
# plus one real `opencode daemon --client` worker. Exercises the fleet
# surface over plain curl (no test harness, no LLM round-trip assumed):
#   ✅ 1  worker registers and reports idle
#   ✅ 2  dispatch accepts a task (task_id + fresh `status:"pending"` field)
#   ✅ 3  task reaches a terminal state (done OR error — error counts too,
#         since the smoke runs with whatever LLM config this machine has)
#   ✅ 4  task-plane read API: single-task detail (+last_event_seq), the
#         fleet-wide filtered list (?status=&node_id=), and the
#         session→task reverse lookup
# Auth is the shared HMAC scheme (`core::auth_sig`): canonical string
#   METHOD\npath_and_query\nts_ms\nsha256_hex(body)
# signed into the `x-sig` / `x-sig-timestamp` headers. `/` , `/static/*` and
# `/api/time` stay unsigned — the readiness probe uses `/api/time`.
# Injection points: OPENCODER_SMOKE_BIN (prebuilt binary path — the cargo
# wrapper test injects the debug binary to skip the release build) and
# OPENCODER_SMOKE_PORT (listen port, avoids clashing with parallel tests).
# Requires: cargo, curl, python3, openssl, GNU date (no jq). Keep assertions
# python3-only.
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

# ---------------------------------------------------------------------------
# HMAC signing helper — byte-for-byte parity with core::auth_sig.
# req METHOD PATH_AND_QUERY [JSON_BODY]   → response body on stdout.
# The timestamp is taken fresh per call: every request carries a new ts, so
# the server-side replay dedup never trips across polls.
# ---------------------------------------------------------------------------
req() {
  local method="$1" pq="$2" body="${3:-}"
  local ts body_hash canonical sig
  ts="$(date +%s%3N)"
  body_hash="$(printf '%s' "${body}" | openssl dgst -sha256 -hex | awk '{print $NF}')"
  canonical="$(printf '%s\n%s\n%s\n%s' "${method}" "${pq}" "${ts}" "${body_hash}")"
  sig="$(printf '%s' "${canonical}" | openssl dgst -sha256 -hmac "${TOKEN}" -hex | awk '{print $NF}')"
  if [ -n "${body}" ]; then
    curl -s -X "${method}" \
      -H 'Content-Type: application/json' \
      -H "x-sig-timestamp: ${ts}" \
      -H "x-sig: ${sig}" \
      -d "${body}" \
      "${BASE}${pq}"
  else
    curl -s -X "${method}" \
      -H "x-sig-timestamp: ${ts}" \
      -H "x-sig: ${sig}" \
      "${BASE}${pq}"
  fi
}

echo "== starting server (daemon --server) on :${PORT} =="
"${BIN}" daemon --server --host 127.0.0.1 --port "${PORT}" --token "${TOKEN}" \
  >"${TMP}/server.log" 2>&1 &
SRV_PID=$!

# Readiness probe over the UNSIGNED clock-bootstrap endpoint: never blocked
# by auth, so a broken signature pipeline still reports itself at checkpoint 1.
SERVER_UP=""
for _ in $(seq 1 60); do
  if curl -sf "${BASE}/api/time" >/dev/null 2>&1; then
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

echo "== starting worker node 'smoke-node' (daemon --client) =="
mkdir -p "${TMP}/work"
# `--workdir` is a GLOBAL flag: it must precede the `daemon` subcommand.
"${BIN}" --workdir "${TMP}/work" daemon --client \
  --name smoke-node --remote "${BASE}" --token "${TOKEN}" \
  >"${TMP}/node.log" 2>&1 &
NODE_PID=$!

# ✅ checkpoint 1: registration — node visible AND idle. NOTE: every probing
# curl runs under `|| true`; with `set -e` a failed connection inside a poll
# assignment would otherwise kill the whole script instead of retrying.
CK1=""
for _ in $(seq 1 60); do
  OUT="$(req GET /api/nodes || true)"
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

# ✅ checkpoint 2: dispatch accepted with a task_id, a bound session_id, and
# the new `status` field reporting the fresh `pending` state.
OUT="$(req POST "/api/nodes/${CK1}/tasks" '{"prompt":"reply with exactly: ok"}' || true)"
CK2="$(printf '%s' "${OUT}" | python3 -c '
import json,sys
try:
    v=json.load(sys.stdin)
    tid=v.get("task_id",""); sid=v.get("session_id","")
    print(f"{tid} {sid}" if tid and sid and v.get("status")=="pending" else "")
except Exception: print("")' 2>/dev/null || true)"
if [ -z "${CK2}" ]; then
  echo "❌ checkpoint 2 FAILED: dispatch did not return task_id/session_id/status=pending: ${OUT}"
  exit 1
fi
TID="${CK2%% *}"
SID="${CK2##* }"
echo "✅ checkpoint 2: task dispatched (task_id=${TID} session=${SID} status=pending)"

# ✅ checkpoint 3: terminal status (done or error both pass).
FINAL=""
for _ in $(seq 1 120); do
  OUT="$(req GET "/api/nodes/tasks/${TID}" || true)"
  FINAL="$(printf '%s' "${OUT}" | python3 -c '
import json,sys
try:
    v=json.load(sys.stdin)
    print(v["status"] if v.get("status") in ("done","error","cancelled") else "")
except Exception: print("")' 2>/dev/null || true)"
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

# ✅ checkpoint 4: task-plane read API over the three new GET endpoints.
# 4a — single-task detail: identity fields + SSE bootstrap cursor.
CK4A="$(req GET "/api/nodes/tasks/${TID}" || true)"
OK4A="$(printf '%s' "${CK4A}" | python3 -c '
import json,sys
try:
    v=json.load(sys.stdin)
    ok=(v.get("id")==sys.argv[1] and v.get("node_id")==sys.argv[2]
        and bool(v.get("session_id")) and v.get("status")==sys.argv[3]
        and int(v.get("last_event_seq",-1))>=1)
    print("ok" if ok else "")
except Exception: print("")' "${TID}" "${CK1}" "${FINAL}" 2>/dev/null || true)"
if [ -z "${OK4A}" ]; then
  echo "❌ checkpoint 4a FAILED: task detail incomplete (want id/node/session/status=${FINAL}/last_event_seq>=1): ${CK4A}"
  exit 1
fi
echo "✅ checkpoint 4a: task detail + last_event_seq verified"

# 4b — fleet-wide list filtering: hits by status and by node_id, and the
# task must be ABSENT from a list filtered on a foreign status.
FOREIGN="cancelled"; [ "${FINAL}" = "cancelled" ] && FOREIGN="done"
LIST_BY_STATUS="$(req GET "/api/nodes/tasks?status=${FINAL}" || true)"
LIST_BY_NODE="$(req GET "/api/nodes/tasks?node_id=${CK1}" || true)"
LIST_FOREIGN="$(req GET "/api/nodes/tasks?status=${FOREIGN}" || true)"
OK4B="$(python3 -c '
import json,sys
def ids(raw):
    try: return [t["id"] for t in json.loads(raw).get("tasks",[])]
    except Exception: return []
hit_s, hit_n, hit_f = sys.argv[1], sys.argv[2], sys.argv[3]
print("ok" if sys.argv[4] in ids(hit_s) and sys.argv[4] in ids(hit_n)
      and sys.argv[4] not in ids(hit_f) else "")
' "${LIST_BY_STATUS}" "${LIST_BY_NODE}" "${LIST_FOREIGN}" "${TID}" 2>/dev/null || true)"
if [ -z "${OK4B}" ]; then
  echo "❌ checkpoint 4b FAILED: filtered fleet list wrong (by-status/by-node miss or foreign-status hit)"
  echo "  by-status=${LIST_BY_STATUS}"
  echo "  by-node=${LIST_BY_NODE}"
  echo "  foreign=${LIST_FOREIGN}"
  exit 1
fi
echo "✅ checkpoint 4b: ?status=${FINAL} and ?node_id= filters verified"

# 4c — session→task reverse lookup from the synthetic session id.
OUT="$(req GET "/api/sessions/${SID}/task" || true)"
OK4C="$(printf '%s' "${OUT}" | python3 -c '
import json,sys
try: print("ok" if json.load(sys.stdin).get("id")==sys.argv[1] else "")
except Exception: print("")' "${TID}" 2>/dev/null || true)"
if [ -z "${OK4C}" ]; then
  echo "❌ checkpoint 4c FAILED: session→task reverse lookup wrong: ${OUT}"
  exit 1
fi
echo "✅ checkpoint 4c: /api/sessions/${SID}/task resolves to the task"

echo "== cleanup =="
echo "SMOKE NODES PASSED"
