"""Real-model steering for Agent, Operator and Team while a native barrier is held."""
from assertions import events
from environment import require
from cases.control import release_all, pump
from cases.execution_types import one_layer, node
from guidance import gates


def run(env, capabilities):
    scheduling = env.api('GET', f'/api/nodes/{env.node_id}/scheduling')
    require(scheduling['max_runs'] >= 5,
            'live steering requires capacity for four held children and one Brain activation')
    original = env.tag + '-initial-marker'
    updated = env.tag + '-human-marker'
    # Each real tool stays held until the controller observes guidance delivery.
    # A model taking longer than a fixed sleep cannot silently remove the targets.
    leaf = ('Perform this bounded timing diagnostic in your own workspace: invoke this local shell '
        f'command once: {gates.diagnostic()}. '
        'Wait until the command prints diagnostic finished; a background job ID is not completion. '
        'Do not create, modify or delete the release file; the controller releases it after guidance. '
        'Then return the latest marker requested for this task, '
        f'initially {original}. Apply later human guidance to the final marker. '
        'Only this diagnostic may create its ready file; no network or other file operations.')
    team = env.tag + '-guidance-team'
    env.api('POST', '/api/teams', {'name': team, 'captain': 'act',
                                 'members': [{'agent': 'act'}, {'agent': 'plan'}]})
    instruction = ('Collect the assigned Agent, Operator and Team reports. Their actual final results must '
        f'contain the latest marker, initially {original}. If a human supplies a new marker while they '
        'are running, convey it to all three supported running executions. Native DAG output is only a '
        'parallel timing gate; its arithmetic checks must pass and it does not need the report marker. '
        'The controller releases the native gate after it observes guidance delivery.')
    spec = one_layer('执行中的人工引导', instruction, [
        node('agent', 'builtin-agent-act', leaf), node('operator', 'builtin-operator', leaf),
        node('team', 'team-' + team, 'Ask act to perform this local diagnostic first: ' + leaf +
             ' Then ask plan to check the latest requested marker; include it in final_summary.'),
        node('hold', capabilities['hold'], 'Bind args as literal ["a + b"].')])
    root = env.create('live-steering', spec)
    try:
        def started():
            view = env.view(root)
            require(view['run']['phase'] not in ('blocked', 'failed', 'cancelled'), 'steering plan failed before guidance')
            return view if len(view['operations']) == 4 and all(op['status'] == 'running' for op in view['operations']) and gates.ready(env, view) else None
        # 240 seconds to start + the full 330-second decision window stay
        # within the first child's 600-second tool deadline.
        before = env.wait(started, 240, 'all four guidance targets admitted')
        env.api('POST', f'/api/brain/runs/{root}/inputs', {'text':
            f'最终报告标记变更为 {updated}。请立即把这个新要求传给本层仍在运行的 Agent、Operator 和 Team；'
            'Team 在下次成员发问时应用。DAG 继续运行，等待整层结束后再评估。'})
        def guided():
            view = env.view(root)
            require(view['run']['activation'] == before['run']['activation'], 'steering crossed the layer barrier')
            return view if events(view, 'guidance_processed') else None
        guidance = env.wait(guided, 330, 'real child guidance')
        actions = events(guidance, 'guidance_processed')[-1]['guidance']
        supported = {op['execution_id'] for op in before['operations'] if op['execution_kind'] in ('agent', 'operator', 'team')}
        require({action['execution_id'] for action in actions} == supported,
                'guide omitted a supported running child or targeted the DAG')
        require(all(updated in action['message'] for action in actions), 'guidance lost the human marker')
        gates.release(env, guidance)
        final = pump(env, root, 1200)
        require(final['run']['phase'] == 'completed', 'steered layer did not complete')
        for op in final['operations']:
            detail = env.detail(op)
            if op['execution_kind'] in ('agent', 'operator', 'team'):
                require(updated in str(detail['result']['scheduler_output']), 'actual child result ignored human guidance')
                env.save('guidance/' + op['execution_kind'] + '-events',
                         env.api('GET', f'/api/executions/{op["execution_id"]}/events-page'))
        return {'run_id': root, 'guided_execution_ids': sorted(supported), 'marker': updated}
    finally:
        view = env.view(root)
        gates.release(env, view)
        release_all(env, view)
