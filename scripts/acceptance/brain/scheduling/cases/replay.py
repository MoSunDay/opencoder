"""Replay real terminal receipts through the deployed authenticated Runtime RPC."""
import copy

from environment import require
from cases.control import held_plan, running_gate, release_all, pump


def check_replay(env, identifier, operation):
    before = env.brain_rpc(identifier, 'snapshot')
    notice = {key: operation[key] for key in ('run_id', 'operation_id', 'execution_kind',
                                             'execution_id', 'status', 'source_sequence')}
    for sequence in (notice['source_sequence'], notice['source_sequence'] + 100):
        for status in (notice['status'], 'error'):
            reply = env.brain_rpc(identifier, 'layered_terminal',
                                  {**notice, 'source_sequence': sequence, 'status': status})
            require(reply.get('duplicate') is True, 'terminal replay was not deduplicated')
    receipt = env.brain_rpc(identifier, 'layered_receipt', {
        'operation_id': operation['operation_id'], 'reply': {'status': 422, 'body': {'error': 'late rejection'}}})
    require(receipt.get('duplicate') is True, 'late admission receipt changed a terminal operation')
    after = env.brain_rpc(identifier, 'snapshot')
    require(before == after, 'replay changed persisted root state')
    return {'execution_id': operation['execution_id'], 'source_sequence': notice['source_sequence']}


def run(env, capabilities):
    identifier = env.create('receipt-replay', held_plan(capabilities))
    try:
        running_gate(env, identifier)
        def fast_done():
            view = env.view(identifier)
            return view if any(op['node_id'] == 'fast' and op['status'] == 'done' for op in view['operations']) else None
        view = env.wait(fast_done, 120, 'fast terminal receipt')
        op = next(op for op in view['operations'] if op['node_id'] == 'fast')
        replay = check_replay(env, identifier, op)
        old = copy.deepcopy(env.brain_rpc(identifier, 'snapshot')['run'])
        context = {'schema_version': 7, 'run_id': identifier, 'generation': old['generation'] - 1,
                   'layer': old['layer'], 'total_layers': 1, 'request': {'schema_version': 7, 'plan': view['plan']},
                   'capabilities': [], 'run': old}
        require(env.brain_rpc(identifier, 'layered_context', context).get('stale') is True,
                'old-generation context was not fenced')
        require(env.brain_rpc(identifier, 'layered_block',
                {'generation': old['generation'] - 1, 'error': 'obsolete context'}).get('stale') is True,
                'old-generation block changed current state')
        completed = pump(env, identifier)
        require(completed['run']['phase'] == 'completed', 'replay prevented completion')
        check_replay(env, identifier, op)
        for action in ('resume', 'set_round_budget'):
            status, _ = env.request('POST', f'/api/brain/runs/{identifier}/commands',
                {'action': action, 'input': {'max_rounds': 10}})
            require(status >= 400, 'terminal history accepted a mutating command')
        return {'run_id': identifier, 'replayed': replay, 'stale_context_fenced': True}
    finally:
        release_all(env, env.view(identifier))
