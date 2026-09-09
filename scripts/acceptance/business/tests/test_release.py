"""Release gates must reject mixed artifacts and incomplete business evidence."""
from pathlib import Path
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
sys.path.insert(0, str(Path(__file__).resolve().parents[3] / 'platform'))
from test_install_bundle import make_bundle
from validation.bundle import verify_platform
from validation.quality import acceptance_passed, quality_results


class ReleaseGate(unittest.TestCase):
    def test_all_candidate_binaries_are_verified(self):
        with tempfile.TemporaryDirectory() as raw:
            bundle = make_bundle(Path(raw), 'a')
            manifest, binaries = verify_platform(bundle)
            self.assertEqual(manifest['commit'], 'a' * 40)
            self.assertEqual(len(binaries), 4)
            with (bundle / 'bin/opencoder-agent').open('ab') as stream:
                stream.write(b'changed artifact')
            with self.assertRaisesRegex(Exception, 'checksum mismatch'):
                verify_platform(bundle)

    def test_incomplete_platform_bundle_is_rejected(self):
        with tempfile.TemporaryDirectory() as raw:
            bundle = make_bundle(Path(raw), 'a', names=('opencoder', 'opencoder-server'))
            with self.assertRaisesRegex(ValueError, 'all four'):
                verify_platform(bundle)

    def test_workflow_done_does_not_accept_unexecuted_tests_or_missing_evidence(self):
        evaluation = {'health': 'internal_failure', 'gaps': ['missing trace'],
                      'reportReady': False, 'findingCount': 0}
        regression = {'verdict': 'inconclusive', 'executions': [
            {'exitCode': 126, 'error': 'Dependency preparation failed', 'timedOut': False}]}
        quality = quality_results(evaluation, regression)
        self.assertFalse(acceptance_passed(True, True, quality))
        self.assertEqual(quality['regression-test']['dependencyPreparationFailures'], 1)
        self.assertFalse(quality['eval-diagnose']['valid'])
        self.assertFalse(quality['regression-test']['valid'])

    def test_clean_result_and_real_assertions_can_pass(self):
        evaluation = {'health': 'clean', 'gaps': [], 'reportReady': False, 'findingCount': 0}
        regression = {'verdict': 'pass', 'executions': [
            {'exitCode': 0, 'error': None, 'timedOut': False}]}
        quality = quality_results(evaluation, regression)
        self.assertTrue(acceptance_passed(True, True, quality))
        self.assertFalse(acceptance_passed(True, False, quality))
        self.assertFalse(acceptance_passed(False, True, quality))
        self.assertFalse(acceptance_passed(True, True, {}))

    def test_empty_or_timed_out_tests_cannot_pass_even_with_pass_verdict(self):
        for executions in [[], [{'exitCode': 0, 'timedOut': True}]]:
            quality = quality_results({}, {'verdict': 'pass', 'executions': executions})
            self.assertFalse(quality['regression-test']['valid'])


if __name__ == '__main__':
    unittest.main()
