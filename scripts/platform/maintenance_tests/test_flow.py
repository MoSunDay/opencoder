from pathlib import Path
from contextlib import closing
import json
import errno
import os
import sqlite3
import sys
import tempfile
import unittest
from unittest.mock import patch
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from fixtures import Fixture, legacy_database
from rolling.maintenance import flow, archive
from rolling import deployment
from rolling.io import Operations
from rolling.state import Journal


class PowerLoss(BaseException):
    pass


class MaintenanceTests(unittest.TestCase):
    def test_kernel_nfs_recovery_finishes_before_candidate_processes_start(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture = Fixture(Path(directory))
            attempts = []
            def resources(*args):
                attempts.append(len(attempts) + 1)
                if len(attempts) == 1:
                    raise OSError(errno.EIO, 'NFS client reconnecting')

            def prepared(*args):
                self.assertEqual(attempts, [1, 2])

            fixture.wait = lambda check, seconds: Operations.wait(fixture, check, seconds)
            with fixture.patches(), patch.object(flow.probes, 'resources', side_effect=resources), \
                    patch.object(flow.units, 'prepare', side_effect=prepared):
                result = flow.deploy(fixture.settings, fixture.bundle, fixture, 1)
            self.assertEqual(result['maintenance']['stage'], 'complete')

    def test_nfs_timeout_preserves_closed_gate_and_resumes_the_same_backup(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture = Fixture(Path(directory))
            fixture.wait = lambda check, seconds: Operations.wait(fixture, check, seconds)
            with fixture.patches():
                with patch.object(flow.probes, 'resources', side_effect=OSError(errno.EIO, 'NFS unavailable')), \
                        patch.object(flow.units, 'prepare') as prepare:
                    with self.assertRaisesRegex(TimeoutError, 'NFS unavailable'):
                        flow.deploy(fixture.settings, fixture.bundle, fixture, 0)
                    prepare.assert_not_called()
                pending = Journal(fixture.settings.state_dir).data['maintenance']
                self.assertEqual(pending['stage'], 'installing')
                self.assertFalse(pending['writes_open'])
                self.assertFalse(pending.get('schema_started', False))
                backup = Path(pending['backup'])
                sealed = archive.inventory(backup)
                result = flow.deploy(fixture.settings, fixture.bundle, fixture, 1)
            self.assertEqual(result['maintenance']['backup'], str(backup))
            self.assertEqual(result['maintenance']['stage'], 'complete')
            self.assertEqual(archive.inventory(backup), sealed)

    def test_resource_catalog_restore_keeps_auth_rows_and_database_inode(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture = Fixture(Path(directory))
            resource = fixture.settings.state_dir / 'resources/definitions.db'
            legacy_database(resource)
            with closing(sqlite3.connect(resource)) as conn:
                conn.execute('PRAGMA journal_mode=WAL')
                conn.executescript('''CREATE TABLE auth_writes(operation TEXT);
                    CREATE TRIGGER record_auth_update AFTER UPDATE ON platform_users
                    BEGIN INSERT INTO auth_writes VALUES ('update'); END;
                    CREATE TRIGGER record_auth_insert AFTER INSERT ON platform_users
                    BEGIN INSERT INTO auth_writes VALUES ('insert'); END;
                    CREATE TRIGGER record_auth_delete AFTER DELETE ON platform_users
                    BEGIN INSERT INTO auth_writes VALUES ('delete'); END;''')
            inode = resource.stat().st_ino
            migrate = fixture.migrate

            def migrate_both():
                migrate()
                with sqlite3.connect(resource) as conn:
                    conn.executescript('''DROP TABLE project_milestones;
                        ALTER TABLE project_todos RENAME COLUMN milestone_id TO initiative_id;
                        CREATE TABLE project_tags(id TEXT);
                        UPDATE schema_version SET version=32;''')

            fixture.migrate = migrate_both
            fixture.fail_internal = True
            with fixture.patches():
                with self.assertRaisesRegex(RuntimeError, 'candidate verification'):
                    flow.deploy(fixture.settings, fixture.bundle, fixture)
                state = Journal(fixture.settings.state_dir).data['maintenance']
                root = Path(state['backup'])
                sealed = archive.inventory(root)
                self.assertTrue((root / 'data/resources/definitions.db').is_file())
                flow.rollback(fixture.settings, fixture)
            self.assertEqual(resource.stat().st_ino, inode)
            self.assertEqual(archive.inventory(root), sealed)
            with sqlite3.connect(resource) as conn:
                self.assertEqual(conn.execute('SELECT version FROM schema_version').fetchone(), (31,))
                self.assertEqual(conn.execute('SELECT milestone_id FROM project_todos').fetchone(), ('i',))
                self.assertEqual(conn.execute('SELECT token_hash FROM platform_users').fetchone(), (b'\x00\x01\x02',))
                self.assertEqual(conn.execute('SELECT * FROM auth_writes').fetchall(), [])

    def test_wal_backups_remain_sealed_when_recovery_crashes_and_resumes(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture = Fixture(Path(directory))
            control = fixture.settings.server_data / 'control.db'
            with closing(sqlite3.connect(control)) as conn:
                conn.execute('CREATE TABLE admission(frozen INTEGER)')
            for database in (fixture.db, control, fixture.settings.state_dir / 'host/host.db'):
                with closing(sqlite3.connect(database)) as conn:
                    self.assertEqual(conn.execute('PRAGMA journal_mode=WAL').fetchone(), ('wal',))
            fixture.fail_internal = True
            checkpoint = flow.checkpoint
            def crash(journal, stage):
                checkpoint(journal, stage)
                if stage == 'restore_services':
                    raise PowerLoss()
            with fixture.patches():
                with self.assertRaisesRegex(RuntimeError, 'candidate verification'):
                    flow.deploy(fixture.settings, fixture.bundle, fixture)
                root = Path(Journal(fixture.settings.state_dir).data['maintenance']['backup'])
                sealed = archive.inventory(root)
                self.assertEqual((root / 'data/server/definitions.db').read_bytes()[18:20], b'\x02\x02')
                with patch.object(flow, 'checkpoint', side_effect=crash), self.assertRaises(PowerLoss):
                    flow.rollback(fixture.settings, fixture)
                archive.verify(root)
                self.assertEqual(archive.inventory(root), sealed)
                self.assertEqual(flow.rollback(fixture.settings, fixture)['maintenance']['stage'], 'rolled_back')
                archive.verify(root)
                self.assertEqual(archive.inventory(root), sealed)

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
            with self.assertRaisesRegex(ValueError, 'restoration is forbidden'):
                deployment.rollback(fixture.settings, fixture)
            with sqlite3.connect(fixture.db) as conn:
                self.assertEqual(conn.execute('SELECT version FROM schema_version').fetchone(), (32,))

    def test_failure_after_migration_restores_old_service_data_resources_and_controller(self):
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
                result = deployment.rollback(fixture.settings, fixture)
            self.assertEqual(result['current'], 'old')
            self.assertEqual(result['maintenance']['stage'], 'rolled_back')
            self.assertEqual(archive.inventory(root), original_hashes)
            self.assertEqual(fixture.settings.nginx_include.read_text(), 'old ingress\n')
            for workdir in (fixture.settings.agent_workdir, fixture.settings.server_workdir):
                self.assertEqual(json.loads((workdir / 'opencoder.json').read_text())['dag'], {'binary_dir': '/old/binaries'})
            self.assertEqual((fixture.settings.state_dir / 'services/opencoder-resources').read_bytes(), b'old resources')
            with sqlite3.connect(fixture.db) as conn:
                self.assertEqual(conn.execute('SELECT version FROM schema_version').fetchone(), (31,))
                self.assertEqual(conn.execute('SELECT token_hash FROM platform_users').fetchone(), (b'\x00\x01\x02',))
                conn.execute("INSERT INTO project_todos VALUES ('old-api-write','i')")
            self.assertFalse((fixture.settings.systemd_dir / 'opencoder-resources-new.service').exists())

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

    @unittest.skipUnless(os.geteuid() == 0, 'database ownership restoration requires root')
    def test_restore_preserves_server_database_owner_and_mode(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture = Fixture(Path(directory))
            control = fixture.settings.server_data / 'control.db'
            with sqlite3.connect(control) as conn:
                conn.execute('CREATE TABLE admission(frozen INTEGER)')
                conn.execute('INSERT INTO admission VALUES (1)')
            os.chown(control, 65534, 65534)
            control.chmod(0o640)
            fixture.fail_internal = True
            with fixture.patches():
                with self.assertRaisesRegex(RuntimeError, 'candidate verification'):
                    flow.deploy(fixture.settings, fixture.bundle, fixture)
                flow.rollback(fixture.settings, fixture)
            self.assertEqual((control.stat().st_uid, control.stat().st_gid, control.stat().st_mode & 0o777),
                             (65534, 65534, 0o640))

    def test_restore_skips_absent_legacy_units(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture = Fixture(Path(directory))
            output, run = fixture.output, fixture.run
            absent = fixture.settings.legacy_agent_unit
            def show(*args):
                if args[0] == 'systemctl' and args[2] == absent:
                    return 'not-found' if '--value' in args else 'LoadState=not-found\n'
                return output(*args)
            def command(*args):
                if args[0] == 'systemctl' and args[1] in ('enable', 'disable', 'start', 'stop') and absent in args:
                    raise RuntimeError('absent unit')
                return run(*args)
            fixture.output, fixture.run = show, command
            fixture.fail_internal = True
            with fixture.patches():
                with self.assertRaisesRegex(RuntimeError, 'candidate verification'):
                    flow.deploy(fixture.settings, fixture.bundle, fixture)
                self.assertEqual(flow.rollback(fixture.settings, fixture)['current'], 'old')

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
