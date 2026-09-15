import contextlib
import io
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import Mock, patch
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from rolling import cli
from rolling.config import Settings
from rolling.state import locked


class CliTests(unittest.TestCase):
    def test_signal_dispatch_releases_deploy_lock_but_keeps_staged_target_pinned(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            settings = Settings(root, root, root, root, root / 'token')
            def signal(settings, action, operations, seconds):
                with locked(root):
                    pass  # The independently running controller can acquire it.
                with self.assertRaises(BlockingIOError), cli.controller.request_lock(settings):
                    self.fail('staged target must remain pinned until receipt')
                self.assertEqual(action, 'deploy')
                return {'current':'r2','phase':'complete'}
            with patch.object(sys, 'argv', ['deploy','--signal','--bundle','/bundle']), \
                    patch.object(cli.config, 'load', return_value=settings), \
                    patch.object(cli, 'Operations', return_value=Mock()), \
                    patch.object(cli.controller, 'install'), \
                    patch.object(cli.controller, 'stage', return_value={'release_id':'r2'}), \
                    patch.object(cli, 'trigger', side_effect=signal) as trigger, \
                    patch.object(cli.deployment, 'deploy') as deploy, contextlib.redirect_stdout(io.StringIO()):
                cli.main()
            trigger.assert_called_once()
            deploy.assert_not_called()

    def test_signal_argument_conflicts_are_rejected_before_loading_configuration(self):
        for args in [['--signal','--migrate'],['--signal','--status'],['--signal','--stage'],
                     ['--rollback','--bundle','/bundle'],['--wait-seconds','0']]:
            with patch.object(sys, 'argv', ['deploy',*args]), patch.object(cli.config,'load') as load, \
                    contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit) as exit:
                cli.main()
            self.assertEqual(exit.exception.code, 2)
            load.assert_not_called()

    def test_signal_rollback_uses_running_server_instead_of_inline_deployer(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            settings = Settings(root,root,root,root,root / 'token')
            with patch.object(sys,'argv',['deploy','--signal','--rollback']), \
                    patch.object(cli.config,'load',return_value=settings), \
                    patch.object(cli,'Operations',return_value=Mock()), \
                    patch.object(cli.controller,'install'), \
                    patch.object(cli,'trigger',return_value={'phase':'complete'}) as trigger, \
                    patch.object(cli.deployment,'rollback') as rollback, contextlib.redirect_stdout(io.StringIO()):
                cli.main()
            self.assertEqual(trigger.call_args.args[1], 'rollback')
            rollback.assert_not_called()
