"""Compatible corrective releases retain the original maintenance recovery anchor."""
import time
from ... import manifest


def active(state):
    return bool(state and state['stage'] not in ('complete', 'rolled_back'))


def validate(data, candidate):
    state = data.get('maintenance')
    if not active(state) or not state.get('writes_open') or state['stage'].startswith('restore'):
        raise ValueError('another maintenance upgrade must be resumed or restored first')
    identifier = candidate['release_id']
    if identifier == state['target']:
        raise ValueError('resume the recorded corrective release instead of its failed predecessor')
    pending = state.get('repair_target')
    if pending and pending != identifier and data['current'] != pending:
        raise ValueError('another corrective release is still being prepared; resume it first')
    current = data['releases'][data['current']]['manifest']
    manifest.compatible(candidate, [current, data['releases'][state['target']]['manifest']])
    if any(record['manifest']['commit'] == candidate['commit'] and name != identifier
           for name, record in data['releases'].items()):
        raise ValueError('corrective release requires a different compiled commit')
    if identifier in data['releases']:
        if identifier != pending or data['releases'][identifier]['manifest'] != candidate:
            raise ValueError('corrective release requires a fresh immutable release ID')


def permitted(data, identifier):
    state = data.get('maintenance')
    return bool(state and state.get('writes_open') and state['stage'] == 'repairing'
                and state.get('repair_target') == identifier)


def deploy(settings, bundle, journal, candidate, operations, seconds):
    from ... import deployment
    validate(journal.data, candidate)
    state = journal.data['maintenance']
    identifier = candidate['release_id']
    if state.get('repair_target') != identifier:
        # Retired pre-migration processes cannot overlap with either modern
        # version. Keep their records and sealed backup for audit, never restore.
        for prior in state['original']['releases']:
            if prior != state['target']:
                journal.record(prior)['maintenance_retired'] = True
        state.update(stage='repairing', repair_target=identifier,
                     repair_origin=journal.data['current'])
        state.setdefault('history', []).append({'stage': 'repairing', 'target': identifier,
                                                'at': time.time_ns()})
        journal.data.update(candidate=None, failure=None)
        journal.phase('complete')
    result = deployment.deploy(settings, bundle, operations, seconds)
    journal.data = result
    state = result['maintenance']
    record = journal.record(identifier)
    for name, prior in state['scope']['services'].items():
        if prior['enabled'] and name != 'opencoder-resources.service':
            operations.run('systemctl', 'disable', name)
    operations.run('systemctl', 'enable', record['server_unit'], record['runtime_unit'], record['host_unit'])
    state['stage'] = 'complete'
    state.setdefault('history', []).append({'stage': 'complete', 'target': identifier,
                                            'at': time.time_ns()})
    journal.save()
    return journal.data
