"""Pure evidence checks: never infer business success from execution status."""
import hashlib

from environment import require


def events(view, kind):
    return [event for event in view['events'] if event['event_type'] == kind]


def barriers(view):
    sequence = [event['seq'] for event in view['events']]
    require(sequence == sorted(set(sequence)), 'duplicate or unordered durable events')
    visits = events(view, 'layer_started')
    require(visits and visits[0]['layer'] == 1, 'first dispatch must target layer one')
    identifiers = [op['execution_id'] for op in view['operations']]
    require(len(identifiers) == len(set(identifiers)), 'execution identity reused')
    for visit in visits:
        ops = [op for op in view['operations'] if op['activation'] == visit['activation']]
        layer_id = view['plan']['layers'][visit['layer'] - 1]['layer_id']
        expected = sorted(node['node_id'] for node in view['plan']['nodes'] if node['layer_id'] == layer_id)
        require(sorted(op['node_id'] for op in ops) == expected, 'layer dispatch omitted or duplicated a node')
    for prior, following in zip(visits, visits[1:]):
        barrier = next((e for e in events(view, 'layer_barrier_reached')
                        if e['activation'] == prior['activation']), None)
        require(barrier and barrier['seq'] < following['seq'], 'dispatch crossed unfinished barrier')
        for op in [op for op in view['operations'] if op['activation'] == prior['activation']]:
            require(any(e.get('execution_id') == op['execution_id'] and e['seq'] < barrier['seq']
                        for e in events(view, 'operation_terminal')), 'terminal receipt missing before barrier')
        if following['layer'] > prior['layer']:
            require(following['layer'] == prior['layer'] + 1, 'forward dispatch skipped a layer')
            require(following['round'] == prior['round'], 'forward dispatch changed the round')
        else:
            require(following['round'] == prior['round'] + 1, 'return did not start a new round')
            require(bool((following.get('reflection') or '').strip()), 'return lacks reflection')
    if view.get('run', {}).get('phase') == 'completed':
        last = visits[-1]
        barrier = next((e for e in events(view, 'layer_barrier_reached')
                        if e['activation'] == last['activation']), None)
        completed = events(view, 'run_completed')
        require(barrier and len(completed) == 1 and barrier['seq'] < completed[0]['seq'],
                'completion crossed unfinished final barrier')
        for op in [o for o in view['operations'] if o['activation'] == last['activation']]:
            require(op['status'] == 'done' and any(e.get('execution_id') == op['execution_id']
                    and e['seq'] < barrier['seq'] for e in events(view, 'operation_terminal')),
                    'completion lacks successful final terminal evidence')
        require(last['layer'] == len(view['plan']['layers']) and
                view['run']['valid_layers'] == len(view['plan']['layers']), 'completion used invalidated layer results')
    return visits


def output(detail):
    require(detail['execution']['status'] == 'done', 'native child did not complete successfully')
    result = detail['result']['scheduler_output']['check']
    require(hashlib.sha256(result['source'].encode()).hexdigest() == result['revision'],
            'source revision does not match its content')
    return result


def closed_loop(view, details, quiet, guided):
    require(view['run']['phase'] == 'completed', 'Brain did not complete: ' + str(view['run'].get('error')))
    visits = barriers(view)
    require([(v['round'], v['layer']) for v in visits] ==
            [(1, 1), (1, 2), (1, 3), (2, 1), (2, 2), (2, 3)], 'unexpected evidence-driven route')
    require(len(view['operations']) == 8, 'expected eight native operations')
    records = {(op['round'], op['node_id']): output(details[op['execution_id']]) for op in view['operations']}
    first, fixed = records[1, 'prepare'], records[2, 'prepare']
    require(first['args'] == ['abs(a) + abs(b)'], 'baseline was modified before actual tests')
    require(first['revision'] != fixed['revision'], 'repair did not change source')
    require(records[1, 'edge']['passed'] is False and records[1, 'edge']['failures'],
            'first business failure was not observed')
    require(records[2, 'edge']['passed'] is True, 'repaired source still fails')
    for round_number, prepared in [(1, first), (2, fixed)]:
        for node in ('fast', 'hold', 'edge'):
            require(records[round_number, node]['revision'] == prepared['revision'],
                    'test used an old or unrelated revision')
    failed = [e for e in events(view, 'milestones_assessed')
              if e.get('assessments', {}).get('boundary', {}).get('met') is False]
    require(failed, 'business failure was not assessed as unmet')
    require(quiet['elapsed_seconds'] >= 30 and quiet['decision_count_before'] == quiet['decision_count_after'],
            'waiting period called the model without an event')
    require(guided['after_activation'] == guided['before_activation'] and guided['after_layer'] == 2,
            'human guidance crossed the unfinished barrier')
    human = next(e for e in events(view, 'human_input') if e.get('user_input') == guided['text'])
    guidance = next(e for e in events(view, 'guidance_processed') if e['seq'] > human['seq'])
    require(any(human['seq'] < e['seq'] < guidance['seq'] for e in events(view, 'decision_started')),
            'human input did not precede its decision')
    require(events(view, 'run_created')[0]['seq'] < events(view, 'decision_started')[0]['seq'] < visits[0]['seq'],
            'run creation did not trigger the initial decision')
    return {'route': [v['layer'] for v in visits], 'rounds': 2, 'operations': 8,
            'baseline_revision': first['revision'], 'repaired_revision': fixed['revision']}
