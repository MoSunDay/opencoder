"""Schema 36 maintenance rollback only touches disposable database fixtures."""
from pathlib import Path
import sqlite3
import tempfile
import unittest
import test_archive
from rolling.maintenance.archive import restore_projects, unrelated


def upgrade_people(path):
    with sqlite3.connect(path) as conn:
        conn.executescript("""CREATE TABLE project_people(object_id TEXT,user_name TEXT);
            CREATE INDEX idx_project_people_object ON project_people(object_id);
            CREATE TRIGGER project_people_delete_todo AFTER DELETE ON project_todos
            BEGIN DELETE FROM project_people WHERE object_id=OLD.id; END;
            UPDATE schema_version SET version=36;""")


class ProjectPeopleTests(unittest.TestCase):
    def test_direct_legacy_upgrade_and_v35_upgrade_restore_without_credential_changes(self):
        for source_version in (34, 35):
            with self.subTest(source_version=source_version), tempfile.TemporaryDirectory() as directory:
                source, target = test_archive.ArchiveTests().identity_pair(Path(directory))
                if source_version == 35:
                    source.write_bytes(target.read_bytes())
                upgrade_people(target)
                with sqlite3.connect(source) as conn:
                    saved_auth = unrelated(conn)
                inode = target.stat().st_ino
                for _ in range(2):
                    restore_projects(source, target)
                    with sqlite3.connect(target) as conn:
                        self.assertEqual(unrelated(conn), saved_auth)
                        self.assertEqual(conn.execute('SELECT version FROM schema_version').fetchone(), (source_version,))
                        self.assertEqual(conn.execute("SELECT name FROM sqlite_schema WHERE name LIKE '%project_people%'").fetchall(), [])
                    self.assertEqual(target.stat().st_ino, inode)

    def test_changed_credentials_refuse_v36_rollback_without_partial_writes(self):
        with tempfile.TemporaryDirectory() as directory:
            source, target = test_archive.ArchiveTests().identity_pair(Path(directory))
            upgrade_people(target)
            with sqlite3.connect(target) as conn:
                conn.execute("UPDATE platform_tokens SET revoked_at=9")
            before = target.read_bytes()
            with self.assertRaisesRegex(ValueError, 'credential changed'):
                restore_projects(source, target)
            self.assertEqual(target.read_bytes(), before)
