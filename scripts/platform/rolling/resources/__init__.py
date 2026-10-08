"""Bounded resource-only upgrade; business processes and data stay in place."""
from pathlib import Path
import time
from .. import manifest, probes
from ..maintenance import services
from ..state import atomic_bytes, write
from .rpc import check_retransmission


def idle(operations, settings, nodes):
    snapshots = {n['id']: n.get('snapshot') or {} for n in
                 operations.http(settings.public_url, '/api/nodes')['nodes'] if n.get('online')}
    for node in nodes:
        status = operations.http(settings.public_url, '/api/nodes/' + node + '/admission')
        if (status.get('mode') != 'frozen' or status.get('active_runs') != 0
                or status.get('owned_processes') != 0
                or snapshots.get(node, {}).get('pending_runs') != 0):
            return False
    return True


def deploy(settings, bundle, operations, nodes, drain_seconds=600):
    if not nodes or len(nodes) != len(set(nodes)):
        raise ValueError('explicit unique resource consumer node IDs required')
    candidate = manifest.verify(bundle)
    probes.resource_service(settings, candidate, operations)
    before = {node: operations.http(settings.public_url, '/api/nodes/' + node + '/admission')
              for node in nodes}
    if any(row.get('mode') not in ('open', 'frozen') for row in before.values()):
        raise ValueError('cannot determine existing node admission state')
    directory = settings.state_dir / 'resource-upgrades' / candidate['release_id']
    directory.mkdir(parents=True, exist_ok=True)
    binary = settings.state_dir / 'services/opencoder-resources'
    unit = settings.systemd_dir / 'opencoder-resources.service'
    backup_binary, backup_unit = directory / 'binary.before', directory / 'unit.before'
    if backup_binary.exists() or backup_unit.exists():
        raise ValueError('resource upgrade receipt already exists; inspect before retrying')
    atomic_bytes(backup_binary, binary.read_bytes(), 0o700)
    atomic_bytes(backup_unit, unit.read_bytes(), 0o600)
    receipt = {'release_id': candidate['release_id'], 'nodes': nodes,
               'admission_before': before, 'phase': 'draining', 'started_at': time.time()}
    path = directory / 'receipt.json'
    write(path, receipt)
    stopped = False
    healthy = True
    try:
        for node in nodes:
            reply = operations.http(settings.public_url, '/api/nodes/' + node + '/admission', 'POST', {})
            if reply.get('mode') != 'frozen':
                raise ValueError('node failed to freeze: ' + node)
        operations.wait(lambda: idle(operations, settings, nodes), drain_seconds)
        receipt['phase'] = 'switching'
        write(path, receipt)
        started = time.monotonic()
        healthy = False
        stopped = True
        services.stop_unit('opencoder-resources.service', operations, 10)
        services.resource_upgrade(settings, bundle, operations)
        operations.wait(lambda: operations.http(settings.resource_url, '/api/health'), 10)
        probes.resource_service(settings, candidate, operations)
        build = operations.http(settings.resource_url, '/api/health')['build']
        if build.get('git_commit') != candidate['commit']:
            raise ValueError('resource service is running a different build')
        status = operations.http(settings.resource_url, '/api/agents/nfs')['status']
        check_retransmission(status['port'])
        probes.resources(settings, operations)
        gap = time.monotonic() - started
        if gap > 30:
            raise ValueError('resource service interruption exceeded 30 seconds')
        healthy = True
        receipt.update(phase='complete', switch_seconds=gap, finished_at=time.time(),
                       build=build)
        write(path, receipt)
    except BaseException as error:
        receipt.update(phase='failed', error=type(error).__name__ + ': ' + str(error))
        write(path, receipt)
        if stopped and not healthy:
            services.stop_unit('opencoder-resources.service', operations, 10)
            atomic_bytes(binary, backup_binary.read_bytes(), 0o755)
            atomic_bytes(unit, backup_unit.read_bytes(), 0o644)
            operations.run('systemctl', 'daemon-reload')
            operations.run('systemctl', 'start', 'opencoder-resources.service')
            operations.wait(lambda: operations.http(settings.resource_url, '/api/health'), 10)
            probes.resources(settings, operations)
            healthy = True
            receipt['phase'] = 'rolled_back'
            write(path, receipt)
        raise
    finally:
        if healthy:
            errors = []
            for node, previous in before.items():
                if previous['mode'] != 'open':
                    continue
                try:
                    reply = operations.http(settings.public_url, '/api/nodes/' + node + '/admission', 'DELETE')
                    if reply.get('mode') != 'open':
                        raise ValueError('node failed to reopen: ' + node)
                except Exception as error:
                    errors.append({'node': node, 'error': str(error)})
            receipt['reopen_errors'] = errors
            if errors:
                receipt['phase'] = 'reopen_failed'
            write(path, receipt)
            if errors:
                raise RuntimeError('resource upgrade could not restore node admission')
    return receipt
