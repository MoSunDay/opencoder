"""A durable rollback floor is written before accepting any TODO file uploads."""
import json
import os
from ..state import Journal, atomic_bytes, write
from ..storage import todos


def contract(manifest):
    value = manifest.get('todo_files_contract', 0)
    if type(value) is not int or value not in (0, 1):
        raise ValueError('unsupported TODO files contract')
    return value


def require_candidate(settings, candidate):
    path = settings.state_dir / 'todo-files-policy.json'
    minimum = json.loads(path.read_text())['minimum_contract'] if path.exists() else 0
    root = todos.files_root(settings.server_data)
    if root is not None:
        if not root.is_dir():
            raise ValueError('bound TODO storage is unavailable')
        if any(path.is_file() for path in root.rglob('*')):
            minimum = max(minimum, 1)
    if contract(candidate) < minimum:
        raise ValueError('TODO files require the compatible bridge; unsafe rollback is forbidden')


def update_config(settings, expected, target):
    """Compare only the owned section; preserve concurrent unrelated settings."""
    path = settings.server_workdir / 'opencoder.json'
    config = json.loads(path.read_text())
    actual = config.get('project', {}).get('todo_storage')
    if actual not in (expected, target):
        raise ValueError('TODO configuration changed since the recovery journal was written')
    if target is None:
        config.get('project', {}).pop('todo_storage', None)
        if not config.get('project'):
            config.pop('project', None)
    else:
        config.setdefault('project', {})['todo_storage'] = target
    stat = path.stat()
    atomic_bytes(path, (json.dumps(config, indent=2, sort_keys=True) + '\n').encode(),
                 stat.st_mode & 0o777, owner=(stat.st_uid, stat.st_gid))


def set_uploads(settings, enabled, operations):
    from .installation import probe
    journal = Journal(settings.state_dir)
    if enabled and (journal.data['phase'] not in ('complete', 'rolled_back') or journal.data['candidate']):
        raise ValueError('finish the current release before changing TODO uploads')
    current = journal.record(journal.data['current'])['manifest']
    if contract(current) < 1:
        raise ValueError('current Server does not protect TODO files')
    if enabled:
        previous = journal.data['previous']
        if not previous or contract(journal.record(previous)['manifest']) < 1:
            raise ValueError('publish the compatible bridge before enabling TODO uploads')
        probe(settings, operations)
    path = settings.server_workdir / 'opencoder.json'
    config = json.loads(path.read_text())
    before = config['project']['todo_storage']
    after = {**before, 'uploads_enabled': enabled}
    policy_path = settings.state_dir / 'todo-files-policy.json'
    policy = json.loads(policy_path.read_text()) if policy_path.exists() else {'minimum_contract': 0, 'operations': []}
    policy['minimum_contract'] = max(policy['minimum_contract'], int(enabled))
    operation = {'release_id': journal.data['current'], 'before': before, 'after': after, 'phase': 'prepared'}
    policy['operations'].append(operation)
    write(policy_path, policy)
    update_config(settings, before, after)
    operation['phase'] = 'complete'
    write(policy_path, policy)
    return {'uploads_enabled': enabled, 'minimum_contract': policy['minimum_contract']}
