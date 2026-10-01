"""Stop every retained writer; replace the independent resource service."""
import shutil
from pathlib import Path
import time
import subprocess
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
        raw = operations.output('systemctl', 'show', name, '-p', 'ActiveState', '-p', 'UnitFileState')
        values = dict(line.split('=', 1) for line in raw.splitlines() if '=' in line)
        result[name] = {'active': values.get('ActiveState') == 'active',
                        'enabled': values.get('UnitFileState') == 'enabled'}
    return result


def stop_unit(name, operations, seconds):
    operations.run('systemctl', '--no-block', 'stop', name)
    last_signal = time.monotonic()
    def stopped():
        nonlocal last_signal
        raw = operations.output('systemctl', 'show', name, '-p', 'ActiveState', '-p', 'MainPID')
        status = dict(line.split('=', 1) for line in raw.splitlines() if '=' in line)
        if status.get('ActiveState') in ('inactive', 'failed'):
            return True
        now = time.monotonic()
        if (status.get('ActiveState') == 'deactivating' and
                int(status.get('MainPID', '0')) > 0 and now - last_signal >= 1):
            # An old process may have missed the first signal while replacing
            # its listener. Repeat the graceful signal for this exact unit.
            try:
                operations.run('systemctl', 'kill', '--kill-who=main', '--signal=SIGTERM', name)
            except subprocess.CalledProcessError:
                # Exit can race the MainPID read. Accept it only after checking
                # the terminal systemd state again.
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
    # Admission is closed and every task is idle. Close Server node channels
    # first so old Hosts can finish shutdown; keep NFS alive until all consumers
    # have stopped and released their mounts.
    order = sorted(names, key=lambda n: (2 if n == 'opencoder-resources.service'
                                        else 0 if n in servers else 1, n))
    for name in order:
        if operations.output('systemctl', 'show', name, '-p', 'LoadState', '--value').strip() == 'not-found':
            continue
        stop_unit(name, operations, seconds)


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
