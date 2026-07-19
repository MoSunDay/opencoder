#!/usr/bin/env bash
# Contract test for scripts/install.sh.
#
# Verifies the installer's observable contracts WITHOUT touching system paths
# or requiring an LLM API key (so it can run in any CI/dev shell):
#   C1  install.sh --no-build --source X --dest D exits 0
#   C2  installed file exists and is executable (0755)
#   C3  installed --version matches the source --version (bytes are identical)
#   C4  idempotent: two installs yield identical md5
#   C5  no atomic-staging leftovers (*.new.* ) after install
#   C6  --source override installs the given binary verbatim
#
# Run:    scripts/e2e/test_install.sh
# Env:    OPENCODER_E2E_SOURCE (default: /data/caches/opencoder-target/release/opencoder)
# Exit:   0 all contracts pass, 1 otherwise.

set -euo pipefail

PROGNAME="$(basename "$0")"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$HERE/../.." && pwd)"
INSTALL="$REPO_ROOT/scripts/install.sh"

passed=0
failed=0
ok()   { echo "  ok   - $1"; passed=$((passed+1)); }
fail() { echo "  FAIL - $1"; failed=$((failed+1)); }

WORK="$(mktemp -d)"
cleanup() { rm -rf "$WORK"; }
trap cleanup EXIT

SRC="${OPENCODER_E2E_SOURCE:-/data/caches/opencoder-target/release/opencoder}"

echo "== install.sh contract tests =="
echo "installer: $INSTALL"
echo "source:    $SRC"

if [[ ! -x "$INSTALL" ]]; then
  echo "$PROGNAME: install.sh not found / not executable at $INSTALL" >&2; exit 1
fi
if [[ ! -x "$SRC" ]]; then
  echo "$PROGNAME: pre-built source binary missing at $SRC" >&2
  echo "$PROGNAME: run \`cargo build --release\` first" >&2; exit 1
fi

mkdir -p "$WORK/bin" "$WORK/bin2"

# --- C1 + C2: basic install ------------------------------------------------
DEST1="$WORK/bin/opencoder"
log1="$WORK/c1.log"
if "$INSTALL" --no-build --source "$SRC" --dest "$DEST1" >"$log1" 2>&1; then
  ok "C1 install.sh exits 0"
else
  fail "C1 install.sh exited non-zero"; cat "$log1"
fi
if [[ -x "$DEST1" ]]; then ok "C2 installed file exists and is executable"; else fail "C2 installed file missing/not executable"; fi

# --- C3: version match -----------------------------------------------------
if [[ -x "$DEST1" ]]; then
  sv="$("$SRC" --version 2>/dev/null || true)"
  dv="$("$DEST1" --version 2>/dev/null || true)"
  if [[ -n "$sv" && "$sv" == "$dv" ]]; then
    ok "C3 installed --version matches source ($dv)"
  else
    fail "C3 version mismatch: source='$sv' dest='$dv'"
  fi
fi

# --- C4: idempotency -------------------------------------------------------
if [[ -x "$DEST1" ]]; then
  h1="$(md5sum "$DEST1" | awk '{print $1}')"
  "$INSTALL" --no-build --source "$SRC" --dest "$DEST1" >"$WORK/c4.log" 2>&1 || true
  h2="$(md5sum "$DEST1" | awk '{print $1}')"
  if [[ "$h1" == "$h2" ]]; then ok "C4 idempotent (md5 stable across two installs)"; else fail "C4 second install changed bytes ($h1 -> $h2)"; fi
fi

# --- C5: no staging leftovers ----------------------------------------------
leftovers="$(find "$WORK/bin" -name '*.new.*' 2>/dev/null || true)"
if [[ -z "$leftovers" ]]; then ok "C5 no atomic-staging leftovers"; else fail "C5 leftover staging files: $leftovers"; fi

# --- C6: --source override honoured ----------------------------------------
STANDIN="$WORK/standin.sh"
printf '#!/usr/bin/env bash\necho standin v1.0\n' > "$STANDIN"
chmod 0755 "$STANDIN"
DEST2="$WORK/bin2/opencoder"
log6="$WORK/c6.log"
if "$INSTALL" --no-build --source "$STANDIN" --dest "$DEST2" >"$log6" 2>&1 \
   && [[ -x "$DEST2" ]] && "$DEST2" 2>/dev/null | grep -q "standin v1.0"; then
  ok "C6 --source override installs the given binary verbatim"
else
  fail "C6 --source override not honoured"; cat "$log6"
fi

# --- summary ---------------------------------------------------------------
echo
echo "result: $passed passed, $failed failed"
if [[ "$failed" -ne 0 ]]; then exit 1; fi
exit 0
