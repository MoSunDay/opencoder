"""Pin Brain admissions to a full real node and verify durable queued identities."""
from environment import require
from cases.execution_types import one_layer, node
from faults.runtime import blockers, wait_idle


def run(env, capabilities):
    wait_idle(env, 0)
    path = f'/api/nodes/{env.node_id}/scheduling'
    current = env.api('GET', path)
    original = {key: current[key] for key in ('max_runs', 'queue_order', 'workdir') if key in current}
    fixture = 'operator-' + env.tag + '-queue-gate'
    admitted = False
    roots = []
    try:
        require(not any(blockers(env).values()), 'unrelated work arrived before capacity test')
        env.save('placement/restore', {'node_id': env.node_id, 'scheduling': original})
        env.api('PUT', path, {**original, 'max_runs': 1})
        env.api('POST', '/api/executions', {'id': fixture, 'kind': 'operator', 'target': 'act',
            'node_id': env.node_id, 'input': {'prompt': 'Run this local timing diagnostic only: use a shell tool '
                'to execute python3 -c "import time; time.sleep(150); print(\'capacity gate finished\')" once. '
                'Wait for that command to finish, then report capacity gate finished. No other file, network or service operations.'}})
        admitted = True
        def saturated():
            nodes = env.api('GET', '/api/nodes')['nodes']
            local = next(n for n in nodes if n['id'] == env.node_id)
            return local if local['snapshot']['active_agent_loops'] >= 1 else None
        env.save('placement/full-node', env.wait(saturated, 120, 'capacity gate active'))
        spec = one_layer('排队身份验收', 'Run the arithmetic diagnostic and complete when check.passed is true.',
                         [node('check', capabilities['fast'], 'Bind args as literal ["a + b"].')])
        for number in range(3):
            roots.append(env.create('queued-' + str(number), spec))
        indexes = [env.api('GET', f'/api/executions/{root}/index') for root in roots]
        require(all(index['node_id'] == env.node_id for index in indexes), 'full pinned node caused reassignment')
        require(all(index['status'] == 'pending' for index in indexes), 'accepted roots started before capacity was free')
        env.save('placement/queued', indexes)
        for root in roots:
            env.wait(lambda: env.api('GET', f'/api/executions/{root}/index')['status'] != 'pending',
                     900, 'queued root starts')
            final = env.terminal(root, 900)
            require(final['run']['phase'] == 'completed' and len(final['operations']) == 1,
                    'queued root failed to run exactly once after capacity became free')
            index = env.api('GET', f'/api/executions/{root}/index')
            require(index['node_id'] == env.node_id, 'queued root changed owner')
        return {'fixture': fixture, 'queued_run_ids': roots, 'node_id': env.node_id}
    finally:
        env.api('PUT', path, original)
        if admitted:
            detail = env.api('GET', '/api/executions/' + fixture)
            if detail['execution']['status'] not in ('done', 'error', 'cancelled'):
                env.api('POST', f'/api/executions/{fixture}/commands', {'action': 'cancel'})
        restored = env.api('GET', path)
        require(all(restored[key] == value for key, value in original.items()), 'node scheduling settings were not restored')
        env.save('placement/restored', restored)
