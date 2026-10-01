from pathlib import Path
import sqlite3
import sys
import tempfile
import unittest
from unittest.mock import patch
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from fixtures import Fixture
from rolling.maintenance import flow, archive
from rolling import deployment
from rolling.io import Operations
from rolling.state import Journal


class PowerLoss(BaseException):
    pass


class MaintenanceTests(unittest.TestCase):
    def test_private_identity_check_waits_for_host_ingress_reload(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture = Fixture(Path(directory))
            original, attempts = fixture.http, []
            def http(base, path, method='GET', body=None):
                if path == '/api/admin/release':
                    attempts.append(path)
                    if len(attempts) == 1:
                        raise RuntimeError('Host route is still reloading')
                return original(base, path, method, body)
            fixture.http = http
            fixture.wait = lambda check, seconds: Operations.wait(fixture, check, seconds)
            record = {**fixture.old, 'id': 'new'}
            with patch.object(flow.probes, 'resources'):
                flow.services.internal(fixture.settings, record, fixture, 1)
            self.assertEqual(len(attempts), 2)
            fixture.http = lambda *args: {'instance_release': 'wrong'}
            with patch.object(flow.probes, 'resources'), self.assertRaisesRegex(ValueError, 'identity differs'):
                flow.services.internal(fixture.settings, record, fixture, 1)

    def test_stop_repeats_graceful_signal_after_lost_first_delivery(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture = Fixture(Path(directory))
            name = fixture.old['host_unit']
            def run(*args):
                fixture.calls.append(args)
                if args[:2] == ('systemctl', 'kill'):
                    self.assertEqual(args, ('systemctl', 'kill', '--kill-who=main', '--signal=SIGTERM', name))
                    fixture.active.remove(name)
            def output(*args):
                active = name in fixture.active
                return f"ActiveState={'deactivating' if active else 'inactive'}\nMainPID={12345 if active else 0}\n"
            def wait(check, seconds):
                for _ in range(3):
                    if check():
                        return True
                raise TimeoutError('the graceful retry did not stop the fixture')
            fixture.run, fixture.output, fixture.wait = run, output, wait
            with patch.object(flow.services.time, 'monotonic', side_effect=[0, 0, 2]):
                flow.services.stop_unit(name, fixture, 90)
            self.assertNotIn(name, fixture.active)
            self.assertEqual(len(fixture.calls), 2)

    def test_stopped_window_closes_server_channels_before_hosts_and_keeps_nfs_alive(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture = Fixture(Path(directory))
            original = fixture.run
            consumers = {fixture.old['host_unit'], fixture.old['runtime_unit']}
            def stop(*args):
                if args[:3] == ('systemctl', '--no-block', 'stop'):
                    name = args[3]
                    if name == fixture.old['host_unit'] and fixture.old['server_unit'] in fixture.active:
                        raise RuntimeError('Host still owns a live Server channel')
                    if name == 'opencoder-resources.service' and consumers & fixture.active:
                        raise RuntimeError('NFS consumers have not stopped')
                original(*args)
            fixture.run = stop
            flow.services.stop(fixture.settings, {'releases': {'old': fixture.old}}, fixture)
            self.assertFalse(fixture.active)

    def test_pre_schema_recovery_keeps_unrelated_units_created_after_backup(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture = Fixture(Path(directory))
            original = flow.checkpoint
            def crash(journal, stage):
                original(journal, stage)
                if stage == 'installing':
                    raise PowerLoss()
            with fixture.patches():
                with patch.object(flow, 'checkpoint', side_effect=crash), self.assertRaises(PowerLoss):
                    flow.deploy(fixture.settings, fixture.bundle, fixture)
                unrelated = fixture.settings.systemd_dir / 'opencoder-unrelated.service'
                unrelated.write_text('another task owns this unit\n')
                result = deployment.rollback(fixture.settings, fixture)
            self.assertEqual(result['current'], 'old')
            self.assertEqual(unrelated.read_text(), 'another task owns this unit\n')

    def test_stopped_upgrade_updates_resources_and_forbids_backup_after_opening(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture = Fixture(Path(directory))
            with fixture.patches():
                result = flow.deploy(fixture.settings, fixture.bundle, fixture)
            self.assertEqual(result['phase'], 'complete')
            self.assertEqual(result['current'], 'new')
            self.assertTrue(result['maintenance']['writes_open'])
            self.assertTrue(result['releases']['old']['maintenance_retired'])
            self.assertEqual((fixture.settings.state_dir / 'services/opencoder-resources').read_bytes(), b'new binary')
            backup = Path(result['maintenance']['backup'])
            metadata = archive.verify(backup)
            self.assertEqual(metadata['original']['current'], 'old')
            self.assertIn('data/server/definitions.db', metadata['files'])
            self.assertIsNone(result['previous'])
            with self.assertRaisesRegex(ValueError, 'restoration is forbidden'):
                deployment.rollback(fixture.settings, fixture)
            with sqlite3.connect(fixture.db) as conn:
                self.assertEqual(conn.execute('SELECT version FROM schema_version').fetchone(), (32,))

    def test_failure_after_migration_forbids_old_restore_and_resumes_forward(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture = Fixture(Path(directory))
            fixture.fail_internal = True
            with fixture.patches():
                with self.assertRaisesRegex(RuntimeError, 'candidate verification'):
                    flow.deploy(fixture.settings, fixture.bundle, fixture)
                state = Journal(fixture.settings.state_dir).data['maintenance']
                root = Path(state['backup'])
                original_hashes = archive.inventory(root)
                self.assertEqual(state['stage'], 'verifying')
                before = list(fixture.calls)
                with self.assertRaisesRegex(ValueError, 'schema migration.*forbidden'):
                    deployment.rollback(fixture.settings, fixture)
                self.assertEqual(fixture.calls, before)
                fixture.fail_internal = False
                result = flow.deploy(fixture.settings, fixture.bundle, fixture)
            self.assertEqual(result['current'], 'new')
            self.assertEqual(result['maintenance']['stage'], 'complete')
            self.assertEqual(archive.inventory(root), original_hashes)
            self.assertEqual((fixture.settings.state_dir / 'services/opencoder-resources').read_bytes(), b'new binary')
            with sqlite3.connect(fixture.db) as conn:
                self.assertEqual(conn.execute('SELECT version FROM schema_version').fetchone(), (32,))
                self.assertEqual(conn.execute('SELECT token_hash FROM platform_users').fetchone(), (b'\x00\x01\x02',))
                conn.execute("INSERT INTO project_todos VALUES ('new-api-write','i')")

    def test_every_durable_stage_resumes_same_backup_and_candidate(self):
        stages = ['waiting', 'stopping', 'backup', 'installing', 'verifying', 'reopening', 'public']
        for stage in stages:
            with self.subTest(stage=stage), tempfile.TemporaryDirectory() as directory:
                fixture = Fixture(Path(directory))
                original = flow.checkpoint
                def crash(journal, current):
                    original(journal, current)
                    if current == stage:
                        raise PowerLoss()
                with fixture.patches():
                    with patch.object(flow, 'checkpoint', side_effect=crash), self.assertRaises(PowerLoss):
                        flow.deploy(fixture.settings, fixture.bundle, fixture)
                    pending = Journal(fixture.settings.state_dir).data['maintenance']
                    saved = Path(pending['backup'])
                    hashes = archive.inventory(saved) if saved.exists() else None
                    result = flow.deploy(fixture.settings, fixture.bundle, fixture)
                self.assertEqual(result['current'], 'new')
                self.assertEqual(result['maintenance']['backup'], str(saved))
                if hashes:
                    self.assertEqual(archive.inventory(saved), hashes)

    def test_preflight_failure_never_closes_admission_or_starts_services(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture = Fixture(Path(directory))
            with fixture.patches(), patch.object(flow.preflight, 'check', side_effect=ValueError('missing NFS')):
                with self.assertRaisesRegex(ValueError, 'missing NFS'):
                    flow.deploy(fixture.settings, fixture.bundle, fixture)
            self.assertEqual(fixture.calls, [])
            self.assertEqual(Journal(fixture.settings.state_dir).data['current'], 'old')

    def test_lost_reopen_acknowledgement_forbids_old_backup_and_resumes_forward(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture = Fixture(Path(directory))
            original = fixture.http
            def lost(base, path, method='GET', body=None):
                reply = original(base, path, method, body)
                if method == 'DELETE' and path == '/api/admin/drain':
                    fixture.http = original
                    raise TimeoutError('reopen reply lost')
                return reply
            fixture.http = lost
            with fixture.patches():
                with self.assertRaisesRegex(TimeoutError, 'reopen reply lost'):
                    flow.deploy(fixture.settings, fixture.bundle, fixture)
                with self.assertRaisesRegex(ValueError, 'restoration is forbidden'):
                    flow.rollback(fixture.settings, fixture)
                self.assertEqual(flow.deploy(fixture.settings, fixture.bundle, fixture)['phase'], 'complete')

    def test_lost_schema_start_reply_forbids_old_restore_before_public_writes(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture = Fixture(Path(directory))
            original = fixture.run
            def lost(*args):
                original(*args)
                if args[:2] == ('systemctl', 'start') and any('server-new' in name for name in args[2:]):
                    fixture.run = original
                    raise PowerLoss()
            fixture.run = lost
            with fixture.patches():
                with self.assertRaises(PowerLoss):
                    flow.deploy(fixture.settings, fixture.bundle, fixture)
                state = Journal(fixture.settings.state_dir).data['maintenance']
                self.assertTrue(state['schema_started'])
                self.assertFalse(state['writes_open'])
                before = list(fixture.calls)
                with self.assertRaisesRegex(ValueError, 'schema migration.*forbidden'):
                    deployment.rollback(fixture.settings, fixture)
                self.assertEqual(fixture.calls, before)
                self.assertEqual(flow.deploy(fixture.settings, fixture.bundle, fixture)['current'], 'new')
