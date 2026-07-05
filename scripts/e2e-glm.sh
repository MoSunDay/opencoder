#!/usr/bin/env bash
# E2E regression against real glm5.2.
#
# Verifies the cross-cutting contracts that only a live model run can prove:
#   E7  reasoning_effort is carried end-to-end (config → models display).
#       Runs FIRST — cheap, no model call — so the display path is verified
#       even if the heavy live-model steps (E1/E2/E6) get reaped by the sandbox.
#   E1  glm writes a runnable snake game (write+bash tools, compiles, >50 lines)
#   E2  --continue resumes the SAME session and extends the artifact
#   E6  glm writes a runnable thunder-fighter game (cross-game-type regression)
#   E1/E2/E6 also exercise reasoning_effort:medium live — if glm-5.2 rejected
#   the field, those artifacts would not exist/compile.
#
# Requires: ZHIPU_API_KEY in env (or loaded from opencode auth.json).
# Usage:    scripts/e2e-glm.sh [binary]
set -euo pipefail

BIN="${1:-./target/debug/opencoder}"
AUTH="${HOME}/.local/share/opencode/auth.json"
if [ -z "${ZHIPU_API_KEY:-}" ]; then
  if [ -f "$AUTH" ]; then
    export ZHIPU_API_KEY=$(python3 -c "import json;print(json.load(open('$AUTH'))['zhipuai-coding-plan']['key'])")
  else
    echo "FAIL: set ZHIPU_API_KEY or install opencode auth.json" >&2; exit 2
  fi
fi

ROOT="$(mktemp -d)"
SNAKE="$ROOT/snake"; THUNDER="$ROOT/thunder"
mkdir -p "$SNAKE" "$THUNDER"
# medium (not high) keeps the live turns lighter so the sandbox is less likely
# to reap a long thinking-heavy resume.
CFG='{"model":"zhipuai-coding-plan/glm-5.2","provider":{"base_url":"https://open.bigmodel.cn/api/coding/paas/v4","api_key":"{ZHIPU_API_KEY}"},"reasoning_effort":"medium","max_tokens":16384,"compaction":{"auto":true,"context_threshold":100000}}'
printf '%s' "$CFG" > "$SNAKE/opencode.json"; printf '%s' "$CFG" > "$THUNDER/opencode.json"

pass=0; fail=0
check() { if [ "$1" = "$2" ]; then echo "  ok: $3"; pass=$((pass+1)); else echo "  FAIL: $3 ($1 != $2)"; fail=$((fail+1)); fi; }

echo "== E7: reasoning_effort carried config → models display (runs first, cheap) =="
# Cheap display-path check before any live model call.
MODELS_OUT=$(cd "$(dirname "$BIN")" && "$BIN" --workdir "$SNAKE" models 2>&1 || true)
THINKING_LINE=$(printf '%s\n' "$MODELS_OUT" | grep -E '^thinking[[:space:]]*:' || true)
[ -n "$THINKING_LINE" ] && check "present" "present" "models shows thinking line" || check "(missing)" "present" "models shows thinking line"
printf '%s\n' "$THINKING_LINE" | grep -q 'medium' && check "medium" "medium" "reasoning_effort resolved to medium" || check "(other)" "medium" "reasoning_effort resolved to medium"

echo "== E1: glm writes a runnable snake game =="
( cd "$(dirname "$BIN")" && "$BIN" --workdir "$SNAKE" "用 python3 写一个终端贪吃蛇游戏，保存为 snake.py。方向键控制、吃食物变长、撞墙/撞自己结束。写完运行 'python3 -m py_compile snake.py' 验证语法（不要运行游戏循环）。" ) >"$SNAKE/run.log" 2>&1 || true
[ -f "$SNAKE/snake.py" ] && check "1" "1" "snake.py exists" || check "0" "1" "snake.py exists"
LINES=$(wc -l < "$SNAKE/snake.py" 2>/dev/null || echo 0)
[ "$LINES" -gt 50 ] && check "big" "big" "snake.py > 50 lines ($LINES)" || check "$LINES" ">50" "snake.py > 50 lines"
python3 -m py_compile "$SNAKE/snake.py" && check "compile" "compile" "snake.py compiles" || check "fail" "compile" "snake.py compiles"
SID=$(grep -oE 'session [0-9A-Z]+' "$SNAKE/run.log" 2>/dev/null | tail -1 | awk '{print $2}' || true)

echo "== E2: --continue resumes the same session and extends the artifact =="
BEFORE=$(wc -l < "$SNAKE/snake.py")
( cd "$(dirname "$BIN")" && "$BIN" --workdir "$SNAKE" --continue "给贪吃蛇加计分板，屏幕顶部显示当前分数和最高分(high score)。修改 snake.py。" ) >"$SNAKE/resume.log" 2>&1 || true
AFTER=$(wc -l < "$SNAKE/snake.py")
RSID=$(grep -oE 'session [0-9A-Z]+' "$SNAKE/resume.log" 2>/dev/null | tail -1 | awk '{print $2}' || true)
[ -n "$SID" ] && [ "$SID" = "$RSID" ] && check "$SID" "$RSID" "resume reuses same session id" || check "$SID" "$RSID" "resume reuses same session id"
SC=$(grep -ciE 'high.?score|最高分|scoreboard|计分' "$SNAKE/snake.py")
[ "$SC" -gt 0 ] && check "scoreboard" "scoreboard" "scoreboard added ($SC hits)" || check "0" ">0" "scoreboard added"

echo "== E6: glm writes a runnable thunder-fighter game =="
( cd "$(dirname "$BIN")" && "$BIN" --workdir "$THUNDER" "用 python3 写一个雷霆战机(飞机射击)游戏，保存为 thunder.py。方向键移动、空格射击、敌机下落、击中得分、被撞结束。写完运行 'python3 -m py_compile thunder.py' 验证语法（不要运行游戏循环）。" ) >"$THUNDER/run.log" 2>&1 || true
[ -f "$THUNDER/thunder.py" ] && check "1" "1" "thunder.py exists" || check "0" "1" "thunder.py exists"
TLINES=$(wc -l < "$THUNDER/thunder.py" 2>/dev/null || echo 0)
[ "$TLINES" -gt 50 ] && check "big" "big" "thunder.py > 50 lines ($TLINES)" || check "$TLINES" ">50" "thunder.py > 50 lines"
python3 -m py_compile "$THUNDER/thunder.py" && check "compile" "compile" "thunder.py compiles" || check "fail" "compile" "thunder.py compiles"

echo
echo "== e2e result: $pass passed, $fail failed =="
rm -rf "$ROOT"
[ "$fail" -eq 0 ]
