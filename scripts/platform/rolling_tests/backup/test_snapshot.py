"""Real SQLite snapshots never traverse immutable execution input replicas."""
from contextlib import closing
import json
import os
from pathlib import Path
import sqlite3
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[2]))
from rolling import backup
from rolling.config import Settings


class BackupScopeTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.settings = Settings(self.root / 'state', self.root / 'token')
        self.runtime = self.settings.state_dir / 'runtimes/retained'
        self.kept = [
            self.settings.server_data / 'control.db',
            self.settings.server_data / 'nested/runtime.db',
            self.settings.state_dir / 'host/host.db',
            self.settings.state_dir / 'resources/control.db',
            self.settings.state_dir / 'resources/resources/user.db',
            self.runtime / 'runtime.db',
            self.runtime / 'operator/op/home/.pki/cert9.db',
            self.runtime / 'operator/op/workspace/resources/work.db',
            self.runtime / 'operator/op/workspace/rootfs-upper/work.db',
            self.runtime / 'project-runs/prun-example/session.db',
            self.runtime / '.cleared-executions-20261009/operator/old/home/runtime.db',
        ]
        self.execution = self.runtime / 'agent/accepted'
        self.execution.mkdir(parents=True)
        (self.execution / 'execution.json').write_text('{}')
        self.pending = self.runtime / 'todos/preparing'
        self.pending.mkdir(parents=True)
        (self.pending / 'pending-create.json').write_text('{}')
        self.archived = self.runtime / '.cleared-executions-20261009/agent/old'
        self.archived.mkdir(parents=True)
        (self.archived / 'execution.json').write_text('{}')
        self.run = self.runtime / 'dag/runs/2026-10-12/definition/run'
        (self.run / 'bundle').mkdir(parents=True)
        (self.run / 'container.json').write_text('{"id":"dag-run-run"}')
        self.oci(True)
        self.kept.extend([
            self.run / 'upper/execute/runtime.db',
            self.run / 'workspace/execute/runtime.db',
        ])
        self.excluded = {
            self.execution / 'resources': 'memory/pinned/v1/resource.db',
            self.pending / 'resources.staging-123': 'memory/pinned/v1/resource.db',
            self.archived / 'resources': 'memory/pinned/v1/resource.db',
            self.runtime / 'resources': 'old-execution/memory/pinned.db',
            self.runtime / 'project-resources': 'prun-example/memory/pinned.db',
            self.runtime / 'dag/rootfs': 'image/input.db',
            self.run / 'rootfs-upper': 'run/opencoder/knowledge/artifacts/tmp/node-state/runtime.db',
            self.run / 'rootfs-work': 'work/mount.db',
            self.run / 'bundle/rootfs': 'run/opencoder/knowledge/artifacts/node-state/runtime.db',
        }
        for path in self.kept:
            self.database(path)
        for directory, relative in self.excluded.items():
            self.database(directory / relative)

    def oci(self, readonly):
        (self.run / 'bundle/config.json').write_text(json.dumps({
            'ociVersion': '1.0.0', 'root': {'path': 'rootfs', 'readonly': readonly}}))

    def database(self, path):
        path.parent.mkdir(parents=True, exist_ok=True)
        with closing(sqlite3.connect(path)) as connection:
            connection.execute('CREATE TABLE evidence(value TEXT)')
            connection.execute('INSERT INTO evidence VALUES (?)', (str(path.relative_to(self.root)),))
            connection.commit()

    def verify(self, output, source):
        relative = source.relative_to(self.settings.state_dir)
        target = output / relative
        with closing(sqlite3.connect(target)) as connection:
            self.assertEqual(connection.execute('SELECT value FROM evidence').fetchone(),
                             (str(source.relative_to(self.root)),))
        self.assertIn(relative.as_posix(), json.loads((output / 'backup-manifest.json').read_text())['files'])

    def test_online_prunes_replicas_before_any_directory_read_and_keeps_real_databases(self):
        original = os.scandir
        def guarded(path):
            if Path(path) in self.excluded:
                raise AssertionError('online backup entered an immutable replica')
            return original(path)
        output = self.root / 'backup'
        with patch('os.scandir', side_effect=guarded):
            backup.snapshot(self.settings, output)
        for source in self.kept:
            self.verify(output, source)
        manifest = json.loads((output / 'backup-manifest.json').read_text())
        self.assertEqual(set(manifest['excluded_database_trees']),
                         {str(path.relative_to(self.settings.state_dir)) for path in self.excluded})
        self.assertEqual(manifest['kind'], 'independent-online-database-backups')
        self.assertFalse(manifest['cross_database_snapshot'])
        self.assertTrue(manifest['runtime_data_included'])

    def test_online_preserves_same_names_in_user_data_and_unrecognized_container_trees(self):
        paths = [
            self.runtime / 'operator/op/home/resources/private.db',
            self.runtime / 'operator/op/workspace/agent/copied/resources/user.db',
            self.runtime / 'operator/op/workspace/dag/runs/2026-10-12/id/run/rootfs-upper/user.db',
            self.settings.server_data / 'rootfs-upper/real.db',
            self.runtime / 'agent/unjournaled/resources/real.db',
            self.runtime / 'workspace/agent/copied/resources/user.db',
            self.runtime / 'workspace/dag/runs/2026-10-12/id/run/rootfs-upper/user.db',
        ]
        for path in paths:
            self.database(path)
        for path in (paths[1], paths[5]):
            (path.parent.parent / 'execution.json').write_text('{}')
        for path in (paths[2], paths[6]):
            user_run = path.parent.parent
            (user_run / 'bundle').mkdir()
            (user_run / 'container.json').write_text('{}')
            (user_run / 'bundle/config.json').write_bytes((self.run / 'bundle/config.json').read_bytes())
        output = self.root / 'user-backup'
        backup.snapshot(self.settings, output)
        for source in paths:
            self.verify(output, source)

    def test_non_sqlite_process_store_is_preserved_as_bytes_without_sql_reads(self):
        source = self.runtime / 'operator/op/home/.vendor/tracking_v2.db'
        source.parent.mkdir(parents=True)
        source.write_bytes(b'opaque-process-store' * 32)
        source.chmod(0o600)
        before = source.read_bytes()
        output = self.root / 'opaque-backup'
        backup.snapshot(self.settings, output)
        relative = source.relative_to(self.settings.state_dir)
        self.assertEqual((output / relative).read_bytes(), before)
        self.assertEqual(source.read_bytes(), before)
        self.assertEqual((output / relative).stat().st_mode & 0o777, 0o600)
        manifest = json.loads((output / 'backup-manifest.json').read_text())
        self.assertEqual(manifest['opaque_process_database_files'][relative.as_posix()],
                         {'kind': 'stable-byte-copy', 'sqlite': False})
        self.assertFalse(manifest['cross_database_snapshot'])

    def test_invalid_platform_database_and_corrupt_sqlite_process_store_fail(self):
        for name, source in [('platform', self.runtime / 'runtime.db'),
                             ('process-sqlite', self.runtime / 'operator/op/home/sqlite.db')]:
            with self.subTest(name=name):
                old = (self.runtime / 'runtime.db').read_bytes()
                source.parent.mkdir(parents=True, exist_ok=True)
                source.write_bytes(b'invalid' if name == 'platform'
                                   else b'SQLite format 3\x00' + b'broken' * 100)
                output = self.root / (name + '-backup')
                with self.assertRaises(sqlite3.DatabaseError):
                    backup.snapshot(self.settings, output)
                self.assertFalse(output.exists())
                if name == 'platform':
                    source.write_bytes(old)
                else:
                    source.unlink()

    def test_writable_or_unrecognized_oci_root_is_never_pruned(self):
        for mode in ('writable', 'unknown', 'wrong-root'):
            with self.subTest(mode=mode):
                self.oci(mode != 'writable')
                if mode == 'unknown':
                    (self.run / 'container.json').unlink()
                elif mode == 'wrong-root':
                    (self.run / 'container.json').write_text('{}')
                    (self.run / 'bundle/config.json').write_text(json.dumps({
                        'root': {'path': 'custom-root', 'readonly': True}}))
                output = self.root / mode
                backup.snapshot(self.settings, output)
                for path in self.excluded:
                    if path.is_relative_to(self.run):
                        self.verify(output, path / self.excluded[path])

    def test_stopped_backup_preserves_all_nested_databases_and_execution_files(self):
        output = self.root / 'stopped'
        backup.snapshot(self.settings, output, stopped=True)
        for source in [*self.kept, *(p / name for p, name in self.excluded.items())]:
            self.verify(output, source)
        journal = self.execution.relative_to(self.settings.state_dir) / 'execution.json'
        self.assertEqual((output / journal).read_bytes(), b'{}')
        manifest = json.loads((output / 'backup-manifest.json').read_text())
        self.assertEqual(manifest.get('excluded_database_trees', {}), {})
        self.assertEqual(manifest['kind'], 'stopped-consistent-copy')
        self.assertTrue(manifest['cross_database_snapshot'])

    def test_online_failure_keeps_only_private_incomplete_staging(self):
        output = self.root / 'interrupted'
        with patch('rolling.backup.database', side_effect=OSError('disk read failed')):
            with self.assertRaisesRegex(OSError, 'disk read failed'):
                backup.snapshot(self.settings, output)
        self.assertFalse(output.exists())
        stages = list(self.root.glob('.interrupted.incomplete-*'))
        self.assertEqual(len(stages), 1)
        self.assertFalse((stages[0] / 'backup-manifest.json').exists())
        self.assertTrue(all(source.exists() for source in self.kept))

    def test_invalid_oci_receipt_fails_without_publishing_or_guessing_exclusions(self):
        (self.run / 'bundle/config.json').write_text('not valid json')
        output = self.root / 'invalid'
        with self.assertRaises(json.JSONDecodeError):
            backup.snapshot(self.settings, output)
        self.assertFalse(output.exists())
        self.assertEqual(len(list(self.root.glob('.invalid.incomplete-*'))), 1)


if __name__ == '__main__':
    unittest.main()
