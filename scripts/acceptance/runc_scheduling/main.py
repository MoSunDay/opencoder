#!/usr/bin/python3
"""Verify portable runc placement and fixed-node queuing using private services."""
from pathlib import Path
import argparse
import base64
import json
import os
import re
import secrets
import shutil
import sys
import traceback

BUSINESS = Path(__file__).resolve().parents[1] / 'business'
sys.path.insert(0, str(BUSINESS))
from common import emit, read, sha, until, write
from environment import Environment
from lifecycle.runtime import initialize_runtime, finalize_runtime
from validation.bundle import verify_platform


def stage(directory):
    library = directory / 'dag/_modules'
    library.mkdir(parents=True)
    (library / 'stdout.wasm').write_text('''(module
      (import "wasi_snapshot_preview1" "fd_write" (func $write (param i32 i32 i32 i32) (result i32)))
      (memory (export "memory") 1) (data (i32.const 0) "portable runc")
      (func (export "_start") (i32.store (i32.const 1024) (i32.const 0))
        (i32.store (i32.const 1028) (i32.const 13))
        (drop (call $write (i32.const 1) (i32.const 1024) (i32.const 1) (i32.const 1032)))))''')
    (library / 'spin.wasm').write_text('(module (func (export "_start") (loop $l (br $l))))')


def launch(environment, binaries, rootfs):
    runtime = environment.runtime
    config = {'model': 'acceptance/unused', 'providers': {'acceptance': {
        'base_url': 'http://127.0.0.1:1/v1', 'api_key': 'unused-local-only', 'model': 'unused'}}}
    server = runtime / 'server'
    write(server / '.opencoder/config.json', config)
    environment.start('server', [binaries['opencoder-server'], '--host', '127.0.0.1', '--port', '0',
        '--workdir', server, '--data-dir', runtime / 'server-data',
        '--token-file', runtime / 'control-token'], server)

    def listening():
        log = environment.root / 'evidence/server.log'
        match = re.search(r'listening on (http://\S+)', log.read_text() if log.exists() else '')
        return match[1] if match else None

    environment.base = until(listening, 'runc test Server')
    for name in ['node-a', 'node-b']:
        workdir = runtime / name
        write(workdir / '.opencoder/config.json', config)
        stage(workdir / 'state')
        shutil.copytree(rootfs, workdir / 'state/dag/rootfs')
        environment.start(name, [binaries['opencoder-agent'], '--remote', environment.base,
            '--name', name, '--workdir', workdir, '--data-dir', workdir / 'state', '--max-runs', '1',
            '--token-file', runtime / 'control-token'], workdir)

    def nodes():
        ready = [n for n in environment.api('/api/nodes')['nodes']
                 if n.get('online') and n.get('snapshot', {}).get('ready')]
        return ready if len(ready) == 2 else None

    return {n['name']: n['id'] for n in until(nodes, 'two ready runc nodes')}


def submit(environment, identifier, module, node=None):
    body = {'id': identifier, 'kind': 'dag', 'input': {'definition': {
        'name': identifier, 'steps': [{'name': 'wasm', 'timeout_secs': 120,
            'kind': {'type': 'wasm', 'command': module, 'sandbox': 'runc'}}]}}}
    if node:
        body['node_id'] = node
    return environment.api('/api/executions', 'POST', body)


def detail(environment, identifier):
    return environment.api('/api/executions/' + identifier)


def finished(environment, identifier):
    value = detail(environment, identifier)
    return value if value['execution']['status'] in ['done', 'error', 'cancelled'] else None


def container_ready(environment, identifier, name):
    directory = environment.runtime / name / 'state/dag/bundles' / identifier / 'wasm'
    config = directory / 'config.json'
    states = list((directory / 'runc-state').glob('*/state.json'))
    if not config.exists() or not states:
        value = detail(environment, identifier)
        if value['execution']['status'] in ['done', 'error', 'cancelled']:
            raise RuntimeError('runc spin ended before its container was observed: ' + json.dumps(value))
        return None
    value = read(config)
    assert value['root']['readonly'] is True
    assert 'mount' in [n['type'] for n in value['linux']['namespaces']]
    write(environment.root / 'evidence' / (identifier + '-oci.json'), value)
    write(environment.root / 'evidence' / (identifier + '-state.json'), read(states[0]))
    return True


def exercise(environment, nodes):
    receipts = []
    for busy_name, free_name in [('node-a', 'node-b'), ('node-b', 'node-a')]:
        hold, portable, pinned = ['dag-' + prefix + '-' + busy_name
                                 for prefix in ['hold', 'portable', 'fixed']]
        submit(environment, hold, 'spin.wasm', nodes[busy_name])
        until(lambda: container_ready(environment, hold, busy_name), 'real runc container')
        submit(environment, portable, 'stdout.wasm')
        value = until(lambda: finished(environment, portable), 'portable runc result')
        assert value['execution']['status'] == 'done', value
        assert value['execution']['node_id'] == nodes[free_name], value
        write(environment.root / 'evidence' / (portable + '.json'), value)
        artifact = environment.api('/api/executions/' + portable + '/commands', 'POST', {
            'action': 'artifact', 'input': {'step': 'wasm', 'file': 'output.txt'}})
        write(environment.root / 'evidence' / (portable + '-artifact.json'), artifact)
        assert base64.b64decode(artifact['bytes_b64']) == b'portable runc', artifact
        assert artifact['eof'] is True
        submit(environment, pinned, 'stdout.wasm', nodes[busy_name])
        queued = detail(environment, pinned)
        assert queued['execution']['status'] == 'pending', queued
        assert queued['execution']['node_id'] == nodes[busy_name], queued
        write(environment.root / 'evidence' / (pinned + '-pending.json'), queued)
        environment.api('/api/executions/' + hold + '/commands', 'POST', {'action': 'cancel'})
        until(lambda: finished(environment, hold), 'cancelled runc container')
        value = until(lambda: finished(environment, pinned), 'fixed node queue drain')
        assert value['execution']['status'] == 'done', value
        assert value['execution']['node_id'] == nodes[busy_name], value
        write(environment.root / 'evidence' / (pinned + '-done.json'), value)
        receipts.append({'busyNode': nodes[busy_name], 'portableNode': nodes[free_name],
                         'portableExecution': portable, 'fixedExecution': pinned})
    return receipts


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--root', required=True, type=Path)
    parser.add_argument('--rootfs', required=True, type=Path)
    parser.add_argument('--platform-bundle', required=True, type=Path)
    parser.add_argument('--destroy-runtime', action='store_true')
    parser.add_argument('--inside', action='store_true', help=argparse.SUPPRESS)
    args = parser.parse_args()
    root = args.root.resolve()
    if root.parent != Path('/root/.cache/opencoder-e2e') or not root.name.startswith('20'):
        raise ValueError('Use a dated private opencoder-e2e root')
    (root / 'evidence').mkdir(parents=True, exist_ok=True)
    if not args.inside:
        os.execv('/usr/bin/python3', ['python3', str(BUSINESS / 'scope.py'), str(root), 'runc-scheduling',
            '/usr/bin/python3', str(Path(__file__).resolve()), *sys.argv[1:], '--inside'])
    manifest, binaries = verify_platform(args.platform_bundle)
    write(root / 'evidence/platform-manifest.json', manifest)
    initialize_runtime(root)
    environment = Environment.attach(root)
    for directory in environment.env.values():
        Path(directory).mkdir(parents=True, exist_ok=True)
    environment.token = secrets.token_hex(32)
    (root / 'runtime/control-token').write_text(environment.token)
    (root / 'runtime/control-token').chmod(0o600)
    result = {'passed': False, 'runtimeSha256': sha(args.rootfs / 'usr/bin/wasmtime')}
    try:
        nodes = launch(environment, binaries, args.rootfs)
        result['nodes'] = nodes
        result['placements'] = exercise(environment, nodes)
        result['passed'] = True
    except Exception as error:
        result['error'] = str(error)
        (root / 'evidence/failure.txt').write_text(traceback.format_exc())
        raise
    finally:
        write(root / 'evidence/result.json', result)
        try:
            environment.stop()
            result.update(finalize_runtime(root, destroy=args.destroy_runtime))
        except Exception as error:
            result.update({'passed': False, 'cleanupError': str(error)})
            raise
        finally:
            write(root / 'evidence/result.json', result)
    emit('runc_scheduling_complete', **result)


if __name__ == '__main__':
    main()
