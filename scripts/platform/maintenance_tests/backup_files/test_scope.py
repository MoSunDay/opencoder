import dataclasses
import json
from pathlib import Path
import socket
import sqlite3
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[2]))
from fixtures import Fixture
from rolling import backup
from rolling.maintenance import archive, flow
from rolling.maintenance.planning import capacity
from rolling.state import Journal


def execution_data(root):
    root.mkdir(parents=True, exist_ok=True)
    database = root / 'runtime.db'
    with sqlite3.connect(database) as conn:
        conn.execute('CREATE TABLE execution_context(id TEXT, content TEXT)')
        conn.execute("INSERT INTO execution_context VALUES ('shared-session', 'keep context')")
    (root / 'artifact').write_bytes(b'execution output')
    return [database, root / 'artifact']


class MaintenanceBackupScopeTests(unittest.TestCase):
    def test_shared_backup_requires_stopped_writers(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture = Fixture(Path(directory))
            output = Path(directory) / 'backup'
            with self.assertRaisesRegex(ValueError, 'requires stopped writers'):
                backup.snapshot(fixture.settings, output, runtime_data=False)
            self.assertFalse(output.exists())

    def test_online_backup_still_includes_existing_execution_databases(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            fixture = Fixture(root)
            runtime = fixture.settings.state_dir / 'runtimes/old'
            execution_data(runtime)
            output = root / 'backup'
            backup.snapshot(fixture.settings, output)
            with sqlite3.connect(output / 'runtimes/old/runtime.db') as conn:
                self.assertEqual(conn.execute('SELECT content FROM execution_context').fetchone(),
                                 ('keep context',))
            manifest = json.loads((output / 'backup-manifest.json').read_text())
            self.assertTrue(manifest['runtime_data_included'])
            self.assertFalse(manifest['cross_database_snapshot'])

    def test_capacity_and_shared_snapshot_never_traverse_retained_execution_trees(self):
        with tempfile.TemporaryDirectory() as directory, socket.socket(socket.AF_UNIX) as listener:
            root = Path(directory)
            fixture = Fixture(root)
            fixture.settings = dataclasses.replace(fixture.settings, legacy_agent_data=root / 'legacy')
            original = Journal(fixture.settings.state_dir).data
            before = capacity.plan(fixture.settings, fixture.candidate, fixture.rootfs, original, {})
            for execution in (fixture.settings.legacy_agent_data,
                              fixture.settings.state_dir / 'runtimes/old'):
                execution_data(execution)
                with (execution / 'large-read-only-image').open('wb') as stream:
                    stream.truncate(1024 * 1024 * 1024)
            listener.bind(str(fixture.settings.legacy_agent_data / 'unrelated.sock'))
            self.assertEqual(capacity.plan(fixture.settings, fixture.candidate,
                                          fixture.rootfs, original, {}), before)
            output = root / 'backup'
            backup.snapshot(fixture.settings, output, stopped=True, runtime_data=False)
            self.assertFalse((output / 'runtimes').exists())
            self.assertFalse((output / 'legacy-node').exists())
            self.assertTrue((output / 'server/definitions.db').is_file())
            self.assertTrue((output / 'host/host.db').is_file())
            manifest = json.loads((output / 'backup-manifest.json').read_text())
            self.assertFalse(manifest['runtime_data_included'])
            self.assertEqual(set(manifest['data_roots']), {'server', 'host'})

    def test_failed_upgrade_and_repeated_rollback_preserve_execution_context_and_inodes(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            fixture = Fixture(root)
            fixture.settings = dataclasses.replace(fixture.settings, legacy_agent_data=root / 'legacy')
            paths = execution_data(fixture.settings.legacy_agent_data)
            paths += execution_data(fixture.settings.state_dir / 'runtimes/old')
            before = {p: (p.stat().st_ino, p.read_bytes()) for p in paths}
            fixture.fail_internal = True
            with fixture.patches():
                with self.assertRaisesRegex(RuntimeError, 'candidate verification'):
                    flow.deploy(fixture.settings, fixture.bundle, fixture)
                state = Journal(fixture.settings.state_dir).data['maintenance']
                saved = Path(state['backup'])
                sealed = archive.verify(saved)
                self.assertFalse((saved / 'data/runtimes').exists())
                self.assertFalse((saved / 'data/legacy-node').exists())
                for _ in range(2):
                    flow.rollback(fixture.settings, fixture)
                    self.assertEqual({p: (p.stat().st_ino, p.read_bytes()) for p in paths}, before)
                    self.assertEqual(archive.verify(saved), sealed)
            with sqlite3.connect(fixture.db) as conn:
                self.assertEqual(conn.execute('SELECT version FROM schema_version').fetchone(), (31,))
                self.assertEqual(conn.execute('SELECT milestone_id FROM project_todos').fetchone(), ('i',))
            self.assertIn(fixture.old['server_unit'], fixture.active)


if __name__ == '__main__':
    unittest.main()
