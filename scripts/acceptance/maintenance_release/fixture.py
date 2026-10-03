"""Fresh old-format Server data and a disclosed candidate Host/Runtime node."""
from contextlib import contextmanager
import json
import os
from pathlib import Path
import secrets
import shutil
import socket
from unittest.mock import patch
from rolling.config import Settings
from rolling.native import effective_config
from rolling import units, probes, manifest
from rolling.state import Journal, atomic_bytes, write
from rolling.maintenance import configuration, services, mounts
from control import Control
from systemd import RealOperations


PORTS = set()


def port():
    low = int(Path('/proc/sys/net/ipv4/ip_local_port_range').read_text().split()[0])
    for attempt in range(500):
        candidate = 2000 + secrets.randbelow(min(low, 10000) - 2000)
        if candidate in PORTS:
            continue
        with socket.socket() as listener:
            try:
                listener.bind(('127.0.0.1', candidate))
            except OSError:
                continue
        PORTS.add(candidate)
        return candidate
    raise RuntimeError('no private listener port is available')


@contextmanager
def configuration_scope(settings, root):
    home = str(root / 'home')
    def actual(current):
        return tuple(effective_config(workdir, home=home)
                     for workdir in (current.agent_workdir, current.server_workdir))
    with patch.dict(os.environ, {'HOME': home, 'XDG_CONFIG_HOME': home + '/.config',
                                'XDG_DATA_HOME': home + '/.local/share'}), \
            patch.object(configuration, 'actual_configs', side_effect=actual), \
            patch.object(services, 'configs', side_effect=actual):
        yield


def create_settings(root):
    for name in ('home', 'units', 'state', 'bin', 'server-work', 'agent-work'):
        (root / name).mkdir()
    public = f'127.0.0.1:{port()}'
    return Settings(root / 'state', root / 'server-work', root / 'server-data',
                    root / 'agent-work', root / 'token', server_user='root',
                    bin_dir=root / 'bin', systemd_dir=root / 'units', node_name=root.name,
                    nginx_include=root / 'ingress.conf', public_url='http://' + public, listen=public,
                    host_port=port(), resource_port=port(), port_base=port(), max_runs=2, min_memory_mb=1,
                    legacy_agent_unit=f'opencoder-fixture-{root.name}-legacy-agent.service',
                    legacy_server_unit=f'opencoder-fixture-{root.name}-legacy-server.service',
                    agent_config=root / 'agent-next.json', server_config=root / 'server-next.json')


def save_settings(settings, path):
    write(path, {'deployment': {key: str(value) if isinstance(value, Path) else value
                                for key, value in settings.__dict__.items()}})


def source_configs(settings, root, rootfs):
    sources, targets, ports = {}, {}, {}
    for name in ('agent', 'binary', 'workspace', 'wasm'):
        sources[name], targets[name], ports[name] = root / 'exports' / name, root / 'mounts' / name, port()
        sources[name].mkdir(parents=True)
        targets[name].mkdir(parents=True)
        (sources[name] / 'immutable.txt').write_text(name + '-fixture-source\n')
    step = sources['workspace'] / 'execute'
    step.mkdir()
    (step / 'source.txt').write_text('Original workspace bytes survive every maintenance stage.\n')
    agent = {'agent': {'agents_dir': str(targets['agent'])}, 'dag': {'wasm_dir': str(targets['wasm'])}}
    server = {'storage': {'backend': 'libsql'}, 'dag': {'wasm_dir': str(sources['wasm']),
              'nfs': {'enabled': True, 'host': '127.0.0.1', 'port': ports['binary'], 'read_only': True}}, 'agent': {'agents_dir': str(sources['agent']),
              'nfs': {'enabled': True, 'host': '127.0.0.1', 'port': ports['agent'], 'read_only': True}}}
    write(settings.agent_workdir / 'opencoder.json', agent)
    write(settings.server_workdir / 'opencoder.json', server)
    for workdir in (settings.agent_workdir, settings.server_workdir):
        write(workdir / '.opencoder/ap.json', {'mode': 'off'})
    write(settings.agent_config, {**agent, 'dag': {'rootfs_dir': str(rootfs),
          'binary_dir': str(targets['binary']), 'workspace_dir': str(targets['workspace'])}})
    write(settings.server_config, {**server, 'dag': {'binary_dir': str(sources['binary']),
          'workspace_dir': str(sources['workspace']),
          'nfs': {'enabled': True, 'host': '127.0.0.1', 'port': ports['binary'], 'read_only': True},
          'workspace_nfs': {'enabled': True, 'host': '127.0.0.1', 'port': ports['workspace']}}})
    return sources, targets, ports


def service(settings, name, command, runtime=False, workdir=None):
    atomic_bytes(settings.systemd_dir / name,
                 units.service(command, 'Private maintenance acceptance', runtime,
                               workdir=workdir).encode(), 0o644)


def launch(settings, operations, old_bundle, old_manifest, candidate_bin, rootfs, targets, ports):
    root = operations.root
    copied = settings.state_dir / 'releases' / old_manifest['release_id'] / 'bundle'
    (copied / 'bin').mkdir(parents=True)
    for name in ('opencoder-server', 'opencoder-agent', 'opencoder', 'opencoder-cli'):
        shutil.copy2(old_bundle / 'bin' / name, copied / 'bin' / name)
    for name in ('manifest.json', 'SHA256SUMS'):
        shutil.copy2(old_bundle / name, copied / name)
    old_server = copied / 'bin/opencoder-server'
    prefix = 'opencoder-fixture-' + root.name
    old = {'id': old_manifest['release_id'], 'manifest': old_manifest, 'server_port': port(),
           'host_port': port(), 'runtime_port': port(), 'created_at': 1,
           'runtime_data': str(settings.state_dir / 'runtimes' / (root.name + '-old-node')),
           'server_unit': prefix + '-old-server.service', 'host_unit': prefix + '-old-host.service',
           'runtime_unit': 'opencoder-runtime-' + root.name + '-old.service'}
    journal = Journal(settings.state_dir)
    journal.data.update(current=old['id'], candidate=None, previous=None, phase='complete', releases={old['id']: old})
    journal.save()
    resource = settings.state_dir / 'services/opencoder-resources'
    atomic_bytes(resource, (copied / 'bin/opencoder-server').read_bytes(), 0o755)
    service(settings, 'opencoder-resources.service', [resource, '--resources', '--workdir', settings.server_workdir,
            '--data-dir', settings.state_dir / 'resources', '--port', settings.resource_port,
            '--token-file', settings.token_file], workdir=settings.server_workdir)
    resource_unit = settings.systemd_dir / 'opencoder-resources.service'
    resource_unit.write_text(resource_unit.read_text().replace(' remote-fs.target opencoder-resources.service', ''))
    operations.run('systemctl', 'enable', '--now', 'opencoder-resources.service')
    operations.wait(lambda: operations.http(settings.resource_url, '/api/health'), 90)
    for kind, export in [('agent', 'agent'), ('wasm', 'binary')]:
        plan = {'path': str(targets[kind]), 'port': ports[export]}
        mount_name = mounts.name(plan['path'])
        atomic_bytes(settings.systemd_dir / mount_name, mounts.unit(plan).encode(), 0o644)
        operations.run('systemctl', 'enable', '--now', mount_name)
    agent = copied / 'bin/opencoder-agent'
    service(settings, old['host_unit'], [agent, '--name', root.name, '--data-dir', settings.state_dir / 'host',
            '--workdir', settings.agent_workdir, '--remote', settings.public_url,
            '--token-file', settings.token_file, '--max-runs', 2, 'host', '--port', old['host_port'], '--standby'])
    operations.run('systemctl', 'start', old['host_unit'])
    host = f"http://127.0.0.1:{old['host_port']}"
    status = operations.wait(lambda: operations.http(host, '/status'), 90)
    data = Path(old['runtime_data'])
    atomic_bytes(data / 'node-id', status['node']['id'].encode())
    write(data / 'host-binding.json', {'database': str(settings.state_dir / 'host/host.db'), 'runtime_id': old['id']})
    runtime_workdir = data / 'workdir'
    write(runtime_workdir / 'opencoder.json', json.loads((settings.agent_workdir / 'opencoder.json').read_text()))
    write(runtime_workdir / '.opencoder/ap.json', {'mode': 'off'})
    service(settings, old['runtime_unit'], [agent, '--data-dir', data, '--workdir', runtime_workdir,
            '--max-runs', 65535, '--token-file', settings.token_file, 'runtime', '--port', old['runtime_port']], True)
    operations.http(host, '/runtimes', 'POST', {'id': old['id'], 'release_id': old['id'], 'mode': 'staged',
                    'config': {'endpoint': f"http://127.0.0.1:{old['runtime_port']}",
                               'data_dir': str(data), 'unit': old['runtime_unit']}})
    operations.run('systemctl', 'start', old['runtime_unit'])
    inventory = operations.wait(lambda: operations.http(f"http://127.0.0.1:{old['runtime_port']}", '/inventory'), 90)
    from legacy_probe import execute
    legacy = execute(old, operations)
    write(root / 'old-stack.json', {'build': manifest._installer.build_info(agent),
          'manifest': old_manifest, 'full_old_binaries': True, 'inventory': inventory, 'probe': legacy})
    release = root / 'old-release.json'
    write(release, {'release_id': old['id'], 'state_dir': str(settings.state_dir),
                    'host_service': settings.host_url, 'resource_service': settings.resource_url})
    service(settings, old['server_unit'], [old_server, '--host', '127.0.0.1', '--port', old['server_port'],
            '--workdir', settings.server_workdir, '--data-dir', settings.server_data,
            '--release-config', release, '--token-file', settings.token_file], workdir=settings.server_workdir)
    operations.run('systemctl', 'start', old['server_unit'])
    base = f"http://127.0.0.1:{old['server_port']}"
    operations.wait(lambda: operations.http(base, '/api/health'), 90)
    operations.http(host, '/servers/' + old['id'], 'POST', {'url': base, 'enabled': True})
    operations.http(host, '/runtimes/' + old['id'] + '/activate', 'POST', {})
    operations.http(host, '/activate-host', 'POST', {})
    atomic_bytes(settings.nginx_include, units.nginx(settings, old['server_port'], old['host_port']).encode(), 0o644)
    operations.nginx_command()
    operations.http(host, '/commit-host', 'POST', {})
    operations.wait(lambda: any(node.get('online') for node in operations.http(base, '/api/nodes')['nodes']), 90)
    return old


def prepare(root, nginx):
    settings = create_settings(root)
    atomic_bytes(settings.token_file, secrets.token_hex(24).encode())
    save_settings(settings, root / 'deployment.json')
    operations = RealOperations(settings, root, nginx)
    return settings, operations, Control(operations)
