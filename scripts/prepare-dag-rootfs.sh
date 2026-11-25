#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

out="${1:-}"
[ -n "$out" ] || { echo "usage: $0 <image-dir>" >&2; exit 2; }
[ "$#" -eq 1 ] || { echo "unknown option: $2" >&2; exit 2; }
mkdir -p "$out"
out="$(cd "$out" && pwd)"

echo "==> building the dag-runner + agent-step-runner + agent-session-runner examples (debug profile reuses workspace artifacts)"
cargo build --manifest-path "$repo_root/Cargo.toml" -p opencoder-dag-runtime \
  --example dag-runner --example agent-step-runner --example agent-session-runner
target_dir="$(cargo metadata --manifest-path "$repo_root/Cargo.toml" --no-deps --format-version 1 |
  python3 -c 'import json, sys; print(json.load(sys.stdin)["target_directory"])')"
bins=("$target_dir/debug/examples/dag-runner" "$target_dir/debug/examples/agent-step-runner" \
  "$target_dir/debug/examples/agent-session-runner")

# The image is only the runners directory (<out>/usr/bin). Steps run with the
# host's tools through the read-only host bindings; nothing else is installed.
mkdir -p "$out/usr/bin"

echo "==> installing the DAG supervisor at usr/bin/dag-runner and the agent runners at usr/bin/agent-step-runner + usr/bin/agent-session-runner"
cp "${bins[0]}" "$out/usr/bin/dag-runner"
cp "${bins[1]}" "$out/usr/bin/agent-step-runner"
cp "${bins[2]}" "$out/usr/bin/agent-session-runner"
# Debug symbols belong in the build tree, not in the image. These examples are
# otherwise over 1 GiB together.
strip --strip-debug "$out/usr/bin/dag-runner" \
  "$out/usr/bin/agent-step-runner" \
  "$out/usr/bin/agent-session-runner"

echo "==> runners image ready at $out; set dag.runners_dir to $out/usr/bin"
echo "    agent steps use the host's python3/git/rg; Codex must live under a mapped host path (/usr/...) or in $out/usr/bin"
