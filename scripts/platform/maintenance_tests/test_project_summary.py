"""Schema 37 rollback preserves every non-project row in disposable fixtures."""
from pathlib import Path
import sqlite3
import tempfile
import unittest
import test_archive
from rolling.maintenance.archive import restore_projects, unrelated
from rolling.maintenance.preflight import database_inventory


class ProjectSummaryTests(unittest.TestCase):
    def test_summary_schema_inventory_and_rollback_preserve_credentials(self):
        for source_version in (34, 35, 36):
            with self.subTest(source_version=source_version), tempfile.TemporaryDirectory() as directory:
                source, target = test_archive.ArchiveTests().identity_pair(Path(directory))
                if source_version >= 35:
                    source.write_bytes(target.read_bytes())
                    with sqlite3.connect(source) as conn:
                        conn.execute('UPDATE schema_version SET version=?', (source_version,))
                with sqlite3.connect(target) as conn:
                    conn.executescript("""CREATE TABLE project_goals(id TEXT PRIMARY KEY,
                        current_summary TEXT NOT NULL DEFAULT '', summary_updated_at INTEGER);
                        INSERT INTO project_goals VALUES('p','summary',100);
                        CREATE TABLE project_summary_versions(scope_type TEXT, scope_id TEXT,
                            version INTEGER, current_summary TEXT, created_at INTEGER, restored_from INTEGER);
                        INSERT INTO project_summary_versions VALUES('project','p',1,'summary',100,NULL);
                        UPDATE schema_version SET version=37;""")
                self.assertEqual(database_inventory(target)['schema_version'], 37)
                with sqlite3.connect(source) as conn:
                    saved_auth = unrelated(conn)
                restore_projects(source, target)
                with sqlite3.connect(target) as conn:
                    self.assertEqual(unrelated(conn), saved_auth)
                    self.assertEqual(conn.execute('SELECT version FROM schema_version').fetchone(), (source_version,))
