#!/usr/bin/python3
"""Run both real business workflows against disposable fixed-node services."""
from pathlib import Path
import argparse
import json
import os
import sys
import time
import traceback
from audit import capture, compare
from common import BASE, HERE, TARGET, HTTPFailure, emit, http, read, run, write
from environment import Environment
from lifecycle.runtime import assert_owned, initialize_runtime, finalize_runtime
from lifecycle.process import stop_fixture


def submit(environment):
    request = read(environment.root / 'evidence/baseline/request.json')
    request['eventId'] = 'opencoder-e2e-case5-' + environment.root.name
    requests = {'eval-diagnose': request, 'regression-test': {
        'repo': 'repos/jianying-openagent-api', 'branch': 'master', 'commit': TARGET, 'baseCommit': BASE}}
    requests = environment.prepared.get('requests', requests)
    jobs = {}
    for name, payload in requests.items():
        reply = http(environment.business, environment.api_token, '/api/v1/' + name, 'POST', payload)
        jobs[name] = reply['jobId']
        write(environment.root / 'evidence' / name / 'submission.json', {'request': payload, 'response': reply})
        emit('submitted', capability=name, jobId=reply['jobId'])
    write(environment.root / 'evidence/jobs.json', jobs)
    return jobs


def monitor(environment, jobs, timeout=7500):
    previous, states = {}, {}
    deadline = time.monotonic() + timeout
    from verify import capture_sandboxes
    seen = set()
    while time.monotonic() < deadline:
        for name, job in jobs.items():
            value = http(environment.business, environment.api_token, f'/api/v1/{name}/jobs/{job}')
            states[name] = value
            write(environment.root / 'evidence' / name / 'business.json', value)
            execution = value.get('execution') or {}
            try:
                detail = environment.api('/api/executions/' + execution['id']) if execution.get('id') else {}
            except HTTPFailure as error:
                # The bridge durably records its ID before Node admission completes.
                if error.status != 404 or value.get('analysis', {}).get('status', value.get('status')) != 'queued':
                    raise
                detail = {}
            if detail:
                write(environment.root / 'evidence' / name / 'execution.json', detail)
            status = value.get('analysis', {}).get('status', value.get('status'))
            state = {'business': status, 'jobId': job, 'stage': value.get('analysis', {}).get('stage', value.get('stage')),
                'execution': detail.get('execution', {}).get('status'), 'executionId': execution.get('id')}
            if previous.get(name) != state:
                with (environment.root / 'evidence/transitions.jsonl').open('a') as stream:
                    stream.write(json.dumps({'time': time.time(), 'capability': name, **state}) + '\n')
                emit('transition', capability=name, **state)
                previous[name] = state
        capture_sandboxes(environment, seen)
        if all(s['business'] in ['done', 'failed'] for s in previous.values()):
            return states
        time.sleep(2)
    raise RuntimeError('Real business E2E exceeded its bounded deadline')


def cleanup(environment, result, destroy=False):
    root = environment.root
    assert_owned(root)
    prefix = 'oc-e2e-' + root.name + '-'
    if any(not unit.startswith(prefix) for unit in environment.units):
        raise RuntimeError('Cleanup contains a service outside this acceptance run')
    environment.stop()
    source = getattr(environment, 'fixture_source', None)
    if source:
        source.terminate()
        source.wait(timeout=10)
    stop_fixture(root)
    remaining = []
    for proc in Path('/proc').iterdir():
        if not proc.name.isdigit() or int(proc.name) == os.getpid():
            continue
        try:
            if any(str(root / 'runtime') in os.readlink(proc / field) for field in ['cwd', 'root']):
                remaining.append(int(proc.name))
        except (FileNotFoundError, ProcessLookupError, PermissionError):
            continue
    write(root / 'evidence/process-cleanup.json', {'remainingRuntimeProcesses': remaining})
    if remaining or str(root / 'runtime') in Path('/proc/self/mountinfo').read_text():
        raise RuntimeError('Temporary processes or mounts remain; refusing to remove the runtime')
    if (root / 'evidence/audit-before.json').exists():
        capture(root, 'after')
        result['sourceAudit'] = compare(root)
    result.update(finalize_runtime(root, destroy=destroy))
    write(root / 'evidence/result.json', result)


def prepare_environment(root, args):
    from validation.bundle import verify_platform
    manifest, binaries = verify_platform(args.platform_bundle)
    write(root / 'evidence/platform-manifest.json', manifest)
    if not (root / 'evidence/audit-before.json').exists():
        capture(root, 'before')
    source = None
    if args.scenario == 'positive':
        from scenarios.positive import prepare
        prepared, source = prepare(root, args.release.resolve())
    else:
        if not (root / 'evidence/audit-before.json').exists():
            capture(root, 'before')
        if not (root / 'prepared.json').exists():
            run(['/usr/bin/python3', HERE / 'scope.py', root, 'preparation',
                 '/usr/local/bin/python3', '-c',
                 'import sys; from pathlib import Path; sys.path.insert(0, sys.argv[1]); '
                 'from snapshots import prepare; prepare(Path(sys.argv[2]), Path(sys.argv[3]))',
                 HERE, root, args.release.resolve()], capture=False, timeout=7200)
        prepared = read(root / 'prepared.json')
    prepared['platformBinaries'] = binaries
    if not Path(prepared['workspace']).is_dir():
        raise RuntimeError('Acceptance workspace is missing; choose a new root')
    try:
        environment = Environment(root, prepared)
        environment.fixture_source = source
        return environment
    except BaseException:
        if source:
            source.terminate()
            source.wait(timeout=10)
        raise


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--root', type=Path, required=True)
    parser.add_argument('--release', type=Path)
    parser.add_argument('--cleanup', action='store_true', help='Clean a retained private run after preserving evidence')
    parser.add_argument('--destroy-runtime', action='store_true',
        help='Explicitly authorize removal of this run\'s owned temporary databases and copies after shutdown')
    parser.add_argument('--platform-bundle', type=Path)
    parser.add_argument('--scenario', choices=['positive', 'historical'], default='positive')
    parser.add_argument('--retain-on-failure', action='store_true',
        help='Keep this private environment for diagnosis; it must be explicitly cleaned afterward')
    parser.add_argument('--inside-private-namespace', action='store_true', help=argparse.SUPPRESS)
    args = parser.parse_args()
    root = args.root.resolve()
    if root.parent != Path('/root/.cache/opencoder-e2e') or not root.name.startswith('20'):
        raise ValueError('Acceptance root must be a dated private opencoder-e2e directory')
    if args.cleanup:
        if not (root / 'runtime').exists() and read(root / 'evidence/result.json').get('runtimeRemoved'):
            emit('private_runtime_already_removed', root=str(root))
            return
        cleanup(Environment.attach(root), read(root / 'evidence/result.json'), args.destroy_runtime)
        return
    if not args.release or not args.platform_bundle:
        parser.error('--release and --platform-bundle are required when starting a run')
    root.mkdir(parents=True, exist_ok=True, mode=0o700)
    (root / 'evidence').mkdir(exist_ok=True, mode=0o700)
    if not args.inside_private_namespace:
        os.execv('/usr/bin/python3', ['python3', str(HERE / 'scope.py'), str(root), 'acceptance',
            '/usr/bin/python3', str(HERE / 'main.py'), *sys.argv[1:], '--inside-private-namespace'])
    initialize_runtime(root)
    environment = None
    result = {'passed': False, 'platformPassed': False, 'businessQuality': {}}
    try:
        environment = prepare_environment(root, args)
        environment.launch()
        jobs = submit(environment)
        monitor(environment, jobs)
        from verify import collect, verify
        collect(environment, jobs)
        result = {**verify(environment, jobs), 'passed': False, 'platformPassed': True}
        run(['/usr/bin/python3', HERE / 'scope.py', root, 'browser',
             '/usr/bin/node', HERE / 'browser.mjs', root], timeout=240, env=environment.env)
        result['browser'] = read(root / 'evidence/browser.json')
        from validation.quality import acceptance_passed
        result['platformPassed'] = True
        result['passed'] = acceptance_passed(True, result['browser'].get('passed'), result['businessQuality'])
    except Exception as error:
        result['error'] = str(error)
        (root / 'evidence/failure.txt').write_text(traceback.format_exc())
        emit('acceptance_failed', error=str(error))
        # Preserve whatever the actual attempt produced, including infrastructure failures.
        if environment and environment.base and (root / 'evidence/jobs.json').exists():
            try:
                from verify import collect
                collect(environment, read(root / 'evidence/jobs.json'))
            except Exception as collection_error:
                result['collectionError'] = str(collection_error)
    finally:
        write(root / 'evidence/result.json', result)
        if not result['passed'] and args.retain_on_failure:
            emit('private_environment_retained_for_diagnosis', root=str(root))
            raise SystemExit(1)
        try:
            cleanup(environment or Environment.attach(root), result, args.destroy_runtime)
        except Exception as error:
            result.update({'passed': False, 'cleanupError': str(error)})
            write(root / 'evidence/result.json', result)
            raise
    emit('acceptance_complete', passed=result['passed'], evidence=str(root / 'evidence'))
    if not result['passed']:
        raise SystemExit(1)


if __name__ == '__main__':
    main()
