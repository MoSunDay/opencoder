"""Real-model creation, barrier, human event, business failure and repair."""
import time
import hashlib
from pathlib import Path

from assertions import closed_loop, events
from environment import require
from scenario import plan


def run(env, capabilities):
    spec = plan(capabilities)
    identifier = env.create('closed-loop', spec)
    released = set()
    quiet = guided = None
    deadline = time.monotonic() + 1200
    try:
        while time.monotonic() < deadline:
            view = env.view(identifier)
            phase = view['run']['phase']
            if phase in ('completed', 'blocked', 'failed', 'cancelled'):
                break
            operations = [op for op in view['operations'] if op['activation'] == view['run']['activation']]
            held = next((op for op in operations if op['node_id'] == 'hold' and op['status'] == 'running'), None)
            fast = next((op for op in operations if op['node_id'] == 'fast'), None)
            if held and fast and fast['status'] == 'done' and held['execution_id'] not in released:
                if quiet is None:
                    require(view['run']['round'] == 1, 'first hold occurred after a return')
                    began = time.monotonic()
                    count = len(events(view, 'decision_started'))
                    activation = view['run']['activation']
                    while time.monotonic() - began < 30:
                        current = env.view(identifier)
                        require(current['run']['activation'] == activation, 'incomplete layer advanced')
                        require(len(events(current, 'decision_started')) == count, 'idle model polling')
                        time.sleep(1)
                    quiet = {'elapsed_seconds': time.monotonic() - began,
                             'decision_count_before': count, 'decision_count_after': len(events(current, 'decision_started'))}
                    env.save('closed-loop/quiet', quiet)
                    text = '验收必须保留负数的加法语义，并核对测试使用的是同一个源码版本。当前尚未完成的测试继续等待控制器，不得跳过。'
                    reply = env.api('POST', f'/api/brain/runs/{identifier}/inputs', {'text': text})
                    require(reply['delivery'] == 'brain_event', 'human input bypassed Brain')
                    def processed():
                        snapshot = env.view(identifier)
                        require(snapshot['run']['activation'] == activation, 'guidance advanced the layer')
                        require(snapshot['run']['phase'] not in ('blocked', 'failed', 'cancelled'), 'guidance failed')
                        return snapshot if events(snapshot, 'guidance_processed') else None
                    current = env.wait(processed, 330, 'human guidance')
                    guided = {'text': text, 'before_activation': activation,
                              'after_activation': current['run']['activation'], 'after_layer': current['run']['layer']}
                    env.save('closed-loop/guidance', guided)
                if env.release_gate(held):
                    released.add(held['execution_id'])
            time.sleep(1)
        else:
            raise TimeoutError('closed-loop exceeded twenty minutes')
        details = {op['execution_id']: env.detail(op) for op in view['operations'] if op['status'] == 'done'}
        require(quiet is not None and guided is not None, 'required event observations did not occur')
        result = closed_loop(view, details, quiet, guided)
        roots = [detail['result']['artifact_root'] for detail in details.values()]
        containers = [detail['dag_context']['container_id'] for detail in details.values()]
        require(len(set(roots)) == 8 and len(set(containers)) == 8, 'child executions shared a workspace or container')
        for op in view['operations']:
            if op['node_id'] == 'prepare':
                detail = details[op['execution_id']]
                source = (Path(detail['result']['artifact_root']) / 'check/calculate.py').read_bytes()
                require(hashlib.sha256(source).hexdigest() == detail['result']['scheduler_output']['check']['revision'],
                        'archived source differs from the source that was verified')
        env.save('closed-loop/workspaces', {'artifact_roots': roots, 'container_ids': containers})
        for visit in events(view, 'layer_started'):
            detail = env.api('GET', f'/api/brain/runs/{identifier}/layered/rounds/{visit["layer"]}?activation={visit["activation"]}')
            require(detail['visit'] == visit, 'history does not match durable dispatch')
            env.save(f'closed-loop/visit-{visit["activation"]}', detail)
        return {'run_id': identifier, **result}
    finally:
        latest = env.view(identifier)
        failures = []
        for op in latest['operations']:
            if op['node_id'] == 'hold' and op['status'] == 'running':
                try:
                    require(env.release_gate(op), 'native cleanup gate failed')
                except Exception as error:
                    failures.append(str(error))
        env.save('closed-loop/gate-cleanup', {'failures': failures})
        require(not failures, 'gate cleanup failed: ' + str(failures))
