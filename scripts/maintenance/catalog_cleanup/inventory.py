"""Read only the definition/execution tables needed for catalog cleanup."""
from collections import defaultdict
from pathlib import Path
import sys
sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "platform"))
from rolling.storage.connection import connect as engine_connect
import json
import sqlite3
import time
from .model import mentions, stamp, select, dag_key, pool_key, ReferenceNames
from .files import resource_texts


def connection(path):
    conn = engine_connect(path, readonly=True)
    conn.execute('PRAGMA query_only=ON')
    return conn


def inventory(data_dir, binary_root, agents_root, config_path, retention_days=7):
    now = time.time()
    data_dir, binary_root, agents_root = map(Path, (data_dir, binary_root, agents_root))
    fleet = connection(data_dir / 'control.db')
    store = connection(data_dir / 'definitions.db')
    definitions = {name: json.loads(body) for name, body in fleet.execute(
        "SELECT id,body FROM fleet_definitions WHERE kind='dag'")}
    pools = {}
    for path in sorted(binary_root.iterdir()):
        if path.name == '.locks':
            continue
        if path.is_symlink() or not (path / 'meta.json').is_file():
            raise ValueError('unverified pool directory: ' + str(path))
        meta = json.loads((path / 'meta.json').read_text())
        versions = [json.loads((path / ('v' + str(v)) / 'meta.json').read_text()) for v in meta['history']]
        pools[path.name] = {**meta, 'history': versions}
    dag_names, pool_names = ReferenceNames(definitions), ReferenceNames(pools)
    dag_refs, pool_refs = defaultdict(set), defaultdict(set)
    dag_use, pool_use, dag_success, pool_success = {}, {}, {}, {}

    def scan(value, source, skip_dag=None):
        for name in mentions(value, dag_names, True) - {skip_dag}:
            dag_refs[name].add(source)
        for name in mentions(value, pool_names):
            pool_refs[name].add(source)

    for kind, name, body in fleet.execute('SELECT kind,id,body FROM fleet_definitions'):
        if kind == 'catalog_retired':
            continue
        scan(json.loads(body), 'definition:' + kind + ':' + name, name if kind == 'dag' else None)
    for name, version, body in fleet.execute('SELECT id,version,body FROM brain_plan_versions'):
        scan(json.loads(body), f'plan:{name}@{version}')
    for name, created, status, body in fleet.execute('SELECT a.id,i.created_at,i.status,a.assignment FROM execution_assignments a LEFT JOIN execution_index i ON a.id=i.id'):
        value = json.loads(body)
        d = mentions(value, dag_names, True)
        b = mentions(value, pool_names)
        when = stamp(created)
        for names, usage, success in [(d, dag_use, dag_success), (b, pool_use, pool_success)]:
            for item in names:
                if when is None:
                    usage[item] = None
                elif usage.get(item, 0) is not None:
                    usage[item] = max(usage.get(item, 0), when)
                if status == 'done' and when is not None:
                    success[item] = max(success.get(item, 0), when)
        if status not in ('done', 'error', 'cancelled'):
            scan(value, 'execution:' + name)
    for scope, name, body in fleet.execute("SELECT scope,id,payload FROM dispatch_receipts WHERE phase NOT IN ('accepted','rejected','done','complete','completed','cancelled')"):
        scan(json.loads(body), 'dispatch:' + scope + ':' + name)
    for name, target, params in store.execute('SELECT id,target,params_json FROM schedules'):
        scan([target, json.loads(params)], 'schedule:' + name)
    for row in store.execute('SELECT id,capability_id,executor_ref,executor_spec,plan_md,draft FROM project_todos'):
        scan(list(row[1:]), 'todo:' + row[0])
    scan(json.loads(Path(config_path).read_text()), 'server-config')
    for path, content in resource_texts(agents_root):
        scan(content, 'agent-file:' + str(path))
    dags, retained_dags = select(definitions, dag_refs, dag_use, dag_success, now, retention_days, dag_key)
    binaries, retained_binaries = select(pools, pool_refs, pool_use, pool_success, now, retention_days, pool_key)
    return {'schema': 1, 'created_at': now, 'retention_days': retention_days,
            'data_dir': str(data_dir.resolve()), 'binary_root': str(binary_root.resolve()),
            'agents_root': str(agents_root.resolve()), 'config_path': str(Path(config_path).resolve()),
            'dag': dags, 'binary': binaries, 'retained_dag': retained_dags,
            'retained_binary': retained_binaries,
            'counts': {'dag': len(definitions), 'binary': len(pools)}}
