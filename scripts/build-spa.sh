#!/usr/bin/env bash
# build-spa.sh — build the fleet-console SPA into crates/web/spa/dist.
# The dist/ output is COMMITTED (no content hashes) so `cargo build` embeds
# it verbatim and never needs node. Verify the output contract before exiting.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root/crates/web/spa"

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

echo "--- spa dist sizes ---"
du -b dist/index.html dist/static/app.js dist/static/app.css | awk '{ printf "%8d  %s\n", $1, $2 }'
