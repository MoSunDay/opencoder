#!/usr/bin/env python3
"""Compare two isolated Runtime binaries using synthetic execution history.

No production configuration or data is read. RSS measurements never invoke
allocator trimming or restart a process between rounds.
"""
import argparse
import concurrent.futures
import json
import hashlib
import os
from pathlib import Path
import socket
import statistics
import subprocess
import threading
import time
import urllib.request
from workload import Model, complete, verify_completed

MIB = 1024 * 1024
TOKEN = 'memory-acceptance-fixture'


def fixture(root, mib, extra_config=None):
    data = root / 'data'
    work = root / 'work'
    home = root / 'home'
    agents = root / 'agents'
    for path in [data, work / '.opencoder', home / '.opencoder', agents]:
        path.mkdir(parents=True, exist_ok=True)
    config = {'agent': {'agents_dir': str(agents)}, 'autopilot': {'mode': 'off'}}
    config.update(extra_config or {})
    for path in [work, home]:
        (path / '.opencoder/config.json').write_text(json.dumps(config))
        (path / '.opencoder/ap.json').write_text('{"mode":"off"}')
    (data / 'node-id').write_text('node-memory-fixture')
    payload = 'x' * (128 * 1024)
    count = mib * 4
    total = 0
    for index in range(count):
        execution = f'agent-history-{index:06}'
        record = {
            'assignment': {
                'index': {'id': execution, 'kind': 'agent', 'node_id': 'node-memory-fixture',
                          'status': 'done', 'created_at': index + 1},
                'request': {'id': execution, 'kind': 'agent', 'input': {}},
                'runtime': {'fixture_payload': payload},
                'definition': {'fixture_payload': payload},
            },
            'annotations': {}, 'result': {'marker': index}, 'error': None,
            'events': [{'seq': 1, 'ts': 1, 'kind': 'done', 'data': {'status': 'done'}}],
        }
        path = data / 'agent' / execution / 'execution.json'
        path.parent.mkdir(parents=True, exist_ok=True)
        body = json.dumps(record, separators=(',', ':'))
        path.write_text(body)
        total += len(body)
    return count, total, data, work, home


def rss(pid):
    values = {}
    for line in Path(f'/proc/{pid}/status').read_text().splitlines():
        if line.startswith(('VmRSS:', 'VmHWM:')):
            key, amount, _ = line.split()
            values[key[:-1]] = int(amount) * 1024
    return values


def request(port, path, body=None):
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    req = urllib.request.Request(
        f'http://127.0.0.1:{port}{path}',
        data=None if body is None else json.dumps(body).encode(),
        headers={'Authorization': f'Bearer {TOKEN}', 'Content-Type': 'application/json'})
    with opener.open(req, timeout=30) as response:
        return json.load(response)


def stable_rss(process, seconds=3):
    readings = []
    for _ in range(seconds * 5):
        if process.poll() is not None:
            raise RuntimeError('Runtime exited while measuring memory')
        readings.append(rss(process.pid)['VmRSS'])
        time.sleep(0.2)
    return int(statistics.median(readings))


def run(binary, root, mib, rounds, queries, tasks=0):
    model = Model() if tasks else None
    count, total, data, work, home = fixture(root, mib, model.config() if model else None)
    with socket.socket() as sock:
        sock.bind(('127.0.0.1', 0))
        port = sock.getsockname()[1]
    environment = {'PATH': os.environ['PATH'], 'HOME': str(home), 'RUST_LOG': 'error',
                   'NO_PROXY': '127.0.0.1,localhost', 'LANG': 'C.UTF-8'}
    command = [str(binary), '--name', 'memory-fixture', '--data-dir', str(data),
               '--workdir', str(work), '--token', TOKEN, '--no-dag',
               'runtime', '--port', str(port)]
    log = (root / 'runtime.log').open('w')
    process = subprocess.Popen(command, env=environment, stdout=log, stderr=log)
    started = time.monotonic()
    peak = [0]
    stop = threading.Event()

    def monitor():
        while not stop.wait(0.05):
            try:
                peak[0] = max(peak[0], rss(process.pid).get('VmRSS', 0))
            except FileNotFoundError:
                return

    watcher = threading.Thread(target=monitor, daemon=True)
    watcher.start()
    result = {'binary': str(binary), 'history_mib': mib, 'record_count': count,
              'history_bytes': total, 'round_rss': [], 'query_ms': [], 'report_ms': [],
              'tasks_per_round': tasks, 'completed_tasks': 0}
    try:
        while True:
            if process.poll() is not None:
                raise RuntimeError(f'Runtime startup failed; see {root / "runtime.log"}')
            if time.monotonic() - started > 900:
                raise TimeoutError('Runtime startup exceeded 900 seconds')
            try:
                inventory = request(port, '/inventory')
                if len(inventory['indexes']) != count:
                    raise AssertionError('history index is incomplete')
                break
            except (OSError, urllib.error.URLError):
                time.sleep(0.2)
        result['startup_seconds'] = round(time.monotonic() - started, 3)
        result['startup_peak_rss'] = max(peak[0], rss(process.pid)['VmHWM'])
        result['idle_rss'] = stable_rss(process)

        def query(index):
            tick = time.monotonic()
            reply = request(port, '/rpc', {'operation': 'inspect',
                'execution': {'kind': 'agent', 'id': f'agent-history-{index:06}'}})
            if reply['status'] != 200 or reply['body']['result']['marker'] != index:
                raise AssertionError(f'history query failed: status {reply["status"]}')
            return (time.monotonic() - tick) * 1000

        for turn in range(rounds):
            if tasks:
                complete(port, turn, tasks, request)
                result['completed_tasks'] += tasks
            tick = time.monotonic()
            if request(port, '/frames'):
                raise AssertionError('unrelated history produced Brain frames')
            result['report_ms'].append(round((time.monotonic() - tick) * 1000, 3))
            if count:
                # Access more records than the cache limit, moving across history.
                ids = [(turn * queries + index) % count for index in range(queries)]
                with concurrent.futures.ThreadPoolExecutor(max_workers=8) as executor:
                    result['query_ms'].extend(executor.map(query, ids))
            result['round_rss'].append(stable_rss(process))
        if model and model.calls < tasks * rounds:
            raise AssertionError('execution bypassed the local model fixture')
        if tasks:
            result['verified_snapshots'] = verify_completed(data, tasks * rounds)
        result['peak_rss'] = max(peak[0], rss(process.pid)['VmHWM'])
        result['final_rss'] = result['round_rss'][-1]
        result['query_p95_ms'] = (round(sorted(result['query_ms'])[int(len(result['query_ms']) * .95)], 3)
                                  if result['query_ms'] else 0)
        result['query_max_ms'] = round(max(result['query_ms'], default=0), 3)
        del result['query_ms']
        return result
    finally:
        stop.set()
        watcher.join(timeout=2)
        process.terminate()
        try:
            process.wait(timeout=30)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()
        log.close()
        if model:
            model.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--before', type=Path, required=True)
    parser.add_argument('--after', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--sizes', type=int, nargs='+', default=[0, 1024, 4096])
    parser.add_argument('--rounds', type=int, default=5)
    parser.add_argument('--queries', type=int, default=256)
    parser.add_argument('--tasks-per-round', type=int, default=0)
    parser.add_argument('--resume', action='store_true', help='continue the same comparison after a build finishes')
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=args.resume)
    results_path = args.output / 'results.json'
    results = json.loads(results_path.read_text()) if args.resume and results_path.exists() else []
    if args.rounds < 2 or args.queries < 1 or args.tasks_per_round < 0 or any(size < 0 for size in args.sizes):
        parser.error('need at least two rounds, positive queries and nonnegative history sizes')
    for label, binary in [('before', args.before), ('after', args.after)]:
        with binary.open('rb') as executable:
            digest = hashlib.file_digest(executable, 'sha256').hexdigest()
        for size in args.sizes:
            previous = next((r for r in results if r['label'] == label and r['history_mib'] == size), None)
            if previous:
                if (previous['sha256'], previous['rounds'], previous['queries'], previous.get('tasks_per_round', 0)) != (digest, args.rounds, args.queries, args.tasks_per_round):
                    raise AssertionError('resume binary or measurement settings changed')
                continue
            root = args.output / f'{label}-{size}'
            print(f'START {label} history={size} MiB', flush=True)
            result = run(binary.resolve(), root, size, args.rounds, args.queries, args.tasks_per_round)
            result.update(label=label, sha256=digest, rounds=args.rounds, queries=args.queries)
            results.append(result)
            (args.output / 'results.json').write_text(json.dumps(results, indent=2))
            print(json.dumps(result), flush=True)
    after = {r['history_mib']: r for r in results if r['label'] == 'after'}
    before = {r['history_mib']: r for r in results if r['label'] == 'before'}
    if 0 not in after:
        raise AssertionError('empty-history baseline is required')
    for size, result in after.items():
        if abs(result['round_rss'][-1] - result['round_rss'][-2]) > 32 * MIB:
            raise AssertionError('RSS is still growing between the final rounds')
        if not size:
            continue
        if result['final_rss'] - after[0]['final_rss'] > 256 * MIB:
            raise AssertionError(f'{size} MiB history exceeds 256 MiB RSS increase')
        if result['final_rss'] > before[size]['final_rss'] * .2:
            raise AssertionError('RSS reduction is less than 80 percent')
        if result['query_max_ms'] > 30000:
            raise AssertionError('a history query exceeded 30 seconds')
    print('PASS memory acceptance', flush=True)


if __name__ == '__main__':
    main()
