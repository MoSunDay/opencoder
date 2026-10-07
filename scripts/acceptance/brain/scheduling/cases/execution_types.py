"""Exercise all six capability kinds through the real public Brain API."""
import time

from assertions import barriers
from environment import require


def one_layer(title, instruction, nodes):
    return {'schema_version': 7, 'title': title, 'objective': instruction,
        'inputs': {}, 'max_rounds': 1,
        'layers': [{'layer_id': 'verify', 'title': title, 'task': instruction,
                    'objective': instruction, 'success_criteria': instruction}],
        'nodes': [{**node, 'layer_id': 'verify'} for node in nodes]}


def node(identifier, capability, instruction):
    return {'node_id': identifier, 'capability_id': capability,
            'title': identifier, 'objective': instruction}


def run(env, capabilities):
    marker = env.tag + '-six-kinds'
    leaf = f'Acceptance task: return the exact marker {marker} in your final result. '
    leaf += 'Do not use tools, read or modify files, or perform network operations. Finish this local task only.'
    child_id = env.tag + '-nested'
    child_plan = one_layer('嵌套能力验收', leaf, [node('leaf', 'builtin-agent-act', leaf)])
    env.api('POST', '/api/brain/plan-defs', {'id': child_id, 'version': 1,
        'created_at': int(time.time() * 1000), 'plan': child_plan, 'changelog': 'Fixed nested version'})
    team_name = env.tag + '-team'
    env.api('POST', '/api/teams', {'name': team_name, 'captain': 'act',
                                 'members': [{'agent': 'act'}, {'agent': 'plan'}]})
    todo_name = env.tag + '-todos'
    env.api('POST', '/api/todo/templates', {'name': todo_name, 'spec': {
        'schema_version': 1, 'id': todo_name, 'name': todo_name, 'objective': leaf,
        'todos': [{'id': 'echo', 'title': 'Return acceptance marker',
                   'requirement_background': 'Brain capability contract acceptance',
                   'instructions': leaf + ' Put the marker in candidate.result.', 'max_attempts': 1,
                   'acceptance': {'criteria': f'The actual result contains {marker}.'}}]}})
    nodes = [node('agent', 'builtin-agent-act', leaf), node('operator', 'builtin-operator', leaf),
             node('team', 'team-' + team_name, leaf + ' Use the Team decision format; include the marker in final_summary.'),
             node('todos', 'todos-' + todo_name + '-v1', leaf),
             node('nested', f'plan-{child_id}@1', leaf),
             node('dag', capabilities['fast'], 'Bind args as literal ["a + b"]. Return real test output.')]
    instruction = f'Collect each assigned local result. All non-DAG results must contain {marker}; '
    instruction += 'the DAG must have check.passed=true. Do not require a marker in the DAG output. '
    instruction += 'Dispatch every node exactly once, then assess the complete outputs against these criteria.'
    identifier = env.create('six-types', one_layer('六种能力验收', instruction, nodes))
    view = env.terminal(identifier, 2400)
    require(view['run']['phase'] == 'completed', 'six-kind run did not complete: ' + str(view['run'].get('error')))
    barriers(view)
    require(sorted(op['execution_kind'] for op in view['operations']) ==
            ['agent', 'brain', 'dag', 'operator', 'team', 'todos'], 'six execution kinds were not exercised exactly once')
    for op in view['operations']:
        detail = env.detail(op)
        require(detail['execution']['status'] == 'done', 'capability did not finish')
        if op['execution_kind'] == 'brain':
            nested = env.view(op['execution_id'])
            require(nested['run']['phase'] == 'completed', 'nested run did not finish')
            parent = nested['run']['parent']
            require(parent['run_id'] == identifier and parent['operation_id'] == op['operation_id'], 'nested parent differs')
            require(detail['execution']['node_id'] == env.node_id, 'nested Brain changed execution node')
            require(marker in str(nested['run']['summary']), 'nested evidence is missing')
        elif op['execution_kind'] == 'dag':
            require(detail['result']['scheduler_output']['check']['passed'] is True, 'native DAG evidence failed')
        else:
            require(marker in str(detail['result']['scheduler_output']), 'actual capability output lacks marker')
    return {'run_id': identifier, 'nested_plan': child_id, 'kinds': [op['execution_kind'] for op in view['operations']]}
