"""Valid size boundaries, fixed nested versions and placement rejection."""
import copy
import time

from environment import require
from scenario import plan
from cases.execution_types import one_layer, node
from assertions import barriers


def run(env, capabilities):
    original = plan(capabilities)
    valid = []
    for layer_count, nodes_per_layer in [(32, 1), (1, 32), (8, 32)]:
        spec = copy.deepcopy(original)
        spec['layers'] = [{**original['layers'][0], 'layer_id': f'l{i}'} for i in range(layer_count)]
        spec['nodes'] = [{**original['nodes'][0], 'node_id': f'n{i}', 'layer_id': f'l{i // nodes_per_layer}'}
                         for i in range(layer_count * nodes_per_layer)]
        env.api('POST', '/api/brain/plan-defs/validate', spec)
        valid.append({'layers': layer_count, 'nodes': layer_count * nodes_per_layer})
    spec = one_layer('固定嵌套版本', 'Run the assigned arithmetic task.',
                     [node('check', capabilities['fast'], 'Bind args as ["a + b"].')])
    saved = []
    for depth in range(4):
        identifier = env.tag + '-depth-' + str(depth)
        env.api('POST', '/api/brain/plan-defs', {'id': identifier, 'version': 1,
            'plan': spec, 'created_at': int(time.time() * 1000), 'changelog': 'Depth boundary acceptance'})
        saved.append(identifier)
        spec = one_layer('固定嵌套版本', 'Run the fixed saved child plan.',
            [node('nested', f'plan-{identifier}@1', 'Run this assigned child plan only.')])
    env.api('POST', '/api/brain/plan-defs', {'id': env.tag + '-depth-4', 'version': 1,
        'plan': spec, 'created_at': int(time.time() * 1000), 'changelog': 'Must reject depth four'}, expected=400)
    missing = one_layer('不存在的嵌套版本', 'Run fixed child version.',
                         [node('nested', f'plan-{saved[0]}@99', 'Run the assigned child.')])
    status, _ = env.request('POST', '/api/brain/runs', {'id': env.tag + '-unknown-version',
        'schema_version': 7, 'plan': missing, 'node_id': env.node_id})
    require(status == 400, 'unsaved nested version was admitted')
    nodes = env.api('GET', '/api/nodes')['nodes']
    unsuitable = next((n for n in nodes if n['online'] and 'brain' not in n['kinds']), None)
    require(unsuitable is not None, 'no online incompatible node available for explicit pin test')
    root = env.tag + '-bad-pin'
    env.created.append(root)
    env.save('owned', env.created)
    status, reply = env.request('POST', '/api/brain/runs', {'id': root, 'schema_version': 7,
        'plan': original, 'node_id': unsuitable['id']})
    require(status == 503, 'explicit incompatible node was accepted or silently replaced')
    env.save('definitions/pinned-rejection', {'node_id': unsuitable['id'], 'status': status, 'reply': reply})
    return {'valid_boundaries': valid, 'nested_depth_3_saved': saved[-1],
            'depth_4_rejected': True, 'incompatible_pin_rejected': unsuitable['id']}


def same_layer(env, capabilities):
    spec = one_layer('同层整改验收',
        'Deliver signed addition. First execute exactly the supplied baseline args. This diagnostic accepts '
        'a revised expression in args and computes new evidence from it. Only after actual failure evidence, '
        'correct the expression and verify it. Success requires the complete boundary checks to pass.',
        [node('check', capabilities['edge'], 'Initially bind args from root initial_args; derive any correction from actual diagnostics.')])
    spec['inputs'] = {'initial_args': ['abs(a) + abs(b)']}
    spec['max_rounds'] = 2
    root = env.create('same-layer-repair', spec)
    final = env.terminal(root)
    require(final['run']['phase'] == 'completed', 'same-layer repair did not complete')
    visits = barriers(final)
    require([(v['layer'], v['round']) for v in visits] == [(1, 1), (1, 2)], 'same-layer rework did not consume one new round')
    outputs = [env.detail(op)['result']['scheduler_output']['check'] for op in final['operations']]
    require(outputs[0]['passed'] is False and outputs[1]['passed'] is True, 'rework lacked negative then positive evidence')
    require(outputs[0]['revision'] != outputs[1]['revision'], 'same-layer repair reused old source')
    return {'run_id': root, 'route': [1, 1], 'rounds': 2}
