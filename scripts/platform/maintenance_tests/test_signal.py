import contextlib
import io
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import Mock, patch
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from fixtures import Fixture
from rolling import cli
from rolling.state import write
from signal_release import controller, runner


class SignalMaintenanceTests(unittest.TestCase):
    def test_maintenance_is_preserved_in_staging_and_signal_dispatch(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture = Fixture(Path(directory))
            with patch.object(controller.manifest, 'verify', return_value=fixture.candidate), \
                 patch.object(controller.manifest, 'resources'), \
                 patch('rolling.maintenance.preflight.check'), patch('rolling.io.Operations'), \
                 patch.object(controller.manifest._installer, 'stage_bundle', return_value=fixture.bundle):
                staged = controller.stage(fixture.settings, fixture.bundle, maintenance=True)
            self.assertTrue(staged['maintenance'])
            write(fixture.settings.state_dir / 'signal-pending.json', staged)
            with patch.object(runner.maintenance, 'deploy', return_value={'current': 'new', 'phase': 'complete'}) as deploy, \
                 patch.object(runner.deployment, 'deploy') as rolling:
                result = runner.run(fixture.settings, 'deploy--old', fixture)
            self.assertTrue(result['maintenance'])
            deploy.assert_called_once()
            rolling.assert_not_called()

    def test_cli_stage_verifies_mode_before_installing_controller(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture = Fixture(Path(directory))
            events = []
            def stage(settings, bundle, maintenance=False):
                events.append(('stage', maintenance))
                return {'release_id': 'new', 'maintenance': maintenance}
            with patch.object(sys, 'argv', ['deploy', '--stage', '--maintenance', '--bundle', '/bundle']), \
                 patch.object(cli.config, 'load', return_value=fixture.settings), \
                 patch.object(cli, 'Operations', return_value=fixture), \
                 patch.object(cli.controller, 'stage', side_effect=stage), \
                 patch.object(cli.controller, 'install', side_effect=lambda *args: events.append(('install', True))), \
                 contextlib.redirect_stdout(io.StringIO()):
                cli.main()
            self.assertEqual(events, [('stage', True), ('install', True)])

    def test_interrupted_maintenance_signal_uses_original_origin_after_reopening_intent(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture = Fixture(Path(directory))
            write(fixture.settings.state_dir / 'signal-pending.json', {'release_id': 'new', 'bundle': str(fixture.bundle), 'maintenance': True})
            journal = {'current': 'new', 'previous': 'old', 'candidate': 'new', 'phase': 'migrating',
                       'maintenance': {'origin': 'old', 'target': 'new', 'stage': 'reopening'}}
            self.assertEqual(runner.target_for(fixture.settings, 'deploy', 'old', journal), ('new', fixture.bundle))
