import copy
import json
from pathlib import Path
import sys
import subprocess
import tempfile
import unittest
from unittest.mock import Mock, patch
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from rolling.config import Settings
from rolling.state import Journal, locked, write
from signal_release import controller, runner, trigger


class SignalTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        root = Path(self.directory.name)
        self.settings = Settings(root, root / 'config', root / 'data', root / 'work', root / 'token',
            systemd_dir=root / 'units')
        self.journal = Journal(root)
        self.journal.data.update(current='r1', previous='r0', phase='complete', releases={
            'r1': {'server_port':1234, 'server_unit':'fixture.service', 'manifest':{}},
            'r0': {'manifest':{}}})
        self.journal.save()
        write(root / 'signal-pending.json', {'bundle':str(root / 'bundle'), 'release_id':'r2'})
        self.operations = Mock()
        self.operations.http.return_value = {'instance_release':'r1', 'signal_protocol':1, 'retiring':False}
        self.operations.inactive.return_value = True
        self.path = root / 'signal-receipts/deploy--r1.json'

    def test_runner_records_verified_success_and_failure_without_changing_origin(self):
        result = {'current':'r2', 'phase':'complete'}
        with patch.object(runner.deployment, 'deploy', return_value=result) as deploy:
            receipt = runner.run(self.settings, 'deploy--r1', self.operations)
        self.assertEqual(receipt['current'], 'r2')
        self.assertEqual(receipt['phase'], 'complete')
        self.assertEqual(json.loads(self.path.read_text()), receipt)
        self.assertEqual(deploy.call_count, 1)
        with patch.object(runner.deployment, 'deploy', side_effect=RuntimeError('probe failed')):
            with self.assertRaisesRegex(RuntimeError, 'probe failed'):
                runner.run(self.settings, 'deploy--r1', self.operations)
        failed = json.loads(self.path.read_text())
        self.assertEqual(failed['phase'], 'failed')
        self.assertIn('probe failed', failed['failure'])
        self.assertNotEqual(failed['attempt'], receipt['attempt'])
        self.assertEqual(Journal(self.settings.state_dir).data['current'], 'r1')

    def test_busy_deployment_writes_failure_and_never_starts_another_operation(self):
        with locked(self.settings.state_dir), patch.object(runner.deployment, 'deploy') as deploy:
            with self.assertRaises(BlockingIOError):
                runner.run(self.settings, 'deploy--r1', self.operations)
        deploy.assert_not_called()
        self.assertEqual(json.loads(self.path.read_text())['phase'], 'failed')

    def test_stale_signals_rejected_and_interrupted_switch_resumes_exact_target(self):
        state = copy.deepcopy(self.journal.data)
        state['current'] = 'r2'
        with self.assertRaisesRegex(ValueError, 'superseded'):
            runner.target_for(self.settings, 'deploy', 'r1', state)
        state.update(previous='r1', candidate='r2', phase='verifying')
        self.assertEqual(runner.target_for(self.settings, 'deploy', 'r1', state),
                         ('r2', self.settings.state_dir / 'bundle'))
        state.update(candidate=None, phase='rolling_back', previous='r0', rollback_from='r1')
        self.assertEqual(runner.target_for(self.settings, 'rollback', 'r1', state), ('r0', None))
        state.update(phase='rolled_back', current='r0')
        self.assertEqual(runner.target_for(self.settings, 'rollback', 'r0', state), ('r0', None))
        with self.assertRaisesRegex(ValueError, 'superseded'):
            runner.target_for(self.settings, 'rollback', 'r1', state)

    def test_old_binary_never_receives_a_terminating_default_signal(self):
        for metadata in [{}, {'signal_protocol':0}, {'signal_protocol':1, 'instance_release':'r0'},
                {'signal_protocol':1, 'instance_release':'r1', 'retiring':True}]:
            self.operations.http.return_value = metadata
            with self.assertRaises(ValueError):
                trigger.trigger(self.settings, 'deploy', self.operations)
        self.operations.run.assert_not_called()

    def test_trigger_waits_for_a_new_receipt_and_surfaces_failure_immediately(self):
        old = {'attempt':'old','phase':'complete', 'current':'r2'}
        write(self.path, old)
        def wait(check, _seconds):
            self.assertIsNone(check(), 'old success must not satisfy this signal')
            write(self.path, {'attempt':'new', 'phase':'running'})
            self.assertIsNone(check())
            write(self.path, {'attempt':'new', 'phase':'complete', 'current':'r2'})
            return check()
        self.operations.wait.side_effect = wait
        self.assertEqual(trigger.trigger(self.settings, 'deploy', self.operations)['attempt'], 'new')
        self.operations.run.assert_called_with('systemctl','kill','--kill-who=main',
            '--signal=SIGUSR2','fixture.service')
        def failed(check, _seconds):
            write(self.path, {'attempt':'failed', 'phase':'failed','failure':'invalid bundle'})
            return check()
        self.operations.wait.side_effect = failed
        with self.assertRaisesRegex(ValueError, 'invalid bundle'):
            trigger.trigger(self.settings, 'deploy', self.operations)

    def test_job_template_survives_retirement_and_controller_is_immutable(self):
        result = controller.install(self.settings, self.settings.state_dir / 'config.json', self.operations)
        target = Path(result['controller'])
        unit = (self.settings.systemd_dir / (result['unit_prefix'] + '@.service')).read_text()
        self.assertIn('TimeoutStartSec=infinity', unit)
        self.assertIn('Restart=no', unit)
        self.assertNotIn('PartOf=', unit)
        self.assertIn('"%i"', unit)
        self.assertIn(sys.executable, unit)
        self.assertTrue((target / 'rolling/deployment.py').is_file())
        self.assertTrue((target / 'signal_release/runner.py').is_file())
        (target / 'rolling/deployment.py').write_text('changed')
        with self.assertRaisesRegex(ValueError, 'modified'):
            controller.install(self.settings, self.settings.state_dir / 'config.json', self.operations)

    def test_systemctl_accepts_emitted_signal_arguments_without_sending_a_signal(self):
        self.operations.wait.return_value = {'phase':'complete'}
        trigger.trigger(self.settings, 'deploy', self.operations)
        arguments = self.operations.run.call_args.args
        # Help parses the real generated options without contacting or
        # signalling any unit. This catches distro systemd option differences.
        subprocess.run([*arguments[:-1], '--help'], capture_output=True, check=True)

    def test_stage_preserves_checked_bundle_and_excludes_concurrent_cli(self):
        candidate = {'release_id':'r2', 'commit':'c'}
        installed = self.settings.state_dir / 'staged/c'
        with patch.object(controller.manifest, 'verify', return_value=candidate), \
                patch.object(controller.manifest, 'compatible'), patch.object(controller.manifest, 'resources'), \
                patch.object(controller.manifest._installer, 'stage_bundle', return_value=installed):
            self.assertEqual(controller.stage(self.settings, Path('/bundle')),
                             {'release_id':'r2', 'bundle':str(installed)})
        self.assertEqual(controller.status(self.settings)['pending']['release_id'], 'r2')
        with controller.request_lock(self.settings):
            with self.assertRaises(BlockingIOError), controller.request_lock(self.settings):
                self.fail('overlapping signal clients must not restage the pending bundle')

    def test_instance_rejects_path_traversal_unknown_actions_and_empty_identity(self):
        self.assertEqual(runner.parse_instance('rollback--rel-123'), ('rollback', 'rel-123'))
        for instance in ['stop--r1', 'deploy--', 'deploy--../r1', 'deploy--r1.service/foo']:
            with self.assertRaises(ValueError):
                runner.parse_instance(instance)


if __name__ == '__main__':
    unittest.main()
