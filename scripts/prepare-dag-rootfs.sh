#!/usr/bin/env bash
# prepare-dag-rootfs.sh — provision a runnable `<workflow_root>/rootfs` for
# `sandbox: runc` wasm steps and the manual runc tests.
#
# `opencoder-agent dag prepare-rootfs` writes the scaffold but leaves
# usr/ empty (runtimes are a provisioning concern). This script fills that
# gap with two in-repo example binaries plus the shared libs they need, all
# placed at their host-absolute paths inside the rootfs:
#   - `wasmtime-cli` — the wasm runtime (same wasmtime/wasi crates as the
#     in-process executor) for `sandbox: runc` wasm steps, at usr/bin/wasmtime;
#   - `agent-step-runner` — the container-side agent-session runner for
#     `dag.agent_sandbox = runc`, at usr/bin/agent-step-runner.
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

echo "==> building the wasmtime-cli + agent-step-runner examples (debug profile reuses workspace artifacts)"
cargo build --manifest-path "$repo_root/Cargo.toml" -p opencoder-dag-runtime \
  --example wasmtime-cli --example agent-step-runner
target_dir="$(cargo metadata --manifest-path "$repo_root/Cargo.toml" --no-deps --format-version 1 |
  python3 -c 'import json, sys; print(json.load(sys.stdin)["target_directory"])')"
bins=("$target_dir/debug/examples/wasmtime-cli" "$target_dir/debug/examples/agent-step-runner")

echo "==> writing the scaffold via 'opencoder-agent dag prepare-rootfs'"
cargo run -q --manifest-path "$repo_root/Cargo.toml" -p opencoder-agent -- dag prepare-rootfs --out "$out" >/dev/null

echo "==> installing the wasm runtime at usr/bin/wasmtime and the agent runner at usr/bin/agent-step-runner"
cp "${bins[0]}" "$out/usr/bin/wasmtime"
cp "${bins[1]}" "$out/usr/bin/agent-step-runner"

echo "==> mirroring the binaries' shared libs into the rootfs"
for bin in "${bins[@]}"; do
  ldd "$bin" | awk '/=> \//{print $3} /^[[:space:]]*\//{print $1}'
done | sort -u | while read -r lib; do
  dest="$out$lib"
  mkdir -p "$(dirname "$dest")"
  cp -L "$lib" "$dest"
done

echo "==> rootfs ready at $out (workflow root: $(dirname "$out"))"
