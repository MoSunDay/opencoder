"""Prune reviewed execution references from hibernated Host inventories."""
import contextlib
import fcntl
import hashlib
import json
from pathlib import Path
from .model import digest


def prune(body, identifiers):
    return {**body, 'indexes': [row for row in body['indexes'] if row['id'] not in identifiers]}


def preview(connection, database, identifiers):
    result = []
    for row in connection.execute("SELECT id,body FROM fleet_definitions WHERE kind='runtime_sleep'"):
        body = json.loads(row['body'])
        if not isinstance(body, dict):
            continue
        removed = sorted({entry['id'] for entry in body['indexes']} & identifiers)
        if removed:
            result.append({'database': str(database), 'id': row['id'],
                           'removed_execution_ids': removed, 'body_digest': digest(body)})
    return result


@contextlib.contextmanager
def locked(entries):
    # Same namespace as Host inventory reads, wake and garbage collection.
    # Other runtimes keep their own locks and may continue executing.
    with contextlib.ExitStack() as stack:
        for entry in sorted(entries, key=lambda row: (row['database'], row['id'])):
            directory = Path(entry['database']).with_suffix('.locks')
            directory.mkdir(exist_ok=True)
            name = hashlib.sha256(('runtime-use\0' + entry['id']).encode()).hexdigest()
            stream = stack.enter_context((directory / name).open('a+'))
            fcntl.flock(stream, fcntl.LOCK_EX | fcntl.LOCK_NB)
        yield


def update(connection, entry):
    row = connection.execute("SELECT body FROM fleet_definitions WHERE kind='runtime_sleep' AND id=?", (entry['id'],)).fetchone()
    if row is None:
        raise ValueError('reviewed inventory disappeared')
    body = json.loads(row[0])
    if digest(body) != entry['body_digest']:
        raise ValueError('inventory changed since preview')
    cleaned = prune(body, set(entry['removed_execution_ids']))
    encoded = json.dumps(cleaned, separators=(',', ':'))
    connection.execute("UPDATE fleet_definitions SET body=? WHERE kind='runtime_sleep' AND id=?", (encoded, entry['id']))
    return {'id': entry['id'], 'before': row[0], 'after': encoded}


def restore(connection, entries):
    for entry in entries:
        row = connection.execute("SELECT body FROM fleet_definitions WHERE kind='runtime_sleep' AND id=?", (entry['id'],)).fetchone()
        if row is None or row[0] not in (entry['before'], entry['after']):
            raise RuntimeError('inventory changed during cleanup rollback')
        connection.execute("UPDATE fleet_definitions SET body=? WHERE kind='runtime_sleep' AND id=?", (entry['before'], entry['id']))


def verify(connection, entries):
    for entry in entries:
        row = connection.execute("SELECT body FROM fleet_definitions WHERE kind='runtime_sleep' AND id=?", (entry['id'],)).fetchone()
        body = json.loads(row[0]) if row else None
        identifiers = {item['id'] for item in body.get('indexes', [])} if isinstance(body, dict) else set()
        if identifiers & set(entry['removed_execution_ids']):
            raise ValueError('a previously cleared inventory reference reappeared')
