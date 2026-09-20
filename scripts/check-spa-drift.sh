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
# Mirror the repo layout, not a bare "spa/" dir: src/ may import repo-root
# fixtures with repo-relative depth (e.g. src/brain/workbench/editor/model.js
# pulls ../../../../../../../examples/brain/repair-loop.json), which only
# resolves when crates/web/spa sits at its real depth next to examples/.
mirror="$tmp/crates/web/spa"
mkdir -p "$mirror"
cp "$spa/package.json" "$spa/package-lock.json" "$spa/index.html" "$spa/vite.config.js" "$mirror/"
cp -R "$spa/src" "$mirror/src"
if [ -d "$spa/public" ]; then
  cp -R "$spa/public" "$mirror/public"
fi
if [ -d "$repo_root/examples" ]; then
  cp -R "$repo_root/examples" "$tmp/examples"
fi

if [ -d "$spa/node_modules" ]; then
  ln -s "$spa/node_modules" "$mirror/node_modules"
else
  (cd "$mirror" && npm ci --no-audit --no-fund)
fi

cd "$mirror"

# A minified bundle diff is megabytes of single lines, so cap both the line
# count and the width. Must not be `printf | head`: head exits early, printf
# takes SIGPIPE, and `set -o pipefail` then aborts the script before it prints
# the verdict (observed: exit 141 with no "DRIFT detected" line). awk reads all
# of stdin, so nothing is left holding a broken pipe.
cap_diff() {
  printf '%s\n' "$1" | awk -v max=40 '
    NR <= max { print (length($0) > 200 ? substr($0, 1, 200) " ..." : $0) }
    NR == max + 1 { cut = 1 }
    END { if (cut) printf "... diff truncated to %d lines\n", max }
  '
}

# The minifier is pinned in package-lock.json. Every build must match; retries
# would conceal a broken reproducibility contract.
npm run build >/dev/null
if out="$(diff -r "$spa/dist" "$mirror/dist" 2>&1)"; then
  echo "spa dist: no drift"
else
  cap_diff "$out"
  echo "spa dist: DRIFT detected — run scripts/build-spa.sh and commit dist/" >&2
  exit 1
fi
