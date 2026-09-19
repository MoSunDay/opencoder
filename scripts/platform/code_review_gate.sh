#!/usr/bin/env bash
# code-review 发布门禁：发布 kb-index wasm 模块 → 保存 code-review DAG 定义
# → dispatch（带 base/head input）→ 轮询 run → 读取 summary 步骤 verdict。
#
# 退出码：0=pass（放行）；3=pass_with_tickets（放行但告警）；
#         2=blocked（P0/P1 实锤，阻断）；1=执行失败/超时/参数错误。
#
# 用法：
#   scripts/platform/code_review_gate.sh --base <sha> --head <sha> \
#       [--dag-id code-review] [--run-id auto] \
#       [--server "$OPENCODER_SERVER_URL"] [--token "$OPENCODER_SERVER_TOKEN"] \
#       [--timeout 1800] [--defs-file <repo>/examples/dag/code-review.json]
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "$script_dir/../.." && pwd)"

BASE="" HEAD=""
DAG_ID="code-review"
RUN_ID=""
SERVER="${OPENCODER_SERVER_URL:-}"
TOKEN="${OPENCODER_SERVER_TOKEN:-}"
TIMEOUT=1800
DEFS_FILE="$repo_root/examples/dag/code-review.json"
POLL_INTERVAL=10

die() { echo "code-review-gate: $*" >&2; exit 1; }
note() { echo "[code-review-gate] $*"; }

usage() {
  sed -n '2,20p' "${BASH_SOURCE[0]:-$0}" | sed 's/^# \{0,1\}//'
  exit 1
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --base) BASE="${2:-}"; shift 2 ;;
    --head) HEAD="${2:-}"; shift 2 ;;
    --dag-id) DAG_ID="${2:-}"; shift 2 ;;
    --run-id) RUN_ID="${2:-}"; shift 2 ;;
    --server) SERVER="${2:-}"; shift 2 ;;
    --token) TOKEN="${2:-}"; shift 2 ;;
    --timeout) TIMEOUT="${2:-}"; shift 2 ;;
    --defs-file) DEFS_FILE="${2:-}"; shift 2 ;;
    -h|--help) usage ;;
    *) die "未知参数: $1（--help 查看用法）" ;;
  esac
done

[[ -n "$BASE" && -n "$HEAD" ]] || die "--base 与 --head 均为必填（git commit sha）"
[[ -n "$SERVER" ]] || die "缺少 --server（或环境变量 OPENCODER_SERVER_URL）"
[[ -n "$TOKEN" ]] || die "缺少 --token（或环境变量 OPENCODER_SERVER_TOKEN）"
[[ -f "$DEFS_FILE" ]] || die "defs 文件不存在: $DEFS_FILE"
command -v python3 >/dev/null 2>&1 || die "依赖检查失败: 需要 python3"
if command -v opencode-cli >/dev/null 2>&1; then
  note "依赖检查: python3 + opencode-cli 就绪（本脚本走 HTTP API，CLI 仅作可选后备）"
else
  note "依赖检查: python3 就绪；未找到 opencode-cli（可用 cargo run -p opencoder-cli -- 代替），本脚本走 HTTP API，不受影响"
fi

SERVER="${SERVER%/}"
if [[ -z "$RUN_ID" ]]; then
  # run id 必须以 dag- 前缀（control 面校验）；head 片段让重跑天然幂等续查。
  RUN_ID="dag-cr-gate-${HEAD:0:12}-$(date +%s)"
fi

# 临时文件登记 + 退出清理（wasm base64 这类大体会走文件而不是 argv）。
TMP_FILES=()
cleanup_tmp() { [[ ${#TMP_FILES[@]} -gt 0 ]] && rm -f "${TMP_FILES[@]}"; return 0; }
trap cleanup_tmp EXIT
new_tmp() { local f; f="$(mktemp)"; TMP_FILES+=("$f"); echo "$f"; }

# ---- HTTP helper（python3 urllib，Bearer 鉴权）--------------------------
# 用法: http <METHOD> <PATH> [BODY_JSON|@FILE]  → 打印 "STATUS\tBODY"
# body 以 @ 开头时从文件读取（发布 wasm 的 body 数百 KB，超过 ARG_MAX）。
http() {
  local method="$1" path="$2" body="${3:-}"
  python3 - "$SERVER" "$TOKEN" "$method" "$path" "$body" <<'PY'
import json, sys, urllib.request, urllib.error
server, token, method, path, body = sys.argv[1:6]
if body.startswith("@"):
    with open(body[1:], "r", encoding="utf-8") as f:
        body = f.read()
url = server + path
data = body.encode() if body else None
req = urllib.request.Request(url, data=data, method=method)
req.add_header("authorization", f"Bearer {token}")
if data is not None:
    req.add_header("content-type", "application/json")
try:
    with urllib.request.urlopen(req, timeout=60) as resp:
        print(resp.status, resp.read().decode("utf-8", "replace"), sep="\t")
except urllib.error.HTTPError as e:
    print(e.code, e.read().decode("utf-8", "replace"), sep="\t")
except Exception as e:  # 连接层失败：与 HTTP 错误同形输出，交由调用方判 0
    print(0, json.dumps({"error": f"{type(e).__name__}: {e}"}), sep="\t")
PY
}

# http + 断言状态码集合；失败带上下文退出 1。
http_expect() {
  local method="$1" path="$2" body="${3:-}" want="$4" label="$5"
  local out status payload
  out="$(http "$method" "$path" "$body")" || die "$label: 请求发送失败（$SERVER）"
  status="${out%%$'\t'*}"
  payload="${out#*$'\t'}"
  [[ ",$want," == *",$status,"* ]] || die "$label: HTTP $status（期望 $want）: $payload"
  echo "$payload"
}

# ---- 1. 发布 kb-index（幂等）--------------------------------------------
publish_kb_index() {
  local out status payload
  out="$(http GET "/api/dag/wasm/kb-index" "")" || die "kb-index 探测失败（$SERVER）"
  status="${out%%$'\t'*}"
  if [[ "$status" == "200" ]]; then
    note "kb-index 已在 wasm 池中（current），跳过构建与发布"
    return 0
  fi
  [[ "$status" == "404" ]] && note "kb-index 不在池中，开始构建 wasm32-wasip1 版本"

  # 显式 target-dir：artifact 路径才可预知（否则受 CARGO_TARGET_DIR 影响）。
  local target_dir="$repo_root/target"
  local wasm="$target_dir/wasm32-wasip1/release/kb-index.wasm"
  if ! cargo build --manifest-path "$repo_root/examples/dag-modules/kb-index/Cargo.toml" \
        --target wasm32-wasip1 --release --target-dir "$target_dir"; then
    # 目标未装或构建失败：再次探测池（可能已被别的流水线发布）。
    out="$(http GET "/api/dag/wasm/kb-index" "")" || true
    status="${out%%$'\t'*}"
    if [[ "$status" == "200" ]]; then
      note "本地构建失败但池中已有 kb-index，直接复用"
      return 0
    fi
    die "kb-index 构建失败且池中不存在（wasm32-wasip1 目标可用 \
rustup target add wasm32-wasip1 安装）"
  fi
  [[ -f "$wasm" ]] || die "kb-index 构建产物缺失: $wasm"

  # base64 与发布 body 都走临时文件（wasm 数百 KB，塞进 argv 会超 ARG_MAX）。
  local b64_tmp body_tmp
  b64_tmp="$(new_tmp)"
  body_tmp="$(new_tmp)"
  python3 - "$wasm" "$b64_tmp" <<'PY'
import base64, sys
with open(sys.argv[2], "w") as out:
    out.write(base64.b64encode(open(sys.argv[1], "rb").read()).decode())
PY
  python3 - "$b64_tmp" "$body_tmp" <<'PY'
import json, sys
with open(sys.argv[2], "w", encoding="utf-8") as out:
    json.dump({
        "name": "kb-index",
        "description": "code-review gate: deterministic knowledge-base index step",
        "wasm_b64": open(sys.argv[1]).read(),
    }, out)
PY
  local body="@$body_tmp"
  out="$(http POST "/api/dag/wasm" "$body")" || die "kb-index 发布请求失败"
  status="${out%%$'\t'*}"
  payload="${out#*$'\t'}"
  case "$status" in
    200|201) note "kb-index 发布成功" ;;
    409) note "kb-index 已存在（409），跳过" ;;
    *) die "kb-index 发布失败: HTTP $status: $payload" ;;
  esac
}

# ---- 2. 保存 DAG 定义（spec.name 改写为 --dag-id）------------------------
put_def() {
  local body
  body="$(python3 - "$DAG_ID" "$DEFS_FILE" <<'PY'
import json, sys
dag_id, path = sys.argv[1], sys.argv[2]
spec = json.load(open(path, encoding="utf-8"))
spec["name"] = dag_id
print(json.dumps({"spec": spec}, ensure_ascii=False))
PY
)" || die "解析 defs 文件失败: $DEFS_FILE"
  local payload
  payload="$(http_expect POST "/api/dag/defs" "$body" 200,201 "defs put $DAG_ID")"
  note "DAG 定义已保存: $DAG_ID（9 步）"
}

# ---- 3. dispatch（504 容错 + 幂等重试）------------------------------------
dispatch_run() {
  local body
  body="$(python3 - "$RUN_ID" "$BASE" "$HEAD" <<'PY'
import json, sys
run_id, base, head = sys.argv[1], sys.argv[2], sys.argv[3]
print(json.dumps({
    "id": run_id,
    "input": {"prompt": f"base={base} head={head} 变更审查请求（发布门禁）"},
}, ensure_ascii=False))
PY
)" || die "构造 dispatch body 失败"

  local out status payload
  out="$(http POST "/api/dag/defs/$DAG_ID/dispatch" "$body")" || die "dispatch 请求发送失败"
  status="${out%%$'\t'*}"
  payload="${out#*$'\t'}"
  if [[ "$status" == "202" ]]; then
    note "已入队 run=$RUN_ID"
    return 0
  fi
  if [[ "$status" == "504" ]]; then
    # 网关超时：响应若带有 run id（已入队标记）则继续轮询该 run-id（幂等），
    # 否则原 run-id 重试一次。
    if [[ "$payload" == *"$RUN_ID"* ]]; then
      note "dispatch 504 但 run 已入队（响应含 run id），继续轮询 run=$RUN_ID"
      return 0
    fi
    note "dispatch 504（响应无 run id），原 id 幂等重试一次"
    out="$(http POST "/api/dag/defs/$DAG_ID/dispatch" "$body")" || true
    status="${out%%$'\t'*}"
    payload="${out#*$'\t'}"
    [[ "$status" == "202" || "$status" == "409" ]] && { note "重试结果: HTTP $status"; return 0; }
    die "dispatch 重试失败: HTTP $status: $payload"
  fi
  # 409 = 同 id 幂等冲突（重跑同 run-id），视为已入队继续。
  [[ "$status" == "409" ]] && { note "run=$RUN_ID 已存在（409），续查"; return 0; }
  die "dispatch 失败: HTTP $status: $payload"
}

# ---- 4. 轮询直到终态 ------------------------------------------------------
wait_run() {
  local deadline=$(( $(date +%s) + TIMEOUT ))
  while :; do
    local out status payload
    out="$(http GET "/api/dag/runs/$RUN_ID" "")" || die "run 查询失败（$SERVER）"
    status="${out%%$'\t'*}"
    payload="${out#*$'\t'}"
    [[ "$status" == "200" ]] || die "run 查询 HTTP $status: $payload"
    local run_status
    run_status="$(python3 -c 'import json,sys; print(json.loads(sys.argv[1]).get("status",""))' "$payload")"
    case "$run_status" in
      done) return 0 ;;
      error|cancelled)
        local detail
        detail="$(http GET "/api/dag/runs/$RUN_ID/progress" "")" || detail='{}'
        python3 - "$detail" <<'PY' || true
import json, sys
try:
    doc = json.loads(sys.argv[1])
except Exception:
    doc = {}
for s in doc.get("steps") or []:
    if s.get("status") in ("error", "cancelled") and s.get("error"):
        print("  步骤 %s: %s: %s" % (s.get("name"), s.get("status"), s.get("error")), file=sys.stderr)
if doc.get("execution_error"):
    print("  run error: %s" % doc["execution_error"], file=sys.stderr)
PY
        die "run=$RUN_ID 终态 $run_status（步骤错误见上）"
        ;;
      *)
        if [[ "$(date +%s)" -ge "$deadline" ]]; then
          die "等待 run=$RUN_ID 超时（${TIMEOUT}s，当前状态 $run_status）"
        fi
        sleep "$POLL_INTERVAL"
        ;;
    esac
  done
}

# ---- 5. 读取 summary / viking-ticket 输出并裁决 ---------------------------
step_output() {
  local step="$1" payload status out
  out="$(http GET "/api/dag/runs/$RUN_ID/steps/$step" "")" || die "步骤查询失败: $step"
  status="${out%%$'\t'*}"
  payload="${out#*$'\t'}"
  [[ "$status" == "200" ]] || die "步骤 $step 查询 HTTP $status: $payload"
  python3 -c 'import json,sys; d=json.loads(sys.argv[1]); o=d.get("output"); print(json.dumps(o if o is not None else {}, ensure_ascii=False))' "$payload"
}

main() {
  publish_kb_index
  put_def
  dispatch_run
  wait_run

  local summary tickets
  summary="$(step_output summary)"
  tickets="$(step_output viking-ticket)"

  python3 - "$summary" "$tickets" <<'PY'
import json, sys
summary, tickets = json.loads(sys.argv[1]), json.loads(sys.argv[2])
verdict = summary.get("verdict")
print("[code-review-gate] 裁决:", verdict)
print("[code-review-gate] P0/P1/P2:", summary.get("p0"), summary.get("p1"), summary.get("p2"))
report = summary.get("report")
if report:
    print("[code-review-gate] 报告:", report)
issues = summary.get("confirmed_issues") or []
for issue in issues:
    print(f"[code-review-gate] 实锤: {issue.get('id')} {issue.get('severity')} "
          f"{issue.get('summary')} @ {issue.get('evidence')}")
tk = tickets.get("tickets")
if tk == "created":
    ids = tickets.get("ids") or []
    print("[code-review-gate] 工单已创建:", ", ".join(map(str, ids)))
elif tk == "not_required":
    print("[code-review-gate] 无实锤问题，未建工单")
else:
    print("[code-review-gate] 工单步骤输出异常:", json.dumps(tickets, ensure_ascii=False))
if verdict == "pass":
    sys.exit(0)
if verdict == "pass_with_tickets":
    print("[code-review-gate] 放行（有 P2 工单，请跟进）", file=sys.stderr)
    sys.exit(3)
if verdict == "blocked":
    print("[code-review-gate] 阻断：存在 P0/P1 实锤问题", file=sys.stderr)
    sys.exit(2)
print(f"[code-review-gate] 未知 verdict: {verdict!r}", file=sys.stderr)
sys.exit(1)
PY
}

main
