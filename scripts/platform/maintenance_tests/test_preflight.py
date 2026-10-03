import copy
import json
from pathlib import Path
import sys
import sqlite3
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import Mock, patch
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
    def test_changed_ontology_binding_rejects_before_any_service_operation(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            data = root / 'server'
            data.mkdir()
            files = root / 'files'
            files.mkdir()
            with sqlite3.connect(data / 'ontology.db') as connection:
                connection.execute('CREATE TABLE ontology_schema_version(version INTEGER,files_root TEXT)')
                connection.execute('INSERT INTO ontology_schema_version VALUES(1,?)', (str(files),))
            agent, server = configs()
            original = copy.deepcopy(server)
            original['ontology'] = {'files_dir': str(files)}
            settings = SimpleNamespace(server_data=data)
            for ontology in [{'files_dir': str(root / 'other')}, {'files_dir': None}, {}]:
                server['ontology'] = ontology
                operations = Mock()
                with patch.object(preflight, 'configs', return_value=(agent, server)), \
                     patch.object(preflight.configuration_files, 'actual_configs', return_value=(agent, original)), \
                     self.assertRaisesRegex(ValueError, 'Ontology files_dir'):
                    preflight.check(settings, {}, operations)
                self.assertEqual(operations.mock_calls, [])
            preflight.ontology_binding(settings, original)

    def test_unchanged_default_ontology_binding_is_accepted_without_requiring_a_new_config_field(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            files = root / 'files'
            files.mkdir()
            with sqlite3.connect(root / 'ontology.db') as connection:
                connection.execute('CREATE TABLE ontology_schema_version(version INTEGER,files_root TEXT)')
                connection.execute('INSERT INTO ontology_schema_version VALUES(1,?)', (str(files),))
            settings = SimpleNamespace(server_data=root)
            with patch.object(preflight.configuration_files, 'actual_configs', return_value=({}, {})):
                preflight.ontology_binding(settings, {})

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
            with self.assertRaisesRegex(ValueError, 'local libsql recovery'):
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
