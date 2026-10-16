#!/usr/bin/env python3
"""Private old/bridge/writer processes: lost replies, contexts and rollback.

Requires root, Nginx, runc, three immutable release bundles and a native rootfs.
Only fixture processes, databases and uniquely named Runtime units are changed.
"""
import argparse
import importlib.util
import hashlib
import http.client
import json
import shutil
from pathlib import Path
import sqlite3
import subprocess
import sys
import tempfile
import threading
import time
import traceback
import urllib.error

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'smooth_release'))
import environment
from environment import until
from model import Model, plan
from transport import ProxiedEnvironment
from fixture import release_native_gate
from rolling.native import publish_binary
from rolling.manifest import compatible, writer_preflight


def select(env, bundle):
    env.bin = bundle / 'bin'
    env.info = json.loads(subprocess.check_output([env.bin / 'opencoder-agent', '--build-info']))
    env.containers.binaries = env.bin


def record(env, name, bundle):
    select(env, bundle)
    value = env.warm(name)
    value['manifest'] = json.loads((bundle / 'manifest.json').read_text())
    inventory = env.http(f"http://127.0.0.1:{value['runtime_port']}", '/inventory')
    assert inventory['build'] == env.info
    assert inventory['build']['git_commit'] == value['manifest']['commit']
    value['runtime_build'] = inventory['build']
    return value


def snapshot(env, runtime, identifier):
    reply = env.http(f"http://127.0.0.1:{runtime['runtime_port']}", '/rpc', 'POST', {
        'operation': 'brain', 'execution': {'id': identifier, 'kind': 'brain'},
        'action': 'snapshot', 'input': {}})
    assert reply['status'] == 200, reply
    return reply['body']


def claim(env, identifier):
    with sqlite3.connect(f'file:{env.settings.server_data}/control.db?mode=ro', uri=True) as conn:
        return conn.execute("SELECT fingerprint,phase FROM dispatch_receipts WHERE scope='execution' AND id=?",
                            (identifier,)).fetchone()


def lost_reply(env, runtime, identifier, server_label):
    """Drop the real acceptance reply before Host can forward it to Control."""
    result = {}
    def accepted(assignment):
        env.children[server_label].kill()
        path = Path(runtime['runtime_data']) / 'agent' / assignment['index']['id'] / 'execution.json'
        saved = json.loads(path.read_text())
        result.update(path=str(path), assignment=saved['assignment'], fault='accepted Create reply dropped')
    env.proxies[runtime['id']].arm(identifier, accepted)
    return result


def interrupted(env, runtime, identifier, evidence):
    until(lambda: evidence, 'Worker accepted the child before the lost reply')
    child = evidence['assignment']['index']['id']
    before = snapshot(env, runtime, identifier)
    operation = next(op for op in before['operations'] if op['execution_id'] == child)
    assert operation['status'] == 'creating', before
    receipt = claim(env, child)
    assert receipt and receipt[1] == 'prepared', receipt
    evidence.update(execution_id=child, claim_before=receipt, root_before=before,
                    owner_before=env.owner(child))
    return child


def completed(env, identifier):
    value = env.api('/api/brain/runs/' + identifier + '/layered')
    phase = value['run']['phase']
    assert phase not in ('failed', 'blocked', 'cancelled'), value
    return value if phase == 'completed' else None


def submit_root(env, identifier, definition, lost):
    try:
        env.api('/api/brain/runs', 'POST', {'id': identifier, 'schema_version': 7, 'plan': definition})
    except urllib.error.HTTPError:
        raise
    except (urllib.error.URLError, http.client.RemoteDisconnected, ConnectionResetError):
        # The same intentional crash may also lose the parent's HTTP reply.
        # Acceptance is proved by its real child's journal, without a new ID.
        until(lambda: lost, 'child proves the same root was accepted')


def unchanged_child(env, evidence):
    child = evidence['execution_id']
    assert env.owner(child) == evidence['owner_before']
    assert claim(env, child) == (evidence['claim_before'][0], 'accepted')
    saved = json.loads(Path(evidence['path']).read_text())
    assert saved['assignment']['request'] == evidence['assignment']['request']


def publish_artifact(env):
    content = b'cross-version artifact\n'
    declaration = {'files': [{'path': 'report.txt', 'bytes': len(content),
                             'sha256': hashlib.sha256(content).hexdigest()}]}
    source = '#include <stdio.h>\nint main(void){FILE*f;'
    for name, data in [('report.txt', content.decode()), ('artifacts.json', json.dumps(declaration)),
                       ('output.json', json.dumps({'passed': True}))]:
        source += f'f=fopen({json.dumps(name)},"w");if(!f)return 1;fputs({json.dumps(data)},f);fclose(f);'
    source += 'return 0;}\n'
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        (root / 'fixture.c').write_text(source)
        subprocess.run(['cc', '-static', '-O2', '-s', str(root / 'fixture.c'), '-o', str(root / 'fixture')], check=True)
        resource = publish_binary(env.settings.resource_url, 'upgrade-io', (root / 'fixture').read_bytes(), env)
    env.api('/api/dag/defs', 'POST', {'spec': {'name': 'upgrade-io', 'steps': [
        {'name': 'write', 'kind': {'type': 'binary', 'resource': resource}}]}})


def hold_runtime(env, runtime):
    # Keep a real native process active across the handoff. Idle Runtime
    # hibernation is a separate protocol, outside this lost-reply fixture.
    identifier = 'dag-upgrade-hold-' + runtime['id']
    env.api('/api/executions', 'POST', {'id': identifier, 'kind': 'dag', 'input': {
        'definition': {'name': 'hold', 'steps': [{'name': 'hold', 'timeout_secs': 900,
            'kind': {'type': 'binary', 'resource': env.hold_resource}}]}}})
    state = until(lambda: (lambda s: s if s['status'] == 'running' else None)(
        env.containers.state(runtime['runtime_data'], identifier)), 'live native handoff workload')
    env.holds.append((runtime, identifier, state))


def exercise(env, old, bridge, writer, legacy_controller):
    first = record(env, 'legacy', old)
    env.switch(first)
    hold_runtime(env, first)
    old_pid = env.runtime_pid(first)
    legacy = lost_reply(env, first, 'brain-upgrade-legacy', 'legacy-server')
    submit_root(env, 'brain-upgrade-legacy', plan(), legacy)
    interrupted(env, first, 'brain-upgrade-legacy', legacy)
    second = record(env, 'bridge', bridge)
    env.switch(second)
    hold_runtime(env, second)
    env.model.legacy_release.set()
    old_done = until(lambda: completed(env, 'brain-upgrade-legacy'), 'legacy root completes on old Runtime', 120)
    unchanged_child(env, legacy)
    assert env.runtime_pid(first) == old_pid
    assert env.owner('brain-upgrade-legacy') == 'legacy'
    legacy_contexts = [c for c in env.model.contexts if c['plan']['title'] == 'legacy transition']
    assert len(legacy_contexts) == 3 and all('execution_artifacts' not in c for c in legacy_contexts)
    try:
        env.api('/api/executions', 'POST', {'id': 'brain-forbidden-writer', 'kind': 'brain',
            'input': {'schema_version': 7, 'execution_io_version': 1, 'layered_request': {}}})
    except urllib.error.HTTPError as error:
        assert error.code == 409
    else:
        raise AssertionError('reader bridge admitted a fresh v1 root')
    assert claim(env, 'brain-forbidden-writer') is None
    definition = plan()
    definition['title'] = 'bridge transition'
    env.api('/api/brain/runs', 'POST', {'id': 'brain-bridge-default', 'schema_version': 7,
                                     'plan': definition})
    until(lambda: completed(env, 'brain-bridge-default'), 'fresh bridge root stays legacy', 120)
    saved = json.loads((Path(second['runtime_data']) / 'brain/brain-bridge-default/execution.json').read_text())
    assert 'execution_io_version' not in saved['assignment']['request']['input']
    assert all('execution_artifacts' not in c for c in env.model.contexts
               if c['plan']['title'] == 'bridge transition')
    third = record(env, 'writer', writer)
    env.switch(third)
    # Match the real release job: the previous Control stops consuming the
    # shared outbox before a new fault is armed. Runtimes remain untouched.
    env.retire(second)
    hold_runtime(env, third)
    publish_artifact(env)
    env.api('/api/brain/runs', 'POST', {'id': 'brain-upgrade-dispatch', 'schema_version': 7,
                                     'plan': plan(True)})
    assert env.model.decision_entered.wait(120), 'second root did not reach its held decision'
    modern = lost_reply(env, third, 'brain-upgrade-artifacts', 'writer-server')
    submit_root(env, 'brain-upgrade-artifacts', plan(True), modern)
    interrupted(env, third, 'brain-upgrade-artifacts', modern)
    root = Path(third['runtime_data']) / 'brain/brain-upgrade-artifacts/execution.json'
    intent = json.loads(root.read_text())['annotations']['layered_intent']
    assert intent['assignments'][0]['inputs']['files']['source'] == 'artifacts'
    writer_pid = env.runtime_pid(third)
    select(env, bridge)
    env.reopen(second)
    env.switch(second)
    env.model.consumer_release.set()
    env.model.decision_release.set()
    modern_done = until(lambda: completed(env, 'brain-upgrade-artifacts'), 'v1 root completes through bridge rollback', 120)
    dispatched = until(lambda: completed(env, 'brain-upgrade-dispatch'), 'bridge resolves and dispatches a fresh v1 binding', 120)
    unchanged_child(env, modern)
    assert env.runtime_pid(third) == writer_pid
    assert env.owner('brain-upgrade-artifacts') == 'writer'
    expected_sources = {modern_done['operations'][0]['execution_id'],
                        dispatched['operations'][0]['execution_id']}
    assert {ref['execution']['id'] for ref in env.model.downloads} == expected_sources, env.model.downloads
    assert env.owner('brain-upgrade-dispatch') == 'writer'
    assert env.owner(dispatched['operations'][-1]['execution_id']) == 'bridge'
    retained = [r['manifest'] for r in env.records]
    compatible(second['manifest'], retained)
    try:
        compatible(first['manifest'], retained)
    except ValueError as error:
        assert 'cannot read execution IO' in str(error)
    else:
        raise AssertionError('unsafe legacy rollback was accepted')
    writer_preflight(third['manifest'], second['manifest'])
    for runtime, identifier, before in env.holds:
        after = env.containers.state(runtime['runtime_data'], identifier)
        assert after['status'] == 'running'
        assert (after['pid'], after['process_start']) == (before['pid'], before['process_start'])
    spec = importlib.util.spec_from_file_location('legacy_release_manifest', legacy_controller)
    legacy_manifest = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(legacy_manifest)
    try:
        legacy_manifest.compatible(first['manifest'], retained)
    except ValueError as error:
        assert 'different data_format' in str(error)
    else:
        raise AssertionError('the actual legacy controller allowed unsafe rollback')
    return {'legacy': legacy, 'modern': modern, 'legacy_done': old_done,
            'modern_done': modern_done, 'fresh_dispatch_done': dispatched,
            'artifact_downloads': env.model.downloads,
            'builds': {r['id']: r['manifest'] for r in env.records},
            'runtime_builds': {r['id']: r['runtime_build'] for r in env.records}, 'result': 'PASS'}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('old', 'bridge', 'writer', 'rootfs'):
        parser.add_argument('--' + name, type=Path, required=True)
    parser.add_argument('--data-parent', type=Path)
    parser.add_argument('--legacy-controller', type=Path, required=True,
                        help='immutable legacy source scripts/platform/rolling/manifest.py')
    parser.add_argument('--nginx', default=shutil.which('nginx'))
    args = parser.parse_args()
    if not args.nginx:
        parser.error('Nginx is required; pass its actual executable with --nginx')
    environment.Model = Model
    env = ProxiedEnvironment(args.old / 'bin', args.nginx, args.rootfs, args.data_parent)
    env.holds = []
    env.model.environment = env
    try:
        result = exercise(env, args.old, args.bridge, args.writer, args.legacy_controller)
        (env.root / 'execution-io-upgrade.json').write_text(json.dumps(result, indent=2))
        print(json.dumps({'result': 'PASS', 'evidence': str(env.root)}), flush=True)
    except BaseException:
        traceback.print_exc()
        env.diagnose()
        raise
    finally:
        env.model.legacy_release.set()
        env.model.consumer_release.set()
        env.model.decision_release.set()
        for runtime, identifier, _ in env.holds:
            release_native_gate(Path(runtime['runtime_data']), identifier)
        if (env.root / 'execution-io-upgrade.json').exists():
            until(lambda: all(not env.runtime_pid(r) or
                env.http(f"http://127.0.0.1:{r['runtime_port']}", '/inventory')['can_hibernate']
                for r in env.records), 'all fixture work and deliveries settled', 60)
        env.close()


if __name__ == '__main__':
    main()
