#!/usr/bin/python3
"""Add the source guard to only this acceptance node's isolated model units."""
from pathlib import Path
import os
import json
import subprocess
import sys

args = sys.argv[1:]
runtime = Path(__file__).resolve().parents[1]
unit = args[args.index('--unit') + 1] if '--unit' in args else ''
strict = 'ProtectSystem=strict' in args
regression = unit.startswith('jy-regression-')
if strict or regression:
    owners = [a.split('=', 1)[1] for a in args if a.startswith('BindsTo=')]
    if len(owners) != 1 or not owners[0].startswith('opencoder-runner-'):
        raise RuntimeError('Strict E2E model unit requires its registered Runner owner')
    expected = 'oc-e2e-' + Path(__file__).resolve().parents[2].name + '-node.service'
    parents = subprocess.check_output(['/usr/bin/systemctl', 'show', owners[0],
        '-p', 'BindsTo', '--value'], universal_newlines=True).split()
    if expected not in parents:
        raise RuntimeError('Refusing to alter a unit outside this temporary node')
if strict:
    # PID 1 creates this namespace afresh; it cannot inherit the Runner's mount guard.
    args[args.index('--'):args.index('--')] = ['--property', 'ReadOnlyPaths=/root/workspace']
if regression:
    specification = Path(args[-1]).resolve()
    if runtime / 'jobs' not in specification.parents:
        raise RuntimeError('Regression specification escaped this acceptance runtime')
    spec = json.loads(specification.read_text())
    if spec.get('preparation') == ['go', 'mod', 'download']:
        cache = Path(spec['root']).resolve() / 'cache/go-mod/cache/download'
        if runtime / 'jobs' not in cache.parents or cache.exists():
            raise RuntimeError('Dependency seed requires a new private test cache')
        cache.parent.mkdir(parents=True, exist_ok=True)
        subprocess.run(['cp', '-a', '--reflink=auto', str(runtime / 'go-module-downloads'), str(cache)], check=True)
    # /usr/local/bin/go may point outside /usr, which the standard jail already binds.
    if spec.get('preparation') == ['go', 'mod', 'download'] or spec.get('command', [None])[0] == 'go':
        go = Path('/usr/local/bin/go').resolve(strict=True)
        spec.setdefault('readOnly', []).append(str(go.parent.parent))
    spec.setdefault('environment', {}).update(json.loads((runtime / 'go-environment.json').read_text()))
    specification.write_text(json.dumps(spec))
os.execv('/usr/bin/systemd-run', ['systemd-run', *args])
