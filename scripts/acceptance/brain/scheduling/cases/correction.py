"""Exercise persisted invalid-decision attempts and the shared five-minute deadline."""
import json
import time

from environment import require
from cases.execution_types import node, one_layer
from faults.model import ModelFault
from faults.runtime import journal, wait_idle


def spec(capabilities):
    return one_layer('非法决策防护验收', 'Run the local arithmetic check and evaluate its real output.',
                     [node('check', capabilities['fast'], 'Bind args as literal ["a + b"].')])


def baseline_config(env, capabilities, label):
    wait_idle(env, 0)
    baseline = env.create(label, spec(capabilities))
    require(env.terminal(baseline)['run']['phase'] == 'completed', 'real-model baseline failed before fault injection')
    return journal(env, baseline)['queue']['config']


def invalid_decision():
    return json.dumps({'decision': 'dispatch_layer', 'layer': 2, 'reason': 'intentional invalid first layer',
                       'assignments': [], 'assessments': {}})


def run(env, capabilities):
    with ModelFault(env, baseline_config(env, capabilities, 'correction-baseline')) as proxy:
        # No operations may be created for any rejected proposal.
        root = proxy.create('invalid-three', spec(capabilities), lambda *_: invalid_decision())
        failed = env.terminal(root, 330)
        marker = journal(env, root)['annotations']['layered_decision_attempt']
        require(failed['run']['phase'] == 'blocked' and not failed['operations'], 'illegal proposal created work')
        require(marker['attempt'] == 3 and proxy.calls[root] == 3, 'invalid decision was not limited to three attempts')
        env.save('faults/three-attempts', marker)
    return {'run_id': root, 'invalid_attempts': 3, 'created_operations': 0}


def deadline(env, capabilities):
    with ModelFault(env, baseline_config(env, capabilities, 'deadline-baseline')) as proxy:
        def delayed(_, body, closed):
            closed.wait(330)
            return invalid_decision()
        root = proxy.create('correction-deadline', spec(capabilities), delayed)
        env.wait(lambda: proxy.calls.get(root, 0) == 1, 90, 'first injected model call')
        before = journal(env, root)['annotations']['layered_decision_attempt']
        final = env.terminal(root, 360)
        after = journal(env, root)['annotations']['layered_decision_attempt']
        require(final['run']['phase'] == 'blocked' and not final['operations'], 'expired correction did not block')
        require(after['deadline_ms'] == before['deadline_ms'], 'correction reset the deadline')
        require(after['attempt'] == before['attempt'] == 1, 'deadline retried an unfinished attempt')
        require(time.time() * 1000 >= after['deadline_ms'], 'five-minute deadline was not exercised')
        require('five-minute' in final['run'].get('error', ''), 'run did not report deadline exhaustion')
        env.save('faults/deadline', {'before': before, 'after': after, 'run_id': root})
    return {'shared_five_minute_deadline': True, 'deadline_run_id': root}
