"""Install once with a recoverable journal; subsequent releases only verify."""
import copy
import hashlib
import json
import os
from pathlib import Path
import pwd
import socket
from ..state import Journal, atomic_bytes, write
from ..storage import allow_server_traversal, todos
from ..units import service
from .policy import contract, update_config

UNIT = 'opencoder-todo-resources.service'


def location(settings):
    return settings.state_dir / 'services/todo-resources'


def digest(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def probe(settings, operations):
    record = json.loads((location(settings) / 'installation.json').read_text())
    binary = Path(record['binary'])
    if binary.is_symlink() or digest(binary) != record['sha256']:
        raise ValueError('TODO service binary differs from its installation journal')
    health = operations.http(settings.todo_resource_url, '/api/health', timeout=10)
    build = health.get('build', {})
    if (health.get('role') != 'todo-resources' or build.get('git_commit') != record['commit']
            or build.get('git_dirty') or contract(build) != 1):
        raise ValueError('running TODO service differs from the pinned artifact')
    pid = int(operations.output('systemctl', 'show', UNIT, '-p', 'MainPID', '--value').strip())
    if pid <= 0 or digest(Path(f'/proc/{pid}/exe')) != record['sha256']:
        raise ValueError('TODO service process is not running the pinned binary')
    status = operations.http(settings.todo_resource_url, '/api/project/todos/nfs', timeout=10)
    expected = record['after']
    export = status.get('status', {})
    if (status.get('root') != expected['files_dir'] or not export.get('running')
            or not export.get('read_only') or export.get('port') != expected['nfs']['port']):
        raise ValueError('TODO export root, port or read-only state differs')
    return health


def prepare(settings, bundle, candidate, operations):
    if contract(candidate) == 0:
        return
    directory = location(settings)
    path = directory / 'installation.json'
    if path.exists():
        record = json.loads(path.read_text())
        if record['phase'] == 'ready':
            probe(settings, operations)
            return
        if record['phase'] == 'restored':
            raise ValueError('restored TODO installation requires a new reviewed installation journal')
        if record['commit'] != candidate['commit']:
            raise ValueError('resume the original TODO service installation before another release')
    else:
        config_path = settings.server_workdir / 'opencoder.json'
        config = json.loads(config_path.read_text())
        before = copy.deepcopy(config.get('project', {}).get('todo_storage'))
        target = {**(before or {})}
        target.setdefault('files_dir', str(settings.state_dir / 'exports/todos'))
        target.setdefault('max_file_bytes', 512 * 1024 * 1024)
        target.setdefault('uploads_enabled', False)
        if target['uploads_enabled']:
            raise ValueError('install the TODO bridge with uploads disabled')
        target['nfs_service_url'] = settings.todo_resource_url
        target['nfs'] = {'enabled': True, 'host': '0.0.0.0', 'port': 2053, **target.get('nfs', {})}
        if target['nfs']['enabled'] is not True:
            raise ValueError('dedicated TODO service requires its read-only export enabled')
        config.setdefault('project', {})['todo_storage'] = target
        todos.validate(config, settings.server_data)
        todo_port = todos.port(config)
        reserved = {settings.host_port, settings.resource_port, int(settings.listen.rsplit(':', 1)[1])}
        for section, key, default in [('agent','nfs',2049), ('dag','nfs',2050),
                ('dag','workspace_nfs',2051), ('ontology','nfs',2052)]:
            export = config.get(section, {}).get(key, {})
            if export.get('enabled'):
                reserved.add(export.get('port', default))
        if todo_port == settings.todo_resource_port or {todo_port, settings.todo_resource_port} & reserved:
            raise ValueError('TODO resource ports conflict with existing services')
        for host, port in [('127.0.0.1', settings.todo_resource_port), (target['nfs']['host'], todo_port)]:
            with socket.socket() as listener:
                listener.bind((host, port))
        unit_path = settings.systemd_dir / UNIT
        if unit_path.exists():
            raise ValueError('TODO service unit exists without an ownership journal')
        binary = directory / 'bin/opencoder-server'
        content = service([binary, '--todo-resources', '--workdir', settings.server_workdir,
            '--data-dir', settings.server_data, '--port', settings.todo_resource_port,
            '--token-file', settings.token_file], UNIT, user=settings.server_user,
            workdir=settings.server_workdir, resource=True)
        record = {'phase': 'prepared', 'commit': candidate['commit'], 'release_id': candidate['release_id'],
            'binary': str(binary), 'sha256': candidate['files']['bin/opencoder-server']['sha256'],
            'config_sha256': digest(config_path), 'before': before, 'after': target,
            'unit': content}
        # Contains only the TODO subsection and credential paths, never credential values.
        write(path, record)
    account = pwd.getpwnam(settings.server_user)
    root = Path(record['after']['files_dir'])
    for target in [root, root.with_name(root.name + '.state')]:
        target.mkdir(parents=True, exist_ok=True)
        if target.is_symlink():
            raise ValueError('TODO directory cannot be a symlink')
        os.chown(target, account.pw_uid, account.pw_gid)
        target.chmod(0o700 if target.name.endswith('.state') else 0o755)
        allow_server_traversal(settings, target.parent)
    binary = Path(record['binary'])
    if binary.exists():
        if binary.is_symlink() or digest(binary) != record['sha256']:
            raise ValueError('TODO service artifact changed during installation')
    else:
        source = bundle / 'bin/opencoder-server'
        if digest(source) != record['sha256']:
            raise ValueError('TODO service source differs from verified bundle')
        atomic_bytes(binary, source.read_bytes(), 0o755)
    allow_server_traversal(settings, binary.parent)
    update_config(settings, record['before'], record['after'])
    unit_path = settings.systemd_dir / UNIT
    if unit_path.exists() and unit_path.read_text() != record['unit']:
        raise ValueError('TODO service unit changed during installation')
    atomic_bytes(unit_path, record['unit'].encode(), 0o644)
    record['phase'] = 'configured'; write(path, record)
    operations.run('systemd-analyze', 'verify', str(unit_path))
    operations.run('systemctl', 'daemon-reload')
    operations.run('systemctl', 'enable', UNIT)
    operations.run('systemctl', 'start', UNIT)
    record['phase'] = 'started'; write(path, record)
    operations.wait(lambda: probe(settings, operations), 30)
    record['phase'] = 'ready'; write(path, record)


def restore(settings, operations):
    """Undo a first installation only while no files have ever been enabled."""
    path = location(settings) / 'installation.json'
    record = json.loads(path.read_text())
    policy = settings.state_dir / 'todo-files-policy.json'
    if policy.exists() and json.loads(policy.read_text())['minimum_contract']:
        raise ValueError('TODO uploads were enabled; retain the service and use compatible rollback')
    journal = Journal(settings.state_dir)
    current = journal.data.get('current')
    if current and contract(journal.record(current)['manifest']):
        raise ValueError('roll back the active TODO Server before restoring its first service installation')
    root = Path(record['after']['files_dir'])
    if root.exists() and any(p.is_file() for p in root.rglob('*')):
        raise ValueError('TODO files exist; installation cannot be removed')
    record['phase'] = 'restoring'; write(path, record)
    unit = settings.systemd_dir / UNIT
    if unit.exists():
        if unit.read_text() != record['unit']:
            raise ValueError('TODO service unit was changed; refusing to stop it')
        operations.run('systemctl', 'disable', '--now', UNIT)
    update_config(settings, record['after'], record['before'])
    if unit.exists():
        if unit.read_text() != record['unit']:
            raise ValueError('TODO service unit was changed; refusing to remove it')
        unit.unlink()
        fd = os.open(unit.parent, os.O_RDONLY | os.O_DIRECTORY)
        try: os.fsync(fd)
        finally: os.close(fd)
    operations.run('systemctl', 'daemon-reload')
    record['phase'] = 'restored'; write(path, record)
    return {'phase': 'restored', 'files_preserved': True}
