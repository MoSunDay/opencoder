import json
from pathlib import Path
import sqlite3
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch
from . import DDL, NAME, QUERY, authorize, maintain


class SessionIndexTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.path = Path(self.temp.name) / 'state.db'
        self.db = sqlite3.connect(self.path)
        self.addCleanup(self.db.close)
        self.db.execute('PRAGMA journal_mode=wal')
        self.db.executescript('''CREATE TABLE sessions(id TEXT PRIMARY KEY,kind TEXT,
            created_at INTEGER,updated_at INTEGER,task_type TEXT);
            CREATE INDEX idx_sessions_task_type ON sessions(task_type);
            CREATE TABLE subagent_tasks(child_session_id TEXT);
            CREATE INDEX idx_subagent_child ON subagent_tasks(child_session_id);
            CREATE TABLE messages(session_id TEXT,body TEXT);
            CREATE TABLE platform_tokens(token_hash TEXT);''')
        self.db.executemany('INSERT INTO sessions VALUES(?,?,?,?,?)', [
            (f'session-{i:05}', None if i % 7 else 'operator', i // 4,
             0 if i % 3 else i, 'agent_step' if i % 5 else 'parent') for i in range(6000)])
        self.db.executemany('INSERT INTO subagent_tasks VALUES(?)', [(f'session-{i:05}',) for i in range(0, 6000, 10)])
        self.db.execute("INSERT INTO messages VALUES('session-00001','unchanged output')")
        self.db.execute("INSERT INTO platform_tokens VALUES('synthetic-token')")
        self.db.commit()

    def pages(self):
        rows = []; cursor = None
        while True:
            query = QUERY; params = ()
            if cursor:
                query = QUERY.replace('ORDER BY MAX', 'AND (MAX(s.updated_at,s.created_at)<? OR '
                                      '(MAX(s.updated_at,s.created_at)=? AND s.id<?)) ORDER BY MAX')
                params = (cursor[0], cursor[0], cursor[1])
            page = self.db.execute(query, params).fetchall()
            rows.extend(page)
            if not page:
                return rows
            cursor = (max(page[-1][2:]), page[-1][0])

    def test_preview_then_apply_preserves_all_pages_rows_and_is_idempotent(self):
        before = self.pages()
        tables = {t: self.db.execute('SELECT * FROM ' + t).fetchall()
                  for t in ('sessions', 'messages', 'subagent_tasks', 'platform_tokens')}
        preview = maintain(self.path)
        self.assertFalse(preview['index_present'])
        result = maintain(self.path, preview)
        self.assertTrue(result['created'])
        self.assertTrue(any(NAME in r for r in result['after_plan']))
        self.assertEqual(before, self.pages())
        self.assertEqual(len(before), len({r[0] for r in before}))
        for table, expected in tables.items():
            self.assertEqual(expected, self.db.execute('SELECT * FROM ' + table).fetchall())
        self.assertFalse(maintain(self.path, preview)['created'])

    def test_changed_identity_and_conflicting_index_are_rejected(self):
        preview = maintain(self.path)
        preview['identity']['inode'] += 1
        with self.assertRaisesRegex(ValueError, 'identity'):
            maintain(self.path, preview)
        self.assertFalse(maintain(self.path)['index_present'])
        self.db.execute(f'CREATE INDEX {NAME} ON sessions(id)')
        with self.assertRaisesRegex(ValueError, 'definition'):
            maintain(self.path)

    def test_engine_marker_rejects_preview_and_apply_before_sqlite_open(self):
        preview = maintain(self.path)
        Path(str(self.path) + '.engine.json').write_text('{"engine":"turso","format":1}')
        with patch('session_index.sqlite3.connect', side_effect=AssertionError('must not open')):
            for expected in (None, preview):
                with self.subTest(apply=expected is not None), self.assertRaisesRegex(ValueError, 'engine-marked'):
                    maintain(self.path, expected)

    def test_verification_failure_rolls_back_index(self):
        preview = maintain(self.path)
        with patch('session_index.QUERY', 'SELECT id FROM sessions LIMIT 1'):
            with self.assertRaisesRegex(ValueError, 'not selected'):
                maintain(self.path, preview)
        self.assertFalse(maintain(self.path)['index_present'])

    def test_busy_writer_and_invalid_preview_leave_schema_unchanged(self):
        preview = maintain(self.path)
        self.db.execute('BEGIN IMMEDIATE')
        try:
            with self.assertRaises(sqlite3.OperationalError):
                maintain(self.path, preview, timeout_seconds=0.05)
        finally:
            self.db.rollback()
        for invalid in ({}, {'identity': {}, 'at_ms': 0}):
            with self.assertRaises(ValueError):
                maintain(self.path, invalid)
        self.assertFalse(maintain(self.path)['index_present'])

    def test_authorizer_denies_application_writes_and_credential_reads(self):
        self.db.set_authorizer(authorize)
        for sql in ("UPDATE sessions SET kind='operator'", 'DELETE FROM messages',
                    'SELECT * FROM platform_tokens', 'DROP TABLE sessions'):
            with self.subTest(sql=sql), self.assertRaises(sqlite3.DatabaseError):
                self.db.execute(sql)
        self.db.set_authorizer(None)

    def test_cli_preview_and_apply_write_receipts(self):
        preview = Path(self.temp.name) / 'preview.json'
        applied = Path(self.temp.name) / 'applied.json'
        base = [sys.executable, '-m', 'session_index', '--database', str(self.path)]
        root = Path(__file__).resolve().parents[1]
        subprocess.run(base + ['--output', str(preview)], cwd=root, check=True, capture_output=True)
        subprocess.run(base + ['--preview', str(preview), '--output', str(applied)], cwd=root, check=True, capture_output=True)
        self.assertTrue(json.loads(applied.read_text())['created'])

    def test_cli_refuses_receipt_over_database(self):
        result = subprocess.run([sys.executable, '-m', 'session_index', '--database', str(self.path),
                                 '--output', str(self.path)], cwd=Path(__file__).resolve().parents[1],
                                capture_output=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(self.db.execute('SELECT count(*) FROM sessions').fetchone()[0], 6000)


if __name__ == '__main__':
    unittest.main()
