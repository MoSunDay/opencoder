#!/usr/bin/env bash
# E2E regression: real multimodal (vision) closed loop against qwen provider.
#
# Feeds logo/logo.png + a prompt to `qwen/qwen3.8-max-preview`, then asserts
# actual business contracts (not surface markers):
#   1. user turn persisted a ContentBlock::Image carrying a base64 data URI
#   2. assistant turn persisted substantive vision text (describes the image)
#   3. streamed stdout exactly equals the persisted assistant text
#
# Requires: DASHSCOPE_API_KEY in env; network to the qwen MaaS endpoint.
# Usage:    scripts/e2e-qwen-vision.sh [binary] [image]
#           binary defaults to target/release/opencoder
#           image  defaults to logo/logo.png
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BIN="${1:-$ROOT/target/release/opencoder}"
IMAGE="${2:-$ROOT/logo/logo.png}"
MODEL="qwen/qwen3.8-max-preview"
WORKDIR="$(mktemp -d /tmp/vision-mm.XXXXXX)"
PROMPT="请详细描述这张图片的内容、配色和可能的用途。"

fail() { echo "FAIL: $*" >&2; exit 1; }

[ -x "$BIN" ]   || fail "binary not found/executable: $BIN"
[ -f "$IMAGE" ] || fail "image not found: $IMAGE"
[ -n "${DASHSCOPE_API_KEY:-}" ] || fail "DASHSCOPE_API_KEY not set in env"

echo ">> binary  : $BIN"
echo ">> image   : $IMAGE"
echo ">> model   : $MODEL"
echo ">> workdir : $WORKDIR"

# --image MUST precede the prompt, else the trailing var-arg swallows it.
"$BIN" --workdir "$WORKDIR" --model "$MODEL" --image "$IMAGE" \
  run "$PROMPT" >"$WORKDIR/out.txt" 2>"$WORKDIR/err.txt" || {
    tail -20 "$WORKDIR/err.txt" >&2; fail "run exited non-zero"; }

SID="$(grep -oP '\[session \K[0-9A-Z]+' "$WORKDIR/err.txt" | head -1)"
[ -n "$SID" ] || fail "no [session ID] marker on stderr"
echo ">> session : $SID"

"$BIN" --workdir "$WORKDIR" session show "$SID" --json >"$WORKDIR/show.json" \
  || fail "session show --json failed"

WORKDIR="$WORKDIR" python3 - <<'PY'
import json, os, re, sys
d = json.load(open(os.path.join(os.environ["WORKDIR"], "show.json")))
msgs = d["messages"]
user = next((m for m in msgs if m["role"] == "user"), None)
asst = next((m for m in msgs if m["role"] == "assistant"), None)
assert user, "no user message persisted"
assert asst, "no assistant message persisted"

# Contract 1: user turn carries an Image block with a base64 data URI.
imgs = [b for b in user["blocks"] if "image" in (b.get("kind") or "").lower()]
assert imgs, "user turn has no Image block"
url = imgs[0].get("url", "")
assert url.startswith("data:image/"), "Image block url is not a data URI"
assert "base64," in url, "Image block data URI is not base64-encoded"

# Contract 2: assistant turn persisted substantive vision text.
txt = "".join(b.get("text", "") for b in asst["blocks"])
assert len(txt) >= 200, f"assistant text too short to be a real description ({len(txt)})"
hits = [w for w in ("霓虹", "花括号", "渐变", "背景") if w in txt]
assert len(hits) >= 2, f"assistant text lacks visual attributes (hits={hits})"

# Contract 3: streamed stdout equals persisted assistant text.
out = open(os.path.join(os.environ["WORKDIR"], "out.txt")).read()
norm = lambda s: re.sub(r"\s+", "", s)
assert norm(out) == norm(txt), "stdout stream != persisted assistant text"

print(f"OK image_block=data_uri(base64,len={len(url)}) "
      f"assistant_chars={len(txt)} attrs={hits} stdout_match=true")
PY

echo "PASS: qwen vision closed loop verified"
