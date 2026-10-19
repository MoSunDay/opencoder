import json
from pathlib import Path
import sqlite3
import tempfile
import unittest
from unittest.mock import Mock

from scripts.acceptance.brain.history.retired import snapshot


class RetiredHistoryTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.database = Path(self.directory.name) / 'host.db'
        self.connection = sqlite3.connect(self.database)
        self.addCleanup(self.connection.close)
        self.connection.executescript('''
            CREATE TABLE host_runtimes(id TEXT, mode TEXT, config TEXT);
            CREATE TABLE runtime_owners(execution_id TEXT, runtime_id TEXT);
        ''')
        self.connection.execute('INSERT INTO host_runtimes VALUES(?,?,?)', (
            'old', 'retired', json.dumps({'unit': 'opencoder-runtime-old.service'})))
        self.connection.execute('INSERT INTO runtime_owners VALUES(?,?)', ('history', 'old'))
        self.connection.commit()
        self.read = Mock(return_value={'schema_version': 7, 'run': {'run_id': 'history'}})
        self.identity = Mock(return_value={'pid': 21, 'start_ticks': '100', 'executable': '/release/agent'})

    def run_snapshot(self):
        return snapshot(self.database, 'history', 'old', 'current', 7, self.read, self.identity)

    def change(self, sql):
        self.connection.execute(sql)
        self.connection.commit()

    def test_pinned_retired_owner_records_real_process_and_unchanged_database(self):
        before = self.database.read_bytes()
        result = self.run_snapshot()
        self.assertEqual(result['runtime'], 'old')
        self.assertEqual(result['execution_id'], 'history')
        self.assertEqual(result['pid'], 21)
        self.assertEqual(len(result['config_sha256']), 64)
        self.assertEqual(self.database.read_bytes(), before)
        self.read.assert_called_once_with('history')
        self.identity.assert_called_once_with('opencoder-runtime-old.service')

    def test_missing_owner_is_rejected_before_reading(self):
        self.change('DELETE FROM runtime_owners')
        with self.assertRaisesRegex(ValueError, 'pinned retired'):
            self.run_snapshot()
        self.read.assert_not_called()

    def test_different_owner_is_rejected_before_reading(self):
        self.change("UPDATE host_runtimes SET id='other'")
        self.change("UPDATE runtime_owners SET runtime_id='other'")
        with self.assertRaisesRegex(ValueError, 'pinned retired'):
            self.run_snapshot()
        self.read.assert_not_called()

    def test_active_owner_is_rejected_before_reading(self):
        self.change("UPDATE host_runtimes SET mode='active'")
        with self.assertRaisesRegex(ValueError, 'pinned retired'):
            self.run_snapshot()
        self.read.assert_not_called()

    def test_current_runtime_cannot_stand_in_for_historical_runtime(self):
        with self.assertRaisesRegex(ValueError, 'older Runtime'):
            snapshot(self.database, 'history', 'old', 'old', 7, self.read, self.identity)
        self.read.assert_not_called()

    def test_unknown_current_runtime_cannot_establish_old_ownership(self):
        with self.assertRaisesRegex(ValueError, 'older Runtime'):
            snapshot(self.database, 'history', 'old', None, 7, self.read, self.identity)
        self.read.assert_not_called()

    def test_owner_changes_during_read_are_rejected(self):
        def read(_):
            self.change('UPDATE host_runtimes SET config=\'{"unit":"opencoder-runtime-other.service"}\'')
            return {'schema_version': 7, 'run': {'run_id': 'history'}}
        self.read.side_effect = read
        with self.assertRaises(ValueError):
            self.run_snapshot()

    def test_missing_process_identity_cannot_pass(self):
        self.identity.return_value = {'pid': 0}
        with self.assertRaisesRegex(ValueError, 'identity is incomplete'):
            self.run_snapshot()

    def test_response_must_match_execution_and_schema(self):
        for response in [{'schema_version': 6, 'run': {'run_id': 'history'}},
                         {'schema_version': 7, 'run': {'run_id': 'other'}}]:
            with self.subTest(response=response):
                self.read.return_value = response
                with self.assertRaisesRegex(ValueError, 'pinned execution/schema'):
                    self.run_snapshot()
        self.identity.assert_not_called()

    def test_http_failure_is_not_retried_or_replaced(self):
        self.read.side_effect = TimeoutError('historical read timed out')
        with self.assertRaises(TimeoutError):
            self.run_snapshot()
        self.read.assert_called_once_with('history')
        self.identity.assert_not_called()

    def test_missing_database_does_not_create_one(self):
        absent = Path(self.directory.name) / 'absent.db'
        with self.assertRaises(sqlite3.OperationalError):
            snapshot(absent, 'history', 'old', 'current', 7, self.read, self.identity)
        self.assertFalse(absent.exists())


if __name__ == '__main__':
    unittest.main()
