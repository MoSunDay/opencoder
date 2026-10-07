"""Replayable recovery of old projects, Host state, launchers and services."""
import os
import base64
import json
from pathlib import Path
import shutil
import sqlite3
from contextlib import closing
from ..state import atomic_bytes
from ..backup_files import tree
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
    if target.is_symlink() or (target.exists() and not target.is_dir()):
        target.unlink()
    elif target.exists():
        shutil.rmtree(target)
    if not source.exists() and not source.is_symlink():
        return
    target.parent.mkdir(parents=True, exist_ok=True)
    if source.is_dir() and not source.is_symlink():
        tree.copy_tree(source, target)
    elif source.is_symlink():
        target.symlink_to(source.readlink())
    else:
        tree.copy_leaf(source, target)
    if not target.is_symlink():
        shutil.chown(target, user=source.stat().st_uid, group=source.stat().st_gid)


def data(settings, root, metadata):
    manifest = json.loads((root / 'data/backup-manifest.json').read_text())
    if manifest.get('ontology_files_root'):
        replace(root / 'data/ontology-files', Path(manifest['ontology_files_root']))
    for label, live in [('server', settings.server_data), ('resources', settings.state_dir / 'resources')]:
        saved = root / 'data' / label
        for source in saved.rglob('*.db'):
            relative = source.relative_to(saved)
            target = live / relative
            with closing(sqlite3.connect(archive.snapshot_uri(source), uri=True)) as conn:
                names = {row[0] for row in conn.execute("SELECT name FROM sqlite_schema WHERE type='table'")}
            if 'schema_version' in names:
                archive.restore_projects(source, target)
            elif 'platform_users' in names:
                raise ValueError('non-project database contains authentication tables')
            else:
                replace(source, target)
                database_owner(target, metadata['data_owners'][label + '/' + str(relative)])
                for suffix in ('-wal', '-shm'):
                    Path(str(target) + suffix).unlink(missing_ok=True)
    host = root / 'data/host'
    if host.exists():
        # Host owns handoff state only. Never replace a DB carrying auth data.
        for source in host.rglob('*.db'):
            with closing(sqlite3.connect(archive.snapshot_uri(source), uri=True)) as conn:
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
                database_owner(target / source.name, metadata['data_owners']['host/' + source.name])
                for suffix in ('-wal', '-shm'):
                    (target / (source.name + suffix)).unlink(missing_ok=True)


def database_owner(path, owner):
    uid, gid, mode = owner
    os.chown(path, uid, gid)
    path.chmod(mode)


def controls(settings, root, metadata):
    for path, files in metadata.get('bundles', {}).items():
        if archive.inventory(Path(path)) != files:
            raise ValueError('retained old release bundle was modified')
    recorded = {item['path'] for item in metadata['control'] if item['exists']}
    from ..state import Journal
    from .mounts import name as mount_name
    state = Journal(settings.state_dir).data['maintenance']
    candidate = Journal(settings.state_dir).record(state['target'])
    names = [candidate[key] for key in ('server_unit', 'host_unit', 'runtime_unit')]
    names.extend(mount_name(p['path']) for p in state['scope'].get('mounts', {}).get('native', []))
    for path in (settings.systemd_dir / name for name in names):
        if str(path) not in recorded:
            replace(root / 'absent', path)
    for item in metadata['control']:
        # Keep the public gate closed until old Server read/write checks pass.
        if item['path'] == str(settings.nginx_include):
            continue
        target = Path(item['path'])
        if target == settings.state_dir / 'resources':
            # Keep the live database inode and its authentication rows. Its
            # project tables were recovered transactionally above.
            if item['exists']:
                tree.copy_tree(root / item['copy'], target, dirs_exist_ok=True,
                                ignore=shutil.ignore_patterns('*.db', '*.db-wal', '*.db-shm'))
        else:
            replace(root / item['copy'] if item['exists'] else root / 'absent', target)
        if item['exists']:
            for relative, (uid, gid) in item.get('owners', {}).items():
                member = target / relative
                if member.exists() or member.is_symlink():
                    os.chown(member, uid, gid, follow_symlinks=False)


def ingress(settings, root, metadata, operations, seconds=90):
    item = next(item for item in metadata['control'] if item['path'] == str(settings.nginx_include))
    workers = operations.ingress_workers()
    replace(root / item['copy'] if item['exists'] else root / 'absent', settings.nginx_include)
    operations.run('nginx', '-t')
    operations.run('systemctl', 'reload', 'nginx')
    operations.wait(lambda: operations.ingress_switched(workers), seconds)
