"""Final idle inventories let a fresh Host open while old Runtimes stay stopped."""
from contextlib import closing
import json
import sqlite3
from urllib.parse import urlparse


def idle(identifier, node, inventory):
    if (inventory.get('runtime_id') != identifier
            or inventory.get('registration', {}).get('id') != node
            or inventory.get('can_hibernate') is not True
            or inventory.get('owned_processes') != 0):
        raise ValueError(f'Runtime cannot be safely stopped: {identifier}')
    snapshot = inventory.get('snapshot', {})
    if any(snapshot.get(key) != 0 for key in ('active_runs', 'pending_runs', 'active_agent_loops')):
        raise ValueError(f'Runtime still has admitted work: {identifier}')
    if any(index.get('status') not in ('done', 'error', 'cancelled')
           and not (index.get('status') == 'idle' and index.get('kind') in ('agent', 'operator'))
           for index in inventory.get('indexes', [])):
        raise ValueError(f'Runtime has nonterminal executions: {identifier}')
    return inventory


def capture(settings, operations):
    node = (settings.state_dir / 'host/node-id').read_text().strip()
    path = settings.state_dir / 'host/host.db'
    with closing(sqlite3.connect(path.resolve().as_uri() + '?mode=ro', uri=True)) as conn:
        rows = conn.execute("SELECT r.id,r.config,d.body FROM host_runtimes r "
                            "LEFT JOIN fleet_definitions d ON d.kind='runtime_sleep' AND d.id=r.id "
                            "WHERE r.mode!='staged'").fetchall()
    saved = {}
    for identifier, config, body in rows:
        inventory = json.loads(body) if body else None
        if inventory is None:
            endpoint = json.loads(config)['endpoint']
            url = urlparse(endpoint)
            if (url.scheme != 'http' or url.hostname != '127.0.0.1' or not url.port
                    or url.username or url.password or url.path not in ('', '/')
                    or url.query or url.fragment):
                raise ValueError('maintenance Runtime must use its explicit local endpoint')
            inventory = operations.http(endpoint.rstrip('/'), '/inventory')
        saved[identifier] = {'config': config, 'inventory': idle(identifier, node, inventory)}
    return saved


def install(settings, saved):
    """Run only after all writers stop and the immutable Host backup exists."""
    node = (settings.state_dir / 'host/node-id').read_text().strip()
    path = settings.state_dir / 'host/host.db'
    with closing(sqlite3.connect(path.resolve().as_uri() + '?mode=rw', uri=True)) as conn:
        conn.execute('BEGIN IMMEDIATE')
        try:
            if conn.execute("SELECT count(*) FROM capacity_queue WHERE phase!='done'").fetchone()[0]:
                raise ValueError('stopped Host still owns admitted work')
            for identifier, item in saved.items():
                row = conn.execute('SELECT config FROM host_runtimes WHERE id=?', (identifier,)).fetchone()
                if row != (item['config'],):
                    raise ValueError('Runtime registration changed after the final inventory')
                inventory = idle(identifier, node, item['inventory'])
                conn.execute("INSERT INTO fleet_definitions(kind,id,body) VALUES ('runtime_sleep',?,?) "
                             "ON CONFLICT(kind,id) DO UPDATE SET body=excluded.body",
                             (identifier, json.dumps(inventory)))
                conn.execute("UPDATE host_runtimes SET mode='retired' WHERE id=?", (identifier,))
            conn.commit()
        except BaseException:
            conn.rollback()
            raise
