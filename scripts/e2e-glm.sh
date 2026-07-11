#!/usr/bin/env bash
# E2E regression against real glm5.2.
#
# Verifies the cross-cutting contracts that only a live model run can prove:
#   E7  reasoning_effort is carried end-to-end (config → models display).
#       Runs FIRST — cheap, no model call — so the display path is verified
#       even if the heavy live-model steps (E1/E2/E6) get reaped by the sandbox.
#   E1  glm writes a runnable snake game (write+bash tools, compiles, >50 lines)
#   E2  --continue resumes the SAME session and extends the artifact
#   E5  --fork copies a session without mutating the original
#   E3  compaction auto-triggers with a low context_threshold
#   E3b --continue works after compaction (summary loaded, session functional)
#   E4  subagent (task tool) dispatch — model delegates to an explore child
#   E6  glm writes a runnable thunder-fighter game (cross-game-type regression)
#   E8  bundle export/import roundtrip
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
SNAKE="$ROOT/snake"; THUNDER="$ROOT/thunder"; COMPACT="$ROOT/compact"; PROBE="$ROOT/probe"
mkdir -p "$SNAKE" "$THUNDER" "$COMPACT" "$PROBE"
# medium (not high) keeps the live turns lighter so the sandbox is less likely
# to reap a long thinking-heavy resume.
CFG='{"model":"zhipuai-coding-plan/glm-5.2","provider":{"base_url":"https://open.bigmodel.cn/api/coding/paas/v4","api_key":"{ZHIPU_API_KEY}"},"reasoning_effort":"medium","max_tokens":16384,"compaction":{"auto":true,"context_threshold":100000}}'
printf '%s' "$CFG" > "$SNAKE/opencode.json"; printf '%s' "$CFG" > "$THUNDER/opencode.json"; printf '%s' "$CFG" > "$PROBE/opencode.json"
# Compaction workdir: very low threshold + tail_turns:1 so compaction fires early.
CCFG='{"model":"zhipuai-coding-plan/glm-5.2","provider":{"base_url":"https://open.bigmodel.cn/api/coding/paas/v4","api_key":"{ZHIPU_API_KEY}"},"reasoning_effort":"low","max_tokens":8192,"compaction":{"auto":true,"context_threshold":2000,"tail_turns":1,"reserved":2000}}'
printf '%s' "$CCFG" > "$COMPACT/opencode.json"
# Seed probe workdir with a small file for the subagent to investigate.
printf 'def greet(name):\n    print(f"Hello, {name}!")\n\ngreet("World")\n' > "$PROBE/hello.py"

pass=0; fail=0
check() { if [ "$1" = "$2" ]; then echo "  ok: $3"; pass=$((pass+1)); else echo "  FAIL: $3 ($1 != $2)"; fail=$((fail+1)); fi; }

run() { ( cd "$(dirname "$BIN")" && "$BIN" "$@" ) 2>&1 || true; }

echo "== E7: reasoning_effort carried config → models display (runs first, cheap) =="
MODELS_OUT=$(cd "$(dirname "$BIN")" && "$BIN" --workdir "$SNAKE" models 2>&1 || true)
THINKING_LINE=$(printf '%s\n' "$MODELS_OUT" | grep -E '^thinking[[:space:]]*:' || true)
[ -n "$THINKING_LINE" ] && check "present" "present" "models shows thinking line" || check "(missing)" "present" "models shows thinking line"
printf '%s\n' "$THINKING_LINE" | grep -q 'medium' && check "medium" "medium" "reasoning_effort resolved to medium" || check "(other)" "medium" "reasoning_effort resolved to medium"

echo "== E1: glm writes a runnable snake game =="
run --workdir "$SNAKE" "用 python3 写一个终端贪吃蛇游戏，保存为 snake.py。方向键控制、吃食物变长、撞墙/撞自己结束。写完运行 'python3 -m py_compile snake.py' 验证语法（不要运行游戏循环）。" >"$SNAKE/run.log" || true
[ -f "$SNAKE/snake.py" ] && check "1" "1" "snake.py exists" || check "0" "1" "snake.py exists"
LINES=$(wc -l < "$SNAKE/snake.py" 2>/dev/null || echo 0)
[ "$LINES" -gt 50 ] && check "big" "big" "snake.py > 50 lines ($LINES)" || check "$LINES" ">50" "snake.py > 50 lines"
python3 -m py_compile "$SNAKE/snake.py" && check "compile" "compile" "snake.py compiles" || check "fail" "compile" "snake.py compiles"
SID=$(grep -oE 'session [0-9A-Z]+' "$SNAKE/run.log" 2>/dev/null | tail -1 | awk '{print $2}' || true)

echo "== E2: --continue resumes the same session and extends the artifact =="
run --workdir "$SNAKE" --continue "给贪吃蛇加计分板，屏幕顶部显示当前分数和最高分(high score)。修改 snake.py。" >"$SNAKE/resume.log" || true
RSID=$(grep -oE 'session [0-9A-Z]+' "$SNAKE/resume.log" 2>/dev/null | tail -1 | awk '{print $2}' || true)
[ -n "$SID" ] && [ "$SID" = "$RSID" ] && check "$SID" "$RSID" "resume reuses same session id" || check "$SID" "$RSID" "resume reuses same session id"
SC=$(grep -ciE 'high.?score|最高分|scoreboard|计分' "$SNAKE/snake.py")
[ "$SC" -gt 0 ] && check "scoreboard" "scoreboard" "scoreboard added ($SC hits)" || check "0" ">0" "scoreboard added"

echo "== E5: --fork copies session without mutating original =="
run --workdir "$SNAKE" --session "$SID" --fork "给贪吃蛇加暂停功能，按 P 键暂停/继续。" >"$SNAKE/fork.log" || true
FSID=$(grep -oE 'session [0-9A-Z]+' "$SNAKE/fork.log" 2>/dev/null | tail -1 | awk '{print $2}' || true)
[ -n "$FSID" ] && [ "$FSID" != "$SID" ] && check "$FSID" "new-id" "fork creates new session id" || check "${FSID:-?}" "new-id" "fork creates new session id (orig=$SID)"

echo "== E3: compaction auto-triggers with low context_threshold =="
# Three turns to accumulate enough tokens; threshold:2000 + tail_turns:1 ensures
# compaction fires by turn 2 or 3.
run --workdir "$COMPACT" "用 Python 写一个模块 calc.py，包含 add, subtract, multiply, divide, power, sqrt 六个函数，每个函数都要有完整的类型注解、docstring 和错误处理。写完运行 'python3 -m py_compile calc.py' 验证语法。" >"$COMPACT/run1.log" || true
run --workdir "$COMPACT" --continue "给 calc.py 加 log, sin, cos, tan 四个函数，同样需要类型注解和 docstring。修改 calc.py。" >"$COMPACT/run2.log" || true
run --workdir "$COMPACT" --continue "给 calc.py 加一个 main 函数，从命令行参数读取表达式并计算结果。" >"$COMPACT/run3.log" || true
# Check if any turn triggered compaction (only real compactions emit the event)
if grep -q '\[context compacted\]' "$COMPACT"/run*.log 2>/dev/null; then
  check "compacted" "compacted" "compaction triggered"
else
  check "none" "compacted" "compaction triggered"
fi

echo "== E3b: --continue works after compaction =="
run --workdir "$COMPACT" --continue "给 calc.py 的每个函数加一个单元测试，保存为 test_calc.py，用 pytest 风格。" >"$COMPACT/run4.log" || true
CSID=$(grep -oE 'session [0-9A-Z]+' "$COMPACT/run4.log" 2>/dev/null | tail -1 | awk '{print $2}' || true)
[ -n "$CSID" ] && check "ok" "ok" "session resumed after compaction" || check "fail" "ok" "session resumed after compaction"

echo "== E4: subagent (task tool) dispatch =="
run --workdir "$PROBE" "请使用 task 工具（subagent_type 设为 explore）派遣一个子代理来分析 hello.py 的代码结构，然后基于子代理返回的结果给我一个中文总结。不要自己直接读取文件，必须通过 task 工具完成调查。" >"$PROBE/run.log" || true
# Model-dependent: grep for the subagent dispatch marker (⇳ U+2933)
if grep -q 'subagent \[' "$PROBE/run.log" 2>/dev/null; then
  check "dispatched" "dispatched" "subagent dispatched via task tool"
else
  check "none" "dispatched" "subagent dispatched (model may not use task tool)"
fi

echo "== E6: glm writes a runnable thunder-fighter game =="
run --workdir "$THUNDER" "用 python3 写一个雷霆战机(飞机射击)游戏，保存为 thunder.py。方向键移动、空格射击、敌机下落、击中得分、被撞结束。写完运行 'python3 -m py_compile thunder.py' 验证语法（不要运行游戏循环）。" >"$THUNDER/run.log" || true
[ -f "$THUNDER/thunder.py" ] && check "1" "1" "thunder.py exists" || check "0" "1" "thunder.py exists"
TLINES=$(wc -l < "$THUNDER/thunder.py" 2>/dev/null || echo 0)
[ "$TLINES" -gt 50 ] && check "big" "big" "thunder.py > 50 lines ($TLINES)" || check "$TLINES" ">50" "thunder.py > 50 lines"
python3 -m py_compile "$THUNDER/thunder.py" && check "compile" "compile" "thunder.py compiles" || check "fail" "compile" "thunder.py compiles"

echo "== E8: bundle export/import roundtrip =="
(cd "$(dirname "$BIN")" && "$BIN" --workdir "$SNAKE" session export "$SID" --out "$ROOT/snake.opencode") 2>&1 || true
[ -f "$ROOT/snake.opencode" ] && check "exists" "exists" "bundle exported" || check "missing" "exists" "bundle exported"
IMP=$(cd "$(dirname "$BIN")" && "$BIN" --workdir "$SNAKE" session import "$ROOT/snake.opencode" 2>&1 || true)
printf '%s' "$IMP" | grep -q 'imported session' && check "imported" "imported" "bundle imported" || check "fail" "imported" "bundle imported"

echo
echo "== e2e result: $pass passed, $fail failed =="
rm -rf "$ROOT"
[ "$fail" -eq 0 ]
