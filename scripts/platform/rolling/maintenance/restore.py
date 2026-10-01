"""Replayable recovery of old projects, Host state, launchers and services."""
import os
import base64
import json
from pathlib import Path
import shutil
import sqlite3
import subprocess
from contextlib import closing
from ..state import atomic_bytes
from . import archive


def prior_controller(settings, release_id):
    path = settings.state_dir / 'maintenance-controller.json'
    if not path.exists():
        return
    receipt = json.loads(path.read_text())
    if receipt['release_id'] != release_id:
        raise ValueError('controller backup belongs to a different candidate')
    target = Path(receipt['path'])
    if receipt['content'] is None:
        target.unlink(missing_ok=True)
    else:
        atomic_bytes(target, base64.b64decode(receipt['content']), 0o644)


def replace(source, target):
    if target.is_symlink() or target.is_file():
        target.unlink()
    elif target.exists():
        shutil.rmtree(target)
    if not source.exists() and not source.is_symlink():
        return
    target.parent.mkdir(parents=True, exist_ok=True)
    if source.is_dir() and not source.is_symlink():
        shutil.copytree(source, target, symlinks=True)
    elif source.is_symlink():
        target.symlink_to(source.readlink())
    else:
        atomic_bytes(target, source.read_bytes(), source.stat().st_mode & 0o777)
        shutil.copystat(source, target)
    if not target.is_symlink():
        shutil.chown(target, user=source.stat().st_uid, group=source.stat().st_gid)


def data(settings, root, metadata):
    for source in (root / 'data/server').rglob('*.db'):
        target = settings.server_data / source.relative_to(root / 'data/server')
        with closing(sqlite3.connect(source.resolve().as_uri() + '?mode=ro&immutable=1', uri=True)) as conn:
            names = {row[0] for row in conn.execute("SELECT name FROM sqlite_schema WHERE type='table'")}
        if 'schema_version' in names:
            archive.restore_projects(source, target)
        elif 'platform_users' in names:
            raise ValueError('non-project database contains authentication tables')
        else:
            replace(source, target)
            for suffix in ('-wal', '-shm'):
                Path(str(target) + suffix).unlink(missing_ok=True)
    host = root / 'data/host'
    if host.exists():
        # Host owns handoff state only. Never replace a DB carrying auth data.
        for source in host.rglob('*.db'):
            with closing(sqlite3.connect(source.resolve().as_uri() + '?mode=ro&immutable=1', uri=True)) as conn:
                names = {row[0] for row in conn.execute("SELECT name FROM sqlite_schema WHERE type='table'")}
                if 'platform_users' in names:
                    raise ValueError('Host backup unexpectedly contains authentication tables')
        target = settings.state_dir / 'host'
        # Keep lock inodes: the recovering controller still holds deploy.lock,
        # and no second writer may acquire a new namespace during recovery.
        for source in host.iterdir():
            if source.name.endswith('.locks') or source.name == 'deployment.json':
                continue
            replace(source, target / source.name)
            if source.suffix == '.db':
                for suffix in ('-wal', '-shm'):
                    (target / (source.name + suffix)).unlink(missing_ok=True)


def controls(settings, root, metadata):
    for path, files in metadata.get('bundles', {}).items():
        if archive.inventory(Path(path)) != files:
            raise ValueError('retained old release bundle was modified')
    recorded = {item['path'] for item in metadata['control'] if item['exists']}
    from ..state import Journal
    journal = Journal(settings.state_dir).data
    state = journal['maintenance']
    candidate = journal['releases'][state['target']]
    names = {candidate[key] for key in ('server_unit', 'host_unit', 'runtime_unit')}
    for plan in state['scope'].get('mounts', {}).get('native', []):
        names.add(subprocess.check_output(['systemd-escape', '--path', '--suffix=mount', plan['path']], text=True).strip())
    for path in (settings.systemd_dir / name for name in names):
        if str(path) not in recorded:
            replace(root / 'absent', path)
    for item in metadata['control']:
        # Keep the public gate closed until old Server read/write checks pass.
        if item['path'] == str(settings.nginx_include):
            continue
        replace(root / item['copy'] if item['exists'] else root / 'absent', Path(item['path']))
        if item['exists']:
            for relative, (uid, gid) in item.get('owners', {}).items():
                os.chown(Path(item['path']) / relative, uid, gid, follow_symlinks=False)


def ingress(settings, root, metadata, operations):
    item = next(item for item in metadata['control'] if item['path'] == str(settings.nginx_include))
    replace(root / item['copy'] if item['exists'] else root / 'absent', settings.nginx_include)
    operations.run('nginx', '-t')
    operations.run('systemctl', 'reload', 'nginx')
