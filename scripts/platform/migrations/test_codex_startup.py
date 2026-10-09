import json
from pathlib import Path
import sqlite3
import tempfile
import unittest
from unittest.mock import patch

import codex_startup as migration


class StartupMigrationTests(unittest.TestCase):
    def setUp(self):
        self.root = tempfile.TemporaryDirectory()
        self.addCleanup(self.root.cleanup)
        self.database = Path(self.root.name) / 'control.db'
        self.journal = Path(self.root.name) / 'rollback.json'
        self.before = json.dumps({'request': {'input': {'auth_slot': 'user text'}},
                                  'codex': {'auth_slot': None, 'envs': {'LITERAL': 'unchanged'}},
                                  'runtime': {'profiles': {'p': {'revision': 7, 'settings': {
                                      'auth_slot': None, 'startup_script': ['/external/launch']}}}}})
        with sqlite3.connect(self.database) as conn:
            conn.execute('CREATE TABLE execution_assignments(id TEXT PRIMARY KEY,assignment TEXT)')
            conn.execute('INSERT INTO execution_assignments VALUES (?,?)', ('test', self.before))
            conn.execute('CREATE TABLE authentication(token TEXT)')
            conn.execute("INSERT INTO authentication VALUES ('must-remain')")

    def raw(self):
        with sqlite3.connect(self.database) as conn:
            self.assertEqual(conn.execute('SELECT token FROM authentication').fetchone()[0], 'must-remain')
            return conn.execute('SELECT assignment FROM execution_assignments WHERE id="test"').fetchone()[0]

    def test_preview_apply_retry_restore_preserves_request_and_script(self):
        self.assertEqual(migration.migrate(self.database), {'mode': 'preview', 'rows': 1})
        self.assertEqual(self.raw(), self.before)
        self.assertEqual(migration.migrate(self.database, self.journal)['changed'], 1)
        anchor = self.journal.read_bytes()
        value = json.loads(self.raw())
        self.assertEqual(value['request'], json.loads(self.before)['request'])
        self.assertEqual(value['codex'], {'envs': {'LITERAL': 'unchanged'}, 'startup_script': []})
        self.assertEqual(value['runtime']['profiles']['p'], {
            'revision': 7, 'settings': {'startup_script': ['/external/launch']}})
        self.assertEqual(migration.migrate(self.database, self.journal)['changed'], 0)
        self.assertEqual(self.journal.read_bytes(), anchor)
        self.assertEqual(self.journal.stat().st_mode & 0o777, 0o600)
        migration.migrate(self.database, self.journal, restore=True)
        self.assertEqual(self.raw(), self.before)
        self.assertEqual(migration.migrate(self.database, self.journal, restore=True)['changed'], 0)

    def test_nonempty_slot_refuses_without_writing(self):
        with sqlite3.connect(self.database) as conn:
            conn.execute('INSERT INTO execution_assignments VALUES (?,?)', ('nonempty', '{"codex":{"auth_slot":2}}'))
        with self.assertRaisesRegex(ValueError, 'explicitly configured startup script'):
            migration.migrate(self.database, self.journal)
        self.assertFalse(self.journal.exists())
        self.assertEqual(self.raw(), self.before)

    def test_crash_after_anchor_and_before_commit_retries_from_original(self):
        real_read = migration.read_anchor
        with patch.object(migration, 'read_anchor', side_effect=RuntimeError('crash after journal')):
            with self.assertRaises(RuntimeError):
                migration.migrate(self.database, self.journal)
        anchor = self.journal.read_bytes()
        self.assertEqual(self.raw(), self.before)
        real_scan = migration.scan
        with patch.object(migration, 'scan', side_effect=RuntimeError('crash before commit')):
            with self.assertRaises(RuntimeError):
                migration.migrate(self.database, self.journal)
        self.assertEqual(self.raw(), self.before)
        migration.migrate(self.database, self.journal)
        self.assertEqual(self.journal.read_bytes(), anchor)
        self.assertEqual(real_scan(sqlite3.connect(self.database)), [])
        self.assertEqual(real_read(self.journal, self.database)['rows'][0]['before'], self.before)

    def test_changed_row_or_corrupt_journal_refuses_restore(self):
        migration.migrate(self.database, self.journal)
        with sqlite3.connect(self.database) as conn:
            conn.execute('UPDATE execution_assignments SET assignment=?', ('{"codex":null}',))
        with self.assertRaisesRegex(ValueError, 'assignment changed'):
            migration.migrate(self.database, self.journal, restore=True)
        self.assertEqual(self.raw(), '{"codex":null}')
        data = json.loads(self.journal.read_text())
        data['rows'][0]['before'] = '{}'
        self.journal.write_text(json.dumps(data))
        with self.assertRaisesRegex(ValueError, 'journal content differs'):
            migration.migrate(self.database, self.journal, restore=True)

    def test_conflict_rolls_back_all_rows_and_new_legacy_rows_need_new_anchor(self):
        with sqlite3.connect(self.database) as conn:
            conn.execute('INSERT INTO execution_assignments VALUES (?,?)', ('z', self.before))
        migration.migrate(self.database, self.journal)
        after = self.raw()
        with sqlite3.connect(self.database) as conn:
            conn.execute('UPDATE execution_assignments SET assignment=? WHERE id="z"', ('{}',))
        with self.assertRaisesRegex(ValueError, 'assignment changed'):
            migration.migrate(self.database, self.journal, restore=True)
        self.assertEqual(self.raw(), after)
        with sqlite3.connect(self.database) as conn:
            conn.execute('UPDATE execution_assignments SET assignment=? WHERE id="z"', (after,))
            conn.execute('INSERT INTO execution_assignments VALUES (?,?)', ('new', self.before))
        with self.assertRaisesRegex(ValueError, 'separate migration journal'):
            migration.migrate(self.database, self.journal)
        self.assertEqual(self.raw(), after)


if __name__ == '__main__':
    unittest.main()
