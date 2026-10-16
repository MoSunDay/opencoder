#!/usr/bin/env bash
# Provision managed code at exactly the profile's immutable absolute path.
# Private configuration, auth and deployment state never enter the image.
set -euo pipefail
out="${1:?rootfs directory required}"
launcher="${2:?immutable launcher directory required}"
codex="${3:?native Codext executable required}"
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"
python3 - "$launcher" "$codex" <<'PY'
from pathlib import Path
import sys
for text in sys.argv[1:]:
    path = Path(text)
    if not path.is_absolute() or path.resolve() != path:
        sys.exit('Managed launcher/Codext paths must be absolute without aliases')
PY
for name in run accounts.py leases.py policy.py state.py client.py broker.py; do
  [ -f "$launcher/$name" ] && [ ! -L "$launcher/$name" ] || {
    echo "managed startup dependency missing: $name" >&2; exit 2;
  }
  mkdir -p "$out$launcher"
  cp "$launcher/$name" "$out$launcher/$name"
  chmod 644 "$out$launcher/$name"
done
chmod 755 "$out$launcher/run"
install_binary "$out" "$codex" "$codex"
paired="$(dirname "$codex")/codex-code-mode-host"
[ -x "$paired" ] || { echo "paired Codex code-mode host missing" >&2; exit 2; }
install_binary "$out" "$paired" "$paired"
python3 - "$out" "$launcher" "$codex" <<'PY'
import hashlib
import json
from pathlib import Path
import sys
out, launcher, codex = map(Path, sys.argv[1:])
names = ('run', 'accounts.py', 'leases.py', 'policy.py', 'state.py', 'client.py', 'broker.py')
manifest = {'schema_version': 1, 'program': str(launcher / 'run'), 'codex': str(codex),
            'files': {name: hashlib.sha256((launcher / name).read_bytes()).hexdigest() for name in names}}
destination = out / 'usr/share/opencoder/codex-startup.json'
destination.parent.mkdir(parents=True, exist_ok=True)
destination.write_text(json.dumps(manifest, sort_keys=True, indent=2) + '\n')
PY
run_in_rootfs "$out" /usr/bin/python3 -c \
  "import sys; sys.path.insert(0, '$launcher'); import accounts, leases, policy, state, client, broker"
run_in_rootfs "$out" "$codex" --version >/dev/null
echo "==> managed Codex startup code installed; private files are mounted at launch"
