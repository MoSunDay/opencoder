"""Durable maintenance stages; restore is forbidden once migration may start."""
import base64
import copy
from pathlib import Path
import time
import uuid
from .. import manifest, probes, units
from ..deployment import record_for, register_runtime, register_server
from ..state import Journal
from . import archive, gates, preflight, restore, services, mounts, configuration, runtimes


def checkpoint(journal, stage):
    current = journal.data['maintenance']
    current['stage'] = stage
    current.setdefault('history', []).append({'stage': stage, 'at': time.time_ns()})
    journal.save()


def endpoint(record):
    return f"http://127.0.0.1:{record['server_port']}"


def deploy(settings, bundle, operations, seconds=90):
    journal = Journal(settings.state_dir)
    candidate = manifest.verify(bundle)
    state = journal.data.get('maintenance')
    if state and state['stage'] not in ('complete', 'rolled_back'):
        if state['target'] != candidate['release_id']:
            raise ValueError('another maintenance upgrade must be resumed or restored first')
        if journal.record(state['target'])['manifest'] != candidate:
            raise ValueError('maintenance release ID already belongs to another immutable bundle')
        if state['stage'].startswith('restore'):
            return rollback(settings, operations, seconds)
    else:
        if journal.data['current'] == candidate['release_id']:
            probes.public(settings, journal.record(candidate['release_id']), operations, seconds)
            return journal.data
        if not journal.data['current'] or journal.data['candidate']:
            raise ValueError('maintenance requires a completed initial migration and no unfinished rolling release')
        manifest.resources(settings, candidate)
        scope = preflight.check(settings, candidate, operations)
        scope['configuration'] = configuration.freeze(settings, candidate['release_id'], scope['configuration_hashes'])
        original = copy.deepcopy(journal.data)
        original.pop('maintenance', None)
        old = journal.record(original['current'])
        if candidate['compatibility']['data_format']['min'] < old['manifest']['compatibility']['data_format']['min']:
            raise ValueError('maintenance cannot downgrade a live database; use an unopened rollback or a new corrective release')
        nodes = operations.http(endpoint(old), '/api/nodes')['nodes']
        node_id = (settings.state_dir / 'host/node-id').read_text().strip()
        if len(nodes) != 1 or nodes[0]['id'] != node_id or not nodes[0].get('online'):
            raise ValueError('maintenance requires the single configured local node; remote writers must be stopped separately')
        scope.update(services=services.states(settings, original, operations),
                     nginx=base64.b64encode(settings.nginx_include.read_bytes()).decode()
                     if settings.nginx_include.exists() else None)
        used = [r[k] for r in original['releases'].values() for k in ('server_port', 'runtime_port', 'host_port')]
        used.extend(h['port'] for r in original['releases'].values()
                    for key in ('previous_hosts', 'previous_servers') for h in r.get(key, []))
        record = record_for(settings, candidate, (max(used) + 3 - settings.port_base) // 3)
        record['probe_epoch'] = time.time_ns()
        if record['id'] in original['releases']:
            raise ValueError('maintenance candidate must have a fresh immutable release ID')
        rootfs = units.freeze_rootfs(record, Path(scope['rootfs']), bundle / 'bin')
        # Execute both packaged runners inside the release image before any
        # service stop; missing dynamic libraries/architecture fail here.
        import json
        expected = manifest._installer.build_info(bundle / 'bin/opencoder-agent')
        for name in ('dag-runner', 'agent-step-runner'):
            info = json.loads(operations.output('chroot', str(rootfs), '/usr/bin/' + name, '--build-info'))
            if info != expected:
                raise ValueError('DAG image runner metadata differs from the candidate bundle')
        from signal_release.controller import remember_controller
        remember_controller(settings, candidate['release_id'])
        journal.data['releases'][record['id']] = record
        state = {'target': record['id'], 'origin': old['id'], 'stage': 'closing',
                 'original': original, 'scope': scope, 'writes_open': False,
                 'backup': str(settings.state_dir / 'backups' / ('maintenance-' + uuid.uuid4().hex))}
        journal.data.update(maintenance=state, candidate=record['id'], previous=old['id'])
        journal.phase('migrating')
    try:
        return advance(settings, bundle, journal, operations, seconds)
    except Exception as error:
        journal.fail(error)
        # Preserve the closed gate and evidence; recovery is an explicit
        # --rollback or an exact-candidate retry, never a silent DB overwrite.
        raise


def advance(settings, bundle, journal, operations, seconds):
    state = journal.data['maintenance']
    old = journal.record(state['origin'])
    record = journal.record(state['target'])
    host_url = f"http://127.0.0.1:{record['host_port']}"
    runtime_url = f"http://127.0.0.1:{record['runtime_port']}"
    if state['stage'] == 'closing':
        gates.close(settings, old, operations)
        operations.http(endpoint(old), '/api/admin/drain', 'POST', {})
        checkpoint(journal, 'waiting')
    if state['stage'] == 'waiting':
        operations.wait(lambda: gates.drained(operations, endpoint(old)), seconds)
        manifest.brain_preflight(settings, record['manifest'], state['original']['releases'].values())
        state['scope']['runtime_inventories'] = runtimes.capture(settings, operations)
        checkpoint(journal, 'stopping')
    if state['stage'] == 'stopping':
        services.stop(settings, state['original'], operations, seconds)
        checkpoint(journal, 'backup')
    if state['stage'] == 'backup':
        archive.create(settings, Path(state['backup']), state['original'], state['scope'])
        checkpoint(journal, 'installing')
    if state['stage'] == 'installing':
        archive.verify(Path(state['backup']))
        runtimes.hibernate_stopped(settings, state['scope']['runtime_inventories'])
        configuration.install(settings, state['scope']['configuration'])
        services.resource_upgrade(settings, bundle, operations)
        operations.wait(lambda: operations.http(settings.resource_url, '/api/health'), seconds)
        probes.resource_service(settings, record['manifest'], operations)
        mounts.install(settings, state['scope'].get('mounts', {}).get('native', []), operations)
        probes.resources(settings, operations)
        units.prepare(settings, bundle, record)
        units.validate(settings, record, operations)
        operations.run('systemctl', 'daemon-reload')
        operations.run('systemctl', 'start', record['host_unit'])
        operations.wait(lambda: operations.http(host_url, '/status'), seconds)
        register_runtime(settings, record, operations, host_url=host_url)
        operations.run('systemctl', 'start', record['runtime_unit'])
        operations.wait(lambda: operations.http(runtime_url, '/inventory'), seconds)
        # Only the private Runtime probe may submit work while the Server
        # admission database and public write entry remain frozen.
        operations.http(runtime_url, '/rpc', 'POST', {'operation': 'admission', 'command': 'reopen'})
        probes.candidate(settings, record, operations, seconds)
        operations.http(host_url, f"/runtimes/{record['id']}/activate", 'POST', {})
        register_server(settings, record, operations, host_url=host_url)
        operations.http(host_url, '/activate-host', 'POST', {})
        # Starting the sole Server performs the transactional schema migration.
        # Persist intent before the command: a lost acknowledgement cannot
        # permit an old binary to open a database that may already be v32.
        state['schema_started'] = True
        journal.save()
        operations.run('systemctl', 'start', record['server_unit'])
        operations.wait(lambda: operations.http(endpoint(record), '/api/health'), seconds)
        gates.close(settings, record, operations)
        operations.http(host_url, '/commit-host', 'POST', {})
        checkpoint(journal, 'verifying')
    if state['stage'] == 'verifying':
        services.internal(settings, record, operations, seconds)
        checkpoint(journal, 'reopening')
    if state['stage'] == 'reopening':
        # Commit the irreversible boundary BEFORE any admission reopening.
        # A lost HTTP/reload reply can never authorize an old-data restore.
        state['writes_open'] = True
        journal.data.update(current=record['id'], candidate=record['id'])
        journal.save()
        operations.wait(lambda: operations.http(endpoint(record), '/api/admin/drain', 'DELETE'), seconds)
        units.switch_ingress(settings, record, operations)
        checkpoint(journal, 'public')
    if state['stage'] == 'public':
        probes.public(settings, record, operations, seconds)
        units.activate_launchers(settings, record, operations)
        for name, prior in state['scope']['services'].items():
            if prior['enabled'] and name != 'opencoder-resources.service':
                operations.run('systemctl', 'disable', name)
        operations.run('systemctl', 'enable', record['server_unit'], record['runtime_unit'], record['host_unit'])
        for other in journal.data['releases'].values():
            if other['id'] != record['id']:
                other['maintenance_retired'] = True
        journal.data.update(candidate=None, failure=None)
        journal.data['previous'] = None
        checkpoint(journal, 'complete')
        journal.phase('complete')
    return journal.data


def rollback(settings, operations, seconds=90):
    journal = Journal(settings.state_dir)
    state = journal.data.get('maintenance')
    if not state:
        raise ValueError('no maintenance upgrade is recorded')
    if state.get('schema_started') or state['writes_open']:
        raise ValueError('schema migration may have committed; old backup restoration is forbidden; resume this candidate or repair with a new compatible release')
    if state['stage'] == 'rolled_back':
        return journal.data
    record = journal.record(state['origin'])
    backup = Path(state['backup'])
    if not state['stage'].startswith('restore'):
        checkpoint(journal, 'restore_stopping')
    try:
        if state['stage'] == 'restore_stopping':
            services.stop(settings, journal.data, operations, seconds)
            checkpoint(journal, 'restore_data')
        metadata = archive.verify(backup) if backup.exists() else None
        if state['stage'] == 'restore_data':
            if metadata:
                restore.data(settings, backup, metadata)
                restore.controls(settings, backup, metadata)
            else:
                restore.prior_controller(settings, state['target'])
            operations.run('systemctl', 'daemon-reload')
            checkpoint(journal, 'restore_services')
        if state['stage'] == 'restore_services':
            for name, prior in state['scope']['services'].items():
                operations.run('systemctl', 'enable' if prior['enabled'] else 'disable', name)
            names = [n for n, s in state['scope']['services'].items() if s['active']]
            for name in sorted(names, key=lambda n: ('resources' not in n, 'server' not in n, n)):
                operations.run('systemctl', 'start', name)
            operations.wait(lambda: operations.http(endpoint(record), '/api/health'), seconds)
            operations.http(endpoint(record), '/api/project/overview')
            operations.wait(lambda: operations.http(endpoint(record), '/api/admin/drain', 'DELETE'), seconds)
            checkpoint(journal, 'restore_ingress')
        if state['stage'] == 'restore_ingress':
            if metadata:
                restore.ingress(settings, backup, metadata, operations)
            elif state['scope']['nginx'] is not None:
                from ..state import atomic_bytes
                atomic_bytes(settings.nginx_include, base64.b64decode(state['scope']['nginx']), 0o644)
                operations.run('nginx', '-t')
                operations.run('systemctl', 'reload', 'nginx')
            operations.wait(lambda: operations.http(settings.public_url, '/api/health'), seconds)
            operations.http(settings.public_url, '/api/project/overview')
            # The saved journal is restored only after the old service answers.
            history = state['history']
            journal.data = copy.deepcopy(state['original'])
            journal.data['maintenance'] = {**state, 'stage': 'rolled_back', 'history': history}
            journal.data['maintenance'].pop('original', None)
            journal.phase('rolled_back')
    except Exception as error:
        journal.fail(error)
        raise
    return journal.data
