#!/usr/bin/env python3
"""Compare 160 completed tasks with matching release Runtime binaries."""
import argparse
import hashlib
import json
from pathlib import Path
import sys
import time

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'runtime_memory'))
import probe
from fixture import write
from metrics import throughput_verdict


def measure(binary, root):
    original, durations = probe.complete, []

    def timed_batch(*args):
        started = time.monotonic()
        result = original(*args)
        durations.append(time.monotonic() - started)
        return result

    before_pressure = Path('/proc/pressure/io').read_text()
    probe.complete = timed_batch
    try:
        # Five batches, 32 tasks each; the shared workload runs four at a time.
        # Each task retains a 1 MiB frozen payload and calls the local model.
        result = probe.run(binary, root, 0, 5, 256, 32)
    finally:
        probe.complete = original
    if result['completed_tasks'] != 160 or result['verified_snapshots'] != 160:
        raise AssertionError('task count or durable snapshots differ')
    if len(durations) != 5:
        raise AssertionError('task batches were omitted')
    result.update(task_round_seconds=durations, throughput=160 / sum(durations),
                  sha256=hashlib.file_digest(binary.open('rb'), 'sha256').hexdigest(),
                  io_pressure_before=before_pressure,
                  io_pressure_after=Path('/proc/pressure/io').read_text())
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--before', type=Path, required=True)
    parser.add_argument('--after', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--rounds', type=int, default=3)
    args = parser.parse_args()
    if args.rounds <= 0:
        parser.error('rounds must be positive')
    args.output.mkdir(parents=True, exist_ok=False)
    results = []
    try:
        for turn in range(args.rounds):
            order = [('before', args.before), ('after', args.after)]
            if turn % 2:
                order.reverse()
            for label, binary in order:
                print(f'START {label} round={turn} tasks=160', flush=True)
                result = measure(binary.resolve(), args.output / f'{turn}-{label}')
                result.update(label=label, round=turn)
                results.append(result)
                write(args.output / 'results.json', results)
                print(json.dumps(result), flush=True)
        verdict = throughput_verdict(results, args.rounds)
        write(args.output / 'verdict.json', verdict)
        print(json.dumps(verdict), flush=True)
        if not verdict['passed']:
            raise SystemExit('active throughput decreased by more than five percent')
    except BaseException as error:
        write(args.output / 'failure.json', {'type': type(error).__name__, 'error': str(error)})
        raise


if __name__ == '__main__':
    main()
