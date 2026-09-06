#!/usr/bin/env bash
# check-spa-drift.sh — rebuild the SPA into a temp copy and diff against the
# committed dist/. Exit 1 on drift (i.e. someone edited src/ without running
# scripts/build-spa.sh). Read-only with respect to the repo's dist/.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
spa="$repo_root/crates/web/spa"
[ -d "$spa" ] || { echo "missing $spa" >&2; exit 1; }
[ -f "$spa/dist/static/app.js" ] || { echo "no committed dist/ — run scripts/build-spa.sh first" >&2; exit 1; }

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
mkdir "$tmp/spa"
cp "$spa/package.json" "$spa/package-lock.json" "$spa/index.html" "$spa/vite.config.js" "$tmp/spa/"
cp -R "$spa/src" "$tmp/spa/src"
if [ -d "$spa/public" ]; then
  cp -R "$spa/public" "$tmp/spa/public"
fi

if [ -d "$spa/node_modules" ]; then
  ln -s "$spa/node_modules" "$tmp/spa/node_modules"
else
  (cd "$tmp/spa" && npm ci --no-audit --no-fund)
fi

cd "$tmp/spa"
npm run build >/dev/null

if diff -r "$spa/dist" "$tmp/spa/dist"; then
  echo "spa dist: no drift"
else
  echo "spa dist: DRIFT detected — run scripts/build-spa.sh and commit dist/" >&2
  exit 1
fi
