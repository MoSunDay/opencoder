#!/usr/bin/env python3
"""Create an immutable device manager package from one committed Git tree."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile
import os

ROOT = Path(__file__).resolve().parents[2]
SOURCE = 'deploy/device-manager/'


def git(*args):
    return subprocess.check_output(['git', '-C', str(ROOT), *args])


def build(output):
    commit = git('rev-parse', 'HEAD').decode().strip()
    names = [name for name in git('ls-tree', '-r', '--name-only', 'HEAD', SOURCE).decode().splitlines()
             if name.startswith(SOURCE + 'src/') or name == SOURCE + 'package.json']
    if not names or not any(name.endswith('/server.mjs') for name in names):
        raise ValueError('Committed device manager source is incomplete')
    if output.exists():
        raise FileExistsError(output)
    output.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix=output.name + '.tmp-', dir=output.parent) as temporary:
        stage = Path(temporary)
        files = {}
        for name in names:
            relative = name[len(SOURCE):]
            data = git('show', 'HEAD:' + name)
            target = stage / relative
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(data)
            files[relative] = hashlib.sha256(data).hexdigest()
        manifest = {'commit': commit, 'files': files, 'package': 'device-manager'}
        (stage / 'manifest.json').write_text(json.dumps(manifest, sort_keys=True, indent=2) + '\n')
        (stage / 'SHA256SUMS').write_text(''.join(f'{value}  {name}\n' for name, value in sorted(files.items())))
        os.rename(stage, output)
    return manifest


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', required=True, type=Path)
    print(json.dumps(build(parser.parse_args().output), sort_keys=True))
