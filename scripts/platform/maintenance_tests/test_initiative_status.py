"""Rollback of schema 38 uses disposable project data and preserves credentials."""
from pathlib import Path
import sqlite3
import tempfile
import unittest
import test_archive
from rolling.maintenance.archive import restore_projects, unrelated
from rolling.maintenance.preflight import database_inventory


class InitiativeStatusTests(unittest.TestCase):
    def test_status_removal_can_restore_previous_project_shape_without_auth_writes(self):
        with tempfile.TemporaryDirectory() as directory:
            source, target = test_archive.ArchiveTests().identity_pair(Path(directory))
            source.write_bytes(target.read_bytes())
            with sqlite3.connect(source) as conn:
                conn.executescript("""CREATE TABLE project_initiatives(
                    id TEXT PRIMARY KEY, title TEXT, status TEXT, current_summary TEXT);
                    INSERT INTO project_initiatives VALUES('i','Direction','done','Summary');
                    UPDATE schema_version SET version=37;""")
                original = unrelated(conn)
            target.write_bytes(source.read_bytes())
            with sqlite3.connect(target) as conn:
                conn.executescript("""ALTER TABLE project_initiatives DROP COLUMN status;
                    UPDATE schema_version SET version=38;""")
            self.assertEqual(database_inventory(target)['schema_version'], 38)
            for _ in range(2):
                restore_projects(source, target)
                with sqlite3.connect(target) as conn:
                    self.assertEqual(unrelated(conn), original)
                    self.assertEqual(conn.execute('SELECT * FROM project_initiatives').fetchall(),
                                     [('i', 'Direction', 'done', 'Summary')])
                    self.assertEqual(conn.execute('SELECT version FROM schema_version').fetchone(), (37,))
