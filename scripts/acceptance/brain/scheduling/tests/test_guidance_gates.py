"""Exercise the real local tool gate, including ownership and timeout failures."""
from pathlib import Path
import json
import subprocess
import sys
import tempfile
import time
from types import SimpleNamespace
import unittest
from unittest.mock import Mock

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from guidance import gates
from guidance import real


class GuidanceGateTests(unittest.TestCase):
    def test_old_or_missing_delivery_receipt_cannot_release_current_guidance(self):
        with tempfile.TemporaryDirectory() as directory:
            env = SimpleNamespace(created=['owned'], record={'runtime_data': directory})
            path = Path(directory) / 'brain/owned/execution.json'
            path.parent.mkdir(parents=True)
            for ack in (None, 3, 4):
                annotation = {} if ack is None else {'layered_guidance_ack': ack}
                path.write_text(json.dumps({'annotations': annotation}))
                self.assertEqual(gates.delivered(env, 'owned', 4), ack == 4)
            with self.assertRaisesRegex(AssertionError, 'not owned'):
                gates.delivered(env, 'foreign', 4)

    def test_full_four_slot_node_is_rejected_before_creating_guidance_work(self):
        env = SimpleNamespace(node_id='node-owned', api=Mock(return_value={'max_runs': 4}), create=Mock())
        with self.assertRaisesRegex(AssertionError, 'four held children and one Brain activation'):
            real.run(env, {})
        env.api.assert_called_once_with('GET', '/api/nodes/node-owned/scheduling')
        env.create.assert_not_called()

    def test_actual_tools_wait_for_all_targets_and_controller_release(self):
        with tempfile.TemporaryDirectory() as directory:
            env = SimpleNamespace(created=['root'], record={'runtime_data': directory},
                                  execution_data=Mock(return_value=Path(directory)))
            operations = [{'run_id': 'root', 'execution_kind': kind, 'execution_id': kind + '-owned',
                           'status': 'running'} for kind in gates.KINDS]
            view, processes = {'operations': operations}, []
            try:
                for operation in operations:
                    path = gates.workspace(env, operation)
                    path.mkdir(parents=True)
                    processes.append(subprocess.Popen(gates.diagnostic(5), shell=True, cwd=path,
                        stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True))
                deadline = time.monotonic() + 3
                while not gates.ready(env, view) and time.monotonic() < deadline:
                    time.sleep(0.01)
                self.assertTrue(gates.ready(env, view))
                self.assertTrue(all(process.poll() is None for process in processes))
                operations[0]['status'] = 'done'
                self.assertFalse(gates.ready(env, view))
                operations[0]['status'] = 'running'
                gates.release(env, view)
                for process in processes:
                    output, error = process.communicate(timeout=3)
                    self.assertEqual((process.returncode, output, error), (0, 'diagnostic finished\n', ''))
            finally:
                gates.release(env, view)
                for process in processes:
                    process.communicate(timeout=6)

    def test_timeout_fails_without_controller_release(self):
        with tempfile.TemporaryDirectory() as directory:
            result = subprocess.run(gates.diagnostic(1), shell=True, cwd=directory,
                                    capture_output=True, text=True, timeout=3)
            self.assertNotEqual(result.returncode, 0)
            self.assertNotIn('diagnostic finished', result.stdout)

    def test_foreign_roots_dags_and_escaping_workspaces_are_refused(self):
        with tempfile.TemporaryDirectory() as directory, tempfile.TemporaryDirectory() as foreign:
            env = SimpleNamespace(created=['root'], record={'runtime_data': directory},
                                  execution_data=Mock(return_value=Path(directory)))
            operation = {'run_id': 'root', 'execution_kind': 'agent', 'execution_id': 'agent-owned'}
            for change in ({'run_id': 'foreign'}, {'execution_kind': 'dag'}, {'execution_id': '../foreign'}):
                with self.subTest(change=change), self.assertRaises(AssertionError):
                    gates.workspace(env, {**operation, **change})
            path = Path(directory) / 'agent/agent-owned'
            path.mkdir(parents=True)
            (path / 'workspace').symlink_to(foreign)
            with self.assertRaisesRegex(AssertionError, 'outside'):
                gates.workspace(env, operation)


if __name__ == '__main__':
    unittest.main()
