"""Native failures, bounded output evidence and explicit model stop decisions."""
import time

from assertions import events
from environment import require
from cases.control import release_all, pump
from cases.execution_types import node, one_layer


def failure_barrier(env, capabilities):
    instruction = ('Run both assigned diagnostic tasks. A deliberate execution failure is expected. '
        'Collect the entire layer before deciding; do not retry or replace either task. '
        'The required business condition is that every task succeeds. A fixed permanent diagnostic '
        'failure is not repairable by any available capability. Explain the unrecoverable failure.')
    spec = one_layer('失败屏障验收', instruction, [node('error', capabilities['error'], 'Run this native diagnostic.'),
        node('hold', capabilities['hold'], 'Bind args as literal ["a + b"].')])
    identifier = env.create('failure-barrier', spec)
    try:
        def partial():
            view = env.view(identifier)
            ops = view['operations']
            return view if any(o['status'] == 'error' for o in ops) and any(o['status'] == 'running' for o in ops) else None
        first = env.wait(partial, 330, 'one failed child and one running child')
        began = time.monotonic()
        while time.monotonic() - began < 30:
            view = env.view(identifier)
            require(len(view['operations']) == 2 and view['run']['activation'] == 1, 'failed child was retried early')
            require(not events(view, 'layer_barrier_reached'), 'error prematurely opened the layer barrier')
            require(len(events(view, 'decision_started')) == len(events(first, 'decision_started')),
                    'failed child triggered a model before sibling completion')
            time.sleep(1)
        final = pump(env, identifier)
        require(final['run']['phase'] == 'failed', 'unrecoverable failure did not produce fail decision')
        require(len(final['operations']) == 2, 'failure was automatically retried')
        require(next(op for op in final['operations'] if op['node_id'] == 'hold')['status'] == 'done',
                'the independent sibling did not finish its successful diagnostic')
        require(events(final, 'layer_barrier_reached'), 'all terminal children did not produce barrier')
        for op in final['operations']:
            env.detail(op)
        return {'run_id': identifier, 'phase': final['run']['phase'], 'quiet_seconds': time.monotonic() - began}
    finally:
        release_all(env, env.view(identifier))


def oversize(env, capabilities):
    instruction = ('Run the diagnostic exactly once. Completion requires readable, complete evidence '
        'within the capability result limit. If the fixed diagnostic cannot deliver valid evidence, '
        'stop with an explanation; never invent a successful result or retry this fixed diagnostic.')
    identifier = env.create('oversize', one_layer('超限输出验收', instruction,
        [node('large', capabilities['oversize'], 'Run the fixed native diagnostic; it intentionally emits a large result.')]))
    final = env.terminal(identifier)
    require(final['run']['phase'] in ('failed', 'blocked'), 'oversize evidence was accepted as business success')
    require(len(final['operations']) == 1 and final['operations'][0]['status'] == 'error', 'oversize output did not fail its contract')
    op = final['operations'][0]
    detail = env.detail(op)
    output = detail['result']['scheduler_output']['check']
    require(len(output['summary']) == 17000 and output['passed'] is False, 'full failing output was discarded')
    summary = env.rpc({'operation': 'brain', 'execution': {'id': op['execution_id'], 'kind': 'dag'},
                       'action': 'layered_summary', 'input': None})
    import json
    evidence = json.loads(summary['summary'])
    require(evidence.get('result_truncated') is True and 'exceeds' in evidence['error'],
            'bounded failure evidence concealed the omitted output')
    require(len(summary['summary'].encode()) <= 16384, 'failure summary exceeds 16 KiB')
    return {'run_id': identifier, 'full_summary_bytes': 17000, 'bounded_bytes': len(summary['summary'].encode())}


def blocked(env, capabilities):
    instruction = ('Calculate a signed arithmetic result from the customer-supplied expression in root input '
        'customer_expression. That required input is absent and must come from a human; no default or '
        'substitute is permitted. Do not launch a diagnostic with invented inputs. Explain which prerequisite is missing.')
    identifier = env.create('missing-prerequisite', one_layer('缺失前提验收', instruction,
        [node('waiting', capabilities['fast'], 'Use only the customer expression when supplied.')]))
    view = env.terminal(identifier)
    require(view['run']['phase'] == 'blocked' and not view['operations'], 'missing prerequisite was not blocked before dispatch')
    watermark = view['run']['last_event_seq']
    env.api('POST', f'/api/brain/runs/{identifier}/inputs', {'text': 'customer_expression 是 a + b；请绑定 args 为 ["a + b"]，现在前提已经齐备。'})
    def resumed():
        current = env.view(identifier)
        decided = any(event['seq'] > watermark for event in events(current, 'decision_started'))
        require(current['run']['phase'] not in ('failed', 'cancelled') and
                not (decided and current['run']['phase'] == 'blocked'),
                'human input was processed without recovery: ' + str(current['run'].get('error')))
        return current if current['run']['phase'] == 'completed' else None
    final = env.wait(resumed, 330, 'human input resumes blocked run')
    require(final['run']['phase'] == 'completed' and len(final['operations']) == 1, 'human input did not resume blocked run')
    result = env.detail(final['operations'][0])['result']['scheduler_output']['check']
    require(result['args'] == ['a + b'], 'resumed execution did not consume the human-supplied expression')
    return {'run_id': identifier, 'before': 'blocked', 'after': 'completed'}
