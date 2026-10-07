"""Live release checks submit frozen DAG definitions through every phase."""
import json
from pathlib import Path
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import Mock, patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from live import observe
from rolling.probes import spec
from transitions import execute


class LiveProbeTests(unittest.TestCase):
    def observe(self, root, ready):
        clock = SimpleNamespace(now=0)
        definition = spec('release-probe-frozen@v1')

        def api(path, method='GET', body=None):
            if method == 'POST':
                clock.now += 1
            return ready

        env = SimpleNamespace(api=Mock(side_effect=api), settings=object(),
                              completed=Mock(return_value=True),
                              wait=lambda predicate, seconds: predicate())
        with patch('live.time.monotonic', side_effect=lambda: clock.now), \
             patch('live.time.sleep'), patch('live.probes.resources'):
            result = observe(env, root, 'candidate', 1, definition)
        return result, env, definition

    def test_observation_submits_the_frozen_native_definition(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            samples, env, definition = self.observe(root, {'mode': 'open', 'ready_nodes': 1})
            submission = env.api.call_args_list[0].args
            self.assertEqual(submission, ('/api/executions', 'POST', {
                'id': 'dag-candidate-observe-0', 'kind': 'dag',
                'input': {'definition': definition}}))
            env.completed.assert_called_once_with('dag-candidate-observe-0')
            self.assertEqual(len(samples), 1)
            self.assertEqual(json.loads((root / 'observation.json').read_text()), samples)

    def test_observation_rejects_unavailable_admission_without_a_success_sample(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            with self.assertRaisesRegex(AssertionError, 'scheduling became unavailable'):
                self.observe(root, {'mode': 'draining', 'ready_nodes': 0})
            self.assertFalse((root / 'observation.json').exists())

    def test_signal_roundtrip_keeps_definitions_separate_from_submission_ids(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            runtime = root / 'runtime'
            initial = {'id': 'candidate', 'runtime_data': str(runtime),
                       'runtime_unit': 'runtime.service',
                       'server_unit': 'server.service', 'host_unit': 'host.service'}
            active = {**initial, 'server_unit': 'server-new.service', 'host_unit': 'host-new.service'}
            states = [{'current': 'candidate', 'releases': {'candidate': record}}
                      for record in [initial, initial, active]]
            submissions = []

            def api(path, method='GET', body=None):
                if method == 'POST':
                    submissions.append(body)
                    run = runtime / 'dag' / body['id']
                    run.mkdir(parents=True)
                    (run / 'execution.json').write_text('{}')
                return {'dag_steps': {'running': 1}}

            env = SimpleNamespace(settings=SimpleNamespace(state_dir=root), api=api,
                                  submit_initial=Mock(), completed=Mock(return_value=True),
                                  wait=lambda predicate, seconds: predicate())
            args = SimpleNamespace(config=root / 'config.json', bundle=root / 'bundle',
                                   signal=True, signal_roundtrip=True, current_roundtrip=False)
            continuity = Mock()
            with patch('transitions.Journal', side_effect=[SimpleNamespace(data=s) for s in states]), \
                 patch('transitions.publish_hold', return_value='hold@v1'), \
                 patch('transitions.probes.publish_probe', return_value='frozen@v1'), \
                 patch('transitions.subprocess.check_output', return_value=b'123'), \
                 patch('transitions.subprocess.run') as run, \
                 patch('transitions.release_native_gate') as release:
                execute(args, root, env, continuity)
            self.assertEqual(len(submissions), 2)
            self.assertNotEqual(submissions[0]['id'], submissions[1]['id'])
            for submission in submissions:
                self.assertEqual(submission['input']['definition'], spec('frozen@v1'))
            self.assertEqual(run.call_count, 3)
            self.assertEqual(continuity.call_count, 3)
            release.assert_called_once_with(runtime, 'dag-' + root.name + '-new-hold')


if __name__ == '__main__':
    unittest.main()
