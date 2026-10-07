"""Crash recovery, migrated rollback, exact retry and post-open refusal."""
import json
from pathlib import Path
import subprocess
import sys
from rolling import probes
from rolling.maintenance import flow, preflight, archive
from rolling.state import Journal, write
from evidence import authentication, schema, snapshots, unchanged_source, native_evidence
from worker import closed_gates


def controller(root, bundle, control, operations, fault, expected_code):
    command = [sys.executable, str(Path(__file__).with_name('worker.py')),
               '--config', str(root / 'deployment.json'), '--bundle', str(bundle),
               '--socket', str(control.path), '--fault', fault]
    with (root / ('controller-' + fault + '.log')).open('ab') as log:
        child = subprocess.Popen(command, env=operations.env, stdout=log, stderr=log)
    try:
        code = child.wait(timeout=600)
    except BaseException:
        child.terminate()
        child.wait(timeout=30)
        raise
    if code != expected_code:
        raise AssertionError(f'{fault} controller exited {code}, expected {expected_code}; see controller-{fault}.log')
    return {'command': command, 'pid': child.pid, 'exit_code': code}


def project_write(operations, url, title, expected_titles):
    created = operations.http(url, '/api/project/goals', 'POST', {'title': title, 'status': 'planned'})
    goals = operations.http(url, '/api/project/goals')
    for expected in [*expected_titles, title]:
        if expected not in json.dumps(goals):
            raise AssertionError('project API lost a goal: ' + expected)
    operations.http(url, '/api/project/overview')
    return {'created': created, 'goals': goals, 'read_write': True}


def authentication_state(settings):
    return {'server': authentication(settings.server_data / 'definitions.db'),
            'resources': authentication(settings.state_dir / 'resources/definitions.db', require_users=False)}


def check_baselines(settings, operations, auth, root, label):
    source_hash = unchanged_source(operations.source, operations.source_frozen)
    actual = authentication_state(settings)
    if actual != auth:
        raise AssertionError('authentication rows changed at ' + label)
    result = {'stage': label, 'source_inventory_sha256': source_hash, 'auth': actual}
    write(root / ('invariants-' + label + '.json'), result)
    return result


def run(settings, operations, old, bundle, candidate, corrective, control, receipt):
    root = operations.root
    path = settings.server_data / 'definitions.db'
    receipt['scenarios'] = {}
    before = schema(path)
    auth = authentication_state(settings)
    old_configs = {name: archive.digest(workdir / 'opencoder.json')
                   for name, workdir in [('agent', settings.agent_workdir), ('server', settings.server_workdir)]}
    receipt.update(schema_before=before, auth_before=auth)
    project_write(operations, settings.public_url, 'before-maintenance', [])
    initial_journal = Journal(settings.state_dir).path.read_bytes()
    receipt['preflight'] = preflight.check(settings, candidate, operations)
    if Journal(settings.state_dir).path.read_bytes() != initial_journal:
        raise AssertionError('preflight changed release state')
    check_baselines(settings, operations, auth, root, 'preflight')

    first = {'passed': False, 'schema_before': before}
    receipt['scenarios']['pre_schema_crash_rollback'] = first
    first['controller'] = controller(root, bundle, control, operations, 'installing', 86)
    state = Journal(settings.state_dir).data['maintenance']
    if state['stage'] != 'installing' or state.get('schema_started') or state['writes_open']:
        raise AssertionError('crash did not preserve the pre-schema installing checkpoint')
    if schema(path) != before or operations.alive(old['server_unit']):
        raise AssertionError('old Server was not stopped with the original schema')
    first['snapshots'] = snapshots(state)
    first['gates'] = closed_gates(settings, operations)
    check_baselines(settings, operations, auth, root, 'installing-crash')
    result = flow.rollback(settings, operations, 120)
    if result['current'] != old['id'] or result['maintenance']['stage'] != 'rolled_back':
        raise AssertionError('rollback did not restore the old release')
    backup_unchanged = True
    try:
        if snapshots(result['maintenance']) != first['snapshots']:
            raise AssertionError('rollback changed the backup or configuration snapshots')
    except (ValueError, AssertionError) as error:
        backup_unchanged = False
        first['preservation_error'] = str(error)
        saved = Path(result['maintenance']['backup'])
        expected = json.loads((saved / 'manifest.json').read_text())['files']
        actual = archive.inventory(saved)
        actual.pop('manifest.json', None)
        actual.pop('manifest.sha256', None)
        first['backup_extra_files'] = sorted(set(actual) - set(expected))
        first['backup_changed_files'] = sorted(name for name in actual.keys() & expected.keys()
                                               if actual[name] != expected[name])
    if schema(path) != before:
        raise AssertionError('rollback changed the old schema')
    restored_configs = {name: archive.digest(workdir / 'opencoder.json')
                        for name, workdir in [('agent', settings.agent_workdir), ('server', settings.server_workdir)]}
    if restored_configs != old_configs:
        raise AssertionError('rollback did not restore old configuration bytes')
    command = operations.children[old['server_unit']].args
    if Path(command[0]) != settings.state_dir / 'releases' / old['id'] / 'bundle/bin/opencoder-server':
        raise AssertionError('rollback API is not served by the actual old Server')
    first['old_api'] = project_write(operations, settings.public_url, 'old-write-after-rollback', ['before-maintenance'])
    first.update(passed=backup_unchanged, schema_after=schema(path), old_configuration_restored=True,
                 source_and_auth=check_baselines(settings, operations, auth, root, 'rollback'),
                 backup_unchanged=backup_unchanged, server_binary=str(command[0]))
    write(root / 'scenario-rollback.json', first)

    second = {'passed': False, 'schema_before': before}
    receipt['scenarios']['post_schema_recovery_retry'] = second
    second['controller_failure'] = controller(root, bundle, control, operations, 'verification', 1)
    failed = Journal(settings.state_dir)
    state = failed.data['maintenance']
    after = schema(path)
    if state['stage'] != 'verifying' or state.get('schema_started') is not True or state['writes_open']:
        raise AssertionError('verification failure was not after closed-gate migration')
    if after != 33:
        raise AssertionError(f'actual old schema did not migrate: {before} -> {after}')
    if schema(Path(state['backup']) / 'data/server/definitions.db', immutable=True) != before:
        raise AssertionError('backup does not contain the honest original schema')
    frozen = snapshots(state)
    second.update(schema_after=after, snapshots=frozen, gates=closed_gates(settings, operations),
                  private_native_probe=native_evidence(failed.record(state['target'])))
    check_baselines(settings, operations, auth, root, 'verification-failure')
    restored = flow.rollback(settings, operations, 120)
    if schema(path) != before or snapshots(restored['maintenance']) != frozen:
        raise AssertionError('post-migration recovery lost old schema or changed the sealed backup')
    second['old_api_after_migration_recovery'] = project_write(operations, settings.public_url,
        'old-write-after-migration-recovery', ['before-maintenance', 'old-write-after-rollback'])
    second['post_migration_recovery'] = check_baselines(settings, operations, auth, root, 'post-migration-recovery')
    second['controller_failure_before_retry'] = controller(root, bundle, control, operations, 'verification', 1)
    failed = Journal(settings.state_dir)
    state = failed.data['maintenance']
    frozen = snapshots(state)
    second['controller_retry'] = controller(root, bundle, control, operations, 'public', 1)
    complete = Journal(settings.state_dir)
    current = complete.record(candidate['release_id'])
    if complete.data['maintenance']['stage'] != 'public' or complete.data['current'] != state['target']:
        raise AssertionError('retry did not reopen the same candidate before the injected public failure')
    if complete.data['maintenance']['writes_open'] is not True or snapshots(complete.data['maintenance']) != frozen:
        raise AssertionError('retry changed verified backup/configuration snapshots or kept gates closed')
    second['new_api'] = project_write(operations, settings.public_url, 'new-write-after-retry',
                                      ['before-maintenance', 'old-write-after-rollback', 'old-write-after-migration-recovery'])
    try:
        flow.rollback(settings, operations, 120)
    except ValueError as error:
        if 'writes may have reopened' not in str(error):
            raise
        second['post_open_restore_rejected'] = True
    else:
        raise AssertionError('old backup restore accepted after reopening')
    host = operations.http(settings.host_url, '/status')
    if host['snapshot']['ready'] is not True:
        raise AssertionError('public Host gate did not reopen to a ready node')
    registered = operations.http(settings.host_url, '/servers/' + current['id'], 'POST', {
        'id': current['id'], 'url': f"http://127.0.0.1:{current['server_port']}", 'enabled': True})
    second.update(passed=True, resumed_same_candidate=True, backup_and_configs_unchanged=True,
                  deliberate_public_failure_after_reopening=True,
                  public_native_probe=native_evidence(current, True), public_host_write=registered,
                  source_and_auth=check_baselines(settings, operations, auth, root, 'successful-retry'))
    write(root / 'scenario-retry.json', second)
    receipt.update(schema_after=schema(path), auth_after=authentication_state(settings), auth_rows_unchanged=True,
                   source_inventory_preserved=True)
    from cases.forward import run as corrective_release
    corrective_release(settings, operations, corrective, control, receipt)
