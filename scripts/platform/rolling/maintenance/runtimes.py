"""Cache idle Runtime inventory before stopping; reuse Host hibernation state."""
from contextlib import closing
import json
import sqlite3


def capture(settings, operations):
    path = settings.state_dir / 'host/host.db'
    result = {}
    with closing(sqlite3.connect(path.resolve().as_uri() + '?mode=ro', uri=True)) as conn:
        rows = conn.execute('SELECT id,config,mode FROM host_runtimes').fetchall()
        for identifier, raw, mode in rows:
            if mode == 'staged':
                continue
            saved = conn.execute("SELECT body FROM fleet_definitions WHERE kind='runtime_sleep' AND id=?",
                                 (identifier,)).fetchone()
            inventory = json.loads(saved[0]) if saved else None
            if inventory is None:
                endpoint = json.loads(raw)['endpoint']
                inventory = operations.http(endpoint, '/inventory')
            if (inventory.get('runtime_id') != identifier or inventory.get('can_hibernate') is not True
                    or inventory.get('owned_processes') != 0):
                raise ValueError(f'Runtime is not idle enough for stopped maintenance: {identifier}')
            result[identifier] = inventory
    return result


def hibernate_stopped(settings, inventories):
    """Called after every writer stopped and the original Host DB was backed up."""
    path = settings.state_dir / 'host/host.db'
    with closing(sqlite3.connect(path.resolve().as_uri() + '?mode=rw', uri=True)) as conn:
        conn.execute('BEGIN IMMEDIATE')
        if conn.execute("SELECT count(*) FROM capacity_queue WHERE phase!='done'").fetchone()[0]:
            raise ValueError('Host still owns reservations; refusing stopped hibernation')
        for identifier, inventory in inventories.items():
            row = conn.execute('SELECT mode FROM host_runtimes WHERE id=?', (identifier,)).fetchone()
            if row is None or row[0] not in ('active', 'retired'):
                raise ValueError('stopped Runtime identity changed')
            conn.execute("UPDATE host_runtimes SET mode='retired' WHERE id=?", (identifier,))
            conn.execute("INSERT INTO fleet_definitions(kind,id,body) VALUES ('runtime_sleep',?,?) "
                         'ON CONFLICT(kind,id) DO UPDATE SET body=excluded.body',
                         (identifier, json.dumps(inventory, sort_keys=True)))
        conn.commit()
