"""Transport faults leave the owned run's decisions, budget and credentials intact."""
import copy
import json
from pathlib import Path
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import Mock, patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from faults.model import ModelFault


class ModelScopeTests(unittest.TestCase):
    def test_stopped_owned_fixture_changes_only_its_frozen_transport(self):
        with tempfile.TemporaryDirectory() as directory:
            root = 'brain-owned-fault'
            path = Path(directory) / 'brain' / root / 'execution.json'
            path.parent.mkdir(parents=True)
            config = {'model': 'provider/model', 'providers': {
                'provider': {'base_url': 'https://configured.example/v1', 'api_key': 'fixture-secret'}}}
            original = {'queue': {'config': config}, 'annotations': {
                'layered_intent': {'generation': 9},
                'layered_decision_attempt': {'attempt': 2, 'deadline_ms': 123456}}}
            path.write_text(json.dumps(original))
            env = SimpleNamespace(tag='brain-owned', created=[root],
                record={'runtime_data': directory, 'runtime_unit': 'opencoder-runtime-brain-e2e-fixture.service'},
                create=Mock(return_value=root), api=Mock(), view=Mock(return_value={'operations': []}),
                wait=lambda predicate, *_: predicate(), save=Mock())
            fault = ModelFault(env, copy.deepcopy(config))
            fault.url = 'http://127.0.0.1:9999/v1'
            with patch('faults.model.FrozenWindow'), patch('faults.model.require_restartable'), \
                    patch('faults.model.blockers', return_value={}) as blockers, \
                    patch('faults.model.subprocess.run'), \
                    patch('faults.model.subprocess.check_output', return_value=b'0'):
                self.assertEqual(fault.create('fault', {}, lambda *_: '{}'), root)
            blockers.assert_called_once_with(env, allow_frozen=True)
            expected = copy.deepcopy(original)
            expected['queue']['config']['providers']['provider']['base_url'] = fault.url
            self.assertEqual(json.loads(path.read_text()), expected)
            self.assertEqual(config['providers']['provider']['base_url'], 'https://configured.example/v1')


if __name__ == '__main__':
    unittest.main()
