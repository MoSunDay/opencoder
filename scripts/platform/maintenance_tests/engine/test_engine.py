"""Local cutover and program rollback preserve data and avoid remote admission."""
from contextlib import contextmanager
import json
from pathlib import Path
import sqlite3
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[2]))
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from fixtures import Fixture
from rolling.maintenance import flow, restore, configuration
from rolling.state import Journal


@contextmanager
def prepared(root):
    fixture = Fixture(root)
    origin_bin = fixture.settings.state_dir / 'releases/old/bundle/bin'
    origin_bin.mkdir(parents=True)
    (origin_bin / 'opencoder-server').write_bytes(b'compatible origin resource program')
    for value, version, engine in ((fixture.old['manifest'], 10, 'libsql'), (fixture.candidate, 11, 'turso')):
        value.update(storage_engine=engine, execution_io={'reader': 1, 'writer': 1}, brain_schema_version=7)
        value['compatibility']['data_format'] = {'min': version, 'max': version}
    journal = Journal(fixture.settings.state_dir)
    journal.data['releases']['old'] = fixture.old
    journal.save()
    Path(fixture.old['runtime_data']).mkdir(parents=True)
    with sqlite3.connect(fixture.db) as db:
        db.execute('UPDATE schema_version SET version=39')
    with sqlite3.connect(fixture.settings.state_dir / 'host/host.db') as db:
        body = json.loads(db.execute("SELECT config FROM host_runtimes WHERE id='old'").fetchone()[0])
        body['data_dir'] = fixture.old['runtime_data']
        db.execute("UPDATE host_runtimes SET config=? WHERE id='old'", (json.dumps(body),))
    previous_http = fixture.http
    fixture.migrate = lambda: None

    def http(base, path, method='GET', body=None, **kwargs):
        if path == '/api/ready':
            mode = json.loads((fixture.settings.server_data / 'admission.json').read_text())['mode']
            return {'mode': mode, 'inflight_admissions': 0}
        if path == '/api/nodes':
            return {'nodes': [{'id': 'node', 'online': True}, {'id': 'remote', 'online': True}]}
        if path == '/rpc':
            fixture.calls.append((base, path, method))
            return {'status': 200, 'body': {'mode': 'frozen' if body['command'] == 'freeze' else 'open'}}
        value = previous_http(base, path, method, body)
        if path == '/api/health':
            value['build'].update(storage_engine='turso', execution_io={'reader': 1, 'writer': 1})
        return value

    fixture.http = http
    scope = {'rootfs': str(fixture.rootfs), 'release_id': 'new', 'database': {'schema_version': 39},
             'configuration_hashes': configuration.hashes(fixture.desired)}
    with fixture.patches(), patch.object(flow.preflight, 'check', return_value=scope), \
            patch.object(flow.units, 'install_bundle', return_value=fixture.bundle), \
            patch.object(flow.programs, 'stage', return_value={}), \
            patch.object(flow.programs, 'install'), patch.object(flow.engine_files, 'mark'), \
            patch.object(flow.services, 'internal') as internal:
        yield fixture, internal


class EngineMaintenanceTests(unittest.TestCase):
    def test_failed_bundle_retention_leaves_existing_services_open(self):
        with tempfile.TemporaryDirectory() as directory, prepared(Path(directory)) as (fixture, _):
            with patch.object(flow.units, 'install_bundle', side_effect=RuntimeError('bundle copy failed')):
                with self.assertRaisesRegex(RuntimeError, 'bundle copy failed'):
                    flow.deploy(fixture.settings, fixture.bundle, fixture, 1)
            self.assertEqual(Journal(fixture.settings.state_dir).data['current'], 'old')
            self.assertIn(fixture.old['server_unit'], fixture.active)
            self.assertEqual(fixture.settings.nginx_include.read_text(), 'old ingress\n')
            self.assertFalse(any('/rpc' in call for call in fixture.calls))

    def test_cutover_with_remote_nodes_never_freezes_the_cluster(self):
        with tempfile.TemporaryDirectory() as directory, prepared(Path(directory)) as (fixture, _):
            inode = fixture.db.stat().st_ino
            result = flow.deploy(fixture.settings, fixture.bundle, fixture, 1)
            self.assertEqual(result['maintenance']['stage'], 'complete')
            self.assertTrue(result['maintenance']['engine_switch'])
            self.assertTrue(result['maintenance']['writes_open'])
            self.assertEqual(fixture.db.stat().st_ino, inode)
            self.assertFalse(any('/api/admin/drain' in call for call in fixture.calls))
            with sqlite3.connect(fixture.db) as db:
                self.assertEqual(db.execute('SELECT token_hash FROM platform_users').fetchone(), (b'\0\1\2',))
                self.assertEqual(db.execute('SELECT version FROM schema_version').fetchone(), (39,))

    def test_failure_before_reopening_restores_programs_without_restoring_database(self):
        with tempfile.TemporaryDirectory() as directory, prepared(Path(directory)) as (fixture, internal):
            internal.side_effect = RuntimeError('verification failed')
            with self.assertRaisesRegex(RuntimeError, 'verification failed'):
                flow.deploy(fixture.settings, fixture.bundle, fixture, 1)
            inode = fixture.db.stat().st_ino
            before = fixture.db.read_bytes()
            with patch.object(restore, 'data', side_effect=AssertionError('database restore forbidden')):
                result = flow.rollback(fixture.settings, fixture, 1)
            self.assertEqual(result['current'], 'old')
            self.assertEqual(result['maintenance']['stage'], 'rolled_back')
            self.assertEqual(fixture.db.stat().st_ino, inode)
            self.assertEqual(fixture.db.read_bytes(), before)
            self.assertEqual((fixture.settings.state_dir / 'services/opencoder-resources').read_bytes(),
                             b'compatible origin resource program')
            self.assertFalse(any('/api/admin/drain' in call for call in fixture.calls))

    def test_early_rollback_preserves_a_compatible_todo_floor(self):
        with tempfile.TemporaryDirectory() as directory, prepared(Path(directory)) as (fixture, _):
            journal = Journal(fixture.settings.state_dir)
            journal.record('old')['manifest']['todo_files_contract'] = 1
            journal.save()
            fixture.candidate['todo_files_contract'] = 1
            (fixture.settings.state_dir / 'todo-files-policy.json').write_text(
                json.dumps({'minimum_contract': 1}))
            controller = fixture.settings.systemd_dir / 'prior-controller.service'
            controller.write_text('candidate controller')
            import base64
            (fixture.settings.state_dir / 'maintenance-controller.json').write_text(json.dumps({
                'release_id': 'new', 'path': str(controller),
                'content': base64.b64encode(b'compatible prior controller').decode()}))
            with patch.object(flow.local_gates, 'drained', side_effect=RuntimeError('drain failed')):
                with self.assertRaisesRegex(RuntimeError, 'drain failed'):
                    flow.deploy(fixture.settings, fixture.bundle, fixture, 1)
            before = fixture.db.read_bytes()
            with patch.object(restore, 'data', side_effect=AssertionError('database restore forbidden')):
                result = flow.rollback(fixture.settings, fixture, 1)
            self.assertEqual(result['maintenance']['stage'], 'rolled_back')
            self.assertEqual(controller.read_text(), 'compatible prior controller')
            self.assertEqual(fixture.db.read_bytes(), before)
            self.assertFalse(any(call[:3] == ('systemctl', '--no-block', 'stop') for call in fixture.calls))
            self.assertIn(fixture.old['runtime_unit'], fixture.active)

    def test_recovery_starts_registered_candidate_before_restored_host(self):
        with tempfile.TemporaryDirectory() as directory, prepared(Path(directory)) as (fixture, internal):
            internal.side_effect = RuntimeError('verification failed')
            with self.assertRaisesRegex(RuntimeError, 'verification failed'):
                flow.deploy(fixture.settings, fixture.bundle, fixture, 1)
            journal = Journal(fixture.settings.state_dir)
            journal.data['maintenance'].update(stage='engine_restore_services',
                                               recovery_program={'release_id': 'old'})
            candidate = journal.record('new')
            journal.save()
            fixture.calls.clear()
            before = fixture.db.read_bytes()
            result = flow.rollback(fixture.settings, fixture, 1)
            candidate_start = fixture.calls.index(('systemctl', 'start', candidate['runtime_unit']))
            host_start = fixture.calls.index(('systemctl', 'start', fixture.old['host_unit']))
            self.assertLess(candidate_start, host_start)
            self.assertEqual(fixture.db.read_bytes(), before)
            self.assertEqual(result['releases']['new']['runtime_program']['release_id'], 'old')

    def test_reopened_writes_forbid_program_rollback(self):
        with tempfile.TemporaryDirectory() as directory, prepared(Path(directory)) as (fixture, _):
            flow.deploy(fixture.settings, fixture.bundle, fixture, 1)
            count = len(fixture.calls)
            with self.assertRaisesRegex(ValueError, 'corrective release'):
                flow.rollback(fixture.settings, fixture, 1)
            self.assertEqual(len(fixture.calls), count)


if __name__ == '__main__':
    unittest.main()
