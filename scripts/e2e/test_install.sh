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
#   C7  --backup saves the prior destination as <dest>.bak.<ts> before swap
#   C8  --backup on a non-existent destination is a no-op and exits 0
#
# Run:    scripts/e2e/test_install.sh
# Env:    OPENCODER_E2E_SOURCE (default: repo-local target/release/opencoder)
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

SRC="${OPENCODER_E2E_SOURCE:-$REPO_ROOT/target/release/opencoder}"

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

# --- C7: --backup saves prior destination before swap -----------------------
DEST3="$WORK/bin3/opencoder"
mkdir -p "$WORK/bin3"
OLDSTANDIN="$WORK/oldstandin.sh"
NEWSTANDIN="$WORK/newstandin.sh"
printf '#!/usr/bin/env bash\necho old v1.0\n'  > "$OLDSTANDIN"
printf '#!/usr/bin/env bash\necho new v2.0\n'  > "$NEWSTANDIN"
chmod 0755 "$OLDSTANDIN" "$NEWSTANDIN"
# Seed DEST3 with the "old" version first.
"$INSTALL" --no-build --source "$OLDSTANDIN" --dest "$DEST3" >"$WORK/c7a.log" 2>&1 || true
# Now install the "new" version WITH --backup.
if "$INSTALL" --no-build --source "$NEWSTANDIN" --dest "$DEST3" --backup >"$WORK/c7b.log" 2>&1; then
  new_ok=0; bak_ok=0; bak=""
  "$DEST3" 2>/dev/null | grep -q "new v2.0" && new_ok=1
  bak="$(find "$WORK/bin3" -name 'opencoder.bak.*' 2>/dev/null | head -n1)"
  [[ -n "$bak" ]] && "$bak" 2>/dev/null | grep -q "old v1.0" && bak_ok=1
  if [[ "$new_ok" -eq 1 && "$bak_ok" -eq 1 ]]; then
    ok "C7 --backup saved prior destination as a .bak.<ts> (content=old v1.0), dest now=new v2.0"
  else
    fail "C7 --backup contract not met (new_ok=$new_ok bak_ok=$bak_ok bak=$bak)"; cat "$WORK/c7b.log"
  fi
else
  fail "C7 --backup install exited non-zero"; cat "$WORK/c7b.log"
fi

# --- C8: --backup on fresh destination: no backup created, exit 0 -----------
DEST4="$WORK/bin4/opencoder"
mkdir -p "$WORK/bin4"
if "$INSTALL" --no-build --source "$NEWSTANDIN" --dest "$DEST4" --backup >"$WORK/c8.log" 2>&1 \
   && [[ -x "$DEST4" ]] \
   && [[ -z "$(find "$WORK/bin4" -name 'opencoder.bak.*' 2>/dev/null)" ]]; then
  ok "C8 --backup on non-existent dest: no .bak created, install still ok"
else
  fail "C8 --backup on fresh dest behaved unexpectedly"; cat "$WORK/c8.log"
fi

# --- summary ---------------------------------------------------------------
echo
echo "result: $passed passed, $failed failed"
if [[ "$failed" -ne 0 ]]; then exit 1; fi
exit 0
