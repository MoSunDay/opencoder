"""Stop every retained writer; replace the independent resource service."""
import shutil
import subprocess
import time
from pathlib import Path
from .. import probes, units
from ..state import atomic_bytes
from .configuration import actual_configs as configs
from .preflight import workspace_source


def service_names(settings, journal):
    names = {settings.legacy_server_unit, settings.legacy_agent_unit, 'opencoder-resources.service'}
    for record in journal['releases'].values():
        names.update(record[key] for key in ('server_unit', 'runtime_unit', 'host_unit'))
        names.update(item['unit'] for key in ('previous_servers', 'previous_hosts') for item in record.get(key, []))
    return sorted(names)


def states(settings, journal, operations):
    result = {}
    for name in service_names(settings, journal):
        raw = operations.output('systemctl', 'show', name, '-p', 'ActiveState', '-p', 'UnitFileState', '-p', 'LoadState')
        values = dict(line.split('=', 1) for line in raw.splitlines() if '=' in line)
        result[name] = {'loaded': values.get('LoadState') != 'not-found',
                        'active': values.get('ActiveState') == 'active',
                        'enabled': values.get('UnitFileState') == 'enabled'}
    return result


def stopped_state(name, status):
    return (status.get('ActiveState') in ('inactive', 'failed')
            and (name.endswith('.mount') or status.get('MainPID') == '0'))


def stop_unit(name, operations, seconds):
    operations.run('systemctl', '--no-block', 'stop', name)
    started = last_signal = time.monotonic()
    forced = False
    def stopped():
        nonlocal last_signal, forced
        raw = operations.output('systemctl', 'show', name, '-p', 'ActiveState', '-p', 'MainPID')
        status = dict(line.split('=', 1) for line in raw.splitlines() if '=' in line)
        if stopped_state(name, status):
            return True
        now = time.monotonic()
        # The maintenance caller has already frozen admission and captured
        # idle inventories. A legacy service may wait forever on its channel
        # tasks (TimeoutStopSec=infinity); finish this explicit unit stop while
        # retaining time to confirm that systemd actually reaped the process.
        if name.endswith('.service') and status.get('ActiveState') == 'deactivating' and not forced and now - started >= max(1, seconds - 5):
            try:
                operations.run('systemctl', 'kill', '--kill-who=all', '--signal=SIGKILL', name)
            except subprocess.CalledProcessError:
                current = operations.output('systemctl', 'show', name, '-p', 'ActiveState', '-p', 'MainPID')
                actual = dict(line.split('=', 1) for line in current.splitlines() if '=' in line)
                if stopped_state(name, actual):
                    return True
                raise
            forced = True
        elif status.get('ActiveState') == 'deactivating' and int(status.get('MainPID', '0')) and now - last_signal >= 1:
            try:
                operations.run('systemctl', 'kill', '--kill-who=main', '--signal=SIGTERM', name)
            except subprocess.CalledProcessError:
                state = operations.output('systemctl', 'show', name, '-p', 'ActiveState', '--value').strip()
                if state in ('inactive', 'failed'):
                    return True
                raise
            last_signal = now
        return False
    operations.wait(stopped, seconds)


def stop(settings, journal, operations, seconds=90):
    names = service_names(settings, journal)
    servers = {settings.legacy_server_unit}
    for record in journal['releases'].values():
        servers.add(record['server_unit'])
        servers.update(item['unit'] for item in record.get('previous_servers', []))
    # Closed, idle Servers release node channels before old Hosts shut down.
    # Keep exporters alive until every consumer has stopped.
    order = sorted(names, key=lambda n: (2 if n == 'opencoder-resources.service'
                                       else 0 if n in servers else 1, n))
    for name in order:
        if operations.output('systemctl', 'show', name, '-p', 'LoadState', '--value').strip() == 'not-found':
            continue
        stop_unit(name, operations, seconds)


def resume(scope, operations, seconds):
    """A restored Host requires its formerly active Runtimes to answer first."""
    import json
    names = [name for name, state in scope['services'].items() if state['active']]
    inventories = {json.loads(item['config']).get('unit'): json.loads(item['config'])['endpoint']
                   for item in scope.get('inventories', {}).values()}
    for name in sorted(names, key=lambda n: ('resources' not in n, 'runtime' not in n,
                                            'host' not in n, n)):
        operations.run('systemctl', 'start', name)
        if name == 'opencoder-resources.service':
            from .mounts import resume_existing
            resume_existing(scope.get('mounts', {}).get('native', []), operations)
        if name in inventories:
            operations.wait(lambda: operations.http(inventories[name].rstrip('/'), '/inventory'), seconds)


def resource_upgrade(settings, bundle, operations):
    _, config = configs(settings)
    dag = config.get('dag', {})
    if dag.get('workspace_dir'):
        managed_paths = [settings.state_dir]
        if dag.get('binary_dir'):
            managed_paths.append(Path(dag['binary_dir']))
        workspace_source(Path(dag['workspace_dir']), settings.server_user, operations, managed_paths)
    if dag.get('binary_dir'):
        path = Path(dag['binary_dir'])
        path.mkdir(parents=True, exist_ok=True)
        shutil.chown(path, user=settings.server_user)
    binary = settings.state_dir / 'services/opencoder-resources'
    atomic_bytes(binary, (bundle / 'bin/opencoder-server').read_bytes(), 0o755)
    data = settings.state_dir / 'resources'
    data.mkdir(exist_ok=True)
    shutil.chown(data, user=settings.server_user)
    content = units.service([binary, '--resources', '--workdir', settings.server_workdir,
                             '--data-dir', data, '--port', settings.resource_port,
                             '--token-file', settings.token_file],
                            'OpenCoder independent resource service', user=settings.server_user,
                            workdir=settings.server_workdir)
    content = content.replace(' remote-fs.target opencoder-resources.service', '')
    content = content.replace('Type=simple', 'Type=simple\n' +
                             units.inherited_environment(settings, settings.legacy_server_unit))
    atomic_bytes(settings.systemd_dir / 'opencoder-resources.service', content.encode(), 0o644)
    operations.run('systemd-analyze', 'verify', str(settings.systemd_dir / 'opencoder-resources.service'))
    operations.run('systemctl', 'daemon-reload')
    operations.run('systemctl', 'enable', '--now', 'opencoder-resources.service')


def internal(settings, record, operations, seconds):
    probes.resources(settings, operations)
    endpoint = f"http://127.0.0.1:{record['server_port']}"
    operations.wait(lambda: operations.http(endpoint, '/api/health'), seconds)
    release = operations.wait(lambda: operations.http(endpoint, '/api/admin/release'), seconds)
    if release.get('instance_release') != record['id']:
        raise ValueError('candidate Server identity differs from its release')
    operations.http(endpoint, '/api/project/overview')
    operations.http(endpoint, '/api/project/tags')
    operations.wait(lambda: operations.http(endpoint, '/api/nodes')['nodes'], seconds)
