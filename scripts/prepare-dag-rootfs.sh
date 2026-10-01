#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

out="${1:-}"
[ -n "$out" ] || { echo "usage: $0 <rootfs-dir> [--codex <native-codex-binary>]" >&2; exit 2; }
shift
codex_binary=""
if [ "$#" -gt 0 ]; then
  [ "$#" -eq 2 ] && [ "$1" = "--codex" ] || {
    echo "expected --codex <native-codex-binary>" >&2; exit 2;
  }
  codex_binary="$2"
fi
mkdir -p "$out"
out="$(cd "$out" && pwd)"

echo "==> building the dag-runner + agent-step-runner + agent-session-runner examples (debug profile reuses workspace artifacts)"
cargo build --manifest-path "$repo_root/Cargo.toml" -p opencoder-dag-runtime \
  --example dag-runner --example agent-step-runner --example agent-session-runner
target_dir="$(cargo metadata --manifest-path "$repo_root/Cargo.toml" --no-deps --format-version 1 |
  python3 -c 'import json, sys; print(json.load(sys.stdin)["target_directory"])')"
bins=("$target_dir/debug/examples/dag-runner" "$target_dir/debug/examples/agent-step-runner" \
  "$target_dir/debug/examples/agent-session-runner")

echo "==> writing the scaffold via 'opencoder-agent dag prepare-rootfs'"
cargo run -q --manifest-path "$repo_root/Cargo.toml" -p opencoder-agent -- dag prepare-rootfs --out "$out" >/dev/null

echo "==> installing the DAG supervisor at usr/bin/dag-runner and the agent runners at usr/bin/agent-step-runner + usr/bin/agent-session-runner"
cp "${bins[0]}" "$out/usr/bin/dag-runner"
cp "${bins[1]}" "$out/usr/bin/agent-step-runner"
cp "${bins[2]}" "$out/usr/bin/agent-session-runner"
# Debug symbols belong in the build tree, not in each container's private
# rootfs copy. These examples are otherwise over 1 GiB together.
strip --strip-debug "$out/usr/bin/dag-runner" \
  "$out/usr/bin/agent-step-runner" \
  "$out/usr/bin/agent-session-runner"

echo "==> mirroring the binaries' shared libs into the rootfs"
for bin in "${bins[@]}"; do
  ldd "$bin" | awk '/=> \//{print $3} /^[[:space:]]*\//{print $1}'
done | sort -u | while read -r lib; do
  dest="$out$lib"
  mkdir -p "$(dirname "$dest")"
  cp -L "$lib" "$dest"
done

bash "$repo_root/scripts/dag-rootfs/install-tools.sh" "$out"
bash "$repo_root/scripts/dag-rootfs/install-python.sh" "$out"

if [ -n "$codex_binary" ]; then
  bash "$repo_root/scripts/dag-rootfs/install-codex.sh" "$out" "$codex_binary"
fi

echo "==> rootfs ready at $out; set dag.rootfs_dir to this immutable directory"
