#!/usr/bin/env python3
"""Compare release binaries with the same real three-process inventory path."""
import argparse
import hashlib
import json
from pathlib import Path
import time
from fixture import Fixture, write
from metrics import counters, percentile, summarize, verdict


def measure(binary, output, args):
    env = Fixture(binary, output, args.history, args.payload_count, args.payload_bytes)
    try:
        env.open()
        # Include several natural report cycles before timing; first full sync
        # is verified separately and cannot be mistaken for the idle cost.
        warm = time.monotonic() + args.warmup
        while time.monotonic() < warm:
            env.check_alive()
            time.sleep(.2)
        initial = {role: counters(child.pid) for role, child in env.children.items()}
        pressure_before = Path('/proc/pressure/io').read_text()
        start = time.monotonic()
        latency, errors, lags, responses, samples = [], [], [], [], []
        for index in range(round(args.seconds * args.rate)):
            due = start + index / args.rate
            time.sleep(max(0, due - time.monotonic()))
            lags.append(max(0, time.monotonic() - due))
            tick = time.monotonic()
            kind = 'history' if index % 2 else 'ready'
            try:
                env.check_alive()
                if index % 2:
                    marker = (index // 2) % args.history
                    reply = env.http('server', f'/api/executions/agent-history-{marker:06}')
                    if reply['result']['marker'] != marker:
                        raise AssertionError('historical result changed')
                    if reply.get('session', {}).get('meta', {}).get('id') != f'agent-history-{marker:06}':
                        raise AssertionError('historical session is missing or returned an embedded error')
                    responses.append(['history', marker])
                else:
                    reply = env.http('server', '/api/ready')
                    if reply['ready_nodes'] != 1:
                        raise AssertionError('node became unavailable')
                    responses.append(['ready', 1])
            except Exception as error:
                errors.append({'request': index, 'error': str(error)})
            latency.append((time.monotonic() - tick) * 1000)
            samples.append({'request': index, 'kind': kind, 'elapsed_ms': latency[-1],
                            'offset_seconds': tick - start, 'schedule_lag_seconds': lags[-1]})
        time.sleep(max(0, start + args.seconds - time.monotonic()))
        elapsed = time.monotonic() - start
        final = {role: counters(child.pid) for role, child in env.children.items()}
        result = summarize(initial, final, elapsed)
        result.update(seconds=elapsed, requests=len(latency), errors=errors,
            query_p95_ms=percentile(latency), query_max_ms=max(latency),
            query_p95_by_kind_ms={kind: percentile([s['elapsed_ms'] for s in samples
                if s['kind'] == kind]) for kind in {s['kind'] for s in samples}},
            schedule_lag_max_seconds=max(lags), history=args.history,
            fixture_version=2,
            payload_count=args.payload_count, payload_bytes=args.payload_bytes,
            inventory_digest=env.verify(),
            response_digest=hashlib.sha256(json.dumps(responses).encode()).hexdigest(),
            io_pressure_before=pressure_before, io_pressure_after=Path('/proc/pressure/io').read_text(),
            binary_sha256={name: hashlib.file_digest((binary / name).open('rb'), 'sha256').hexdigest()
                           for name in ('opencoder-agent', 'opencoder-server')})
        write(output / 'requests.json', samples)
        write(output / 'result.json', result)
        return result
    finally:
        env.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--before', type=Path, required=True, help='baseline bin directory')
    parser.add_argument('--after', type=Path, required=True, help='candidate bin directory')
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--history', type=int, default=20000)
    parser.add_argument('--payload-count', type=int, default=4000)
    parser.add_argument('--payload-bytes', type=int, default=131072)
    parser.add_argument('--seconds', type=int, default=120)
    parser.add_argument('--warmup', type=int, default=30)
    parser.add_argument('--rounds', type=int, default=3)
    parser.add_argument('--rate', type=float, default=2)
    args = parser.parse_args()
    if min(args.history, args.seconds, args.rounds, args.rate) <= 0 or args.warmup < 0:
        parser.error('history, duration, rounds and request rate must be positive')
    if not 0 <= args.payload_count <= args.history or args.payload_bytes < 0:
        parser.error('invalid historical payload size')
    args.output.mkdir(parents=True, exist_ok=False)
    results = []
    try:
        for turn in range(args.rounds):
            pairs = [('before', args.before), ('after', args.after)]
            if turn % 2:
                pairs.reverse()
            for label, binary in pairs:
                print(f'START {label} round={turn}', flush=True)
                result = measure(binary.resolve(), args.output / f'{turn}-{label}', args)
                result.update(label=label, round=turn)
                results.append(result)
                write(args.output / 'results.json', results)
                print(json.dumps(result), flush=True)
        result = verdict(sorted(results, key=lambda row: row['round']))
        write(args.output / 'verdict.json', result)
        print(json.dumps(result), flush=True)
        if not result['passed']:
            raise SystemExit('CPU or latency gate failed')
    except BaseException as error:
        write(args.output / 'failure.json', {'type': type(error).__name__, 'error': str(error)})
        raise


if __name__ == '__main__':
    main()
