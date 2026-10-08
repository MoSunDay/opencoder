"""Fault safety, cleanup completion, and acceptance-report integrity."""
import json
from pathlib import Path
import sqlite3
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import Mock, patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from environment import Environment
from cases.control import pump
from cases.negative import blocked
from faults.runtime import FrozenWindow, NotRun, require_restartable, restart, require_isolated
from main import CASES
from report import summarize


class GuardTests(unittest.TestCase):
    def test_fault_restore_waits_for_owned_runtime_and_public_node_readiness(self):
        inventory = {'registration': {'id': 'node-owned'}, 'snapshot': {'ready': False}}
        public = {'nodes': [{'id': 'node-owned', 'online': True, 'snapshot': {'ready': False}}]}
        env = SimpleNamespace(node_id='node-owned', runtime_url='http://isolated',
                              http=Mock(return_value=inventory), api=Mock(return_value=public))
        window = FrozenWindow(env)
        self.assertFalse(window.reopen())
        env.api.assert_not_called()
        inventory['snapshot']['ready'] = True
        self.assertFalse(window.reopen())
        public['nodes'][0]['snapshot']['ready'] = True
        self.assertTrue(window.reopen())
        env.http.side_effect = OSError('Runtime has not opened its listener')
        self.assertFalse(window.reopen())

    def test_faults_require_matching_isolated_node_units_and_data(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            record = {key + '_unit': f'opencoder-{key}-brain-e2e-fixture.service' for key in ('server', 'host', 'runtime')}
            record['runtime_data'] = str(root / 'runtime')
            env = SimpleNamespace(settings=SimpleNamespace(state_dir=root), node_id='node-isolated', record=record)
            with self.assertRaises(NotRun):
                require_isolated(env)
            (root / 'isolation.json').write_text(json.dumps({'root': directory, 'node_id': env.node_id, 'record': record}))
            require_isolated(env)
            env.node_id = 'node-production'
            with self.assertRaisesRegex(AssertionError, 'identity mismatch'):
                require_isolated(env)
            env.node_id = 'node-isolated'
            env.record['runtime_unit'] = 'opencoder-runtime-production.service'
            with self.assertRaisesRegex(AssertionError, 'outside the isolated stack'):
                require_isolated(env)

    def test_decision_evidence_excludes_activation_configuration(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            activation = root / 'runtime/brain/owned/activations/4'
            activation.mkdir(parents=True)
            (activation / 'decision.json').write_text('{"decision":"dispatch_layer","layer":0}')
            (activation / 'context.json').write_text(json.dumps({'layer': 0, 'total_layers': 1,
                'run': {'layer': 0, 'error': 'unknown target layer'},
                'config': {'api_key': 'must-not-enter-evidence'},
                'capabilities': [{'definition': {'token': 'must-not-enter-evidence'}}]}))
            env = Environment.__new__(Environment)
            env.created, env.output = ['owned'], root / 'evidence'
            env.record = {'runtime_data': str(root / 'runtime')}
            env.capture_decisions('owned')
            saved = (env.output / 'decisions/owned/4.json').read_text()
            self.assertNotIn('must-not-enter-evidence', saved)
            self.assertEqual(json.loads(saved)['decision']['layer'], 0)
            self.assertEqual(json.loads(saved)['run']['error'], 'unknown target layer')

    def test_human_recovery_reports_a_new_rejection_without_rejecting_the_old_snapshot(self):
        before = {'run': {'phase': 'blocked', 'last_event_seq': 3}, 'operations': [],
                  'events': [{'seq': 2, 'event_type': 'decision_started'}]}
        rejected = {'run': {'phase': 'blocked', 'last_event_seq': 6, 'error': 'unknown target layer'},
                    'operations': [], 'events': [*before['events'], {'seq': 5, 'event_type': 'decision_started'}]}
        views = iter([before, rejected])
        def wait(predicate, *_):
            self.assertIsNone(predicate(), 'an old blocked snapshot does not prove the human event was rejected')
            return predicate()
        env = SimpleNamespace(create=lambda *_: 'owned', terminal=lambda _: before, api=lambda *_: {},
                              view=lambda _: next(views), wait=wait)
        with self.assertRaisesRegex(AssertionError, 'human input was processed without recovery: unknown target layer'):
            blocked(env, {'fast': 'fixture'})

    def test_running_admission_waits_for_native_gate_and_actual_terminal(self):
        running = {'run': {'phase': 'waiting'}, 'operations': [{'node_id': 'hold', 'status': 'running'}]}
        finished = {'run': {'phase': 'completed'}, 'operations': [{'node_id': 'hold', 'status': 'done'}]}
        views, readiness = iter([running, running, finished]), iter([False, True])
        def wait(predicate, *_):
            self.assertIsNone(predicate(), 'Running does not prove the container gate is available')
            self.assertIsNone(predicate(), 'releasing a gate does not prove execution completion')
            return predicate()
        env = SimpleNamespace(view=lambda _: next(views), release_gate=lambda _: next(readiness), wait=wait)
        self.assertEqual(pump(env, 'owned'), finished)

    def test_running_host_reservation_prevents_kill_without_changing_database(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            database = root / 'host.db'
            with sqlite3.connect(database) as connection:
                connection.execute('CREATE TABLE capacity_queue (runtime_id TEXT, phase TEXT)')
                connection.executemany('INSERT INTO capacity_queue VALUES (?,?)',
                                       [('ours', 'running'), ('ours', 'waiting'), ('other', 'running')])
            (root / 'host-binding.json').write_text(json.dumps({'database': str(database), 'runtime_id': 'ours'}))
            env = SimpleNamespace(record={'runtime_data': directory, 'runtime_unit': 'test-runtime.service'})
            before = database.read_bytes()
            with patch('faults.runtime.blockers', return_value={}), \
                    patch('faults.runtime.subprocess.check_output', return_value=b'123'), \
                    patch('faults.runtime.subprocess.run') as mutate:
                with self.assertRaisesRegex(NotRun, '1 running Host reservations'):
                    restart(env, 'runtime')
                mutate.assert_not_called()
            self.assertEqual(database.read_bytes(), before)
            with sqlite3.connect(database) as connection:
                connection.execute("UPDATE capacity_queue SET phase='done' WHERE runtime_id='ours'")
            before = database.read_bytes()
            require_restartable(env)
            self.assertEqual(database.read_bytes(), before)

    def test_cleanup_uses_owned_runtime_and_waits_for_child_receipts(self):
        env = Environment.__new__(Environment)
        env.created, env.runtime_url = ['owned'], 'http://runtime'
        views = iter([
            {'run': {'phase': 'waiting'}, 'operations': [{'status': 'running'}]},
            {'run': {'phase': 'cancelled'}, 'operations': [{'status': 'running'}]},
            {'run': {'phase': 'cancelled'}, 'operations': [{'status': 'cancelled'}]},
        ])
        receipts, requests, commands = [], [], []
        def request(method, path, body, base):
            requests.append((method, path, body, base))
            return 200, {'status': 200, 'body': next(views)}
        def wait(predicate, *_):
            self.assertIsNone(predicate(), 'cancelled parent alone does not prove child cleanup')
            return predicate()
        env.request, env.wait = request, wait
        env.capture_decisions = lambda _: None
        env.brain_rpc = lambda *args: commands.append(args)
        env.save = lambda _, value: receipts.extend(value)
        env.cleanup()
        self.assertEqual(commands, [('owned', 'cancel')])
        self.assertTrue(receipts[0]['children_settled'])
        self.assertTrue(all(row[2]['execution']['id'] == 'owned' for row in requests))
        with self.assertRaisesRegex(AssertionError, 'not owned'):
            env.cleanup_snapshot('another-task')

    def test_cleanup_failure_is_not_reported_as_success(self):
        env = Environment.__new__(Environment)
        env.created = ['owned']
        env.cleanup_snapshot = lambda _: {'run': {'phase': 'cancelled'}, 'operations': [{'status': 'running'}]}
        env.capture_decisions = lambda _: None
        env.wait = lambda *_: (_ for _ in ()).throw(TimeoutError('child still running'))
        receipts = []
        env.save = lambda _, value: receipts.extend(value)
        with self.assertRaisesRegex(AssertionError, 'cleanup incomplete'):
            env.cleanup()
        self.assertIn('child still running', receipts[0]['error'])

    def test_report_refuses_mixed_commits_and_keeps_latest_failure(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            first, second = root / 'first', root / 'second'
            first.mkdir()
            second.mkdir()
            report = {'commit': 'aaa', 'result': 'PASS',
                      'cases': [{'case': name, 'result': 'PASS'} for name in CASES]}
            (first / 'result.json').write_text(json.dumps(report))
            self.assertEqual(summarize([first])['result'], 'PASS')
            report['result'] = 'RUNNING'
            (second / 'result.json').write_text(json.dumps(report))
            self.assertEqual(summarize([first, second])['result'], 'FAIL')
            report['result'] = 'PASS'
            report['commit'] = 'bbb'
            (second / 'result.json').write_text(json.dumps(report))
            self.assertEqual(summarize([first, second])['result'], 'FAIL')
            report['commit'] = 'aaa'
            report['cases'] = [{'case': 'closed-loop', 'result': 'FAIL', 'error': 'live failure'}]
            (second / 'result.json').write_text(json.dumps(report))
            summary = summarize([first, second])
            self.assertEqual(summary['result'], 'FAIL')
            case = next(row for row in summary['cases'] if row['case'] == 'closed-loop')
            self.assertEqual(case['latest']['result'], 'FAIL')
            self.assertEqual(case['last_success']['result'], 'PASS')
            report['cases'] = [{'case': 'closed-loop', 'result': 'PASS'}]
            report['error'] = 'public readiness failed'
            (second / 'result.json').write_text(json.dumps(report))
            self.assertEqual(summarize([first, second])['result'], 'FAIL')


if __name__ == '__main__':
    unittest.main()
