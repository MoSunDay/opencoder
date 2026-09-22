"""Apply a reviewed manifest with execution stores offline and shared ownership locked."""
import contextlib
import json
import os
import shutil
import sqlite3
from pathlib import Path
from .model import ALLOWED_TABLES, digest
from . import inventory


def shared_ownership_databases(manifest):
    tables = {}
    for change in manifest['databases']:
        tables.setdefault(change['database'], set()).add(change['table'])
    return {database for database, names in tables.items()
            if Path(database).name == 'host.db' and names == {'runtime_owners'}}


def restore_ownership(database, rows):
    # Other runtimes keep working through the shared host. Restore only our
    # deleted keys, never replace a live database with a whole-file snapshot.
    with sqlite3.connect(database) as connection:
        connection.row_factory = sqlite3.Row
        connection.execute('BEGIN IMMEDIATE')
        for row in rows:
            current = connection.execute('SELECT * FROM runtime_owners WHERE execution_id=?',
                                         (row['execution_id'],)).fetchone()
            if current is not None:
                if dict(current) != row:
                    raise RuntimeError('ownership changed during cleanup rollback')
                continue
            columns = ','.join(f'"{name}"' for name in row)
            placeholders = ','.join('?' for _ in row)
            connection.execute(f'INSERT INTO runtime_owners ({columns}) VALUES ({placeholders})', list(row.values()))


def validated_rows(connection, change):
    table = change['table']
    columns = {row['name'] for row in connection.execute(f'PRAGMA table_info("{table}")')} | {'rowid'}
    result = []
    for entry in change['rows']:
        if not entry['key'] or set(entry['key']) - columns:
            raise ValueError('invalid row identity')
        where = ' AND '.join(f'"{key}" IS ?' for key in entry['key'])
        rows = list(connection.execute(f'SELECT rowid AS rowid,* FROM "{table}" WHERE {where}', list(entry['key'].values())))
        if len(rows) != 1 or digest(dict(rows[0])) != entry['digest']:
            raise ValueError(f'row changed since preview: {table} {entry["key"]}')
        result.append({key: value for key, value in dict(rows[0]).items() if key != 'rowid'})
    return result


def delete_rows(connection, change):
    connection.execute('PRAGMA defer_foreign_keys=ON')
    for entry in change['rows']:
        where = ' AND '.join(f'"{key}" IS ?' for key in entry['key'])
        connection.execute(f'DELETE FROM "{change["table"]}" WHERE {where}', list(entry['key'].values()))


def ensure_offline(paths):
    targets = {Path(path).resolve() for path in paths}
    owners = []
    for process in Path('/proc').iterdir():
        if not process.name.isdigit() or int(process.name) == os.getpid():
            continue
        try:
            for fd in (process / 'fd').iterdir():
                try:
                    if fd.resolve() in targets:
                        owners.append(process.name)
                        break
                except (OSError, RuntimeError):
                    continue
        except (OSError, PermissionError):
            continue
    if owners:
        raise RuntimeError(f'affected databases are open; stop their owning services first: PIDs {sorted(set(owners))}')


def apply(manifest, approved_digest, backup):
    if digest(manifest) != approved_digest:
        raise ValueError('reviewed manifest digest does not match')
    if manifest.get('missing_local_journals'):
        raise ValueError('resolve every owning-node journal before applying cleanup')
    if manifest['mixed_plan_definitions']:
        raise ValueError('mixed plan versions require an explicit definition-pointer review')
    for change in manifest['databases']:
        if change['table'] not in ALLOWED_TABLES:
            raise ValueError('table outside cleanup scope')
        for entry in change['rows']:
            if not entry['key'] or any(not key.replace('_', '').isalnum() for key in entry['key']):
                raise ValueError('invalid row identity')
    backup = Path(backup).resolve()
    if backup.exists():
        receipt = backup / 'completed.json'
        if receipt.is_file() and json.loads(receipt.read_text()).get('manifest_digest') == approved_digest:
            for change in manifest['databases']:
                with sqlite3.connect(Path(change['database']).as_uri() + '?mode=ro', uri=True) as connection:
                    for entry in change['rows']:
                        where = ' AND '.join(f'"{key}" IS ?' for key in entry['key'])
                        if connection.execute(f'SELECT 1 FROM "{change["table"]}" WHERE {where}', list(entry['key'].values())).fetchone():
                            raise ValueError('a previously cleared row reappeared')
            if any(Path(entry['path']).exists() for entry in manifest['directories']):
                raise ValueError('a previously cleared journal reappeared')
            for database in {entry['database'] for entry in manifest.get('cached_inventories', [])}:
                with sqlite3.connect(Path(database).as_uri() + '?mode=ro', uri=True) as connection:
                    inventory.verify(connection, [entry for entry in manifest['cached_inventories'] if entry['database'] == database])
            return
        raise ValueError('backup directory already exists without a matching completed receipt')
    databases = sorted({change['database'] for change in manifest['databases']})
    shared = shared_ownership_databases(manifest)
    cached = manifest.get('cached_inventories', [])
    approved_ids = {row['id'] for row in manifest.get('executions', [])}
    for entry in cached:
        if entry['database'] not in shared or not set(entry['removed_execution_ids']) <= approved_ids:
            raise ValueError('inventory outside reviewed execution scope')
    locks = contextlib.ExitStack()
    locks.enter_context(inventory.locked(cached))
    try:
        ensure_offline(set(databases) - shared)
    except Exception:
        locks.close()
        raise
    connections = {}
    moved = []
    backed_up = []
    ownership_rows = {}
    inventory_rows = {}
    try:
        backup.mkdir(mode=0o700, parents=True)
        (backup / 'review.json').write_text(json.dumps(manifest, indent=2))
        # Keep shared ownership writes available while backing up and checking
        # the offline stores. No database commits before all checks pass.
        for index, database in enumerate(databases):
            connection = sqlite3.connect(database)
            connection.row_factory = sqlite3.Row
            destination = sqlite3.connect(backup / f'database-{index}.sqlite')
            connection.backup(destination)
            destination.close()
            backed_up.append((database, backup / f'database-{index}.sqlite'))
            connections[database] = connection
            if database not in shared:
                connection.execute('BEGIN IMMEDIATE')
        for change in manifest['databases']:
            if change['database'] not in shared:
                validated_rows(connections[change['database']], change)
        for entry in manifest['directories']:
            directory = Path(entry['path'])
            if directory.is_symlink() or directory.name != entry['id']:
                raise ValueError('unsafe execution directory')
            if digest(json.loads((directory / 'execution.json').read_text())) != entry['record_digest']:
                raise ValueError(f'journal changed since preview: {directory}')
        # Keep reverse table order for FK children. Global FK validation detects
        # an incomplete closure and rolls back before any database is committed.
        for change in reversed(manifest['databases']):
            if change['database'] not in shared:
                delete_rows(connections[change['database']], change)
        for database, connection in connections.items():
            if database not in shared and list(connection.execute('PRAGMA foreign_key_check')):
                raise RuntimeError('cleanup would leave foreign-key references')
        for index, entry in enumerate(manifest['directories']):
            destination = backup / f'execution-{index}'
            shutil.move(entry['path'], destination)
            moved.append((entry['path'], destination))
        # The shared write lock covers only its small ownership change and
        # commits; large offline integrity scans and directory moves are over.
        for database in sorted(shared):
            connections[database].execute('BEGIN IMMEDIATE')
        for change in manifest['databases']:
            if change['database'] in shared:
                connection = connections[change['database']]
                ownership_rows.setdefault(change['database'], []).extend(validated_rows(connection, change))
                delete_rows(connection, change)
        for entry in cached:
            inventory_rows.setdefault(entry['database'], []).append(inventory.update(connections[entry['database']], entry))
        for database in shared:
            if list(connections[database].execute('PRAGMA foreign_key_check')):
                raise RuntimeError('cleanup would leave ownership references')
        # Commit the shared store last; its rollback below is scoped to the
        # reviewed keys even if unrelated owners write immediately afterwards.
        for database in sorted(connections, key=lambda item: item in shared):
            connections[database].commit()
        (backup / 'completed.json').write_text(json.dumps({'manifest_digest': approved_digest, 'databases': databases}))
    except Exception:
        for connection in connections.values():
            connection.rollback()
            connection.close()
        connections.clear()
        for database, snapshot in backed_up:
            if database in shared:
                restore_ownership(database, ownership_rows.get(database, []))
                with sqlite3.connect(database) as connection:
                    inventory.restore(connection, inventory_rows.get(database, []))
                continue
            with sqlite3.connect(snapshot) as source, sqlite3.connect(database) as destination:
                source.backup(destination)
        for original, archived in reversed(moved):
            shutil.move(archived, original)
        raise
    finally:
        for connection in connections.values():
            connection.close()
        locks.close()
