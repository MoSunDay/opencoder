#!/usr/bin/env python3
"""Summarize immutable attempt receipts without replacing failed attempts with passes."""
import argparse
import json
from pathlib import Path

from main import CASES


def summarize(paths):
    records = {name: [] for name in CASES}
    attempts = []
    for path in paths:
        result = json.loads((path / 'result.json').read_text())
        attempts.append({'path': str(path), 'result': result['result'], 'commit': result['commit'],
                         'error': result.get('error'), 'cleanup_error': result.get('cleanup_error')})
        for row in result['cases']:
            records.setdefault(row['case'], []).append({'path': str(path), **row})
    cases = []
    for name, rows in records.items():
        latest = rows[-1] if rows else {'result': 'NOT_RUN', 'reason': 'no receipt'}
        last_pass = next((row for row in reversed(rows) if row['result'] == 'PASS'), None)
        cases.append({'case': name, 'latest': latest, 'last_success': last_pass, 'attempt_count': len(rows)})
    commits = sorted({attempt['commit'] for attempt in attempts})
    return {'result': 'PASS' if all(row['latest']['result'] == 'PASS' for row in cases)
            and len(commits) == 1 and attempts and attempts[-1]['result'] == 'PASS'
            and not attempts[-1].get('error')
            and all(not row.get('cleanup_error') for row in attempts) else 'FAIL',
            'commits': commits, 'cases': cases, 'attempts': attempts}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('attempts', type=Path, nargs='+', help='Attempt directories, oldest first')
    args = parser.parse_args()
    print(json.dumps(summarize(args.attempts), ensure_ascii=False, indent=2))


if __name__ == '__main__':
    main()
