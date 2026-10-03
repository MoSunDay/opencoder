import copy
from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[2]))
from rolling.maintenance import gates


class Operations:
    def __init__(self, status):
        self.status = status

    def http(self, endpoint, path):
        return self.status


class DrainTests(unittest.TestCase):
    def status(self):
        return {'server': {'mode': 'frozen', 'inflight_admissions': 0,
                           'active_executions': 45000}, 'drained': False,
                'offline_nodes': ['historical-node'],
                'nodes': [{'node_id': 'local', 'status': 200,
                           'body': {'mode': 'frozen', 'active_runs': 0, 'owned_processes': 0}}]}

    def test_retains_offline_history_and_idle_sessions_without_blocking_backup(self):
        self.assertTrue(gates.drained(Operations(self.status()), 'server', 'local'))

    def test_inflight_work_failed_ack_and_reconnecting_external_nodes_fail_closed(self):
        base = self.status()
        mutations = [lambda s: s['server'].update(mode='open'),
                     lambda s: s['server'].update(inflight_admissions=1),
                     lambda s: s['nodes'][0].update(status=503),
                     lambda s: s['nodes'][0]['body'].update(mode='open'),
                     lambda s: s['nodes'][0]['body'].update(active_runs=1),
                     lambda s: s['nodes'][0]['body'].update(owned_processes=1),
                     lambda s: s['nodes'][0].update(node_id='external'),
                     lambda s: s['nodes'].append(copy.deepcopy(s['nodes'][0])),
                     lambda s: s.update(nodes=[]),
                     lambda s: s.update(server={})]
        for mutation in mutations:
            with self.subTest(mutation=mutation):
                status = copy.deepcopy(base)
                mutation(status)
                self.assertFalse(gates.drained(Operations(status), 'server', 'local'))


if __name__ == '__main__':
    unittest.main()
