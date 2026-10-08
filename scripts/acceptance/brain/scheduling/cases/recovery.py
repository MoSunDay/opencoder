"""Real Server and Runtime process restarts under an exclusive idle window."""
import copy
import subprocess

from environment import require
from cases.control import command, held_plan, pump, release_all, running_gate
from faults.runtime import FrozenWindow, journal, restart, wait_idle
from assertions import events


def active_decision(env, root):
    view = env.view(root)
    require(not view['operations'], 'model outran offline-dispatch fault preparation')
    marker = journal(env, root)['annotations'].get('layered_decision_attempt') or {}
    return view if view['run']['phase'] == 'deciding' and marker.get('generation') == view['run']['generation'] else None


def retry_receipts(env, capabilities):
    wait_idle(env, 0)
    spec = held_plan(capabilities)
    # A model-requested rework is different from retrying a creation intent.
    # One round keeps the transport fault from starting a legitimate new round.
    spec['max_rounds'] = 1
    spec['objective'] += 'If one diagnostic is definitively rejected, collect its sibling then stop with a failure explanation. Never rework this diagnostic.'
    root = env.create('dispatch-retry', spec)
    unit = env.record['server_unit']
    try:
        # Stop Control while the first real model call is still generating, so its durable
        # intent cannot race with child admission. The Worker remains the sole state writer.
        env.wait(lambda: active_decision(env, root), 330, 'first real decision starts before Server stops')
        with FrozenWindow(env):
            subprocess.run(['systemctl', 'stop', unit], check=True, timeout=90)
            def intention():
                snapshot = env.brain_rpc(root, 'snapshot')
                return snapshot if snapshot['operations'] else None
            pending = env.wait(intention, 330, 'offline committed dispatch intent')
            require(subprocess.check_output(['systemctl', 'show', unit, '-p', 'MainPID', '--value']).strip() == b'0',
                    'Server was not offline when dispatch intent committed')
            require(all(op['status'] == 'creating' for op in pending['operations']), 'child admission raced offline fault')
            operation = next(op for op in pending['operations'] if op['node_id'] == 'fast')
            intent = journal(env, root)['annotations']['layered_intent']
            for status in (500, 503, 408, 423, 429):
                reply = env.brain_rpc(root, 'layered_receipt', {'operation_id': operation['operation_id'],
                    'reply': {'status': status, 'body': {'error': 'injected transient admission fault'}}})
                require(reply.get('retry') is True, f'HTTP {status} did not preserve the retry intent')
                require(env.brain_rpc(root, 'snapshot') == pending, 'transient receipt changed operation IDs or state')
                require(journal(env, root)['annotations']['layered_intent'] == intent, 'retry rewrote committed intent')
            rejected = {'operation_id': operation['operation_id'],
                        'reply': {'status': 422, 'body': {'error': 'input patch: output path /revision missing'}}}
            env.brain_rpc(root, 'layered_receipt', rejected)
            after = env.brain_rpc(root, 'snapshot')
            require(next(op for op in after['operations'] if op['node_id'] == 'fast')['status'] == 'error',
                    'definitive rejection did not become terminal')
            require(env.brain_rpc(root, 'layered_receipt', rejected).get('duplicate') is True,
                    'definitive rejection replay was not deduplicated')
            require(env.brain_rpc(root, 'snapshot') == after, 'replayed rejection changed state')
            env.save('faults/offline-intent', intent)
        def child_running():
            view = env.view(root)
            return view if any(op['node_id'] == 'hold' and op['status'] == 'running' for op in view['operations']) else None
        env.wait(child_running, 180, 'replayed sibling dispatch after Server restart')
        final = pump(env, root)
        require(final['run']['phase'] in ('failed', 'blocked') and len(final['operations']) == 2,
                'rejected task was retried or root falsely completed')
        failure = [e for e in events(final, 'operation_terminal') if e.get('execution_id') == operation['execution_id']]
        require(len(failure) == 1 and '/revision' in failure[0]['reason_summary'], 'rejection lost its diagnostic or duplicated its event')
        return {'run_id': root, 'retried_statuses': [500, 503, 408, 423, 429], 'definitive_status': 422}
    finally:
        subprocess.run(['systemctl', 'start', unit], check=True, timeout=90)
        release_all(env, env.view(root))


def run(env, capabilities):
    wait_idle(env, 0)
    identifier = env.create('restart', held_plan(capabilities))
    try:
        running_gate(env, identifier)
        command(env, identifier, 'pause')
        before = env.view(identifier)
        record = journal(env, identifier)
        durable = {key: copy.deepcopy(record['annotations'][key])
                   for key in ('layered_decision', 'layered_intent')}
        env.save('faults/durable-before', durable)
        identities = [(o['operation_id'], o['execution_id']) for o in before['operations']]
        with FrozenWindow(env):
            restart(env, 'server')
            after_server = env.view(identifier)
            require(after_server['run']['phase'] == 'paused', 'Server restart lost pause state')
            require([(o['operation_id'], o['execution_id']) for o in after_server['operations']] == identities,
                    'Server restart replaced child identities')
            def children_settled():
                view = env.view(identifier)
                release_all(env, view)
                return all(op['status'] in ('done', 'error', 'cancelled') for op in view['operations'])
            env.wait(children_settled, 180, 'paused children settle before Worker restart')
            restart(env, 'runtime')
        env.wait(lambda: env.view(identifier)['run']['phase'] == 'failed', 180, 'crashed Worker fails root')
        recovered = journal(env, identifier)
        for key, value in durable.items():
            require(recovered['annotations'][key] == value, 'restart rewrote durable decision or intent')
        status, _ = env.request('POST', f'/api/brain/runs/{identifier}/commands', {'action': 'resume'})
        require(status == 409, f'failed root resume must return 409, received {status}')
        final = env.view(identifier)
        require(final['run']['phase'] == 'failed', 'crashed root did not remain failed')
        require([(o['operation_id'], o['execution_id']) for o in final['operations']] == identities,
                'restart created duplicate executions')
        return {'run_id': identifier, 'identities': identities, 'server_continuity': True, 'worker_crash_phase': 'failed'}
    finally:
        release_all(env, env.view(identifier))
