"""Measure acceptance and scheduling from durable, Runtime-owned records."""
import json
from pathlib import Path


def summarize(traffic, executions):
    if len(traffic) < 2 or len(executions) != len(traffic):
        raise AssertionError('continuous traffic needs at least two complete executions')
    accepted = sorted(row['at'] + row['seconds'] for row in traffic)
    started = sorted(row['started_at_ms'] for row in executions)
    latencies = sorted(row['seconds'] for row in traffic)
    return {
        'max_accept_seconds': max(row['seconds'] for row in traffic),
        'p95_accept_seconds': latencies[(95 * len(latencies) + 99) // 100 - 1],
        'max_accept_gap_seconds': max(b - a for a, b in zip(accepted, accepted[1:])),
        'max_scheduling_gap_seconds': max(b - a for a, b in zip(started, started[1:])) / 1000,
        'max_scheduling_delay_seconds': max(row['started_at_ms'] - row['created_at_ms'] for row in executions) / 1000,
    }


def check_continuity(metrics, latency_gate='tail'):
    if latency_gate == 'p95':
        if metrics['p95_accept_seconds'] > 1:
            raise AssertionError(f'release P95 admission exceeded one second: {metrics["p95_accept_seconds"]}')
        return
    if latency_gate != 'tail':
        raise ValueError(f'unknown release latency gate: {latency_gate}')
    for key in ('max_accept_seconds', 'max_accept_gap_seconds', 'max_scheduling_gap_seconds'):
        if metrics[key] > 1:
            raise AssertionError(f'release continuity exceeded one second: {key}={metrics[key]}')


def verify(runtime_roots, traffic, latency_gate='tail'):
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
    # The isolated process fixture retains its strict tail gate. Production
    # measures public availability independently; serial submission gaps can
    # otherwise count one durable write tail as three separate outages.
    check_continuity(metrics, latency_gate)
    return {'metrics': metrics, 'executions': executions}


def verify_ready(samples, failures, maximum_gap=1):
    if failures:
        raise AssertionError(f'release readiness failed: {failures}')
    if len(samples) < 2:
        raise AssertionError('release readiness needs at least two successful samples')
    completed = sorted(item['completed_at'] for item in samples)
    gap = max(b - a for a, b in zip(completed, completed[1:]))
    if gap > maximum_gap:
        raise AssertionError(f'release readiness gap exceeded {maximum_gap} seconds: {gap}')
    return {'samples': len(samples), 'max_gap_seconds': gap,
            'max_response_seconds': max(item['completed_at'] - item['started_at'] for item in samples)}
