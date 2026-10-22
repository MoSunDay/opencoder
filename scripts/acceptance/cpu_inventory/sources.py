#!/usr/bin/env python3
"""Compare full Runtime polls with committed source cursors at fixed sizes."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import time
import urllib.error
import urllib.parse

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'runtime_memory'))
import probe
from fixture import port, write
from metrics import counters, percentile


def run(binary, root, count, incremental):
    _, _, data, work, home = probe.fixture(root, 0, {'agent': {'agents_dir': None}})
    for index in range(count):
        identifier = f'agent-history-{index:06}'
        write(data / 'agent' / identifier / 'execution.json', {
            'assignment': {'index': {'id': identifier, 'kind': 'agent',
                'node_id': 'node-memory-fixture', 'status': 'done', 'created_at': index + 1},
                'request': {'id': identifier, 'kind': 'agent', 'input': {}}, 'runtime': None},
            'annotations': {}, 'result': {'marker': index}, 'error': None, 'events': []})
    listener = port()
    environment = {'PATH': os.environ['PATH'], 'HOME': str(home), 'RUST_LOG': 'error',
                   'NO_PROXY': '127.0.0.1,localhost', 'LANG': 'C.UTF-8'}
    with (root / 'runtime.log').open('w') as log:
        process = subprocess.Popen([str(binary), '--name', 'source-fixture', '--data-dir',
            str(data), '--workdir', str(work), '--token', probe.TOKEN, '--no-dag',
            'runtime', '--port', str(listener)], env=environment, stdout=log, stderr=log)
    try:
        started = time.monotonic()
        while True:
            if process.poll() is not None:
                raise RuntimeError('private Runtime exited before startup')
            try:
                inventory = probe.request(listener, '/inventory')
                if len(inventory['indexes']) != count or not inventory['snapshot']['ready']:
                    raise AssertionError('Runtime history incomplete or not ready')
                break
            except (OSError, urllib.error.URLError):
                if time.monotonic() - started > 900:
                    raise TimeoutError('private Runtime did not become ready')
                time.sleep(.2)
        startup = time.monotonic() - started
        expected = {f'agent-history-{index:06}' for index in range(count)}
        if {row['id'] for row in inventory['indexes']} != expected:
            raise AssertionError('Runtime initial inventory identities differ')
        query, received, identities = {}, 0, set()
        sync_started = time.monotonic()
        if incremental:
            while True:
                page = probe.request(listener, '/inventory/changes?' +
                                     urllib.parse.urlencode(query))['changes']
                if not page['full'] or page['removed_ids']:
                    raise AssertionError('initial snapshot was not complete')
                received += len(page['records'])
                identities.update(row['id'] for row in page['records'])
                if not page.get('next'):
                    break
                query = {'page': page['next']}
            if received != count or identities != expected:
                raise AssertionError('initial source pages lost history')
            query = {'generation': page['generation'], 'after': page['revision']}
        sync_seconds = time.monotonic() - sync_started
        before, latencies = counters(process.pid), []
        for _ in range(30):
            tick = time.monotonic()
            if incremental:
                page = probe.request(listener, '/inventory/changes?' +
                                     urllib.parse.urlencode(query))['changes']
                if page['full'] or page['records'] or page['removed_ids']:
                    raise AssertionError('unchanged history unexpectedly rebuilt or changed')
            elif len(probe.request(listener, '/inventory')['indexes']) != count:
                raise AssertionError('baseline history incomplete')
            latencies.append((time.monotonic() - tick) * 1000)
        after = counters(process.pid)
        if before['start'] != after['start']:
            raise AssertionError('Runtime restarted while measuring')
        return {'history': count, 'polls': 30, 'p95_ms': percentile(latencies),
                'cpu_ticks': after['ticks'] - before['ticks'], 'rss_bytes': after['rss_bytes'],
                'startup_seconds': startup, 'initial_sync_seconds': sync_seconds,
                'sha256': hashlib.file_digest(binary.open('rb'), 'sha256').hexdigest(),
                'io_pressure': Path('/proc/pressure/io').read_text()}
    finally:
        process.terminate()
        try:
            process.wait(timeout=30)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait(timeout=10)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--before', type=Path, required=True)
    parser.add_argument('--after', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=False)
    results, verdicts = [], []
    for turn, count in enumerate((2000, 20000, 100000)):
        order = [('before', args.before), ('after', args.after)]
        if turn % 2:
            order.reverse()
        pair = {}
        for label, binary in order:
            print(f'START {label} history={count}', flush=True)
            row = run(binary.resolve(), args.output / f'{label}-{count}', count, label == 'after')
            row['label'] = label
            pair[label] = row
            results.append(row)
            write(args.output / 'results.json', results)
            print(json.dumps(row), flush=True)
        old, new = pair['before'], pair['after']
        verdicts.append({'history': count, 'cpu_ratio': new['cpu_ticks'] / max(1, old['cpu_ticks']),
            'p95_ratio': new['p95_ms'] / old['p95_ms'],
            'passed': new['cpu_ticks'] <= old['cpu_ticks'] * .5 and new['p95_ms'] <= old['p95_ms'] * 1.1})
    write(args.output / 'verdict.json', verdicts)
    print(json.dumps(verdicts), flush=True)
    if not all(row['passed'] for row in verdicts):
        raise SystemExit('source polling CPU or latency gate failed')


if __name__ == '__main__':
    main()
