"""Schema checks are read-only; only verified maintenance can authorize DDL."""
import hashlib
import json
from pathlib import Path
import sqlite3
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[2]))
from rolling.native import ontology_schema as schema
from rolling.state import Journal


class OntologySchemaTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        root = Path(self.temp.name)
        self.settings = SimpleNamespace(state_dir=root, server_data=root / 'server')
        self.settings.server_data.mkdir()
        self.database = self.settings.server_data / 'ontology.db'
        self.files = root / 'files'
        self.files.mkdir()
        with sqlite3.connect(self.database) as db:
            for name, sql in schema.contract().items():
                if not name.startswith('catalog_'):
                    db.execute(sql)
            db.execute('INSERT INTO ontology_schema_version VALUES(1,?)', (str(self.files),))
            db.execute("INSERT INTO environments VALUES(1,'debug','{}')")
        self.candidate = {'ontology_catalog_schema': 1, 'release_id': 'candidate'}

    def test_old_database_rejected_before_prewarm_and_read_only_preflight(self):
        before = self.database.read_bytes()
        with self.assertRaisesRegex(ValueError, '--maintenance'):
            schema.check(self.settings, self.candidate)
        observed = schema.check(self.settings, self.candidate, maintenance=True)
        self.assertEqual(observed['missing'], ['catalog_entries', 'catalog_objects', 'catalog_state'])
        self.assertEqual(self.database.read_bytes(), before)
        with sqlite3.connect(self.database) as db:
            for name in observed['missing']:
                db.execute(schema.contract()[name])
        self.assertEqual(schema.check(self.settings, self.candidate)['missing'], [])

    def test_constraints_indexes_and_binding_are_checked(self):
        with sqlite3.connect(self.database) as db:
            db.execute('CREATE TABLE catalog_entries(env_num INTEGER,id TEXT,body TEXT)')
        with self.assertRaisesRegex(ValueError, 'conflicts with catalog_entries'):
            schema.check(self.settings, self.candidate, maintenance=True)
        with sqlite3.connect(self.database) as db:
            db.execute('DROP TABLE catalog_entries')
            db.execute('DROP INDEX entities_entity_type_id')
        self.assertIn('entities_entity_type_id', schema.inspect(self.database)['missing'])
        with self.assertRaisesRegex(ValueError, 'binding'):
            schema.inspect(self.database, files=self.files.parent / 'other')

    def test_metadata_has_no_bool_or_unknown_version_escape(self):
        self.assertNotEqual(schema.normalized("CREATE INDEX i ON t(json_extract(body,'$.KEY'))"),
                            schema.normalized("CREATE INDEX i ON t(json_extract(body,'$.key'))"))
        self.assertEqual(schema.version({}), 0)
        for value in [True, None, '1', 2, -1]:
            with self.subTest(value=value), self.assertRaises(ValueError):
                schema.version({'ontology_catalog_schema': value})

    def test_authorization_requires_verified_backup_and_completion_requires_schema(self):
        journal = Journal(self.settings.state_dir)
        backup = self.settings.state_dir / 'backup'
        backup.mkdir()
        (backup / 'manifest.json').write_text('{}')
        journal.data['maintenance'] = {'stage': 'installing', 'writes_open': False,
                                      'target': 'candidate', 'backup': str(backup)}
        record = {'id': 'candidate', 'server_port': 18001, 'manifest': self.candidate}
        with patch('rolling.maintenance.archive.verify', side_effect=ValueError('corrupt backup')):
            with self.assertRaisesRegex(ValueError, 'corrupt backup'):
                schema.authorize(self.settings, journal, record)
        self.assertNotIn('ontology_migration', journal.data['maintenance'])
        with patch('rolling.maintenance.archive.verify', return_value={}):
            schema.authorize(self.settings, journal, record)
        proof = journal.data['maintenance']['ontology_migration']
        self.assertEqual(proof['backup_sha256'], hashlib.sha256(b'{}').hexdigest())
        with self.assertRaisesRegex(ValueError, '--maintenance'):
            schema.completed(self.settings, journal, record)
        self.assertNotIn('ontology_schema_complete', journal.data['maintenance'])
        journal.data['maintenance']['writes_open'] = True
        with self.assertRaisesRegex(ValueError, 'unopened'):
            schema.authorize(self.settings, journal, record)

    def test_staging_rejects_missing_schema_before_writing_any_release_files(self):
        from signal_release import controller
        old = dict(release_id='old', report_state=dict(reader=1, writer=0))
        journal = Journal(self.settings.state_dir)
        journal.data.update(current='old', releases={'old': {'manifest': old}})
        journal.save()
        before = (self.settings.state_dir / 'release-state.json').read_bytes()
        with patch.object(controller.manifest, 'verify', return_value=self.candidate), \
             patch('rolling.todo_resources.require_candidate'), \
             patch.object(controller.manifest, 'writer_preflight'), \
             patch.object(controller.manifest, 'compatible'):
            with self.assertRaisesRegex(ValueError, '--maintenance'):
                controller.stage(self.settings, self.settings.state_dir / 'bundle')
        self.assertEqual((self.settings.state_dir / 'release-state.json').read_bytes(), before)
        self.assertFalse((self.settings.state_dir / 'staged').exists())
        self.assertFalse((self.settings.state_dir / 'signal-pending.json').exists())


if __name__ == '__main__':
    unittest.main()
