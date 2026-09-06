#!/usr/bin/env bash
# Build one auditable opencoder/opencoder-server/opencoder-agent release bundle.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
output=""

usage() {
  cat <<'USAGE'
Usage: scripts/platform/release/build.sh [--output DIR]

Builds all three release binaries from a clean commit, verifies their compiled
build metadata, and writes checksums plus manifest.json to an atomic bundle.
USAGE
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --output) output="${2:?--output requires a directory}"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "unknown argument: $1" >&2; usage >&2; exit 2 ;;
  esac
done

cd "$repo_root"
if [[ -n "$(git status --porcelain --untracked-files=normal)" ]]; then
  echo "release build requires a clean git worktree and index" >&2
  exit 3
fi
commit="$(git rev-parse HEAD)"
short="$(git rev-parse --short HEAD)"
output="${output:-$repo_root/dist/opencoder-platform-$short}"
[[ ! -e "$output" ]] || { echo "release output already exists: $output" >&2; exit 4; }
mkdir -p "$(dirname "$output")"

"$repo_root/scripts/check-spa-drift.sh"
spa_digest="$({ cd crates/web/spa/dist; find . -type f -print0 | sort -z | xargs -0 sha256sum; } | sha256sum | awk '{print $1}')"
OPENCODER_SPA_SHA256="$spa_digest" cargo build --release --locked -p opencoder -p opencoder-server -p opencoder-agent
target_dir="$(cargo metadata --no-deps --format-version 1 | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')"

stage="$(mktemp -d "${output}.tmp.XXXXXX")"
cleanup() { rm -rf "$stage"; }
trap cleanup EXIT
mkdir -p "$stage/bin"

binaries=(opencoder opencoder-server opencoder-agent)
for binary in "${binaries[@]}"; do
  source_path="$target_dir/release/$binary"
  [[ -x "$source_path" ]] || { echo "missing release binary: $source_path" >&2; exit 5; }
  cp "$source_path" "$stage/bin/$binary"
  chmod 0755 "$stage/bin/$binary"
  "$stage/bin/$binary" --build-info >"$stage/$binary.build-info.json"
done

python3 - "$stage" "$commit" <<'PY'
import json
import pathlib
import sys

stage = pathlib.Path(sys.argv[1])
expected_commit = sys.argv[2]
names = ("opencoder", "opencoder-server", "opencoder-agent")
infos = {name: json.loads((stage / f"{name}.build-info.json").read_text()) for name in names}
first = infos[names[0]]
for name, info in infos.items():
    if info != first:
        raise SystemExit(f"compiled build metadata differs for {name}")
if first["git_commit"] != expected_commit:
    raise SystemExit("compiled commit does not match release commit")
if first["git_dirty"]:
    raise SystemExit("compiled build metadata is dirty")
if not isinstance(first["protocol_version"], int) or first["protocol_version"] <= 0:
    raise SystemExit("compiled protocol version is invalid")
if len(first.get("spa_sha256", "")) != 64:
    raise SystemExit("compiled SPA digest is invalid")
PY

python3 - "$stage" "$commit" "$spa_digest" <<'PY'
import hashlib
import json
import pathlib
import sys

stage = pathlib.Path(sys.argv[1])
commit, spa_digest = sys.argv[2:]
names = ("opencoder", "opencoder-server", "opencoder-agent")
info = json.loads((stage / "opencoder.build-info.json").read_text())
if info["spa_sha256"] != spa_digest:
    raise SystemExit("compiled SPA digest does not match dist tree")
files = {}
for name in names:
    data = (stage / "bin" / name).read_bytes()
    files[f"bin/{name}"] = {"sha256": hashlib.sha256(data).hexdigest(), "bytes": len(data)}
manifest = {
    "schema_version": 1,
    "commit": commit,
    "version": info["version"],
    "version_long": info["version_long"],
    "protocol_version": info["protocol_version"],
    "spa_sha256": spa_digest,
    "files": files,
}
(stage / "manifest.json").write_text(json.dumps(manifest, indent=2, sort_keys=True) + "\n")
PY
rm "$stage"/*.build-info.json

(
  cd "$stage"
  sha256sum manifest.json bin/opencoder bin/opencoder-server bin/opencoder-agent >SHA256SUMS
  sha256sum -c SHA256SUMS
)
mv "$stage" "$output"
trap - EXIT
echo "release bundle: $output"
