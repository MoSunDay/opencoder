from contextlib import ExitStack
from pathlib import Path
from types import SimpleNamespace
import json
import sys
import tempfile
import unittest
from unittest.mock import Mock, patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import native


class NativeCandidate(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.home = Path(self.tmp.name)
        self.source = self.home / 'case.json'
        self.case = {'case_id': 'music', 'turns': [{'turn': 1, 'prompt': 'original'}]}
        self.source.write_text(json.dumps(self.case))
        self.args = SimpleNamespace(home=self.home, case_source=self.source,
            case_id=['music'], concurrency=1, timeout_ms=900000, run_id=None,
            asset_root=None, machine=None, detach=True, full_evidence=False,
            inspect_ui=True)
        self.context = {'assignment': {'case_ids': ['music']}, 'input': {
            'case_source': str(self.source),
            'candidate': {'manifest_path': '/frozen/manifest.json', 'sha256': 'a' * 64}}}
        self.root = self.home / 'runs' / 'owned'
        self.stack = self.enterContext(ExitStack())
        self.stack.enter_context(patch('controller.device.client.context', return_value=self.context))
        self.stack.enter_context(patch('controller.device.ownership.native_id', return_value='owned'))

    def prior(self):
        (self.root / 'sources/music').mkdir(parents=True)
        (self.root / 'sources/music/case.json').write_text(json.dumps(self.case))
        (self.root / 'run.json').write_text(json.dumps({'candidate': self.context['input']['candidate']}))

    def test_no_candidate_rejected_before_registration(self):
        self.context['input'].pop('candidate')
        with patch.object(native, 'reserve') as reserve:
            with self.assertRaisesRegex(ValueError, 'requires a frozen'):
                native.launch_owned(self.args, {})
            reserve.assert_not_called()
        self.assertFalse(self.root.exists())

    def test_changed_candidate_never_resumes_or_resubmits(self):
        self.prior()
        self.context['input']['candidate'] = {'manifest_path': '/different', 'sha256': 'b' * 64}
        with patch.object(native, 'resume_owned') as resume:
            with self.assertRaisesRegex(ValueError, 'changed its candidate'):
                native.launch_owned(self.args, {})
            resume.assert_not_called()

    def test_same_candidate_resumes_original_work_only(self):
        self.prior()
        with patch.object(native, 'resume_owned', return_value={'status': 'original'}) as resume:
            with patch.object(native, 'reserve') as reserve:
                self.assertEqual(native.launch_owned(self.args, {}), {'status': 'original'})
                reserve.assert_not_called()
            resume.assert_called_once_with(self.home, {}, 'owned')

    def prepare(self, deploy_error=None):
        bundle = self.home / 'launcher.mjs'; bundle.write_text('original launcher')
        self.stack.enter_context(patch('controller.device.ownership.assert_next_case'))
        self.stack.enter_context(patch.object(native, 'freeze', return_value=[self.case]))
        self.stack.enter_context(patch.object(native, 'launcher', return_value=bundle))
        self.stack.enter_context(patch.object(native.subprocess, 'run', return_value=SimpleNamespace(returncode=0, stdout='', stderr='')))
        self.stack.enter_context(patch.object(native, 'reserve', return_value=({'name':'win-19'}, {'app_path':r'C:\registered\app.exe'})))
        driver = Mock(); driver.json.return_value = {'submitted': True}
        self.stack.enter_context(patch.object(native, 'CandidateDriver', return_value=driver))
        events = []
        self.stack.enter_context(patch('controller.inspection.occupancy.assert_idle', side_effect=lambda *a: events.append('idle')))
        def deploy(*args):
            events.append('deploy')
            if deploy_error: raise deploy_error
            return {'app_path':r'C:\candidate\app.exe', 'app_commit':'final'}
        self.stack.enter_context(patch('candidate.deploy_candidate', side_effect=deploy))
        self.stack.enter_context(patch.object(native, 'payload', return_value=bundle))
        return driver, events

    def test_occupancy_before_deploy_and_spec_uses_candidate(self):
        driver, events = self.prepare()
        result = native.launch_owned(self.args, {'guest_root':r'C:\runs','host_ready_timeout_s':90})
        self.assertEqual(events, ['idle','deploy'])
        self.assertEqual(result['status'], 'submitted')
        spec = json.loads((self.root/'spec.json').read_text())
        self.assertEqual(spec['app_path'],r'C:\candidate\app.exe')
        self.assertEqual(spec['app_commit'],'final')
        self.assertTrue(json.loads((self.root/'run.json').read_text())['task_start_attempted'])

    def test_failed_deployment_never_starts_business_and_completes_cleanup(self):
        driver, events = self.prepare(ValueError('bad archive'))
        with patch('controller.transport.network.stop') as stop, patch.object(native,'release') as release:
            with self.assertRaisesRegex(ValueError,'bad archive'):
                native.launch_owned(self.args, {'guest_root':r'C:\runs'})
            stop.assert_called_once(); release.assert_called_once()
        driver.upload.assert_not_called(); driver.json.assert_not_called()
        self.assertEqual(json.loads((self.root/'run.json').read_text())['status'],'prepare_failed')

    def test_uncertain_submission_retains_ownership_for_supervisor_recovery(self):
        driver, _ = self.prepare()
        driver.json.side_effect = TimeoutError('receipt lost')
        with patch.object(native,'release') as release:
            with self.assertRaises(TimeoutError):
                native.launch_owned(self.args, {'guest_root':r'C:\runs','host_ready_timeout_s':90})
            release.assert_not_called()
        self.assertEqual(json.loads((self.root/'run.json').read_text())['status'],'submission_uncertain')


if __name__ == '__main__':
    unittest.main()
