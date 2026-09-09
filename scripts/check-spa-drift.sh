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

# The minifier is not bit-stable. Repeated `npm run build` of the SAME src/
# occasionally emits static/app.js with a different set of mangled identifiers
# (measured here: 1 variant in 4 back-to-back builds, a ~44-byte cascade of
# renames starting at byte 4; static/app.css and index.html never varied). One
# byte-diff therefore cries DRIFT at a perfectly faithful dist/. So while the
# difference is confined to static/app.js, rebuild and compare again. A real
# src/ edit changes semantics and differs on every attempt, so the retries can
# only drop false positives -- they can never hide drift.
MAX_BUILDS=3

drifted_paths() {
  printf '%s\n' "$1" | awk -v root="$spa/dist/" '
    /^diff -r / { p = $3; if (index(p, root) == 1) p = substr(p, length(root) + 1); print p; next }
    /^Files /   { p = $2; if (index(p, root) == 1) p = substr(p, length(root) + 1); print p; next }
    /^Only in / { print "only-in" }
  ' | sort -u
}

attempt=0
while :; do
  attempt=$((attempt + 1))
  npm run build >/dev/null
  out="$(diff -r "$spa/dist" "$tmp/spa/dist" 2>&1)" && {
    echo "spa dist: no drift (build $attempt/$MAX_BUILDS)"
    exit 0
  }
  touched="$(drifted_paths "$out")"
  if [ "$touched" != "static/app.js" ]; then
    printf '%s\n' "$out" | head -40   # a minified bundle diff is megabytes; the verdict is the point
    echo "spa dist: DRIFT detected — run scripts/build-spa.sh and commit dist/" >&2
    exit 1
  fi
  if [ "$attempt" -ge "$MAX_BUILDS" ]; then
    printf '%s\n' "$out" | head -40
    echo "spa dist: DRIFT detected — static/app.js differs on all $attempt builds" >&2
    echo "spa dist: run scripts/build-spa.sh and commit dist/" >&2
    exit 1
  fi
  echo "spa dist: only static/app.js differs (minifier naming) — rebuilding ($attempt/$MAX_BUILDS)" >&2
done
