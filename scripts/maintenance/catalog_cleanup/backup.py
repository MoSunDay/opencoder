"""Immutable object backups are published only after complete verification."""
import hashlib
import json
from pathlib import Path
import shutil
import tempfile
from .client import require, write, sync_tree, sync_directory
from .model import digest


def verify_pool(path, snapshot):
    require(not path.is_symlink() and not any(p.is_symlink() for p in path.rglob('*')), 'pool symlink')
    meta = json.loads((path / 'meta.json').read_text())
    require({**meta, 'history': snapshot['history']} == snapshot, 'pool metadata changed')
    require(meta['history'] == [v['version'] for v in snapshot['history']], 'version list changed')
    expected = {'meta.json'}
    for version in snapshot['history']:
        require(type(version['version']) is int and version['version'] > 0, 'invalid version')
        directory = path / ('v' + str(version['version']))
        require(json.loads((directory / 'meta.json').read_text()) == version, 'version metadata changed')
        data = (directory / 'binary.bin').read_bytes()
        require(len(data) == version['size_bytes'] and hashlib.sha256(data).hexdigest() == version['sha256'], 'binary checksum mismatch')
        expected.add(directory.name)
        require({p.name for p in directory.iterdir()} == {'meta.json', 'binary.bin'}, 'unlisted version data')
    require({p.name for p in path.iterdir()} == expected, 'unlisted pool data')



def verify_receipt(receipt):
    backup = Path(receipt['backup'])
    require(not backup.is_symlink(), 'backup symlink')
    snapshot = json.loads((backup / 'snapshot.json').read_text())
    require(digest(snapshot) == receipt.get('digest'), 'backup digest mismatch')
    require(snapshot.get('name', snapshot.get('id')) == receipt['name'], 'backup name mismatch')
    require(receipt['kind'] in ('dag', 'binary'), 'invalid backup kind')
    if receipt['kind'] == 'binary':
        verify_pool(backup / 'pool', snapshot)
    return snapshot


def create_backup(directory, kind, name, value, binary_root):
    backup = directory / (kind + '-' + hashlib.sha256(name.encode()).hexdigest()[:24])
    if not backup.exists():
        stage = Path(tempfile.mkdtemp(prefix='.backup-stage-', dir=directory))
        # A crash leaves only an unpublished stage. Retrying never mistakes it
        # for the immutable backup or overwrites an already published anchor.
        write(stage / 'snapshot.json', value)
        if kind == 'binary':
            source = Path(binary_root) / name
            verify_pool(source, value)
            shutil.copytree(source, stage / 'pool', symlinks=True)
            verify_pool(stage / 'pool', value)
        sync_tree(stage)
        stage.rename(backup)
        sync_directory(directory)
    verify_receipt(dict(backup=str(backup), kind=kind, name=name, digest=digest(value)))
    return backup
