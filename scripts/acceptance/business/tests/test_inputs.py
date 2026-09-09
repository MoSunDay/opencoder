"""Input overrides and branch evidence must fail before accepting invented context."""
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch
import json
import sys
import tempfile
import unittest
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from common import git
from main import prepare_environment
from snapshots import branch_provenance


class InputEvidence(unittest.TestCase):
    def test_invalid_override_cannot_start_a_fixture_or_prepare_platform(self):
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            request = root / 'request.json'
            request.write_text(json.dumps([]))
            args = SimpleNamespace(evaluation_request=request, regression_fixture=None, scenario='positive')
            with patch('validation.bundle.verify_platform') as verify, patch('scenarios.positive.prepare') as prepare:
                with self.assertRaisesRegex(ValueError, 'JSON object'):
                    prepare_environment(root, args)
                verify.assert_not_called()
                prepare.assert_not_called()

    def test_metrics_override_cannot_silently_modify_positive_scenario(self):
        args = SimpleNamespace(evaluation_request=None, regression_fixture='metricw-offline', scenario='positive')
        with patch('validation.bundle.verify_platform') as verify:
            with self.assertRaisesRegex(ValueError, 'historical Go'):
                prepare_environment(Path('/unused'), args)
            verify.assert_not_called()

    def test_provenance_uses_real_git_ancestry_and_rejects_wrong_branch(self):
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            git(root, 'init', '-q')
            def commit(value):
                (root / 'source.txt').write_text(value)
                git(root, 'add', 'source.txt')
                git(root, '-c', 'user.name=Acceptance', '-c', 'user.email=acceptance@example.test',
                    'commit', '-qm', value)
                return git(root, 'rev-parse', 'HEAD')
            baseline = commit('baseline')
            target = commit('target')
            head = commit('head')
            git(root, 'update-ref', 'refs/remotes/origin/master', head)
            proof = branch_provenance(root, baseline, target)
            self.assertEqual(proof['branchHead'], head)
            self.assertEqual(proof['commit'], target)
            self.assertEqual(proof['baseCommit'], baseline)
            self.assertIn('does not establish historical', proof['scope'])
            with self.assertRaises(RuntimeError):
                branch_provenance(root, head, target)
            git(root, 'update-ref', 'refs/remotes/origin/master', baseline)
            with self.assertRaises(RuntimeError):
                branch_provenance(root, baseline, target)


if __name__ == '__main__':
    unittest.main()
