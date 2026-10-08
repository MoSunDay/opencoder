"""Kill only isolated, owned Brain processes; failure never resumes a root."""
import json
from pathlib import Path
import subprocess
import time

from environment import require
from cases.correction import baseline_config, invalid_decision, spec
from faults.model import ModelFault
from faults.runtime import FrozenWindow, journal, require_isolated


def delayed(_, body, closed):
    closed.wait(330)
    return invalid_decision()


def assert_failed(env, root, before, proxy):
    final = env.terminal(root, 90)
    require(final['run']['phase'] == 'failed' and not final['operations'], 'crash did not fail the entire root')
    status, _ = env.request('POST', f'/api/brain/runs/{root}/commands', {'action': 'resume'})
    require(status == 409, f'failed root resume must return 409, received {status}')
    after = journal(env, root)['annotations']['layered_decision_attempt']
    require(after == before and proxy.calls[root] == 1, 'crash replayed a decision or reset its budget')
    # Check several report cycles, including public and authoritative snapshots.
    for _ in range(5):
        require(env.brain_rpc(root, 'snapshot')['run']['phase'] == 'failed', 'late report revived failed root')
        time.sleep(1)
    require(proxy.calls[root] == 1, 'background recovery retried failed root')
    return final


def decision(env, capabilities):
    require_isolated(env)
    with ModelFault(env, baseline_config(env, capabilities, 'decision-crash-baseline')) as proxy:
        root = proxy.create('decision-crash', spec(capabilities), delayed)
        env.wait(lambda: proxy.calls.get(root, 0) == 1, 90, 'decision model process starts')
        before = journal(env, root)['annotations']['layered_decision_attempt']
        generation = before['generation']
        state = Path(env.record['runtime_data']) / 'brain/bundles' / root / str(generation) / 'runc-state'
        container = f'{root}-a{generation}'
        info = json.loads(subprocess.check_output(['runc', '--root', str(state), 'state', container]))
        require(info['status'] == 'running' and info['pid'] > 0, 'owned decision container not running')
        subprocess.run(['runc', '--root', str(state), 'kill', container, 'KILL'], check=True, timeout=15)
        final = assert_failed(env, root, before, proxy)
        env.wait(lambda: not (state / container).exists(), 30, 'decision container cleanup')
    return {'run_id': root, 'phase': final['run']['phase'], 'attempts': 1, 'container_removed': True}


def worker(env, capabilities):
    require_isolated(env)
    with ModelFault(env, baseline_config(env, capabilities, 'worker-crash-baseline')) as proxy:
        root = proxy.create('worker-crash', spec(capabilities), delayed)
        env.wait(lambda: proxy.calls.get(root, 0) == 1, 90, 'held decision before Worker crash')
        before = journal(env, root)['annotations']['layered_decision_attempt']
        unit = env.record['runtime_unit']
        pid = int(subprocess.check_output(['systemctl', 'show', unit, '-p', 'MainPID', '--value']))
        binary = Path(f'/proc/{pid}/exe').resolve()
        receipts = env.output / 'crash-settlement'
        command = [str(binary), '--data-dir', env.record['runtime_data'], 'storage', 'settle-brain-crash',
                   '--run-id', root, '--receipt-dir', str(receipts)]
        # Active-runtime refusal must happen before creating a receipt or writing state.
        refused = subprocess.run(command, capture_output=True, timeout=30)
        require(refused.returncode != 0 and not receipts.exists(), 'settlement accepted a live Runtime')
        require(journal(env, root)['annotations']['layered_decision_attempt'] == before, 'refused settlement changed state')
        with FrozenWindow(env):
            subprocess.run(['systemctl', 'kill', '--kill-who=main', '--signal=SIGKILL', unit], check=True, timeout=15)
            subprocess.run(['systemctl', 'stop', unit], check=True, timeout=90)
            # Startup must fail while the crashed decision still owns its slot.
            subprocess.run(['systemctl', 'start', unit], check=True, timeout=30)
            env.wait(lambda: subprocess.check_output(['systemctl', 'show', unit, '-p', 'ActiveState', '--value']).strip() == b'failed',
                     30, 'unsettled capacity blocks Runtime startup')
            first = subprocess.run(command, capture_output=True, text=True, timeout=60)
            require(first.returncode == 0, 'controlled settlement failed: ' + first.stderr[-2000:])
            settled = json.loads(first.stdout)
            require(settled['phase'] == 'failed' and settled['capacity'] == 'done' and settled['resumed'] is False,
                    'settlement did not persist failure and exact capacity completion')
            intent = (receipts / (root + '.intent.json')).read_bytes()
            record = journal(env, root)
            # Re-enter after the final write as well as after a missing final receipt.
            for remove_result in (False, True):
                if remove_result:
                    (receipts / (root + '.result.json')).unlink()
                replay = subprocess.run(command, capture_output=True, text=True, timeout=60)
                require(replay.returncode == 0 and json.loads(replay.stdout) == settled, 'settlement re-entry changed result')
                require((receipts / (root + '.intent.json')).read_bytes() == intent, 'settlement rewrote immutable intent')
                require(journal(env, root) == record, 'settlement re-entry changed Brain history')
            env.save('faults/settlement', settled)
        final = assert_failed(env, root, before, proxy)
    fresh = env.create('after-worker-crash', spec(capabilities))
    require(env.terminal(fresh)['run']['phase'] == 'completed', 'new run could not use settled capacity')
    return {'run_id': root, 'phase': final['run']['phase'], 'active_runtime_refused': True,
            'startup_guard': True, 'idempotent_settlement': True, 'new_run_id': fresh}
