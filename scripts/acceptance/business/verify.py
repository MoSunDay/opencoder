"""Capture real execution evidence and check business, scheduling and isolation."""
from pathlib import Path
from datetime import datetime
import base64
import json
import os
import shutil
import time
from urllib.parse import urlencode
from common import BASE, TARGET, emit, http, read, run, sha, until, write
from lifecycle.evidence import copy_evidence


def read_only_at(mounts, path):
    covering = [m.split() for m in mounts if m.split()[4] == '/' or
                path == m.split()[4] or path.startswith(m.split()[4].rstrip('/') + '/')]
    selected = max(covering, key=lambda m: len(m[4]))
    return 'ro' in selected[5].split(',')


def capture_sandboxes(environment, seen):
    # PID 1 starts model units in separate namespaces: verify their actual mounts too.
    if getattr(environment, '_sandbox_scan', 0) + 10 > time.monotonic():
        return
    environment._sandbox_scan = time.monotonic()
    units = run(['systemctl', 'list-units', '--all', '--no-legend', '--plain', '--no-pager',
        'opencoder-runner-*', 'eval-diagnose-agent-*', 'jy-regression-*']).splitlines()
    owned = set(environment.units)
    details = {}
    for row in units:
        unit = row.split()[0]
        text = run(['systemctl', 'show', unit, '-p', 'BindsTo', '-p', 'MainPID'])
        details[unit] = dict(line.split('=', 1) for line in text.splitlines() if '=' in line)
    for _ in range(3):
        owned.update(unit for unit, info in details.items() if set(info['BindsTo'].split()) & owned)
    for proc in Path('/proc').iterdir():
        if not proc.name.isdigit():
            continue
        try:
            cgroup = (proc / 'cgroup').read_text()
            unit = next((u for u in owned if '/' + u in cgroup), None)
            if not unit or not unit.startswith(('eval-diagnose-agent-', 'jy-regression-')):
                continue
            name = (proc / 'comm').read_text().strip()
            model = 'codex' in name
            if not model and not (unit.startswith('jy-regression-') and name == 'go'):
                continue
            namespace = os.readlink(proc / 'ns/mnt')
            key = unit + namespace
            if key in seen:
                continue
            mounts = (proc / 'mountinfo').read_text().splitlines()
            matching = [m for m in mounts if '/root/workspace' in m]
            value = {'unit': unit, 'pid': int(proc.name), 'process': name, 'namespace': namespace,
                'workspaceMounts': matching, 'root': os.readlink(proc / 'root'), 'time': time.time(),
                'workspaceInode': (proc / 'root/root/workspace').stat().st_ino}
            if unit.startswith('eval-diagnose-agent-'):
                value['originalWorkspaceReadOnly'] = read_only_at(mounts, '/root/workspace')
                if not value['originalWorkspaceReadOnly']:
                    raise RuntimeError('Real evaluation model can write the original workspace')
            else:
                value['privateWorkspace'] = value['workspaceInode'] != Path('/root/workspace').stat().st_ino
                if not value['privateWorkspace']:
                    raise RuntimeError('Regression model sees the original workspace')
            role = 'model' if model else 'test'
            write(environment.root / 'evidence/isolation' / f'{role}-{proc.name}.json', value)
            seen.add(key)
        except (FileNotFoundError, ProcessLookupError):
            continue


def messages(environment, execution):
    cursor, chunks = {'seq': 0, 'offset': 0}, {}
    for _ in range(10000):
        page = environment.api(f'/api/executions/{execution}/messages?' + urlencode(cursor))
        for chunk in page['chunks']:
            seq = chunk['seq']
            current = chunks.setdefault(seq, bytearray())
            if len(current) != chunk['offset']:
                raise RuntimeError('Message paging lost or duplicated bytes')
            current.extend(base64.b64decode(chunk['bytes_b64']))
        if not page.get('more'):
            return [json.loads(v) for _, v in sorted(chunks.items())]
        next_cursor = page['next_cursor']
        if next_cursor == cursor:
            raise RuntimeError('Message cursor failed to advance')
        cursor = next_cursor
    raise RuntimeError('Unbounded message paging')


def collect(environment, jobs):
    for name, job in jobs.items():
        directory = environment.root / 'evidence' / name
        value = http(environment.business, environment.api_token, f'/api/v1/{name}/jobs/{job}')
        write(directory / 'business.json', value)
        execution = (value.get('execution') or {}).get('id')
        if execution:
            detail = environment.api('/api/executions/' + execution)
            write(directory / 'execution.json', detail)
            cursor, events = 0, []
            while True:
                page = environment.api(f'/api/executions/{execution}/events-page?after={cursor}')
                events.extend(page['events'])
                if not page.get('more'):
                    break
                next_cursor = page['events'][-1]['seq']
                if next_cursor <= cursor:
                    raise RuntimeError('Event paging failed to advance')
                cursor = next_cursor
            write(directory / 'events.json', events)
            write(directory / 'messages.json', messages(environment, execution))
            for runner in detail.get('runners') or []:
                for artifact in runner.get('artifacts') or []:
                    file = artifact['file']
                    if Path(file).is_absolute() or '..' in Path(file).parts:
                        raise RuntimeError('Unsafe remote artifact')
                    target = directory / 'artifacts' / file
                    target.parent.mkdir(parents=True, exist_ok=True)
                    content = environment.api(f'/api/executions/{execution}/artifact?' +
                        urlencode({'step': runner['step'], 'file': 'artifacts/' + file}), binary=True)
                    target.write_bytes(content)
                    if sha(target) != artifact['sha256']:
                        raise RuntimeError('Downloaded artifact checksum mismatch: ' + file)
        attempts = environment.runtime / 'jobs/opencoder/attempts' / job
        if attempts.exists():
            # Preserve checkpoint and failure evidence, excluding all credentials/private homes.
            for attempt in attempts.iterdir():
                destination = directory / 'attempts' / attempt.name
                destination.mkdir(parents=True, exist_ok=True)
                for file in ['checkpoint.json', 'publication.json']:
                    if (attempt / file).exists():
                        shutil.copy2(attempt / file, destination / file)
                evidence = attempt / 'evidence'
                if evidence.exists():
                    copy_evidence(evidence, destination / 'evidence')
        emit('evidence_collected', capability=name, executionId=execution)


def verify(environment, jobs):
    transitions = [json.loads(s) for s in (environment.root / 'evidence/transitions.jsonl').read_text().splitlines()]
    current = {name: read(environment.root / 'evidence' / name / 'business.json')['execution']['id'] for name in jobs}
    pending = [s for s in transitions if s['capability'] == 'regression-test' and s['execution'] == 'pending'
               and environment.api('/api/executions/' + s['executionId'])['request']['input']['job_id'] == jobs['regression-test']]
    assert pending, 'Second real execution never demonstrated pending admission'
    queue_execution = pending[-1]['executionId']
    queue_ids = {**current, 'regression-test': queue_execution}
    transitions = [s for s in transitions if s.get('executionId') == queue_ids.get(s['capability'])]
    starts = {name: next(s['time'] for s in transitions if s['capability'] == name and s['execution'] == 'running') for name in jobs}
    assert starts['eval-diagnose'] < starts['regression-test'], 'FIFO start order incorrect'
    done = next(s['time'] for s in transitions if s['capability'] == 'eval-diagnose' and s['execution'] == 'done')
    assert done <= starts['regression-test'] + 2, 'Concurrent execution exceeded the one-slot node'
    results = {}
    isolation = [read(file) for file in (environment.root / 'evidence/isolation').glob('model-*.json')]
    for name, job in jobs.items():
        directory = environment.root / 'evidence' / name
        business = read(directory / 'business.json')
        detail = read(directory / 'execution.json')
        assert detail['execution']['status'] == 'done', f'{name}: actual execution did not succeed'
        assert business.get('analysis', {}).get('status', business.get('status')) == 'done', f'{name}: business publication incomplete'
        runner = detail['runners'][0]
        assert runner['configuration']['profile_revision'] >= 1
        assert runner['configuration']['resources']['skills']['version'].startswith('v')
        pinned = environment.runtime / 'node-data/dag' / business['execution']['id'] / 'resources'
        hashes = read(environment.root / 'evidence/nfs-package-hashes.json')
        assert all((pinned / file).is_file() and sha(pinned / file) == digest
                   for file, digest in hashes.items()), 'Node pinned an incomplete resource package'
        transcript = read(directory / 'messages.json')
        text = json.dumps(transcript)
        assert 'tool_call' in text or 'tool_use' in text, f'{name}: no real Codex tool execution'
        if name == 'eval-diagnose':
            first = datetime.fromisoformat(business['createdAt'].replace('Z', '+00:00')).timestamp()
            last = datetime.fromisoformat(business['updatedAt'].replace('Z', '+00:00')).timestamp()
            proof = [p for p in isolation if p.get('originalWorkspaceReadOnly') and first <= p['time'] <= last]
        else:
            attempt = f"/{job}/attempt-{business['attempt']}/"
            proof = [p for p in isolation if p.get('privateWorkspace') and attempt in p['root']]
        assert proof, f'{name}: missing isolation evidence for this actual attempt'
        results[name] = {'jobId': job, 'executionId': business['execution']['id'],
            'status': 'done', 'artifactCount': len(runner['artifacts']), 'messageCount': len(transcript),
            'verifiedResourceFiles': len(hashes), 'modelIsolationProofs': len(proof),
            'configuration': runner['configuration']}
    regression = read(environment.root / 'evidence/regression-test/artifacts/report.json')
    expected = environment.prepared.get('requests', {}).get('regression-test', {'commit': TARGET, 'baseCommit': BASE})
    assert regression['manifest']['commit'] == expected['commit'] and regression['manifest']['baseCommit'] == expected['baseCommit']
    assert regression['executions'], 'No real regression tests executed'
    results['regression-test']['verdict'] = regression['verdict']
    results['regression-test']['testAttemptCount'] = len(regression['executions'])
    results['regression-test']['testCommandsSucceeded'] = sum(e['exitCode'] == 0 for e in regression['executions'])
    results['regression-test']['dependencyPreparationFailures'] = sum(
        e.get('error') == 'Dependency preparation failed' for e in regression['executions'])
    brief = read(environment.root / 'evidence/eval-diagnose/artifacts/brief.json')
    expected_case = read(environment.root / 'evidence/baseline/request.json')
    assert str(brief['request']['caseId']) == str(expected_case['caseId'])
    assert brief['request'].get('runId') == expected_case.get('runId')
    def delivered():
        file = environment.root / 'evidence/delivery.jsonl'
        return file.exists() and any(row['type'] == 'local-delivery' and row['jobId'] == jobs['eval-diagnose']
            for row in map(json.loads, file.read_text().splitlines()))
    until(delivered, 'local result delivery')
    from validation.quality import quality_results
    evaluation = read(environment.root / 'evidence/eval-diagnose/artifacts/result.json')
    quality = quality_results(evaluation, regression)
    expected = environment.prepared.get('expectedVerdicts', {})
    if expected:
        assert evaluation['health'] == expected['eval-diagnose'], 'Unexpected controlled evaluation result'
        assert regression['verdict'] == expected['regression-test'], 'Unexpected controlled regression result'
    return {'businessQuality': quality, 'workflows': results, 'fifoPendingVerified': True, 'queueExecutionId': queue_execution, 'fixedNode': environment.node,
            'outboundDelivery': 'local sink only', 'realCodex': True}
