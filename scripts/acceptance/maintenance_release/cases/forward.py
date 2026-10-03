"""Actual Server signal repairs a post-open failure through an independent systemd job."""
import json
import os
import signal
from rolling.maintenance import flow
from rolling.state import Journal, write
from signal_release import controller


def run(settings, operations, bundle, control, receipt):
    from scenarios import project_write, check_baselines, authentication_state
    from evidence import snapshots, native_evidence
    root = operations.root
    bad, fix = receipt['candidate_manifest'], receipt['corrective_manifest']
    auth = authentication_state(settings)
    case = {'passed': False, 'artifact_note': 'Two verified bundles from different clean compiled commits; the primary public failure is deliberately injected.'}
    receipt['scenarios']['post_open_signal_corrective_release'] = case
    case['failure'] = receipt['scenarios']['post_schema_recovery_retry']['controller_retry']
    failed = Journal(settings.state_dir)
    state = failed.data['maintenance']
    if not state['writes_open'] or state['stage'] != 'public' or failed.data['current'] != bad['release_id']:
        raise AssertionError('failure did not occur after actual admission reopened')
    frozen = snapshots(state)
    case['new_write'] = project_write(operations, settings.public_url, 'written-before-corrective-release',
                                      ['before-maintenance', 'new-write-after-retry'])
    try:
        flow.rollback(settings, operations, 120)
    except ValueError as error:
        if 'restoration is forbidden' not in str(error):
            raise
    else:
        raise AssertionError('post-open backup restore was accepted')
    controller.install(settings, root / 'deployment.json', operations)
    case['staged'] = controller.stage(settings, bundle, maintenance=True, wait_seconds=120)
    record = failed.record(bad['release_id'])
    server = operations.children[record['server_unit']]
    os.kill(server.pid, signal.SIGUSR2)
    instance = 'deploy--' + bad['release_id']
    evidence = settings.state_dir / 'signal-receipts' / (instance + '.json')
    def finished():
        value = json.loads(evidence.read_text()) if evidence.exists() else {}
        return value if value.get('phase') in ('complete', 'failed') else None
    result = operations.wait(finished, 600)
    if result['phase'] != 'complete' or result['target'] != fix['release_id']:
        raise AssertionError('signal corrective release failed: ' + json.dumps(result))
    complete = Journal(settings.state_dir)
    if complete.data['current'] != fix['release_id'] or snapshots(complete.data['maintenance']) != frozen:
        raise AssertionError('corrective release lost its original backup or activation')
    retirement = complete.data.get('retirement', {}).get(bad['release_id'], {})
    if retirement.get('failure') or retirement.get('phase') == 'failed':
        raise AssertionError('corrective release failed to retire its predecessor: ' + json.dumps(retirement))
    case['api_after_repair'] = project_write(operations, settings.public_url, 'written-after-corrective-release',
        ['written-before-corrective-release', 'before-maintenance', 'new-write-after-retry'])
    case.update(passed=True, signal='SIGUSR2', job=result, backups=frozen,
                public_native_probe=native_evidence(complete.record(fix['release_id']), True),
                invariants=check_baselines(settings, operations, auth, root, 'corrective-release'))
    write(root / 'scenario-corrective.json', case)
