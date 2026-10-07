#!/usr/bin/env python3
"""Verify Brain rules against the currently deployed Server and real model."""
import argparse
import json
import time
import traceback
from pathlib import Path

from environment import Environment, require
from scenario import prepare
from cases import admission, closed_loop, contracts, control, correction, definitions, execution_types, negative, recovery, replay
from faults.runtime import NotRun, wait_idle
from guidance import real as guidance
from placement import queue as placement


CASES = {'admission': admission.run, 'definitions': definitions.run, 'closed-loop': closed_loop.run,
         'controls': control.run, 'round-budget': control.budget,
         'six-types': execution_types.run, 'receipt-replay': replay.run,
         'failure-barrier': negative.failure_barrier, 'oversize-output': negative.oversize,
         'missing-prerequisite': negative.blocked, 'same-layer-repair': definitions.same_layer,
         'contracts': contracts.run, 'frozen-contract': contracts.frozen,
         'live-steering': guidance.run,
         'context-capacity': admission.capacity, 'dispatch-retry': recovery.retry_receipts,
         'process-recovery': recovery.run, 'correction-budget': correction.run,
         'correction-deadline': correction.deadline,
         'capacity-queue': placement.run}
FAULT_CASES = {'dispatch-retry', 'process-recovery', 'correction-budget', 'correction-deadline', 'capacity-queue'}


def wait_ready(env, seconds):
    deadline = time.monotonic() + seconds
    last_error = None
    while True:
        try:
            nodes = env.http(env.settings.public_url, '/api/nodes')['nodes']
            node = next(n for n in nodes if n['id'] == env.node_id)
            snapshot = node.get('snapshot') or {}
            if node['online'] and snapshot.get('ready') is True:
                return
            last_error = snapshot.get('resource_error') or 'local node is not online and ready'
        except Exception as error:
            last_error = str(error)
        env.save('readiness', {'error': last_error, 'at': time.time()})
        if time.monotonic() >= deadline:
            raise NotRun(last_error)
        print(json.dumps({'stage': 'waiting-for-local-node', 'reason': last_error}), flush=True)
        time.sleep(min(30, max(0, deadline - time.monotonic())))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--config', required=True)
    parser.add_argument('--expected-commit', required=True)
    parser.add_argument('--out', required=True)
    parser.add_argument('--case', choices=[*CASES, 'all'], nargs='+', default=['all'])
    parser.add_argument('--wait-ready', type=int, default=0, help='Maximum seconds to wait for local resources (0..1800)')
    args = parser.parse_args()
    require(0 <= args.wait_ready <= 1800, '--wait-ready must be in 0..1800')
    require(not Path(args.out).exists(), '--out must be a new directory; previous evidence is immutable')
    require('all' not in args.case or args.case == ['all'], 'all cannot be combined with individual cases')
    selected = CASES if args.case == ['all'] else {name: CASES[name] for name in args.case}
    report = {'commit': args.expected_commit, 'scope': list(selected), 'cases': [], 'result': 'RUNNING'}
    env = None
    try:
        env = Environment(args.config, args.out, args.expected_commit)
        report['tag'] = env.tag
        env.save('result', report)
        wait_ready(env, args.wait_ready)
        capabilities = prepare(env)
        fault_checked, fault_error = False, None
        for name, run in selected.items():
            print(json.dumps({'case': name, 'stage': 'started'}), flush=True)
            began = time.monotonic()
            try:
                if name in FAULT_CASES:
                    if not fault_checked:
                        fault_checked = True
                        try:
                            wait_idle(env)
                        except Exception as error:
                            fault_error = str(error)
                    if fault_error:
                        raise NotRun(fault_error)
                result = run(env, capabilities)
                record = {'case': name, 'result': 'PASS', 'evidence': result}
            except NotRun as error:
                record = {'case': name, 'result': 'NOT_RUN', 'reason': str(error)}
            except Exception as error:
                record = {'case': name, 'result': 'FAIL', 'error': str(error), 'traceback': traceback.format_exc()}
            record['elapsed_seconds'] = time.monotonic() - began
            report['cases'].append(record)
            env.save('result', report)
            print(json.dumps(record), flush=True)
        final_runtime = env.verify_release()
        env.save('final-runtime', final_runtime)
        require(final_runtime['ready_status'] == 200, 'public readiness failed at the end of acceptance')
        wait_ready(env, 0)
    except (Exception, KeyboardInterrupt) as error:
        report.update(error=str(error) or 'acceptance interrupted', traceback=traceback.format_exc())
    finally:
        if env:
            try:
                env.cleanup()
            except Exception as error:
                report['cleanup_error'] = str(error)
        observed = {record['case'] for record in report['cases']}
        report['cases'].extend({'case': name, 'result': 'NOT_RUN', 'reason': report.get('error', 'not executed')}
                               for name in selected if name not in observed)
        report['result'] = 'PASS' if (all(c['result'] == 'PASS' for c in report['cases'])
            and 'error' not in report and 'cleanup_error' not in report) else 'FAIL'
        report['full_suite'] = set(selected) == set(CASES)
        output = Path(args.out)
        output.mkdir(parents=True, exist_ok=True, mode=0o700)
        from rolling.state import write
        write(output / 'result.json', report)
    print(json.dumps(report), flush=True)
    return 0 if report['result'] == 'PASS' else 1


if __name__ == '__main__':
    raise SystemExit(main())
