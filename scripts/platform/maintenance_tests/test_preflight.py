import copy
import json
from pathlib import Path
import sys
import unittest
import tempfile
import sqlite3
from unittest.mock import Mock, patch
from fixtures import Fixture
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from rolling.maintenance import preflight
from rolling.manifest import compatible, overlapping


def configs():
    agent = {'dag': {'rootfs_dir': '/images/dag', 'binary_dir': '/mnt/bin',
                     'workspace_dir': '/mnt/workspace'}, 'agent': {'agents_dir': '/mnt/agents'}}
    server = copy.deepcopy(agent)
    server['dag'].update(nfs={'enabled': True}, workspace_nfs={'enabled': True})
    server['agent']['nfs'] = {'enabled': True, 'read_only': True}
    return agent, server


class PreflightTests(unittest.TestCase):
    def test_workspace_export_rejects_fields_that_config_loader_would_ignore(self):
        agent, server = configs()
        server['dag']['workspace_nfs']['read_only'] = True
        with self.assertRaisesRegex(ValueError, 'unsupported Server dag.workspace_nfs fields'):
            preflight.configuration(agent, server)

    def test_native_mounts_are_planned_before_old_exporter_is_replaced(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture = Fixture(Path(directory))
            agent, server = configs()
            agents = Path(directory) / 'agents'
            agents.mkdir()
            agent['agent']['agents_dir'] = str(agents)
            source = Path(directory) / 'source'
            source.mkdir()
            server['dag']['workspace_dir'] = str(source)
            agent['dag'].update(rootfs_dir=str(fixture.rootfs),
                                binary_dir=str(Path(directory) / 'future/binaries'),
                                workspace_dir=str(Path(directory) / 'future/workspace'))
            fixture.output = lambda *args: json.dumps({'filesystems': [{
                'target': str(agents), 'source': '127.0.0.1:/', 'fstype': 'nfs',
                'options': 'ro,vers=3,port=2049'}]})
            with patch.object(preflight, 'configs', return_value=(agent, server)):
                receipt = preflight.check(fixture.settings, fixture.candidate, fixture)
            self.assertTrue(receipt['mounts']['binary_dir']['planned'])
            self.assertTrue(receipt['mounts']['workspace_dir']['planned'])
            self.assertEqual(receipt['mounts']['native'], [
                {'path': agent['dag']['binary_dir'], 'source': server['dag']['binary_dir'], 'port': 2050},
                {'path': agent['dag']['workspace_dir'], 'source': str(source), 'port': 2051}])
            self.assertEqual(receipt['database']['schema_version'], 31)
            self.assertEqual(receipt['database']['tables']['project_todos'], 1)
            self.assertFalse(Path(agent['dag']['binary_dir']).exists())
            self.assertEqual(fixture.calls, [('runc', '--version'),
                ('runuser', '-u', 'root', '--', 'test', '-r', str(source)),
                ('runuser', '-u', 'root', '--', 'test', '-x', str(source))])
            with sqlite3.connect(fixture.db) as conn:
                self.assertEqual(conn.execute('SELECT version FROM schema_version').fetchone(), (31,))

    def test_complete_config_and_read_only_nfs_are_accepted(self):
        agent, server = configs()
        self.assertEqual(preflight.configuration(agent, server)['rootfs_dir'], Path('/images/dag'))
        operations = Mock()
        operations.output.return_value = json.dumps({'filesystems': [
            {'target': '/mnt/bin', 'fstype': 'nfs', 'options': 'ro,vers=3', 'source': '127.0.0.1:/'}]})
        self.assertEqual(preflight.mount(Path('/mnt/bin'), operations)['target'], '/mnt/bin')

    def test_missing_paths_writable_exports_and_external_databases_fail_before_stop(self):
        for section, key in [('dag', 'rootfs_dir'), ('dag', 'binary_dir'), ('dag', 'workspace_dir'), ('agent', 'agents_dir')]:
            agent, server = configs()
            agent[section].pop(key)
            with self.subTest(key=key), self.assertRaisesRegex(ValueError, key):
                preflight.configuration(agent, server)
        for backend in ['mysql', 'starrocks']:
            agent, server = configs()
            server['storage'] = {'backend': backend}
            with self.assertRaisesRegex(ValueError, 'MySQL/StarRocks'):
                preflight.configuration(agent, server)
        agent, server = configs()
        server['dag']['nfs']['read_only'] = False
        with self.assertRaisesRegex(ValueError, 'read-only'):
            preflight.configuration(agent, server)
        server['dag']['nfs'] = {'enabled': True, 'port': 2049}
        with self.assertRaisesRegex(ValueError, 'distinct'):
            preflight.configuration(agent, server)

    def test_writable_or_local_mounts_fail_closed(self):
        operations = Mock()
        for fstype, options in [('nfs', 'rw,vers=3'), ('ext4', 'ro'), ('nfs4', 'ro,rw')]:
            operations.output.return_value = json.dumps({'filesystems': [
                {'fstype': fstype, 'options': options, 'target': '/mnt/bin'}]})
            with self.subTest(fstype=fstype, options=options), self.assertRaises(ValueError):
                preflight.mount(Path('/mnt/bin'), operations)

    def test_format_is_symmetric_and_stopped_old_releases_leave_overlap_set(self):
        old = {'release_id': 'old', 'protocol_version': 10,
               'compatibility': {'protocol': {'min': 1, 'max': 1}, 'data_format': {'min': 1, 'max': 1}}}
        new = copy.deepcopy(old)
        new['release_id'] = 'new'
        new['compatibility']['data_format'] = {'min': 2, 'max': 2}
        for left, right in [(old, new), (new, old)]:
            with self.assertRaisesRegex(ValueError, '--maintenance'):
                compatible(left, [right])
        journal = {'releases': {'old': {'manifest': old, 'maintenance_retired': True},
                                'new': {'manifest': new}}}
        compatible(new, overlapping(journal))
