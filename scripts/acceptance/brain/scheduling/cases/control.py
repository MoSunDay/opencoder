"""Live pause/guidance/resume, cancellation and round-budget recovery."""
import time

from assertions import barriers, events
from environment import require
from scenario import plan


def held_plan(capabilities):
    spec = plan(capabilities)
    spec['title'] = '运行控制验收'
    spec['objective'] = 'Run the supplied arithmetic expression and finish when its two actual tests pass. '
    spec['inputs'] = {'initial_args': ['a + b']}
    spec['layers'] = [spec['layers'][1]]
    spec['nodes'] = [node for node in spec['nodes'] if node['layer_id'] == 'normal']
    for node in spec['nodes']:
        node['objective'] = 'Bind args as the literal array ["a + b"]. The controller releases any gate.'
    return spec


def running_gate(env, identifier):
    def ready():
        view = env.view(identifier)
        require(view['run']['phase'] not in ('blocked', 'failed', 'cancelled'), 'gate admission failed')
        ops = view['operations']
        return view if any(op['node_id'] == 'hold' and op['status'] == 'running' for op in ops) else None
    return env.wait(ready, 330, 'running native gate')


def release_all(env, view):
    released = []
    for op in view['operations']:
        if op['node_id'] == 'hold' and op['status'] == 'running':
            # Admission precedes container creation. The next observation retries
            # while the journal/bundle is being prepared; terminal checks stay strict.
            released.append(env.release_gate(op))
    return all(released)


def pump(env, identifier, seconds=1200):
    def completed():
        view = env.view(identifier)
        if not release_all(env, view):
            return None
        return view if view['run']['phase'] in ('completed', 'blocked', 'failed', 'cancelled') else None
    return env.wait(completed, seconds, identifier)


def command(env, identifier, action, **extra):
    return env.api('POST', f'/api/brain/runs/{identifier}/commands', {'action': action, **extra})


def run(env, capabilities):
    identifier = env.create('pause-resume', held_plan(capabilities))
    try:
        before = running_gate(env, identifier)
        command(env, identifier, 'pause')
        paused = env.view(identifier)
        require(paused['run']['phase'] == 'paused', 'pause was not persisted')
        text = '恢复后核对算术测试的实际输出，保留零值语义。'
        env.api('POST', f'/api/brain/runs/{identifier}/inputs', {'text': text})
        quiet = env.view(identifier)
        count = len(events(quiet, 'decision_started'))
        for _ in range(5):
            quiet = env.view(identifier)
            require(quiet['run']['phase'] == 'paused', 'paused human input woke execution')
            require(len(events(quiet, 'decision_started')) == count, 'paused run invoked model')
            require(any(op['node_id'] == 'hold' and op['status'] == 'running' for op in quiet['operations']),
                    'pause killed the running child')
            time.sleep(1)
        command(env, identifier, 'resume')
        def guided():
            view = env.view(identifier)
            require(view['run']['activation'] == before['run']['activation'], 'resume crossed a barrier')
            return view if events(view, 'guidance_processed') else None
        env.wait(guided, 330, 'resumed guidance')
        completed = pump(env, identifier)
        require(completed['run']['phase'] == 'completed', 'resumed run did not complete')
        barriers(completed)
    finally:
        release_all(env, env.view(identifier))
    cancelled = env.create('cancel', held_plan(capabilities))
    try:
        running_gate(env, cancelled)
        command(env, cancelled, 'cancel')
        def settled():
            view = env.view(cancelled)
            require(view['run']['phase'] == 'cancelled', 'root cancellation missing')
            return view if all(op['status'] in ('done', 'error', 'cancelled') for op in view['operations']) else None
        final = env.wait(settled, 120, 'cancel child receipts')
        require(events(final, 'cancel_requested'), 'cancellation did not propagate')
    finally:
        release_all(env, env.view(cancelled))
    return {'pause_resume_run': identifier, 'cancelled_run': cancelled}


def budget(env, capabilities):
    spec = plan(capabilities)
    spec['max_rounds'] = 1
    identifier = env.create('round-budget', spec)
    try:
        blocked = pump(env, identifier)
        require(blocked['run']['phase'] == 'blocked', 'exhausted rework budget was not blocked')
        require(blocked['run']['round'] == 1 and len(blocked['operations']) == 4, 'budget created extra work')
        count = len(blocked['operations'])
        for invalid in (1, 33):
            status, reply = env.request('POST', f'/api/brain/runs/{identifier}/commands',
                                    {'action': 'set_round_budget', 'input': {'max_rounds': invalid}})
            require(status >= 400 and 'budget must exceed current round' in reply.get('error', ''),
                    'invalid budget did not produce the budget validation error')
        unchanged = env.view(identifier)
        require(all(unchanged[key] == blocked[key] for key in ('run', 'operations', 'events')),
                'budget rejection changed persisted state')
        command(env, identifier, 'set_round_budget', input={'max_rounds': 2})
        command(env, identifier, 'resume')
        completed = pump(env, identifier)
        require(completed['run']['phase'] == 'completed' and completed['run']['round'] == 2,
                'increased budget did not permit repair and completion')
        barriers(completed)
        return {'run_id': identifier, 'blocked_operations': count, 'final_operations': len(completed['operations'])}
    finally:
        release_all(env, env.view(identifier))
