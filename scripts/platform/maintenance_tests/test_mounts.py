import json
from pathlib import Path
import tempfile
import subprocess
import unittest
from unittest.mock import Mock, call
from rolling.maintenance import archive, mounts, preflight, services
from fixtures import Fixture


class MountTests(unittest.TestCase):
    def test_identical_stacked_read_only_mounts_are_accepted_but_conflicts_rejected(self):
        operations = Mock()
        row = {'target': '/mnt/agents', 'source': '127.0.0.1:/', 'fstype': 'nfs', 'options': 'ro,port=2049'}
        operations.output.return_value = json.dumps({'filesystems': [row, row]})
        self.assertEqual(preflight.mount(Path('/mnt/agents'), operations), row)
        operations.output.return_value = json.dumps({'filesystems': [row, {**row, 'options': 'ro,port=2050'}]})
        with self.assertRaises(ValueError):
            preflight.mount(Path('/mnt/agents'), operations)

    def test_backup_does_not_capture_unrelated_services_or_mounts(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture = Fixture(Path(directory))
            foreign = fixture.settings.systemd_dir / 'opencoder-other-task.service'
            foreign.write_text('another independent task')
            paths = archive.managed_units(fixture.settings, {'releases': {'old': fixture.old}}, {})
            self.assertNotIn(foreign, paths)
            self.assertIn(fixture.settings.systemd_dir / fixture.old['server_unit'], paths)

    def test_native_mount_installation_uses_read_only_unit_and_owned_path(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture = Fixture(Path(directory))
            plan = {'path': str(Path(directory) / 'native-mount'), 'port': 2051}
            mounts.install(fixture.settings, [plan], fixture)
            path = fixture.settings.systemd_dir / mounts.name(plan['path'])
            self.assertIn('Options=ro,', path.read_text())
            self.assertIn('port=2051,', path.read_text())
            self.assertIn(('systemctl', 'enable', '--now', path.name), fixture.calls)

    def test_stop_closes_servers_before_hosts_and_keeps_resources_last(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture = Fixture(Path(directory))
            services.stop(fixture.settings, {'releases': {'old': fixture.old}}, fixture)
            stopped = [call[-1] for call in fixture.calls if call[:3] == ('systemctl', '--no-block', 'stop')]
            self.assertLess(stopped.index(fixture.old['server_unit']), stopped.index(fixture.old['host_unit']))
            self.assertEqual(stopped[-1], 'opencoder-resources.service')

    def test_failed_mount_job_still_removes_only_verified_kernel_mounts(self):
        operations = Mock()
        plan = {'path': '/mnt/native', 'port': 2051}
        row = {'target': plan['path'], 'source': '127.0.0.1:/', 'fstype': 'nfs',
               'options': 'ro,vers=3,port=2051'}
        operations.output.side_effect = ['loaded', json.dumps({'filesystems': [row, row]}),
            json.dumps({'filesystems': [row]}),
            subprocess.CalledProcessError(1, ['findmnt'], output='')]
        mounts.stop_new(Mock(), [plan], operations)
        unit_name = mounts.name(plan['path'])
        self.assertEqual(operations.run.call_args_list, [call('systemctl', 'stop', unit_name),
            call('umount', plan['path']), call('umount', plan['path']),
            call('systemctl', 'disable', unit_name)])

    def test_unmount_refuses_foreign_exports_and_stalled_cleanup(self):
        plan = {'path': '/mnt/native', 'port': 2051}
        row = {'target': plan['path'], 'source': '127.0.0.1:/', 'fstype': 'nfs',
               'options': 'ro,vers=3,port=2051'}
        for changed in [{'source': 'remote:/'}, {'fstype': 'ext4'}, {'target': '/unowned'},
                        {'options': 'rw,port=2051'}, {'options': 'ro,port=2052'}]:
            operations = Mock()
            operations.output.return_value = json.dumps({'filesystems': [{**row, **changed}]})
            with self.subTest(changed=changed), self.assertRaisesRegex(ValueError, 'unexpected'):
                mounts.clear(plan, operations)
            operations.run.assert_not_called()
        operations = Mock()
        operations.output.return_value = json.dumps({'filesystems': [row]})
        with self.assertRaisesRegex(ValueError, 'did not disappear'):
            mounts.clear(plan, operations)
        operations.run.assert_called_once_with('umount', plan['path'])
