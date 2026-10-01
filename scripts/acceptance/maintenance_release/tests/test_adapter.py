"""Adapter routes generated mounts and current graceful stop commands privately."""
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import Mock, patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
sys.path.insert(0, str(Path(__file__).resolve().parents[3] / 'platform'))
from fixture import create_settings
from operations import PrivateOperations, systemctl_command
from control import Control, request
from rolling.maintenance.mounts import unit


class AdapterTests(unittest.TestCase):
    def fixture(self, root):
        settings = create_settings(root)
        settings.token_file.write_text('fresh-test-fixture')
        return PrivateOperations(settings, root, '/private/nginx')

    def test_no_block_stop_and_main_signal_never_contact_systemd(self):
        with tempfile.TemporaryDirectory() as directory:
            operations = self.fixture(Path(directory))
            name = 'opencoder-private-test.service'
            child = Mock(pid=1234)
            child.poll.return_value = None
            operations.children[name] = child
            with patch('operations.subprocess.run') as external:
                operations.run('systemctl', '--no-block', 'stop', name)
                self.assertEqual(operations.output('systemctl', 'show', name, '-p', 'ActiveState', '--value'),
                                 'deactivating\n')
                operations.run('systemctl', 'kill', '--kill-who=main', '--signal=SIGTERM', name)
                self.assertEqual(child.send_signal.call_count, 2)
                external.assert_not_called()
            with self.assertRaisesRegex(ValueError, 'unowned'):
                operations.run('systemctl', 'stop', 'production.service')
            with self.assertRaisesRegex(ValueError, 'graceful'):
                operations.run('systemctl', 'kill', '--signal=SIGKILL', name)

    def test_generated_mount_enable_now_mounts_and_stops_inside_fixture(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            operations = self.fixture(root)
            target = root / 'mounts/workspace'
            target.mkdir(parents=True)
            name = 'private-workspace.mount'
            (operations.settings.systemd_dir / name).write_text(unit({'path': str(target), 'port': 12345}))
            with patch('operations.subprocess.run') as external:
                operations.run('systemctl', 'enable', '--now', name)
                self.assertEqual(external.call_args.args[0][0:3], ['mount', '-t', 'nfs'])
                self.assertIn('ro,vers=3', external.call_args.args[0][4])
                self.assertEqual(operations.output('systemctl', 'show', name, '-p', 'ActiveState', '--value'), 'active\n')
                operations.run('systemctl', 'stop', name)
                self.assertEqual(external.call_args.args[0], ['umount', str(target)])
            self.assertFalse(operations.mounted)

    def test_generated_mount_cannot_escape_private_mount_directory(self):
        with tempfile.TemporaryDirectory() as directory:
            operations = self.fixture(Path(directory))
            name = 'outside.mount'
            (operations.settings.systemd_dir / name).write_text(unit({'path': '/production', 'port': 12345}))
            with patch('operations.subprocess.run') as external, self.assertRaisesRegex(ValueError, 'escapes'):
                operations.run('systemctl', 'enable', '--now', name)
            external.assert_not_called()

    def test_host_socket_routes_only_owned_service_operations(self):
        with tempfile.TemporaryDirectory() as directory:
            operations = self.fixture(Path(directory))
            control = Control(operations)
            try:
                value = request(control.path, 'output', ['systemctl', 'show', 'absent.service', '-p', 'LoadState', '--value'])
                self.assertEqual(value, 'not-found\n')
                with self.assertRaisesRegex(RuntimeError, 'unowned'):
                    request(control.path, 'run', ['systemctl', 'start', 'production.service'])
                with self.assertRaisesRegex(RuntimeError, 'unsupported control'):
                    request(control.path, 'delete', ['/production'])
            finally:
                control.close()
            self.assertFalse(control.path.exists())

    def test_command_parser_understands_current_stop_prefix(self):
        self.assertEqual(systemctl_command(['--no-block', 'stop', 'own.service']), ('stop', ['own.service']))
        with self.assertRaises(ValueError):
            systemctl_command(['--root=/production', 'stop', 'own.service'])

    def test_generated_failure_restart_runs_only_while_service_is_desired(self):
        with tempfile.TemporaryDirectory() as directory:
            operations = self.fixture(Path(directory))
            name = 'opencoder-private-restart.service'
            (operations.settings.systemd_dir / name).write_text('Restart=on-failure\nRestartSec=0\n')
            child = Mock()
            child.poll.return_value = 1
            operations.children[name] = child
            operations.desired.add(name)
            with patch.object(operations, 'start') as start:
                operations.supervise()
                start.assert_called_once_with(name)
                operations.stop(name)
                start.reset_mock()
                operations.supervise()
                start.assert_not_called()


if __name__ == '__main__':
    unittest.main()
