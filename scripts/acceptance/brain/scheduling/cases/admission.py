"""Public schema/version validation and deployed capacity-feature checks."""
import copy
import time

from environment import require
from scenario import plan


def run(env, capabilities):
    original = plan(capabilities)
    require(env.api('POST', '/api/brain/plan-defs/validate', original) == {'valid': True}, 'valid plan rejected')
    invalid = []
    for schema in (4, 5, 6):
        value = copy.deepcopy(original)
        value['schema_version'] = schema
        invalid.append((f'old-schema-{schema}', value))
    for field in ('title', 'task', 'objective', 'success_criteria'):
        value = copy.deepcopy(original)
        value['layers'][0][field] = ''
        invalid.append(('empty-layer-' + field, value))
    for budget in (0, 33):
        value = copy.deepcopy(original)
        value['max_rounds'] = budget
        invalid.append(('round-budget-' + str(budget), value))
    value = copy.deepcopy(original)
    value['layers'] = [{**original['layers'][0], 'layer_id': 'l' + str(i)} for i in range(33)]
    value['nodes'] = [{**original['nodes'][0], 'node_id': 'n' + str(i), 'layer_id': 'l' + str(i)} for i in range(33)]
    invalid.append(('33-layers', value))
    value = copy.deepcopy(original)
    value['layers'] = [value['layers'][0]]
    value['nodes'] = [{**original['nodes'][0], 'node_id': 'n' + str(i)} for i in range(33)]
    invalid.append(('33-nodes-in-layer', value))
    value = copy.deepcopy(original)
    value['layers'] = [{**original['layers'][0], 'layer_id': 'l' + str(i)} for i in range(9)]
    value['nodes'] = [{**original['nodes'][0], 'node_id': 'n' + str(i), 'layer_id': 'l' + str(i // 32)} for i in range(257)]
    invalid.append(('257-total-nodes', value))
    for label, value in invalid:
        env.api('POST', '/api/brain/plan-defs/validate', value, expected=400)
    plan_id = env.tag + '-immutable'
    saved = {'id': plan_id, 'version': 1, 'plan': original, 'created_at': int(time.time() * 1000),
             'changelog': 'Brain scheduling acceptance'}
    env.api('POST', '/api/brain/plan-defs', saved)
    env.api('POST', '/api/brain/plan-defs', saved)
    changed = copy.deepcopy(saved)
    changed['plan']['objective'] = 'different objective'
    env.api('POST', '/api/brain/plan-defs', changed, expected=409)
    doc = env.api('GET', f'/api/brain/plan-defs/{plan_id}/versions/1')
    require(doc['plan']['objective'] == original['objective'], 'immutable version changed')
    return {'rejections': [label for label, _ in invalid], 'saved_plan': plan_id,
            'immutable_version': 1}


def capacity(env, capabilities):
    reply = env.rpc({'operation': 'brain', 'execution': {'id': env.tag + '-probe', 'kind': 'brain'},
                     'action': 'capability_probe', 'input': {'layered_request': {
                         'schema_version': 7, 'plan': plan(capabilities), 'inputs': {}}}})
    env.save('capacity/probe', reply)
    require('brain_context_budget_v1' in reply.get('features', []),
            'current Runtime does not advertise brain_context_budget_v1; the working-tree capacity rule is not deployed')
    large = plan(capabilities)
    status, response = env.request('POST', '/api/brain/runs', {
        'id': env.tag + '-capacity-overflow', 'node_id': env.node_id,
        'schema_version': 7, 'plan': large, 'inputs': {'evidence': 'x' * (1024 * 1024)}})
    require(status == 413, 'oversized context was not rejected before root creation')
    env.save('capacity/rejected', response)
    root = env.tag + '-capacity-overflow'
    status, _ = env.request('GET', f'/api/brain/runs/{root}/layered')
    require(status == 404, 'capacity rejection created a root')
    from cases.control import command, held_plan, running_gate, release_all
    identifier = env.create('guidance-capacity', held_plan(capabilities))
    try:
        running_gate(env, identifier)
        command(env, identifier, 'pause')
        rejected_at = None
        for count in range(1, 257):
            before = env.brain_rpc(identifier, 'snapshot')
            status, _ = env.request('POST', f'/api/brain/runs/{identifier}/inputs', {'text': 'x' * 4000})
            if status == 413:
                require(env.brain_rpc(identifier, 'snapshot') == before,
                        'rejected human input changed generation/events/state')
                rejected_at = count
                break
            require(status == 200, 'unexpected human capacity response')
        require(rejected_at is not None, 'cumulative human inputs bypassed capacity limit')
        return {'features': reply['features'], 'root_overflow_status': 413, 'human_input_rejected_at': rejected_at}
    finally:
        release_all(env, env.view(identifier))
