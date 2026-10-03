from pathlib import Path
import sqlite3
import sys
import tempfile
import unittest
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from rolling.maintenance.archive import restore_projects, unrelated
from fixtures import legacy_database


class ArchiveTests(unittest.TestCase):
    def test_restore_reinstates_v31_projects_indexes_and_old_writes_without_auth_writes(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source, target = root / 'saved.db', root / 'live.db'
            legacy_database(source)
            target.write_bytes(source.read_bytes())
            with sqlite3.connect(target) as conn:
                conn.executescript('''DROP TABLE project_milestones;
                    ALTER TABLE project_todos RENAME COLUMN milestone_id TO initiative_id;
                    CREATE TABLE project_tags(id TEXT);
                    UPDATE schema_version SET version=32;''')
                before = unrelated(conn)
            restore_projects(source, target)
            restore_projects(source, target)
            with sqlite3.connect(target) as conn:
                self.assertEqual(unrelated(conn), before)
                self.assertEqual(conn.execute('SELECT * FROM schema_version').fetchall(), [(31,)])
                self.assertEqual(conn.execute('SELECT milestone_id FROM project_todos').fetchall(), [('i',)])
                conn.execute("UPDATE project_milestones SET title='restored' WHERE id='i'")
                conn.execute("INSERT INTO project_todos VALUES ('second','i')")
                self.assertEqual(conn.execute('SELECT count(*) FROM project_todos').fetchone(), (2,))
                self.assertEqual(conn.execute("SELECT name FROM sqlite_schema WHERE type='index' AND name='idx_project_todos_milestone'").fetchone(), ('idx_project_todos_milestone',))
                self.assertIsNone(conn.execute("SELECT name FROM sqlite_schema WHERE name='project_tags'").fetchone())

    def test_changed_authentication_refuses_restore_and_leaves_project_rows_intact(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source, target = root / 'saved.db', root / 'live.db'
            legacy_database(source)
            target.write_bytes(source.read_bytes())
            with sqlite3.connect(target) as conn:
                conn.execute("UPDATE platform_users SET token_hash=X'03'")
                conn.execute("UPDATE project_milestones SET title='new-data'")
            with self.assertRaisesRegex(ValueError, 'non-project'):
                restore_projects(source, target)
            with sqlite3.connect(target) as conn:
                self.assertEqual(conn.execute('SELECT title FROM project_milestones').fetchone(), ('new-data',))
                self.assertEqual(conn.execute('SELECT token_hash FROM platform_users').fetchone(), (b'\x03',))
