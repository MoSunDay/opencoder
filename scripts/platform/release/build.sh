#!/usr/bin/env bash
# Build one auditable platform release bundle from a clean commit.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
output=""

usage() {
  cat <<'USAGE'
Usage: scripts/platform/release/build.sh [--output DIR]

Builds the release binaries for the current platform from a clean commit
(Linux: opencoder, opencoder-cli, opencoder-server, opencoder-agent; macOS:
opencoder, opencoder-cli and opencoder-server, the agent binary is Linux-only), verifies their compiled
build metadata, and writes checksums plus manifest.json to an atomic bundle.
USAGE
}

sum_files() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$@"
  else
    shasum -a 256 "$@"
  fi
}

sum_check() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum -c "$1"
  else
    shasum -a 256 -c "$1"
  fi
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
case "$(uname -s)" in
  Linux) binaries=(opencoder opencoder-cli opencoder-server opencoder-agent) ;;
  Darwin) binaries=(opencoder opencoder-cli opencoder-server) ;;
  *) echo "unsupported release platform: $(uname -s)" >&2; exit 6 ;;
esac
commit="$(git rev-parse HEAD)"
short="$(git rev-parse --short HEAD)"
output="${output:-$repo_root/dist/opencoder-platform-$short}"
[[ ! -e "$output" ]] || { echo "release output already exists: $output" >&2; exit 4; }
mkdir -p "$(dirname "$output")"

"$repo_root/scripts/check-spa-drift.sh"
# SPA digest: sha256 over the contents of every file under crates/web/spa/dist,
# hashed in sorted relative-path order (pure python3, no GNU sha256sum needed).
# The digest input definition only has to stay self-consistent inside this
# script: the same value is injected via OPENCODER_SPA_SHA256, checked against
# the compiled build metadata, and recorded in the bundle manifest.
spa_digest="$(python3 - "$repo_root/crates/web/spa/dist" <<'PY'
import hashlib
import pathlib
import sys

root = pathlib.Path(sys.argv[1])
digest = hashlib.sha256()
for path in sorted(
    (p for p in root.rglob("*") if p.is_file() and not p.is_symlink()),
    key=lambda p: p.relative_to(root).as_posix(),
):
    digest.update(path.read_bytes())
print(digest.hexdigest())
PY
)"
packages=()
for binary in "${binaries[@]}"; do
  packages+=(-p "$binary")
done
OPENCODER_SPA_SHA256="$spa_digest" cargo build --release --locked "${packages[@]}"
target_dir="$(cargo metadata --no-deps --format-version 1 | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')"

stage="$(mktemp -d "${output}.tmp.XXXXXX")"
cleanup() { rm -rf "$stage"; }
trap cleanup EXIT
mkdir -p "$stage/bin"

for binary in "${binaries[@]}"; do
  source_path="$target_dir/release/$binary"
  [[ -x "$source_path" ]] || { echo "missing release binary: $source_path" >&2; exit 5; }
  cp "$source_path" "$stage/bin/$binary"
  chmod 0755 "$stage/bin/$binary"
  "$stage/bin/$binary" --build-info >"$stage/$binary.build-info.json"
done

python3 - "$stage" "$commit" "${binaries[@]}" <<'PY'
import json
import pathlib
import sys

stage = pathlib.Path(sys.argv[1])
expected_commit = sys.argv[2]
names = tuple(sys.argv[3:])
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

python3 - "$stage" "$commit" "$spa_digest" "${binaries[@]}" <<'PY'
import hashlib
import json
import pathlib
import sys

stage = pathlib.Path(sys.argv[1])
commit, spa_digest = sys.argv[2], sys.argv[3]
names = tuple(sys.argv[4:])
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

sum_paths=(manifest.json)
for binary in "${binaries[@]}"; do
  sum_paths+=("bin/$binary")
done
(
  cd "$stage"
  sum_files "${sum_paths[@]}" >SHA256SUMS
  sum_check SHA256SUMS
)
mv "$stage" "$output"
trap - EXIT
echo "release bundle: $output"
