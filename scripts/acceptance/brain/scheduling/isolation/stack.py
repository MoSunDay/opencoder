#!/usr/bin/env python3
"""Start a disjoint hosted stack from the exact candidate bundle and model config."""
import argparse
import copy
import json
import os
from pathlib import Path
import secrets
import subprocess
import sys
import time

sys.path.insert(0, str(Path(__file__).resolve().parents[4] / 'platform'))
from rolling.config import load
from rolling.deployment import register_runtime, register_server
from rolling.io import Operations
from rolling.manifest import verify
from rolling.network.ports import first_available
from rolling import probes, units
from rolling.state import Journal, atomic_bytes, write


def run(*command):
    subprocess.run([str(arg) for arg in command], check=True, timeout=120)


def provision(bundle, model_config, source_rootfs, root, port_base):
    if root.exists():
        raise ValueError('isolated stack root must be new; retain previous evidence')
    manifest = verify(bundle)
    root.mkdir(parents=True, mode=0o700)
    os.environ['HOME'] = str(root / 'home')
    (root / 'home').mkdir()
    port = first_available(port_base, 8)
    suffix = 'brain-e2e-' + secrets.token_hex(6)
    state = root / 'state'
    record = {'id': manifest['release_id'], 'manifest': manifest,
        'runtime_data': str(state / 'runtimes' / manifest['release_id']),
        'server_port': port, 'host_port': port + 1, 'runtime_port': port + 2,
        'created_at': int(time.time() * 1000),
        **{key + '_unit': f'opencoder-{key}-{suffix}.service' for key in ('server', 'host', 'runtime')}}
    resource_unit = f'opencoder-resources-{suffix}.service'
    incompatible_unit = f'opencoder-incompatible-{suffix}.service'
    incompatible_data = root / 'incompatible-host'
    token = root / 'token'
    atomic_bytes(token, secrets.token_urlsafe(40).encode())
    server_workdir, agent_workdir = root / 'server-work', root / 'agent-work'
    for directory in [server_workdir, agent_workdir, incompatible_data, root / 'source', root / 'mounts',
                      root / 'teams', root / 'knowledge', state / 'host']:
        directory.mkdir(parents=True, exist_ok=True)
    source_config = json.loads(model_config.read_text())
    server_config = copy.deepcopy(source_config)
    # No global operator/domain configuration or production resource path is inherited.
    server_config['opencoder_server'] = {'enabled': False, 'url': f'http://127.0.0.1:{port}'}
    server_config['team_root'] = str(root / 'teams')
    server_config['ontology'] = {'nfs': {'enabled': False}}
    server_config.setdefault('agent', {})['nfs'] = {'enabled': True, 'host': '127.0.0.1', 'port': port + 4, 'read_only': True}
    server_config.setdefault('dag', {}).update(
        nfs={'enabled': True, 'host': '127.0.0.1', 'port': port + 5, 'read_only': True},
        workspace_nfs={'enabled': True, 'host': '127.0.0.1', 'port': port + 6},
        rootfs_dir=str(source_rootfs), data_dir=str(root / 'resource-runs'),
        knowledge_root=str(root / 'knowledge'))
    mappings = [('agent', 'agents_dir', 'agents', port + 4),
                ('dag', 'binary_dir', 'binaries', port + 5),
                ('dag', 'workspace_dir', 'workspace', port + 6)]
    for section, key, name, _ in mappings:
        (root / 'source' / name).mkdir()
        (root / 'mounts' / name).mkdir()
        server_config[section][key] = str(root / 'source' / name)
    write(server_workdir / 'opencoder.json', server_config)
    agent_config = copy.deepcopy(server_config)
    for section, key, name, _ in mappings:
        agent_config[section][key] = str(root / 'mounts' / name)
    write(agent_workdir / 'opencoder.json', agent_config)
    deployment = dict(state_dir=str(state), server_workdir=str(server_workdir),
        server_data=str(root / 'server-data'), agent_workdir=str(agent_workdir), token_file=str(token),
        server_user='root', node_name=suffix, public_url=f'http://127.0.0.1:{port}',
        listen=f'127.0.0.1:{port}', host_port=port + 1, resource_port=port + 3,
        port_base=port, max_runs=4, bin_dir=str(root / 'bin'),
        nginx_include=str(root / 'unused-nginx.conf'),
        legacy_agent_unit=f'{suffix}-unused-agent.service', legacy_server_unit=f'{suffix}-unused-server.service')
    config_path = root / 'opencoder.json'
    write(config_path, {'deployment': deployment})
    settings = load(config_path)
    node_id = 'node-' + secrets.token_hex(12)
    atomic_bytes(state / 'host/node-id', node_id.encode())
    receipt = {'root': str(root), 'node_id': node_id, 'record': record,
        'resource_unit': resource_unit, 'config': str(config_path), 'mounts': []}
    write(state / 'isolation.json', receipt)
    # Preparation uses the same immutable bundle and rootfs machinery as release.
    units.prepare(settings, bundle, record)
    binary = state / 'releases' / record['id'] / 'bundle/bin/opencoder-server'
    resource_command = [binary, '--resources', '--port', port + 3, '--workdir', server_workdir,
                        '--data-dir', root / 'resource-data', '--token-file', token]
    content = units.service(resource_command, resource_unit, workdir=server_workdir)
    atomic_bytes(settings.systemd_dir / resource_unit, content.encode(), 0o644)
    for unit in [resource_unit, *[record[key + '_unit'] for key in ('server', 'host', 'runtime')]]:
        path = settings.systemd_dir / unit
        content = path.read_text().replace('Type=simple', 'Type=simple\nEnvironment="HOME=' + str(root / 'home') + '"\nEnvironment="PATH=/root/.local/bin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"')
        # Tests, never the service manager, decide when a killed Runtime restarts.
        content = content.replace('Restart=on-failure', 'Restart=no')
        atomic_bytes(path, content.encode(), 0o644)
    run('systemctl', 'daemon-reload')
    run('systemctl', 'start', resource_unit)
    operations = Operations(token)
    operations.wait(lambda: operations.http(settings.resource_url, '/api/health'), 60)
    for _, _, name, nfs_port in mappings:
        destination = root / 'mounts' / name
        run('mount', '-t', 'nfs', '-o', f'ro,vers=3,tcp,port={nfs_port},mountport={nfs_port},nolock,soft,timeo=10,retrans=1,actimeo=0,lookupcache=none', '127.0.0.1:/', destination)
        receipt['mounts'].append(str(destination))
        write(state / 'isolation.json', receipt)
    run('systemctl', 'start', record['host_unit'])
    operations.wait(lambda: operations.http(settings.host_url, '/status'), 60)
    register_runtime(settings, record, operations)
    run('systemctl', 'start', record['runtime_unit'])
    probes.candidate(settings, record, operations, 120)
    operations.http(settings.host_url, '/runtimes/' + record['id'] + '/activate', 'POST', {})
    run('systemctl', 'start', record['server_unit'])
    register_server(settings, record, operations)
    # Keep one real online node that deliberately lacks Brain.  The definitions
    # case pins a Brain run to it and must observe the server's explicit 503
    # placement rejection rather than silently having no incompatible target.
    incompatible_binary = state / 'releases' / record['id'] / 'bundle/bin/opencoder-agent'
    incompatible_command = [incompatible_binary, '--remote', settings.public_url,
        '--token-file', token, '--name', suffix + '-incompatible', '--workdir', agent_workdir,
        '--data-dir', incompatible_data, '--max-runs', 1, 'host',
        '--port', port + 7, '--no-brain']
    incompatible_content = units.service(incompatible_command, incompatible_unit, workdir=agent_workdir)
    incompatible_content = incompatible_content.replace('Type=simple',
        'Type=simple\nEnvironment="HOME=' + str(root / 'home') + '"\nEnvironment="PATH=/root/.local/bin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"')
    incompatible_content = incompatible_content.replace('Restart=on-failure', 'Restart=no')
    atomic_bytes(settings.systemd_dir / incompatible_unit, incompatible_content.encode(), 0o644)
    run('systemctl', 'daemon-reload')
    run('systemctl', 'start', incompatible_unit)
    incompatible_node = incompatible_data / 'node-id'
    operations.wait(lambda: incompatible_node.exists(), 30)
    incompatible_node_id = incompatible_node.read_text().strip()
    operations.wait(lambda: any(node['id'] == incompatible_node_id and node['online']
                                for node in operations.http(settings.public_url, '/api/nodes')['nodes']), 60)
    receipt.update(incompatible_unit=incompatible_unit, incompatible_node_id=incompatible_node_id)
    write(state / 'isolation.json', receipt)
    operations.http(settings.host_url, '/activate-host', 'POST', {})
    operations.http(settings.host_url, '/commit-host', 'POST', {})
    journal = Journal(state)
    journal.data.update(current=record['id'], phase='complete', releases={record['id']: record})
    journal.save()
    probes.ready(settings, record, node_id, operations, 120)
    probes.public(settings, record, operations, 120)
    write(root / 'ready.json', {'config':str(config_path), 'commit':manifest['commit'], 'node_id':node_id, 'isolated':True})
    print(json.dumps({'config':str(config_path), 'commit':manifest['commit'], 'node_id':node_id}))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('bundle', 'model-config', 'rootfs', 'root'):
        parser.add_argument('--' + name, required=True, type=Path)
    parser.add_argument('--port-base', type=int, default=42000)
    args = parser.parse_args()
    provision(args.bundle.resolve(), args.model_config.resolve(), args.rootfs.resolve(), args.root.resolve(), args.port_base)


if __name__ == '__main__':
    main()
