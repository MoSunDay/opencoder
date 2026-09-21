"""Read-only manifest of exact retired-plan rows and execution directories."""
import json
import sqlite3
from pathlib import Path
from .model import ID_COLUMNS, ALLOWED_TABLES, RETIRED_TABLES, digest, execution_scope, selected_row


def connect(path):
    connection = sqlite3.connect(Path(path).resolve().as_uri() + '?mode=ro', uri=True)
    connection.row_factory = sqlite3.Row
    return connection


def scan(config_path, extra_roots=()):
    settings = json.loads(Path(config_path).read_text())['deployment']
    state = Path(settings['state_dir'])
    server = Path(settings['server_data'])
    release_state = json.loads((state / 'release-state.json').read_text())
    runtime_roots = {Path(r['runtime_data']) for r in release_state['releases'].values() if r.get('runtime_data')}
    runtime_roots.update(path for path in (state / 'agents').glob('*') if path.is_dir())
    if settings.get('legacy_agent_data'):
        runtime_roots.add(Path(settings['legacy_agent_data']))
    assignments = {}
    with connect(server / 'control.db') as connection:
        for row in connection.execute('SELECT id,assignment FROM execution_assignments'):
            assignments[row['id']] = json.loads(row['assignment'])
        tables = {row[0] for row in connection.execute("SELECT name FROM sqlite_master WHERE type='table'")}
        historical = list(connection.execute('SELECT id,body FROM brain_runs')) if 'brain_runs' in tables else []
        persisted_roots = {row['id'] for row in historical}
        historical_children = {child.get('result', {}).get('execution_id')
            for row in historical for stage in json.loads(row['body']).get('stages', [])
            for child in stage.get('children', [])}
        indexes = {row['id']: dict(row) for row in connection.execute('SELECT * FROM execution_index')}
        versions = [dict(row) for row in connection.execute('SELECT id,version,body FROM brain_plan_versions')]
    plans = {(row['id'], row['version']) for row in versions if json.loads(row['body'])['plan']['schema_version'] != 4}
    retained = {row['id'] for row in versions if (row['id'], row['version']) not in plans}
    # A mixed definition needs a pointer update, not deletion. The apply tool
    # leaves it intact; current-version references are checked in verification.
    mixed = retained & {key for key, _ in plans}
    roots, executions = execution_scope(assignments, persisted_roots)
    historical_children = historical_children & indexes.keys()
    index_only = historical_children - assignments.keys()
    executions |= historical_children
    owner_ids = {indexes[key]["node_id"] for key in executions}
    for path in extra_roots:
        root = Path(path).resolve()
        if (root / "node-id").read_text().strip() not in owner_ids:
            raise ValueError(f"extra runtime does not own selected executions: {root}")
        runtime_roots.add(root)
    records = []
    for root in sorted(runtime_roots):
        for key in sorted(executions):
            kind = indexes[key]['kind']
            for directory in (root / kind / key, root / 'executions' / key):
                record = directory / 'execution.json'
                if record.is_file():
                    data = json.loads(record.read_text())
                    if data.get('assignment', {}).get('index', {}).get('id') != key:
                        raise ValueError(f'journal identity mismatch: {record}')
                    records.append({'path': str(directory), 'id': key, 'record_digest': digest(data)})
    databases = {server / 'control.db', server / 'definitions.db', state / 'host' / 'host.db'}
    for root in runtime_roots:
        databases.update(root.glob('*.db'))
    changes = []
    allowed = ALLOWED_TABLES
    for database in sorted(path for path in databases if path.is_file()):
        with connect(database) as connection:
            tables = {row[0] for row in connection.execute("SELECT name FROM sqlite_master WHERE type='table'")}
            sessions = set(executions)
            if 'subagent_tasks' in tables:
                links = list(connection.execute('SELECT parent_session_id,child_session_id FROM subagent_tasks'))
                while True:
                    expanded = sessions | {child for parent, child in links if parent in sessions}
                    if expanded == sessions:
                        break
                    sessions = expanded
            for table in sorted(tables & allowed):
                columns = list(connection.execute(f'PRAGMA table_info("{table}")'))
                keys = [row['name'] for row in sorted(columns, key=lambda r: r['pk']) if row['pk']]
                if not keys:
                    keys = ['rowid']
                selected = []
                names = {row['name'] for row in columns}
                identifiers = sorted(sessions if table in ID_COLUMNS or table == 'dispatch_receipts' else {key for key, _ in plans})
                filter_keys = [key for key in ID_COLUMNS.get(table, ('id',)) if key in names]
                if table not in RETIRED_TABLES and (not identifiers or not filter_keys):
                    continue
                placeholders = ','.join('?' for _ in identifiers)
                predicate = ' OR '.join(f'"{key}" IN ({placeholders})' for key in filter_keys)
                query = f'SELECT rowid AS rowid,* FROM "{table}" WHERE {predicate}'
                if table in RETIRED_TABLES:
                    query = f'SELECT rowid AS rowid,* FROM "{table}"'
                for raw in connection.execute(query, [] if table in RETIRED_TABLES else identifiers * len(filter_keys)):
                    row = dict(raw)
                    if table == 'fleet_definitions' and row['id'] in mixed:
                        continue
                    if selected_row(table, row, sessions, plans):
                        selected.append({'key': {key: row[key] for key in keys}, 'digest': digest(row)})
                if selected:
                    changes.append({'database': str(database), 'table': table, 'rows': selected})
    return {'schema_version': 1, 'config': str(Path(config_path).resolve()),
            'release': release_state['current'], 'roots': sorted(roots),
            'executions': [{'id': key, 'kind': indexes[key]['kind'],
                            'status': indexes[key]['status'], 'node_id': indexes[key]['node_id']}
                           for key in sorted(executions)],
            'plan_versions': sorted([list(key) for key in plans]), 'mixed_plan_definitions': sorted(mixed),
            'databases': changes, 'directories': records,
            'index_only_executions': sorted(index_only),
            'missing_local_journals': sorted(executions - index_only - {entry['id'] for entry in records})}
