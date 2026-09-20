#!/usr/bin/env python3
"""Real model acceptance against a migrated platform and a compatible bundle.

Creates only uniquely named acceptance work. Retains all evidence and releases
its own wait markers on failure; never cancels tasks or removes database data.
"""
import argparse
import json
from pathlib import Path
import secrets
import shlex
import subprocess
import sys
import threading
import time
import traceback

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / 'platform'))
from rolling import config, manifest, probes
from rolling.io import Operations
from rolling.state import Journal, atomic_bytes, write
from fixture import HOLD_WASM, release_wasi_gate, todo_spec
from metrics import verify as verify_traffic
from streams import Stream
import transitions


class Live(Operations):
    def __init__(self, settings):
        super().__init__(settings.token_file)
        self.settings = settings
        state = Journal(settings.state_dir).data
        current = state['releases'][state['current']]
        inventory = self.http(f"http://127.0.0.1:{current['runtime_port']}", '/inventory')
        self.node_id = inventory['registration']['id']

    def http(self, base, path, method='GET', body=None, timeout=90):
        return super().http(base, path, method, body, timeout=timeout)

    def api(self, path, method='GET', body=None):
        if path == '/api/executions' and method == 'POST':
            # The process and journal assertions below refer to this host's
            # Runtime; other online nodes cannot own its acceptance work.
            body = {'node_id': self.node_id, **body}
        return self.http(self.settings.public_url, path, method, body)

    def submit_initial(self, request):
        # The old Server may still have the 15s Create deadline. Recover its
        # durable acceptance with the same frozen request before measuring
        # traffic; measured submissions below deliberately never retry.
        request = {'node_id': self.node_id, **request}
        probes.submit_probe(self, self.settings.public_url, request['id'], request, 120)
        receipt = self.api(f"/api/executions/{request['id']}/receipt")
        assert receipt['phase'] == 'accepted', receipt
        return receipt['receipt']['body']

    def completed(self, identifier):
        status = self.api('/api/executions/' + identifier)['execution']['status']
        if status in ('error', 'interrupted', 'cancelled'):
            raise ValueError(f'acceptance execution {identifier}: {status}')
        return status == 'done'


def pid(unit):
    return int(subprocess.check_output(['systemctl', 'show', unit, '-p', 'MainPID', '--value']))


def process_identity(identifier):
    return (identifier, Path(f'/proc/{identifier}/stat').read_text().split()[21])


def chain(root):
    scripts = {
        'first': '\n'.join([
            'import os, pathlib, time',
            f'root = pathlib.Path({str(root)!r})',
            '(root / "model-shell.pid").write_text(str(os.getpid()))',
            'while not (root / "release").exists(): time.sleep(.1)',
            '(root / "first.done").write_text("first dependency completed\\n")',
            'print((root / "first.done").read_text())',
        ]),
        'second': '\n'.join([
            'import pathlib',
            f'root = pathlib.Path({str(root)!r})',
            'assert (root / "first.done").read_text() == "first dependency completed\\n"',
            '(root / "second.done").write_text("second consumed first dependency\\n")',
            'print((root / "second.done").read_text())',
        ]),
    }
    spec = todo_spec()
    spec['id'] = root.name
    spec['name'] = '平滑发布真实模型依赖链'
    spec['objective'] = '执行给定的两个验收脚本，确认依赖顺序与跨发布执行连续性。只操作指定验收目录，不修改项目文件。'
    for todo in spec['todos']:
        path = root / (todo['id'] + '.py')
        atomic_bytes(path, scripts[todo['id']].encode(), 0o644)
        command = 'python3 ' + shlex.quote(str(path))
        todo['instructions'] = (
            '必须使用 bash 工具执行下面的命令，并在脚本自然返回后才提交候选结果。'
            '脚本可能等待发布程序自动释放信号，不需要用户操作，不得自行创建 release 文件。'
            '若工具转为后台运行，请等待并检查脚本完成，不得提前声称完成。'
            '不得修改脚本或项目文件。命令：' + command)
        todo['acceptance'] = {
            'criteria': f'脚本已真实执行并自然返回，{root / (todo["id"] + ".done")} 存在且包含脚本输出；结果附真实工具证据。',
            'required_tool_calls': [{'name': 'bash', 'arguments_contains': {}, 'result_ok': True}],
        }
    return spec


def verify_stream(env, stream, identifier):
    env.wait(lambda: stream.finished.is_set() or stream.errors, 90)
    if stream.errors:
        raise AssertionError(stream.errors)
    expected, cursor = [], 0
    while True:
        rows = env.api(f'/api/executions/{identifier}/events-page?after={cursor}')['events']
        if not rows:
            break
        expected.extend(row['seq'] for row in rows)
        cursor = expected[-1]
    assert stream.ids == expected, 'SSE differs from durable events'
    assert stream.resume_delays and max(stream.resume_delays) < 5, 'SSE did not resume within five seconds'


def observe(env, root, tag, seconds):
    deadline = time.monotonic() + seconds
    samples = []
    while time.monotonic() < deadline:
        identifier = f'dag-{tag}-observe-{len(samples)}'
        began = time.monotonic()
        env.api('/api/executions', 'POST', {'id': identifier, 'kind': 'dag', 'input': {'definition': probes.spec()}})
        env.wait(lambda: env.completed(identifier), 30)
        ready = env.api('/api/ready')
        assert ready['mode'] == 'open' and ready['ready_nodes'] >= 1, 'scheduling became unavailable'
        probes.resources(env.settings, env)
        samples.append({'id': identifier, 'elapsed_seconds': time.monotonic() - began, 'at': time.time()})
        write(root / 'observation.json', samples)
        if len(samples) % 12 == 0:
            print(json.dumps({'observation_samples': len(samples), 'remaining_seconds': max(0, round(deadline - time.monotonic()))}), flush=True)
        time.sleep(min(5, max(0, deadline - time.monotonic())))
    return samples


def exercise(args, settings, root):
    env = Live(settings)
    previous = Journal(settings.state_dir).data
    assert previous['current'] and previous['phase'] in ('complete', 'rolled_back'), 'first migration must already be complete'
    candidate = manifest.verify(args.bundle)
    assert candidate['release_id'] != previous['current'], 'acceptance requires another release'
    old = previous['releases'][previous['current']]
    tag = root.name
    todo_id, dag_id = f'todos-{tag}', f'dag-{tag}-hold'
    module = tag + '.wasm'
    old_data = Path(old['runtime_data'])
    atomic_bytes(old_data / 'dag/_modules' / module, HOLD_WASM.encode(), 0o444)
    before = process_identity(pid(old['runtime_unit']))
    resources_before = process_identity(pid('opencoder-resources.service'))
    todo = {'id': todo_id, 'kind': 'todos', 'input': {'spec': chain(root)}}
    traffic, failures = [], []
    stop = threading.Event()
    thread = None
    stream = None
    wasm_submitted = False
    try:
        receipt = env.submit_initial(todo)
        env.wait(lambda: (root / 'model-shell.pid').exists(), 240)
        model_shell = process_identity(int((root / 'model-shell.pid').read_text()))
        assert not (root / 'first.done').exists(), 'model bypassed the release gate'
        wasm_submitted = True
        env.submit_initial({'id': dag_id, 'kind': 'dag', 'input': {'definition': {
            'name': '跨发布真实 WASI 工具', 'steps': [{'name': 'hold', 'timeout_secs': 1800,
                'kind': {'type': 'wasm', 'command': module}}]}}})
        env.wait(lambda: env.api('/api/executions/' + dag_id)['dag_steps']['running'] == 1, 90)
        stream = Stream(env, dag_id)
        env.wait(lambda: bool(stream.ids), 30)

        def submit():
            while not stop.is_set():
                identifier = f'dag-{tag}-traffic-{len(traffic)}'
                began = time.monotonic()
                try:
                    env.api('/api/executions', 'POST', {'id': identifier, 'kind': 'dag', 'input': {'definition': probes.spec()}})
                    traffic.append({'id': identifier, 'seconds': time.monotonic() - began, 'at': began})
                except Exception as error:
                    failures.append(str(error))
                    return
                stop.wait(.1)

        thread = threading.Thread(target=submit, daemon=True)
        thread.start()
        # The reviewed bundle is built before starting long tasks. This is the
        # same public deployment command an operator uses for future releases.
        def continuity_check():
            assert process_identity(pid(old['runtime_unit'])) == before, 'old Runtime changed process'
            assert process_identity(model_shell[0]) == model_shell, 'real model shell changed process'
            assert process_identity(pid('opencoder-resources.service')) == resources_before, 'NFS service restarted'
            assert env.api('/api/executions/' + dag_id)['dag_steps']['running'] == 1
            probes.resources(settings, env)
        transitions.execute(args, root, env, continuity_check)
        current = Journal(settings.state_dir).data
        assert current['current'] == candidate['release_id'] and current['phase'] == 'complete'
        assert process_identity(pid(old['runtime_unit'])) == before, 'old Runtime changed process'
        assert process_identity(model_shell[0]) == model_shell, 'real model shell changed process'
        assert process_identity(pid('opencoder-resources.service')) == resources_before, 'NFS service restarted'
        assert env.api('/api/executions', 'POST', todo) == receipt, 'TODO receipt changed'
        latest = f'dag-{tag}-latest'
        env.api('/api/executions', 'POST', {'id': latest, 'kind': 'dag', 'input': {'definition': probes.spec()}})
        env.wait(lambda: env.completed(latest), 30)
        new_data = Path(current['releases'][current['current']]['runtime_data'])
        assert (new_data / 'dag' / latest / 'execution.json').is_file(), 'new task missed the new Runtime'
        assert env.api('/api/executions/' + dag_id)['dag_steps']['running'] == 1
        probes.resources(settings, env)
    finally:
        stop.set()
        if thread:
            thread.join(35)
        write(root / 'traffic.json', {'requests': traffic, 'failures': failures})
        (root / 'release').touch()
        # Release only this test's WASI gate, regardless of deployment outcome.
        if wasm_submitted:
            release_wasi_gate(old_data, dag_id)
    assert not failures, failures
    env.wait(lambda: env.completed(todo_id), 300)
    env.wait(lambda: env.completed(dag_id), 90)
    assert (root / 'first.done').is_file() and (root / 'second.done').is_file()
    for row in traffic:
        env.wait(lambda: env.completed(row['id']), 30)
    current = Journal(settings.state_dir).data
    continuity = verify_traffic([record['runtime_data'] for record in current['releases'].values()], traffic)
    write(root / 'scheduling.json', continuity)
    verify_stream(env, stream, dag_id)
    samples = observe(env, root, tag, args.observe_seconds)
    result = {'result': 'PASS', 'previous': old['id'], 'current': current['current'],
        'signal': args.signal, 'signal_roundtrip': args.signal_roundtrip,
        'todo': todo_id, 'dag': dag_id, 'continuity': continuity['metrics'],
        'runtime_process': before, 'model_shell_process': model_shell,
        'sse_resume_seconds': stream.resume_delays, 'sse_ids': stream.ids,
        'observation_seconds': args.observe_seconds, 'observation_samples': len(samples)}
    write(root / 'result.json', result)
    print(json.dumps(result), flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--config', type=Path, required=True)
    parser.add_argument('--bundle', type=Path, required=True)
    parser.add_argument('--evidence-parent', type=Path, default=Path('/var/tmp'))
    parser.add_argument('--observe-seconds', type=int, default=900)
    parser.add_argument('--signal', action='store_true', help='publish through the running Server signal protocol')
    parser.add_argument('--signal-roundtrip', action='store_true', help='also roll back and republish while old and new work remain running')
    args = parser.parse_args()
    if args.observe_seconds < 900:
        parser.error('final acceptance requires at least 900 seconds of observation')
    if args.signal_roundtrip and not args.signal:
        parser.error('--signal-roundtrip requires --signal and two signal-capable releases')
    settings = config.load(args.config)
    root = args.evidence_parent / ('release-live-' + secrets.token_hex(8))
    root.mkdir(parents=True)
    print(json.dumps({'evidence': str(root)}), flush=True)
    try:
        exercise(args, settings, root)
    except BaseException:
        atomic_bytes(root / 'failure.txt', traceback.format_exc().encode())
        raise


if __name__ == '__main__':
    main()
