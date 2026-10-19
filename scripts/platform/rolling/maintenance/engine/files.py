"""Stopped snapshots and engine ownership; application rows are never rewritten."""
from concurrent.futures import ThreadPoolExecutor
import hashlib
import json
import os
from pathlib import Path
import tempfile
from ... import backup
from ...state import atomic_bytes, write


def snapshot_bytes(registered):
    from ...backup_files.scope import RuntimeScope
    total = 0
    for item in registered.values():
        root = Path(json.loads(item['config'])['data_dir'])
        scope = RuntimeScope(root)
        for directory, directories, names in os.walk(root, followlinks=False):
            scope.prune(directory, directories, names)
            for name in names:
                path = Path(directory) / name
                if name.endswith(('.db', '.db-wal')) and not path.is_symlink():
                    total += path.stat().st_size
    return total


def managed(settings, registered):
    paths = set()
    for root in (settings.server_data, settings.state_dir / 'resources'):
        paths.update(root / name for name in ('definitions.db', 'control.db', 'ontology.db'))
    paths.add(settings.state_dir / 'host/host.db')
    paths.update(Path(json.loads(item['config'])['data_dir']) / 'runtime.db'
                 for item in registered.values())
    return sorted(path for path in paths if path.is_file())


def snapshot(output, registered):
    if output.exists():
        verify(output)
        return
    stage = Path(tempfile.mkdtemp(prefix='.runtime-backup-', dir=output.parent))
    with ThreadPoolExecutor(max_workers=4) as executor:
        tasks = [executor.submit(backup.databases, Path(json.loads(item['config'])['data_dir']),
                                 stage / identifier, True)
                 for identifier, item in registered.items()]
        for task in tasks:
            task.result()
    hashes = {str(path.relative_to(stage)): hashlib.sha256(path.read_bytes()).hexdigest()
              for path in stage.rglob('*.db')}
    write(stage / 'manifest.json', {'kind': 'stopped-runtime-databases', 'sha256': hashes})
    for directory in [stage, *(path for path in stage.rglob('*') if path.is_dir())]:
        fd = os.open(directory, os.O_RDONLY)
        try:
            os.fsync(fd)
        finally:
            os.close(fd)
    os.rename(stage, output)
    fd = os.open(output.parent, os.O_RDONLY)
    try:
        os.fsync(fd)
    finally:
        os.close(fd)


def verify(output):
    metadata = json.loads((output / 'manifest.json').read_text())
    for name, digest in metadata['sha256'].items():
        if hashlib.sha256((output / name).read_bytes()).hexdigest() != digest:
            raise ValueError('stopped Runtime database backup checksum differs')


def mark(paths):
    for path in paths:
        marker = Path(str(path) + '.engine.json')
        value = {'engine': 'turso', 'format': 1}
        if marker.exists() and json.loads(marker.read_text()) != value:
            raise ValueError('database belongs to an unknown engine')
        owner = path.stat()
        atomic_bytes(marker, json.dumps(value).encode(), 0o644, (owner.st_uid, owner.st_gid))


def checkpoint_and_unmark(paths, executable, operations):
    for path in paths:
        marker = Path(str(path) + '.engine.json')
        if not marker.exists():
            continue
        operations.run(str(executable), 'storage', 'checkpoint', '--database', str(path), '--writers-stopped')
        wal = Path(str(path) + '-wal')
        if wal.exists() and wal.stat().st_size:
            raise ValueError('Turso WAL is not empty after stopped checkpoint')
        marker.unlink()
        fd = os.open(path.parent, os.O_RDONLY)
        try:
            os.fsync(fd)
        finally:
            os.close(fd)
