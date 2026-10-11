#!/usr/bin/env bash
# Provision the DAG task runtime, without site packages.
set -euo pipefail
out="${1:?rootfs directory required}"
[ -d "$out" ] || { echo "rootfs directory missing" >&2; exit 2; }
out="$(cd "$out" && pwd)"
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"
"${2:-python3}" -I - "$out" <<'PY'
import os
import pathlib
import re
import shutil
import ssl
import subprocess
import sys
import sysconfig
root = pathlib.Path(sys.argv[1])
if sys.version_info < (3, 11):
    raise RuntimeError('DAG tasks require Python >= 3.11; pass its executable explicitly')
stdlib = pathlib.Path(sysconfig.get_path('stdlib')).resolve()

def copy_file(source, target):
    destination = root / str(target).lstrip('/')
    destination.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(source, destination)

def libraries(source):
    result = subprocess.run(['ldd', str(source)], stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                            text=True, env={'PATH': os.defpath, 'LD_LIBRARY_PATH': str(stdlib.parent)})
    if result.returncode and 'not a dynamic executable' not in result.stderr + result.stdout:
        raise RuntimeError('cannot resolve Python runtime libraries: ' + str(source))
    if 'not found' in result.stdout:
        raise RuntimeError('missing Python runtime library: ' + str(source))
    for line in result.stdout.splitlines():
        match = re.search(r'(?:=>\s*)?(/\S+)\s+\(', line)
        if match:
            copy_file(match[1], match[1])

binary = pathlib.Path(sys.executable).resolve()
# Preserve the interpreter layout, including $ORIGIN-relative bundled libraries.
copy_file(binary, binary)
guest_binary = root / 'usr/bin/python3'
if guest_binary != root / str(binary).lstrip('/'):
    guest_binary.parent.mkdir(parents=True, exist_ok=True)
    guest_binary.unlink(missing_ok=True)
    guest_binary.symlink_to(os.path.relpath(root / str(binary).lstrip('/'), guest_binary.parent))
libraries(binary)
for directory, folders, files in os.walk(str(stdlib)):
    folders[:] = [name for name in folders if name not in {'site-packages', 'dist-packages', '__pycache__', 'test', 'tests'}]
    for name in files:
        source = pathlib.Path(directory) / name
        copy_file(source, source)
for extension in (stdlib / 'lib-dynload').glob('*.so'):
    libraries(extension)
# Use the system trust store at this interpreter's compiled OpenSSL default.
certificates = pathlib.Path('/etc/ssl/certs/ca-certificates.crt')
copy_file(certificates, ssl.get_default_verify_paths().openssl_cafile)
shutil.copytree('/usr/share/zoneinfo', root / 'usr/share/zoneinfo', dirs_exist_ok=True)
PY
# Verify the runtime imports, default trust store and timezone in the guest.
# Some Python builds need /dev/urandom before importing any modules. As in the
# DAG container, provide devices at runtime, without baking them into the image.
privilege=()
[ "$(id -u)" -eq 0 ] || privilege=(sudo -n --)
"${privilege[@]}" unshare --mount --propagation private bash -c '
  set -e
  mount --bind /dev "$1/dev"
  mount -o remount,bind,ro "$1/dev"
  chroot "$1" /usr/bin/python3 -I -c "$2"
' -- "$out" 'import hashlib,json,pathlib,shutil,ssl,sys,tempfile,urllib.request; from zoneinfo import ZoneInfo; assert sys.version_info >= (3,11); assert ssl.create_default_context().cert_store_stats()["x509_ca"] > 0; assert ZoneInfo("Asia/Shanghai").key == "Asia/Shanghai"'
