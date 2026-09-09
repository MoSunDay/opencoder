#!/usr/bin/env bash
# prepare-dag-rootfs.sh — provision a runnable `<workflow_root>/rootfs` for
# `sandbox: runc` wasm steps and the manual runc tests.
#
# `opencoder-agent dag prepare-rootfs` writes the scaffold but leaves
# usr/ empty (a wasm runtime is a provisioning concern). This script fills
# that gap with the in-repo `wasmtime-cli` example — same wasmtime/wasi
# crates as the in-process executor — plus the shared libs it needs, all
# placed at their host-absolute paths inside the rootfs.
#
# Usage: scripts/prepare-dag-rootfs.sh <dir>        (dir must be named rootfs)
# Then:  DAG_TEST_ROOTFS=<dir> cargo test -p opencoder-dag-runtime --lib \
#          sandbox::runc -- --ignored
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

out="${1:-}"
[ -n "$out" ] || { echo "usage: $0 <rootfs-dir>" >&2; exit 2; }
mkdir -p "$out"
out="$(cd "$out" && pwd)"
[ "$(basename "$out")" = "rootfs" ] || {
  echo "rootfs dir must be named 'rootfs' (its parent becomes the workflow root)" >&2
  exit 2
}

echo "==> building the wasmtime-cli example (debug profile reuses workspace artifacts)"
cargo build --manifest-path "$repo_root/Cargo.toml" -p opencoder-dag-runtime --example wasmtime-cli
target_dir="$(cargo metadata --manifest-path "$repo_root/Cargo.toml" --no-deps --format-version 1 |
  python3 -c 'import json, sys; print(json.load(sys.stdin)["target_directory"])')"
bin="$target_dir/debug/examples/wasmtime-cli"

echo "==> writing the scaffold via 'opencoder-agent dag prepare-rootfs'"
cargo run -q --manifest-path "$repo_root/Cargo.toml" -p opencoder-agent -- dag prepare-rootfs --out "$out" >/dev/null

echo "==> installing the wasm runtime at usr/bin/wasmtime"
cp "$bin" "$out/usr/bin/wasmtime"

echo "==> mirroring the binary's shared libs into the rootfs"
ldd "$bin" | awk '/=> \//{print $3} /^[[:space:]]*\//{print $1}' | sort -u | while read -r lib; do
  dest="$out$lib"
  mkdir -p "$(dirname "$dest")"
  cp -L "$lib" "$dest"
done

echo "==> rootfs ready at $out (workflow root: $(dirname "$out"))"
