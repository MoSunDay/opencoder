"""Read-only source, schema, authentication and snapshot evidence."""
from contextlib import closing
import hashlib
import json
import os
from pathlib import Path
import sqlite3
import stat
from rolling.maintenance import archive


def fingerprint(value):
    return hashlib.sha256(json.dumps(value, sort_keys=True).encode()).hexdigest()


def source_inventory(root):
    result = {}
    for path in [root, *sorted(root.rglob('*'))]:
        metadata = path.lstat()
        item = {'uid': metadata.st_uid, 'gid': metadata.st_gid,
                'mode': stat.S_IMODE(metadata.st_mode), 'inode': metadata.st_ino,
                'device': metadata.st_dev}
        if path.is_symlink():
            item.update(kind='symlink', target=os.readlink(path))
        elif path.is_file():
            item.update(kind='file', bytes=metadata.st_size, sha256=archive.digest(path))
        elif path.is_dir():
            item['kind'] = 'directory'
        else:
            raise ValueError('unsupported source entry: ' + str(path))
        result[str(path.relative_to(root))] = item
    return {'path': str(root), 'entries': result}


def unchanged_source(root, expected):
    actual = source_inventory(root)
    if actual != expected:
        raise AssertionError('source workspace path, identity, ownership, mode or content changed')
    return fingerprint(actual)


def schema(path, immutable=False):
    suffix = '?mode=ro' + ('&immutable=1' if immutable else '')
    with closing(sqlite3.connect(path.resolve().as_uri() + suffix, uri=True)) as connection:
        return connection.execute('SELECT version FROM schema_version LIMIT 1').fetchone()[0]


def authentication(path):
    with closing(sqlite3.connect(path.resolve().as_uri() + '?mode=ro', uri=True)) as connection:
        names = [row[0] for row in connection.execute(
            "SELECT name FROM sqlite_schema WHERE type='table' ORDER BY name")]
        tables = {}
        for name in names:
            if name == 'platform_users' or any(part in name.lower() for part in ('auth', 'token')):
                quoted = '"' + name.replace('"', '""') + '"'
                rows = connection.execute('SELECT * FROM ' + quoted).fetchall()
                tables[name] = {'rows': len(rows), 'sha256': hashlib.sha256(
                    repr(sorted(map(repr, rows))).encode()).hexdigest()}
        if not tables or not tables.get('platform_users', {}).get('rows'):
            raise AssertionError('fixture has no real authentication rows')
        return tables


def snapshots(state):
    backup = Path(state['backup'])
    archive.verify(backup)
    configs = {}
    for name, item in state['scope']['configuration'].items():
        actual = archive.digest(Path(item['path']))
        if actual != item['sha256']:
            raise AssertionError('frozen configuration checksum changed')
        configs[name] = {'path': item['path'], 'sha256': actual}
    return {'backup': str(backup), 'backup_manifest_sha256': archive.digest(backup / 'manifest.json'),
            'backup_inventory_sha256': fingerprint(archive.inventory(backup)), 'configs': configs}


def native_evidence(record, public=False):
    from rolling.probes import probe_id
    identifier = probe_id(record, public)
    path = Path(record['runtime_data']) / 'dag' / identifier / 'execution.json'
    saved = json.loads(path.read_text())
    parent = saved.get('annotations', {}).get('dag_parent')
    if not parent or saved['assignment']['index']['status'] != 'done':
        raise AssertionError('native probe lacks a completed container execution')
    run = Path(parent) / identifier
    config = json.loads((run / 'bundle/config.json').read_text())
    return {'id': identifier, 'journal': str(path), 'journal_sha256': archive.digest(path),
            'run_dir': str(run), 'oci_config_sha256': fingerprint(config),
            'status': 'done', 'dag_parent': parent}
