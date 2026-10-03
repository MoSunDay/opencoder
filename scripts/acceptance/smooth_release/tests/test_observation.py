import json
from pathlib import Path
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import Mock, patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from domain_checks.observation import observe


class ObservationTests(unittest.TestCase):
    def exercise(self, directory, verify):
        clock = SimpleNamespace(now=0)
        def sleep(seconds):
            clock.now += seconds
        def api(*args):
            clock.now += 1
        env = SimpleNamespace(root=Path(directory), api=Mock(side_effect=api))
        with patch('domain_checks.observation.time.monotonic', side_effect=lambda: clock.now), \
             patch('domain_checks.observation.time.sleep', side_effect=sleep), \
             patch('domain_checks.observation.ontology.verify', side_effect=verify):
            result = observe(env, {}, {'steps': []}, 31, lambda *args: True,
                             lambda predicate, description: predicate())
        return result, env

    def test_observation_requires_the_entire_duration_and_runs_real_submissions(self):
        with tempfile.TemporaryDirectory() as directory:
            result, env = self.exercise(directory, lambda *args: None)
            self.assertTrue(result['passed'])
            self.assertGreaterEqual(result['elapsed_seconds'], 31)
            self.assertEqual(len(result['samples']), 2)
            self.assertEqual(env.api.call_count, 2)
            for call in env.api.call_args_list:
                self.assertEqual(call.args[:2], ('/api/executions', 'POST'))
                self.assertEqual(call.args[2]['kind'], 'dag')
            self.assertEqual(json.loads((Path(directory)/'observation.json').read_text()), result)

    def test_domain_failure_keeps_the_receipt_unsuccessful(self):
        calls = 0
        def verify(*args):
            nonlocal calls
            calls += 1
            if calls == 2:
                raise RuntimeError('NFS or ontology data drifted')
        with tempfile.TemporaryDirectory() as directory:
            with self.assertRaisesRegex(RuntimeError, 'drifted'):
                self.exercise(directory, verify)
            self.assertFalse(json.loads((Path(directory)/'observation.json').read_text())['passed'])


if __name__ == '__main__':
    unittest.main()
