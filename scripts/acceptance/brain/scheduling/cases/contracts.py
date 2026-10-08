"""Live capability input/output contracts and immutable admission snapshots."""
import copy
import json
from pathlib import Path

from assertions import barriers, events
from environment import require
from cases.control import command, held_plan, pump, running_gate, release_all
from cases.execution_types import node, one_layer
from faults.runtime import journal


def capability(env, target, required_inputs=(), required_outputs=()):
    created = env.api('POST', '/api/brain/capabilities', {
        'capability_type': 'acceptance', 'summary': env.tag + ' explicit input/output contract',
        'input_desc': 'Bind args as ["a + b"]. Preserve probe exactly, including false, zero, or an empty array.',
        'output_desc': 'Actual native arithmetic evidence in check.', 'eng_inputs': ['Verify bounded native evidence']})
    identifier = created['capability']['id']
    binding = {'kind': 'dag', 'target': target.removeprefix('dag-'),
               'required_inputs': list(required_inputs), 'required_outputs': list(required_outputs)}
    env.api('PUT', f'/api/brain/capabilities/{identifier}/target', binding)
    return identifier, binding


def run(env, capabilities):
    cap, binding = capability(env, capabilities['fast'], ['args', 'probe'], ['check'])
    instructions = 'Run each probe exactly once. Success requires all native check.passed fields to be true.'
    values = {'false': False, 'zero': 0, 'empty-array': []}
    nodes = [node(name, cap, f'Bind probe from root input {name} exactly and args as literal ["a + b"].')
             for name in values]
    identifier = env.create('valid-inputs', one_layer('有效空值语义', instructions, nodes), values)
    final = env.terminal(identifier)
    require(final['run']['phase'] == 'completed', 'false/zero/empty-array inputs were rejected')
    barriers(final)
    for op in final['operations']:
        detail = env.detail(op)
        require(detail['execution']['status'] == 'done', 'valid input did not complete')
        path = Path(env.record['runtime_data']) / 'dag' / op['execution_id'] / 'execution.json'
        bound = json.loads(path.read_text())['assignment']['request']['input']['layered_inputs']['probe']
        require(type(bound) is type(values[op['node_id']]) and bound == values[op['node_id']],
                'input resolution changed false/zero/empty-array semantics')
    reference = detail['result']['scheduler_artifacts'][0]
    artifact_root = env.create('artifact-input', one_layer('登记产物绑定', instructions,
        [node('artifact', cap, 'Bind probe using artifact reference source, and args as literal ["a + b"].')]),
        artifacts={'source': reference})
    artifact_view = env.terminal(artifact_root)
    require(artifact_view['run']['phase'] == 'completed', 'registered artifact binding failed')
    artifact_detail = env.detail(artifact_view['operations'][0])
    supplied = artifact_detail['request']['input']
    require(supplied['bindings']['probe'] == {'kind': 'artifact', 'reference': 'source'} and
            supplied['layered_inputs']['probe'] == reference, 'artifact binding did not resolve the registered reference')
    missing_pointer = env.rpc({'operation': 'brain', 'execution': {'id': artifact_detail['execution']['id'], 'kind': 'dag'},
        'action': 'layered_output', 'input': {'path': '/check/missing'}}, expected=422)
    require('/check/missing' in str(missing_pointer), 'missing JSON pointer lacks actionable diagnostics')
    failures = []
    for name, value in [('null', None), ('blank', '   ')]:
        instruction = ('This diagnostic must use the exact raw probe from root input raw, without a substitute. '
            'Attempt the diagnostic once; if the supplied value violates the contract, explain the missing prerequisite '
            'and stop. Never invent or repair the caller input.')
        root = env.create('invalid-' + name, one_layer('拒绝非法输入', instruction,
            [node('probe', cap, 'Bind probe using root raw exactly; bind args as ["a + b"].')]), {'raw': value})
        view = env.terminal(root)
        require(view['run']['phase'] in ('blocked', 'failed'), 'invalid input was accepted')
        require(all(op['status'] == 'error' for op in view['operations']), 'invalid input produced a successful child')
        if view['operations']:
            require(len(view['operations']) == 1, 'definitive rejection retried the task')
            require(any('422' in (event.get('reason_summary') or '') for event in events(view, 'operation_terminal')),
                    'explicit admission failure lost its status/diagnostic')
        failures.append({'run_id': root, 'created_operations': len(view['operations'])})
    missing, _ = capability(env, capabilities['missing'], [], ['must_exist'])
    root = env.create('missing-output', one_layer('必填输出验收',
        'Run the fixed diagnostic once. A missing required output is an unrecoverable diagnostic failure; explain and stop.',
        [node('output', missing, 'Run the supplied fixed binary.')]))
    view = env.terminal(root)
    require(view['run']['phase'] in ('failed', 'blocked') and len(view['operations']) == 1,
            'missing output was accepted or retried')
    require(view['operations'][0]['status'] == 'error', 'required output contract did not fail execution')
    env.detail(view['operations'][0])
    return {'valid_input_run': identifier, 'artifact_run': artifact_root, 'invalid_inputs': failures,
            'missing_output_run': root, 'capability': cap}


def frozen(env, capabilities):
    cap, binding = capability(env, capabilities['hold'], ['args'], ['check'])
    spec = held_plan(capabilities)
    spec['nodes'] = [node('hold', cap, 'Bind args as literal ["a + b"].') | {'layer_id': 'normal'}]
    spec['layers'][0]['success_criteria'] = 'The native check.passed field is true.'
    root = env.create('frozen-contract', spec)
    try:
        running_gate(env, root)
        command(env, root, 'pause')
        before = copy.deepcopy(journal(env, root)['assignment']['request']['input']['frozen_capabilities'])
        env.api('PUT', f'/api/brain/capabilities/{cap}/target', {**binding, 'required_outputs': ['new_required_field']})
        require(journal(env, root)['assignment']['request']['input']['frozen_capabilities'] == before,
                'editing library mutated admitted capability contract')
        # Repeat the original public request: receipt replay must not refreeze the changed library.
        request = json.loads((env.output / ('requests/' + root + '.json')).read_text())
        env.api('POST', '/api/brain/runs', request, expected=202)
        changed = copy.deepcopy(request)
        changed['plan']['objective'] += ' changed'
        env.api('POST', '/api/brain/runs', changed, expected=409)
        command(env, root, 'resume')
        final = pump(env, root)
        require(final['run']['phase'] == 'completed' and len(final['operations']) == 1,
                'changed library affected a frozen run or duplicated admission')
        return {'run_id': root, 'capability': cap, 'frozen_required_outputs': ['check']}
    finally:
        release_all(env, env.view(root))
