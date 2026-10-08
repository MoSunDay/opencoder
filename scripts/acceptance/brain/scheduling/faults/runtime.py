"""Scoped process faults; refuse to interrupt executions owned by anybody else."""
import json
from pathlib import Path
import subprocess
import sqlite3
import time

from environment import require


class NotRun(RuntimeError):
    """A required external precondition is unavailable; never a passing result."""


def require_isolated(env):
    path = env.settings.state_dir / 'isolation.json'
    if not path.is_file():
        raise NotRun('process/model/capacity faults require the dedicated isolated hosted stack')
    receipt = json.loads(path.read_text())
    require(receipt['node_id'] == env.node_id, 'isolation node identity mismatch')
    for kind in ('server', 'host', 'runtime'):
        key = kind + '_unit'
        require(receipt['record'][key] == env.record[key]
                and env.record[key].startswith('opencoder-' + kind + '-brain-e2e-'),
                'fault target is outside the isolated stack')
    require(Path(env.record['runtime_data']).resolve().is_relative_to(Path(receipt['root']).resolve()),
            'Runtime data is outside the isolated stack')


def journal(env, identifier):
    require(identifier in env.created, 'journal target is not an owned test root')
    return json.loads((Path(env.record['runtime_data']) / 'brain' / identifier / 'execution.json').read_text())


def owned_ids(env):
    identifiers = set(env.created)
    for root in env.created:
        path = Path(env.record['runtime_data']) / 'brain' / root / 'execution.json'
        if path.exists():
            snapshot = env.brain_rpc(root, 'snapshot')
            identifiers.update(op['execution_id'] for op in snapshot['operations'])
    return identifiers


def blockers(env):
    identifiers = owned_ids(env)
    inventory = env.http(env.runtime_url, '/inventory')
    active = [index for index in inventory['indexes']
              if index['status'] in ('pending', 'running', 'cancelling') and index['id'] not in identifiers]
    nodes = env.api('GET', '/api/nodes')['nodes']
    remote = [node['id'] for node in nodes if node['id'] != env.node_id and node['online']
              and (node.get('snapshot') or {}).get('active_runs', 0) +
                  (node.get('snapshot') or {}).get('pending_runs', 0) > 0]
    return {'local_executions': active, 'other_busy_nodes': remote,
            'resource_error': inventory['snapshot'].get('resource_error')}


def wait_idle(env, seconds=1800):
    deadline = time.monotonic() + seconds
    while True:
        status = blockers(env)
        env.save('faults/preflight', {'at': time.time(), **status})
        if not any(status.values()):
            return
        if time.monotonic() >= deadline:
            raise NotRun('current fleet did not become idle and ready within the fault window: ' + str(status))
        print(json.dumps({'stage': 'waiting-for-fault-window', **status}), flush=True)
        time.sleep(min(30, max(0, deadline - time.monotonic())))


class FrozenWindow:
    def __init__(self, env):
        self.env = env
        self.frozen = False

    def __enter__(self):
        env = self.env
        require_isolated(env)
        require(not any(blockers(env).values()), 'unrelated work arrived before process fault')
        status = env.api('GET', '/api/admin/drain')
        require(status['server']['mode'] == 'open', 'admission was already frozen by another owner')
        # Record intent first: recovery must reopen even if the HTTP reply is lost.
        self.frozen = True
        env.save('faults/restore', {'admission': 'open', 'units': [env.record['server_unit'], env.record['runtime_unit']]})
        try:
            env.api('POST', '/api/admin/drain', {})
            require(not any(blockers(env).values()), 'unrelated admission raced the freeze')
            return self
        except BaseException:
            self.restore()
            raise

    def restore(self):
        env = self.env
        for kind in ('runtime', 'server'):
            subprocess.run(['systemctl', 'start', env.record[kind + '_unit']], check=True, timeout=90)
        if self.frozen:
            env.wait(lambda: self.reopen(), 120, 'restore admission')
            self.frozen = False
        env.save('faults/restored', env.verify_release())

    def reopen(self):
        try:
            runtime = self.env.http(self.env.runtime_url, '/inventory')
            if runtime['registration']['id'] != self.env.node_id or not runtime['snapshot']['ready']:
                return False
            self.env.api('DELETE', '/api/admin/drain')
            nodes = self.env.api('GET', '/api/nodes')['nodes']
            return any(node['id'] == self.env.node_id and node['online']
                       and (node.get('snapshot') or {}).get('ready') for node in nodes)
        except (OSError, RuntimeError, AssertionError):
            return False

    def __exit__(self, *_):
        self.restore()


def restart(env, kind):
    require(not any(blockers(env).values()), 'unrelated work prevents restart')
    unit = env.record[kind + '_unit']
    before = int(subprocess.check_output(['systemctl', 'show', unit, '-p', 'MainPID', '--value']))
    require(before > 0, 'fault target is not running')
    if kind == 'runtime':
        require_restartable(env)
        subprocess.run(['systemctl', 'kill', '--kill-who=main', '--signal=SIGKILL', unit], check=True, timeout=15)
    subprocess.run(['systemctl', 'restart', unit], check=True, timeout=90)
    after = int(subprocess.check_output(['systemctl', 'show', unit, '-p', 'MainPID', '--value']))
    require(after > 0 and before != after, 'process was not restarted')
    env.save('faults/' + kind + '-restart', {'unit': unit, 'before_pid': before, 'after_pid': after})


def require_restartable(env):
    # This release refuses startup when Host owns a running capacity ticket.
    # Read only the scheduling table of the local libSQL file; never touch auth data
    # or manufacture recovery by deleting reservations after a crash.
    binding_path = Path(env.record['runtime_data']) / 'host-binding.json'
    if not binding_path.exists():
        return
    binding = json.loads(binding_path.read_text())
    database = Path(binding['database']).resolve()
    with sqlite3.connect(database.as_uri() + '?mode=ro', uri=True, timeout=2) as connection:
        count = connection.execute("SELECT COUNT(*) FROM capacity_queue WHERE runtime_id=? AND phase='running'",
                                   (binding['runtime_id'],)).fetchone()[0]
    if count:
        raise NotRun(f'Runtime has {count} running Host reservations; this release refuses automatic restart '
                     'with unresolved tickets. Crash injection requires a proven recovery path.')
