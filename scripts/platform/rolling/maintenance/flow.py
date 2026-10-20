"""Durable maintenance stages; no automatic backup restore after writes reopen."""
import base64
import copy
from pathlib import Path
import time
import uuid
from .. import manifest, probes, units
from ..deployment import record_for, register_runtime, register_server
from ..state import Journal
from . import archive, gates, preflight, restore, services, runtimes, configuration, mounts
from .recovery import forward
from .engine import files as engine_files, gates as local_gates, programs


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
    from ..todo_resources import require_candidate
    require_candidate(settings, candidate)
    state = journal.data.get('maintenance')
    if state and state['stage'] not in ('complete', 'rolled_back'):
        if state['stage'] == 'repairing' or state['target'] != candidate['release_id']:
            return forward.deploy(settings, bundle, journal, candidate, operations, seconds)
        if journal.record(state['target'])['manifest'] != candidate:
            raise ValueError('maintenance release ID already belongs to another immutable bundle')
        if state['stage'].startswith(('restore', 'engine_restore')):
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
        engine_switch = manifest.storage_engine(candidate) != manifest.storage_engine(old['manifest'])
        if engine_switch and (scope['database']['schema_version'] != 39
                or candidate['protocol_version'] != old['manifest']['protocol_version']
                or candidate.get('brain_schema_version') != old['manifest'].get('brain_schema_version')):
            raise ValueError('storage engine maintenance requires the current schema and execution protocol')
        if candidate['compatibility']['data_format']['min'] < old['manifest']['compatibility']['data_format']['min']:
            raise ValueError('maintenance cannot downgrade a live database; use an unopened rollback or a new corrective release')
        nodes = operations.http(endpoint(old), '/api/nodes')['nodes']
        node_id = (settings.state_dir / 'host/node-id').read_text().strip()
        online = [node for node in nodes if node.get('online')]
        if not engine_switch and (len(online) != 1 or online[0]['id'] != node_id):
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
        if engine_switch:
            # The native checkpoint tool must survive failures before any
            # candidate service is prepared or started. Pin it before stopping.
            bundle = units.install_bundle(settings, bundle, record)
        rootfs = units.freeze_rootfs(record, Path(scope['rootfs']), bundle / 'bin')
        if engine_switch:
            all_registered = programs.registrations(settings, include_unavailable=True)
            registered = programs.registrations(settings)
            scope['unavailable_registered'] = {key: value for key, value in all_registered.items()
                                               if key not in registered}
            scope['registered'] = registered
            from .planning import capacity
            scope['program_capacity'] = capacity.check(
                capacity.retained_programs(candidate, rootfs, original, registered))
            scope['programs'] = programs.stage(settings, bundle, candidate, original, registered, rootfs, operations)
            scope['engine_databases'] = [str(path) for path in engine_files.managed(settings, registered)]
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
                 'engine_switch': engine_switch,
                 'original': original, 'scope': scope, 'writes_open': False,
                 'backup': str(settings.state_dir / 'backups' / ('maintenance-' + uuid.uuid4().hex))}
        journal.data.update(maintenance=state, candidate=record['id'], previous=old['id'])
        journal.phase('migrating')
    try:
        if state.get('engine_switch'):
            from ..storage.database import using
            with using(bundle / 'bin/opencoder-cli'):
                return advance(settings, bundle, journal, operations, seconds)
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
        # Recompute after image freezing and on retries. Earlier copies and
        # interrupted staging consume real free space and cannot be ignored.
        from .planning import capacity
        capacity.check(capacity.plan(settings, record['manifest'], Path(state['scope']['rootfs']),
                                     state['original'], state['scope']))
        if state.get('engine_switch'):
            local_gates.close(settings, old, operations, seconds)
        else:
            gates.close(settings, old, operations, seconds)
            operations.http(endpoint(old), '/api/admin/drain', 'POST', {})
        checkpoint(journal, 'waiting')
    if state['stage'] == 'waiting':
        node_id = (settings.state_dir / 'host/node-id').read_text().strip()
        if state.get('engine_switch'):
            operations.wait(lambda: local_gates.drained(settings, old, operations), seconds)
        else:
            operations.wait(lambda: gates.drained(operations, endpoint(old), node_id), seconds)
        manifest.brain_preflight(settings, record['manifest'], state['original']['releases'].values())
        state['scope']['inventories'] = runtimes.capture(settings, operations)
        checkpoint(journal, 'stopping')
    if state['stage'] == 'stopping':
        services.stop(settings, state['original'], operations, seconds)
        checkpoint(journal, 'backup')
    if state['stage'] == 'backup':
        archive.create(settings, Path(state['backup']), state['original'], state['scope'])
        if state.get('engine_switch'):
            engine_files.snapshot(Path(state['backup'] + '-runtimes'), state['scope']['registered'])
        checkpoint(journal, 'installing')
    if state['stage'] == 'installing':
        archive.verify(Path(state['backup']))
        # This offline registry update runs once, before the owning engine is
        # changed. Retrying installation must never open live Turso with SQLite.
        if not state.get('runtimes_installed'):
            runtimes.install(settings, state['scope']['inventories'])
            state['runtimes_installed'] = True
            journal.save()
        if state.get('engine_switch'):
            engine_files.verify(Path(state['backup'] + '-runtimes'))
            programs.install(settings, journal, state['scope']['programs'])
            engine_files.mark([Path(path) for path in state['scope']['engine_databases']])
        configuration.install(settings, state['scope']['configuration'])
        services.resource_upgrade(settings, bundle, operations)
        operations.wait(lambda: operations.http(settings.resource_url, '/api/health'), seconds)
        probes.resource_service(settings, record['manifest'], operations)
        mounts.install(settings, state['scope'].get('mounts', {}).get('native', []), operations)
        # HTTP readiness precedes recovery of existing kernel NFS clients.
        # Keep writers stopped until an actual mounted-directory read succeeds.
        operations.wait(lambda: probes.resources(settings, operations) or True, seconds)
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
        # Record migration intent before the command. Recovery must restore
        # the project schema before starting the old Server.
        state['schema_started'] = True
        journal.save()
        operations.run('systemctl', 'start', record['server_unit'])
        operations.wait(lambda: operations.http(endpoint(record), '/api/health'), seconds)
        gates.close(settings, record, operations, seconds)
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
        if state.get('engine_switch'):
            local_gates.reopen(settings, record, operations, seconds)
        else:
            operations.wait(lambda: operations.http(endpoint(record), '/api/admin/drain', 'DELETE'), seconds)
            gates.reopen(settings, record, operations, seconds)
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
    if state.get('engine_switch'):
        from .engine.recovery import rollback as engine_rollback
        return engine_rollback(settings, journal, operations, seconds)
    from ..todo_resources import require_candidate
    require_candidate(settings, journal.record(state['origin'])['manifest'])
    policy = settings.state_dir / 'todo-files-policy.json'
    if policy.exists():
        import json
        if json.loads(policy.read_text())['minimum_contract']:
            raise ValueError('TODO uploads have been enabled; old database restoration is forbidden')
    if state['writes_open']:
        raise ValueError('writes may have reopened; old backup restoration is forbidden; repair with a new release')
    if state['stage'] == 'rolled_back':
        return journal.data
    record = journal.record(state['origin'])
    backup = Path(state['backup'])
    if not state['stage'].startswith('restore'):
        checkpoint(journal, 'restore_stopping')
    try:
        if state['stage'] == 'restore_stopping':
            services.stop(settings, journal.data, operations, seconds)
            mounts.stop_new(settings, state['scope'].get('mounts', {}).get('native', []), operations)
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
                if prior.get('loaded', True):
                    operations.run('systemctl', 'enable' if prior['enabled'] else 'disable', name)
            services.resume(state['scope'], operations, seconds)
            operations.wait(lambda: operations.http(endpoint(record), '/api/health'), seconds)
            operations.http(endpoint(record), '/api/project/overview')
            operations.wait(lambda: operations.http(endpoint(record), '/api/admin/drain', 'DELETE'), seconds)
            checkpoint(journal, 'restore_ingress')
        if state['stage'] == 'restore_ingress':
            if metadata:
                restore.ingress(settings, backup, metadata, operations, seconds)
            elif state['scope']['nginx'] is not None:
                from ..state import atomic_bytes
                workers = operations.ingress_workers()
                atomic_bytes(settings.nginx_include, base64.b64decode(state['scope']['nginx']), 0o644)
                operations.run('nginx', '-t')
                operations.run('systemctl', 'reload', 'nginx')
                operations.wait(lambda: operations.ingress_switched(workers), seconds)
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
