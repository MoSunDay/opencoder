"""Restore programs and routing before reopening; never restore a database."""
import copy
from pathlib import Path
from ...deployment import register_server
from ...state import atomic_bytes
from ...storage.database import using
from .. import archive, restore, services
from . import files, gates, programs


def rollback(settings, journal, operations, seconds):
    from ..flow import checkpoint, endpoint
    state = journal.data['maintenance']
    if state['writes_open']:
        raise ValueError('writes may have reopened; use a corrective release, never restore old data')
    if state['stage'] == 'rolled_back':
        return journal.data
    old = journal.record(state['origin'])
    from ...todo_resources import require_candidate
    require_candidate(settings, old['manifest'])
    candidate = journal.record(state['target'])
    native = settings.state_dir / 'releases' / candidate['id'] / 'bundle/bin/opencoder-cli'
    root = Path(state['backup'])
    if state['stage'] in ('closing', 'waiting'):
        checkpoint(journal, 'engine_restore_admission')
    if state['stage'] == 'engine_restore_admission':
        # Drain may have failed while accepted work is still running. No
        # program or database has changed yet: reopen without stopping tasks.
        restore.prior_controller(settings, state['target'], old['manifest'])
        operations.run('systemctl', 'daemon-reload')
        gates.reopen(settings, old, operations, seconds)
        operations.wait(lambda: operations.http(settings.public_url, '/api/ready'), seconds)
        recovered = copy.deepcopy(state['original'])
        recovered['maintenance'] = {**state, 'stage': 'rolled_back'}
        recovered['maintenance'].pop('original', None)
        journal.data = recovered
        journal.phase('rolled_back')
        return journal.data
    if not state['stage'].startswith('engine_restore'):
        checkpoint(journal, 'engine_restore_stopping')
    try:
        if state['stage'] == 'engine_restore_stopping':
            services.stop(settings, journal.data, operations, seconds)
            with using(native):
                registered = programs.registrations(settings)
            state['recovery_databases'] = [str(path) for path in files.managed(settings, registered)]
            # A private probe may have created a candidate Runtime. Its ID and
            # data remain, with the old program attached before any Host wakes it.
            if candidate['id'] in registered and (settings.systemd_dir / candidate['runtime_unit']).exists():
                bundle = settings.state_dir / 'releases' / old['id'] / 'bundle'
                state['recovery_program'] = programs.stage_one(settings, bundle, old['manifest'], candidate,
                                                                Path(old['runtime_data']) / 'dag/rootfs')
            checkpoint(journal, 'engine_restore_programs')
        if state['stage'] == 'engine_restore_programs':
            files.checkpoint_and_unmark([Path(path) for path in state['recovery_databases']], native, operations)
            if root.exists():
                metadata = archive.verify(root)
                restore.controls(settings, root, metadata)
            else:
                restore.prior_controller(settings, state['target'], old['manifest'])
            if state.get('recovery_program'):
                program = state['recovery_program']
                atomic_bytes(settings.systemd_dir / candidate['runtime_unit'],
                             Path(program['unit_source']).read_bytes(), 0o644)
            operations.run('systemctl', 'daemon-reload')
            checkpoint(journal, 'engine_restore_services')
        if state['stage'] == 'engine_restore_services':
            # The independent exporter may predate the origin release and
            # reject an additive schema already supported by that release.
            # Recover it from the origin's verified bundle, never old data.
            bundle = settings.state_dir / 'releases' / old['id'] / 'bundle'
            services.resource_upgrade(settings, bundle, operations)
            operations.wait(lambda: operations.http(settings.resource_url, '/api/health'), seconds)
            if state.get('recovery_program'):
                # A failed candidate can already own the active registration.
                # Its compatible program must answer before an old Host reads it.
                operations.run('systemctl', 'start', candidate['runtime_unit'])
                candidate_url = f"http://127.0.0.1:{candidate['runtime_port']}"
                operations.wait(lambda: operations.http(candidate_url, '/inventory'), seconds)
            services.resume(state['scope'], operations, seconds)
            host = f"http://127.0.0.1:{old['host_port']}"
            operations.wait(lambda: operations.http(host, '/status'), seconds)
            runtime = {'endpoint': f"http://127.0.0.1:{old['runtime_port']}"}
            gates.runtime_call(operations, runtime, 'reopen')
            operations.http(host, f"/runtimes/{old['id']}/activate", 'POST', {})
            register_server(settings, candidate, operations, enabled=False, host_url=host)
            operations.http(host, '/activate-host', 'POST', {})
            operations.http(host, '/commit-host', 'POST', {})
            operations.wait(lambda: operations.http(endpoint(old), '/api/health'), seconds)
            gates.reopen(settings, old, operations, seconds)
            operations.wait(lambda: operations.http(settings.public_url, '/api/ready'), seconds)
            recovered = copy.deepcopy(state['original'])
            if state.get('recovery_program'):
                candidate['runtime_program'] = state['recovery_program']
                candidate['maintenance_retired'] = True
                recovered['releases'][candidate['id']] = candidate
            recovered['maintenance'] = {**state, 'stage': 'rolled_back'}
            recovered['maintenance'].pop('original', None)
            journal.data = recovered
            journal.phase('rolled_back')
    except Exception as error:
        journal.fail(error)
        raise
    return journal.data
