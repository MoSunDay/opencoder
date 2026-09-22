import json
import sqlite3
import shutil
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch
from .apply import apply
from .model import digest, execution_scope, selected_row


class CleanupTests(unittest.TestCase):
    def test_scope_follows_only_retired_parent_links(self):
        def row(kind, data):
            return {'request': {'kind': kind, 'input': data}}
        assignments = {'old': row('brain', {'schema_version': 3}),
                       'new': row('brain', {'schema_version': 4}),
                       'child': row('operator', {'brain_scheduler': {'run_id': 'old'}}),
                       'unrelated': row('operator', {})}
        self.assertEqual(execution_scope(assignments), ({'old'}, {'old', 'child'}))
        old_origin = {'child': row('agent', {'brain_origin': {'run_id': 'archived'}})}
        self.assertEqual(execution_scope(old_origin, {'archived'}), ({'archived'}, {'child'}))
        self.assertEqual(execution_scope({'empty': row('brain', None)}), ({'empty'}, {'empty'}))
        self.assertFalse(selected_row('platform_users', {'id': 'old'}, {'old'}, set()))
        self.assertFalse(selected_row('brain_layered_runs', {'run_id': 'new'}, {'old'}, set()))

    def fixture(self, directory):
        database = directory / 'test.db'
        with sqlite3.connect(database) as connection:
            connection.executescript('CREATE TABLE execution_index(id TEXT PRIMARY KEY, status TEXT);'
                                     "INSERT INTO execution_index VALUES('old','done'),('new','running');")
            connection.row_factory = sqlite3.Row
            row = dict(connection.execute("SELECT rowid AS rowid,* FROM execution_index WHERE id='old'").fetchone())
        journal = directory / 'brain' / 'old'
        journal.mkdir(parents=True)
        record = {'assignment': {'index': {'id': 'old'}}}
        (journal / 'execution.json').write_text(json.dumps(record))
        return {'mixed_plan_definitions': [], 'executions': [{'id': 'old'}], 'databases': [{'database': str(database), 'table': 'execution_index',
                 'rows': [{'key': {'id': 'old'}, 'digest': digest(row)}]}],
                'directories': [{'path': str(journal), 'id': 'old', 'record_digest': digest(record)}]}

    def test_exact_removal_backup_and_repeat(self):
        with tempfile.TemporaryDirectory() as root:
            root = Path(root); manifest = self.fixture(root)
            apply(manifest, digest(manifest), root / 'backup')
            apply(manifest, digest(manifest), root / 'backup')
            with sqlite3.connect(root / 'test.db') as connection:
                self.assertEqual(connection.execute('SELECT id FROM execution_index').fetchall(), [('new',)])
            self.assertTrue((root / 'backup/execution-0/execution.json').exists())
            self.assertFalse((root / 'brain/old').exists())

    def test_changed_row_aborts_without_removal(self):
        with tempfile.TemporaryDirectory() as root:
            root = Path(root); manifest = self.fixture(root)
            with sqlite3.connect(root / 'test.db') as connection:
                connection.execute("UPDATE execution_index SET status='running' WHERE id='old'")
            with self.assertRaisesRegex(ValueError, 'row changed'):
                apply(manifest, digest(manifest), root / 'backup')
            self.assertTrue((root / 'brain/old/execution.json').exists())
            with sqlite3.connect(root / 'test.db') as connection:
                self.assertEqual(connection.execute('SELECT count(*) FROM execution_index').fetchone()[0], 2)

    def test_shared_ownership_rollback_preserves_other_runtime_writes(self):
        with tempfile.TemporaryDirectory() as root:
            root = Path(root); manifest = self.fixture(root)
            database = root / 'host.db'
            with sqlite3.connect(database) as connection:
                connection.executescript('CREATE TABLE runtime_owners(execution_id TEXT PRIMARY KEY, status TEXT);'
                                         "INSERT INTO runtime_owners VALUES('old','done'),('other','running');")
                connection.row_factory = sqlite3.Row
                row = dict(connection.execute("SELECT rowid AS rowid,* FROM runtime_owners WHERE execution_id='old'").fetchone())
            manifest['databases'].append({'database': str(database), 'table': 'runtime_owners',
                'rows': [{'key': {'execution_id': 'old'}, 'digest': digest(row)}]})
            cache = self.cached_inventory(database, manifest)
            write = Path.write_text
            move = shutil.move

            def move_while_other_runtime_writes(source, destination):
                if Path(destination).name.startswith('execution-'):
                    with sqlite3.connect(database, timeout=0.1) as connection:
                        connection.execute("UPDATE runtime_owners SET status='checkpointed' WHERE execution_id='other'")
                return move(source, destination)

            def fail_receipt(path, *args, **kwargs):
                if path.name == 'completed.json':
                    with sqlite3.connect(database) as connection:
                        connection.execute("UPDATE runtime_owners SET status='done' WHERE execution_id='other'")
                    raise OSError('receipt write failed')
                return write(path, *args, **kwargs)

            with patch.object(Path, 'write_text', fail_receipt), patch.object(shutil, 'move', move_while_other_runtime_writes), self.assertRaisesRegex(OSError, 'receipt write failed'):
                apply(manifest, digest(manifest), root / 'backup')
            with sqlite3.connect(database) as connection:
                self.assertEqual(connection.execute('SELECT execution_id,status FROM runtime_owners ORDER BY execution_id').fetchall(),
                                 [('old', 'done'), ('other', 'done')])
            self.assertTrue((root / 'brain/old/execution.json').exists())
            with sqlite3.connect(database) as connection:
                self.assertEqual(json.loads(connection.execute("SELECT body FROM fleet_definitions WHERE id='runtime-old'").fetchone()[0]), cache)
            with sqlite3.connect(root / 'test.db') as connection:
                self.assertEqual(connection.execute('SELECT count(*) FROM execution_index').fetchone()[0], 2)

    def cached_inventory(self, database, manifest):
        body = {'runtime_id': 'runtime-old', 'indexes': [{'id': 'old'}, {'id': 'other'}], 'build': {'commit': 'preserve'}}
        with sqlite3.connect(database) as connection:
            connection.execute('CREATE TABLE fleet_definitions(kind TEXT, id TEXT, body TEXT, PRIMARY KEY(kind,id))')
            connection.execute('INSERT INTO fleet_definitions VALUES(?,?,?)', ('runtime_sleep', 'runtime-old', json.dumps(body)))
        manifest['cached_inventories'] = [{'database': str(database), 'id': 'runtime-old', 'removed_execution_ids': ['old'], 'body_digest': digest(body)}]
        return body

    def test_cached_inventory_prunes_only_reviewed_ids_and_repeats(self):
        with tempfile.TemporaryDirectory() as root:
            root = Path(root); manifest = self.fixture(root); database = root / 'host.db'
            with sqlite3.connect(database) as connection:
                connection.execute('CREATE TABLE runtime_owners(execution_id TEXT PRIMARY KEY)')
                connection.execute("INSERT INTO runtime_owners VALUES('old')")
                connection.row_factory = sqlite3.Row
                row = dict(connection.execute('SELECT rowid AS rowid,* FROM runtime_owners').fetchone())
            manifest['databases'].append({'database': str(database), 'table': 'runtime_owners', 'rows': [{'key': {'execution_id': 'old'}, 'digest': digest(row)}]})
            self.cached_inventory(database, manifest)
            apply(manifest, digest(manifest), root / 'backup')
            apply(manifest, digest(manifest), root / 'backup')
            with sqlite3.connect(database) as connection:
                cached = json.loads(connection.execute("SELECT body FROM fleet_definitions WHERE id='runtime-old'").fetchone()[0])
                self.assertEqual(cached['indexes'], [{'id': 'other'}])
                self.assertEqual(cached['build'], {'commit': 'preserve'})


if __name__ == '__main__':
    unittest.main()
