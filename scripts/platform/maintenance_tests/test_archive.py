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


    def identity_pair(self, root):
        source, target = root / 'saved.db', root / 'live.db'
        with sqlite3.connect(source) as conn:
            conn.executescript("""CREATE TABLE schema_version(version INTEGER NOT NULL);
                INSERT INTO schema_version VALUES (34);
                CREATE TABLE platform_users(name TEXT PRIMARY KEY,token_hash TEXT NOT NULL UNIQUE,role TEXT NOT NULL,created_at INTEGER NOT NULL);
                CREATE TABLE project_todos(id TEXT PRIMARY KEY);
                INSERT INTO project_todos VALUES ('before');""")
            conn.executemany('INSERT INTO platform_users VALUES (?,?,?,?)',
                             [('admin','digest-a','admin',1),('reader','digest-b','user',2),('legacy','digest-c','root',3)])
        target.write_bytes(source.read_bytes())
        with sqlite3.connect(target) as conn:
            conn.executescript("""ALTER TABLE platform_users RENAME TO platform_users_legacy;
                CREATE TABLE platform_users(name TEXT PRIMARY KEY,role TEXT NOT NULL,created_at INTEGER NOT NULL);
                INSERT INTO platform_users SELECT name,CASE WHEN role IN ('user','root') THEN 'viewer' ELSE role END,created_at FROM platform_users_legacy;
                CREATE TABLE platform_tokens(id TEXT PRIMARY KEY,user_name TEXT NOT NULL REFERENCES platform_users(name) ON DELETE CASCADE,name TEXT NOT NULL,token_hash TEXT NOT NULL UNIQUE,created_at INTEGER NOT NULL,expires_at INTEGER,revoked_at INTEGER);
                INSERT INTO platform_tokens SELECT 'user:' || name,name,'初始 Token',token_hash,created_at,NULL,NULL FROM platform_users_legacy;
                DROP TABLE platform_users_legacy;
                CREATE INDEX idx_platform_tokens_user ON platform_tokens(user_name);
                UPDATE schema_version SET version=35;
                INSERT INTO project_todos VALUES ('during-upgrade');""")
        return source, target

    def test_unopened_identity_migration_restores_old_schema_without_changing_any_token(self):
        with tempfile.TemporaryDirectory() as directory:
            source, target = self.identity_pair(Path(directory))
            restore_projects(source, target)
            restore_projects(source, target)
            with sqlite3.connect(target) as conn, sqlite3.connect(source) as saved:
                self.assertEqual(conn.execute('SELECT * FROM platform_users ORDER BY name').fetchall(),
                                 saved.execute('SELECT * FROM platform_users ORDER BY name').fetchall())
                self.assertEqual(unrelated(conn), unrelated(saved))
                self.assertEqual(conn.execute('SELECT version FROM schema_version').fetchone(), (34,))
                self.assertEqual(conn.execute('SELECT * FROM project_todos').fetchall(), [('before',)])

    def test_identity_restore_refuses_any_new_or_changed_credential_or_identity(self):
        for mutation in ["UPDATE platform_tokens SET token_hash='changed' WHERE user_name='admin'",
                         "UPDATE platform_tokens SET expires_at=1", "UPDATE platform_tokens SET revoked_at=1",
                         "UPDATE platform_users SET role='editor' WHERE name='reader'",
                         "INSERT INTO platform_tokens VALUES ('extra','admin','new','extra-digest',9,NULL,NULL)"]:
            with self.subTest(mutation=mutation), tempfile.TemporaryDirectory() as directory:
                source, target = self.identity_pair(Path(directory))
                with sqlite3.connect(target) as conn:
                    conn.execute(mutation)
                    before = unrelated(conn)
                with self.assertRaisesRegex(ValueError, 'non-project'):
                    restore_projects(source, target)
                with sqlite3.connect(target) as conn:
                    self.assertEqual(unrelated(conn), before)
                    self.assertEqual(conn.execute('SELECT version FROM schema_version').fetchone(), (35,))
                    self.assertEqual(conn.execute('SELECT count(*) FROM project_todos').fetchone(), (2,))
