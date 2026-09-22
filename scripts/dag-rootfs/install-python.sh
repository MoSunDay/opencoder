#!/usr/bin/env bash
# Provision the standard-library-only task collector runtime, without site packages.
set -euo pipefail
out="${1:?rootfs directory required}"
[ -d "$out" ] || { echo "rootfs directory missing" >&2; exit 2; }
out="$(cd "$out" && pwd)"
/usr/bin/python3 - "$out" <<'PY'
import os
import pathlib
import re
import shutil
import subprocess
import sys
import sysconfig
root = pathlib.Path(sys.argv[1])

def copy_file(source, target):
    destination = root / str(target).lstrip('/')
    destination.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(source, destination)

def libraries(source):
    result = subprocess.run(['ldd', str(source)], stdout=subprocess.PIPE, stderr=subprocess.PIPE, universal_newlines=True)
    if result.returncode and 'not a dynamic executable' not in result.stderr + result.stdout:
        raise RuntimeError('cannot resolve Python runtime libraries: ' + str(source))
    if 'not found' in result.stdout:
        raise RuntimeError('missing Python runtime library: ' + str(source))
    for line in result.stdout.splitlines():
        match = re.search(r'(?:=>\s*)?(/\S+)\s+\(', line)
        if match:
            copy_file(match[1], match[1])

binary = pathlib.Path(sys.executable).resolve()
copy_file(binary, '/usr/bin/python3')
libraries(binary)
stdlib = pathlib.Path(sysconfig.get_path('stdlib')).resolve()
destination = root / str(stdlib).lstrip('/')
for directory, folders, files in os.walk(str(stdlib)):
    folders[:] = [name for name in folders if name not in {'site-packages', 'dist-packages', '__pycache__', 'test', 'tests'}]
    for name in files:
        source = pathlib.Path(directory) / name
        copy_file(source, source)
for extension in (stdlib / 'lib-dynload').glob('*.so'):
    libraries(extension)
# A relocated /usr/bin interpreter searches /usr/lib even when the host uses /usr/local.
guest_stdlib = root / 'usr/lib' / ('python%d.%d' % sys.version_info[:2])
if guest_stdlib != destination:
    guest_stdlib.parent.mkdir(parents=True, exist_ok=True)
    if guest_stdlib.is_symlink() and guest_stdlib.resolve() == destination:
        pass
    elif guest_stdlib.exists() or guest_stdlib.is_symlink():
        raise RuntimeError('conflicting guest Python standard library')
    else:
        guest_stdlib.symlink_to(os.path.relpath(destination, guest_stdlib.parent))
PY
# Verify the same imports used by the collector inside the actual rootfs.
chroot "$out" /usr/bin/python3 -I -c 'import hashlib,json,pathlib,ssl,tempfile,urllib.request; assert ssl.OPENSSL_VERSION'
