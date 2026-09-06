#!/usr/bin/env bash
# build-spa.sh — build the fleet-console SPA into crates/web/spa/dist.
# The dist/ output is COMMITTED (no content hashes) so `cargo build` embeds
# it verbatim and never needs node. Verify the output contract before exiting.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root/crates/web/spa"

# Optional build-time embedded server base (VITE_OC_BASE) for standalone SPA
# deployments: --base https://fleet.example.com, SPA_BASE=... env, or a local
# .env (VITE_OC_BASE=...). Precedence: --base > SPA_BASE > .env. Default
# (empty) keeps the committed dist same-origin — rebuild without the flag
# before committing when experimenting.
spa_base="${SPA_BASE:-}"
while [ $# -gt 0 ]; do
  case "$1" in
    --base) spa_base="${2:-}"; shift 2 ;;
    --base=*) spa_base="${1#--base=}"; shift ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac done
# Export only when set: an empty export would mask a local .env (inline env
# wins over .env files in vite). With no flag/env the build is plain `vite
# build` semantics — .env applies, absent here means same-origin dist.
if [ -n "$spa_base" ]; then
  export VITE_OC_BASE="$spa_base"
else
  unset VITE_OC_BASE
fi

if [ -f package-lock.json ]; then
  npm ci --no-audit --no-fund || npm install --no-audit --no-fund
else
  npm install --no-audit --no-fund
fi

npm run build

missing=0
for f in dist/index.html dist/static/app.js dist/static/app.css; do
  if [ ! -f "$f" ]; then
    echo "MISSING: $f" >&2
    missing=1
  fi
done
if [ "$missing" -ne 0 ]; then
  exit 1
fi

echo "--- spa dist sizes (VITE_OC_BASE=\"${spa_base:-}\") ---"
du -b dist/index.html dist/static/app.js dist/static/app.css | awk '{ printf "%8d  %s\n", $1, $2 }'
