"""Explicit migration of empty, retired auth_slot fields in frozen assignments.

Only execution configuration is changed. Requests, receipts, authentication
tables and nonempty account selections are never rewritten by this tool.
"""
import argparse
import copy
from contextlib import closing
import fcntl
import hashlib
import json
import os
from pathlib import Path
import sqlite3
import tempfile


def convert(raw):
    value = json.loads(raw)
    updated = copy.deepcopy(value)
    settings = [updated.get('codex')]
    runtime = updated.get('runtime') or {}
    settings.extend(profile.get('settings') for profile in runtime.get('profiles', {}).values())
    for item in settings:
        if not isinstance(item, dict) or 'auth_slot' not in item:
            continue
        if item['auth_slot'] is not None:
            raise ValueError('nonempty auth_slot requires an explicitly configured startup script')
        del item['auth_slot']
        item.setdefault('startup_script', [])
    return raw if value == updated else json.dumps(updated, ensure_ascii=False, separators=(',', ':'))


def digest(value):
    return hashlib.sha256(value.encode()).hexdigest()


def scan(conn):
    changes = []
    for identifier, before in conn.execute('SELECT id,assignment FROM execution_assignments ORDER BY id'):
        after = convert(before)
        if before != after:
            changes.append({'id': identifier, 'before': before, 'after': after,
                            'before_sha256': digest(before), 'after_sha256': digest(after)})
    return changes


def identity(database):
    stat = database.stat()
    return {'path': str(database), 'device': stat.st_dev, 'inode': stat.st_ino}


def save_anchor(path, value):
    """Publish a complete, private rollback journal without replacing any anchor."""
    data = json.dumps(value, ensure_ascii=False).encode()
    fd, name = tempfile.mkstemp(prefix='.codex-startup-', dir=path.parent)
    try:
        with os.fdopen(fd, 'wb') as stream:
            stream.write(data)
            stream.flush()
            os.fsync(stream.fileno())
        os.link(name, path)
        directory = os.open(path.parent, os.O_RDONLY | os.O_DIRECTORY)
        try:
            os.fsync(directory)
        finally:
            os.close(directory)
    finally:
        os.unlink(name)


def read_anchor(path, database):
    if path.is_symlink() or not path.is_file() or path.stat().st_mode & 0o077:
        raise ValueError('rollback journal must be a private regular file')
    anchor = json.loads(path.read_text())
    if anchor.get('schema') != 1 or anchor.get('database') != identity(database):
        raise ValueError('rollback journal database identity differs')
    ids = set()
    for row in anchor['rows']:
        if row['id'] in ids:
            raise ValueError('duplicate rollback journal row')
        ids.add(row['id'])
        if (digest(row['before']) != row['before_sha256']
                or digest(row['after']) != row['after_sha256']
                or convert(row['before']) != row['after']):
            raise ValueError('rollback journal content differs')
    return anchor


def migrate(database, journal=None, restore=False):
    database = Path(database).resolve(strict=True)
    if journal is None:
        with closing(sqlite3.connect(database.as_uri() + '?mode=ro', uri=True)) as conn:
            return {'mode': 'preview', 'rows': len(scan(conn))}
    journal = Path(journal).absolute()
    if journal.parent.is_symlink() or not journal.parent.is_dir():
        raise ValueError('rollback journal directory must already exist')
    lock_fd = os.open(str(journal) + '.lock', os.O_CREAT | os.O_RDWR | os.O_NOFOLLOW, 0o600)
    with os.fdopen(lock_fd, 'w') as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        with closing(sqlite3.connect(database.as_uri() + '?mode=rw', uri=True, timeout=30)) as conn, conn:
            conn.execute('BEGIN IMMEDIATE')
            if not journal.exists() and not journal.is_symlink():
                if restore:
                    raise ValueError('rollback journal does not exist')
                save_anchor(journal, {'schema': 1, 'database': identity(database), 'rows': scan(conn)})
            anchor = read_anchor(journal, database)
            source, target = ('after', 'before') if restore else ('before', 'after')
            changed = 0
            for row in anchor['rows']:
                current = conn.execute('SELECT assignment FROM execution_assignments WHERE id=?',
                                       (row['id'],)).fetchone()
                if current is None or current[0] not in (row[source], row[target]):
                    raise ValueError('assignment changed since the rollback journal was created')
                if current[0] == row[target]:
                    continue
                conn.execute('UPDATE execution_assignments SET assignment=? WHERE id=? AND assignment=?',
                             (row[target], row['id'], row[source]))
                changed += 1
            # A matched retry must not silently leave newly introduced legacy rows.
            if not restore and scan(conn):
                raise ValueError('additional legacy assignments require a separate migration journal')
            return {'mode': 'restore' if restore else 'apply', 'rows': len(anchor['rows']), 'changed': changed}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--database', required=True, type=Path)
    parser.add_argument('--journal', type=Path, help='Private immutable rollback file outside the repository')
    action = parser.add_mutually_exclusive_group()
    action.add_argument('--apply', action='store_true')
    action.add_argument('--restore', action='store_true')
    args = parser.parse_args()
    if (args.apply or args.restore) != bool(args.journal):
        parser.error('--apply/--restore requires --journal; preview does not use a journal')
    print(json.dumps(migrate(args.database, args.journal, args.restore)))


if __name__ == '__main__':
    main()
