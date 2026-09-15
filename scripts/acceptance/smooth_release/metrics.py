"""Measure acceptance and scheduling from durable, Runtime-owned records."""
import json
from pathlib import Path


def summarize(traffic, executions):
    if len(traffic) < 2 or len(executions) != len(traffic):
        raise AssertionError('continuous traffic needs at least two complete executions')
    accepted = sorted(row['at'] + row['seconds'] for row in traffic)
    started = sorted(row['started_at_ms'] for row in executions)
    return {
        'max_accept_seconds': max(row['seconds'] for row in traffic),
        'max_accept_gap_seconds': max(b - a for a, b in zip(accepted, accepted[1:])),
        'max_scheduling_gap_seconds': max(b - a for a, b in zip(started, started[1:])) / 1000,
        'max_scheduling_delay_seconds': max(row['started_at_ms'] - row['created_at_ms'] for row in executions) / 1000,
    }


def verify(runtime_roots, traffic):
    executions = []
    for row in traffic:
        paths = [Path(root) / 'dag' / row['id'] / 'execution.json' for root in runtime_roots]
        paths = [path for path in paths if path.is_file()]
        if len(paths) != 1:
            raise AssertionError('execution does not have exactly one Runtime: ' + row['id'])
        path = paths[0]
        index = json.loads(path.read_text())['assignment']['index']
        step = json.loads((path.parent / 'execute/meta.json').read_text())
        if step['outcome'] != 'done':
            raise AssertionError('traffic did not execute: ' + row['id'])
        executions.append({'id': row['id'], 'runtime': str(path.parents[2]),
            'created_at_ms': index['created_at'], 'started_at_ms': step['started_at_ms']})
    metrics = summarize(traffic, executions)
    # Individual queue delay may exceed one second when the fixed global
    # capacity is occupied. Acceptance and scheduling must remain continuous.
    for key in ('max_accept_seconds', 'max_accept_gap_seconds', 'max_scheduling_gap_seconds'):
        if metrics[key] > 1:
            raise AssertionError(f'release continuity exceeded one second: {key}={metrics[key]}')
    return {'metrics': metrics, 'executions': executions}
