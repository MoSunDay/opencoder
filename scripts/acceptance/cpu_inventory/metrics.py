"""Use CPU time for every service, never a single process's percentage."""
import math
import os
from pathlib import Path
import statistics


def counters(pid):
    fields = Path(f'/proc/{pid}/stat').read_text().rsplit(')', 1)[1].split()
    return {'start': fields[19], 'ticks': int(fields[11]) + int(fields[12]),
            'rss_bytes': int(fields[21]) * os.sysconf('SC_PAGE_SIZE')}


def percentile(values, fraction=.95):
    return sorted(values)[max(0, math.ceil(len(values) * fraction) - 1)]


def summarize(before, after, seconds):
    roles = {}
    for role, first in before.items():
        last = after[role]
        if first['start'] != last['start'] or last['ticks'] < first['ticks']:
            raise AssertionError('service restarted during CPU measurement')
        cpu = (last['ticks'] - first['ticks']) / os.sysconf('SC_CLK_TCK')
        roles[role] = {'cpu_seconds': cpu, 'cpu_percent': 100 * cpu / seconds,
                       'rss_bytes': last['rss_bytes']}
    return {'roles': roles, 'aggregate_cpu_seconds': sum(r['cpu_seconds'] for r in roles.values()),
            'aggregate_cpu_percent': sum(r['cpu_percent'] for r in roles.values())}


def verdict(results):
    left = [r for r in results if r['label'] == 'before']
    right = [r for r in results if r['label'] == 'after']
    if not left or len(left) != len(right):
        raise AssertionError('both versions require the same number of rounds')
    for before, after in zip(left, right):
        for key in ('fixture_version', 'requests', 'history', 'payload_bytes', 'payload_count',
                    'response_digest', 'inventory_digest'):
            if before[key] != after[key]:
                raise AssertionError(f'workloads or responses differ: {key}')
        if before['errors'] or after['errors']:
            raise AssertionError('request failures cannot be treated as CPU improvement')
        if before['schedule_lag_max_seconds'] > 1 or after['schedule_lag_max_seconds'] > 1:
            raise AssertionError('fixed request rate was not sustained')
    old_cpu = statistics.median(r['aggregate_cpu_percent'] for r in left)
    new_cpu = statistics.median(r['aggregate_cpu_percent'] for r in right)
    old_p95 = statistics.median(r['query_p95_ms'] for r in left)
    new_p95 = statistics.median(r['query_p95_ms'] for r in right)
    if old_cpu <= 0:
        raise AssertionError('baseline CPU signal was too small')
    return {'cpu_ratio': new_cpu / old_cpu, 'p95_ratio': new_p95 / max(.001, old_p95),
            'before_cpu_percent': old_cpu, 'after_cpu_percent': new_cpu,
            'passed': new_cpu <= old_cpu * .5 and new_p95 <= old_p95 * 1.1}


def throughput_verdict(results, rounds):
    """Include every batch; a slow first pair cannot be discarded by later runs."""
    rates = {}
    for label in ('before', 'after'):
        rows = [row for row in results if row['label'] == label]
        if len(rows) != rounds or {row['round'] for row in rows} != set(range(rounds)):
            raise AssertionError('throughput rounds are missing or duplicated')
        if len({row['sha256'] for row in rows}) != 1:
            raise AssertionError('throughput binary changed between rounds')
        seconds = 0
        for row in rows:
            if row['completed_tasks'] != 160 or row['verified_snapshots'] != 160:
                raise AssertionError('throughput task count or snapshots differ')
            durations = row['task_round_seconds']
            if len(durations) != 5 or any(not math.isfinite(t) or t <= 0 for t in durations):
                raise AssertionError('throughput batches are incomplete')
            seconds += sum(durations)
        rates[label] = 160 * rounds / seconds
    ratio = rates['after'] / rates['before']
    return {'throughput_ratio': ratio, 'minimum_ratio': .95, 'passed': ratio >= .95,
            'throughputs': rates, 'verified_tasks_per_binary': 160 * rounds,
            'rounds': rounds, 'aggregation': 'all tasks / all batch durations'}
