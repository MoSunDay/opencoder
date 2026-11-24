"""Build a small database, orphan two b-trees and check the rebuild restores every row."""
import os
from pathlib import Path
import shutil
import sqlite3
import struct
import tempfile
import unittest

from . import repair

SESSIONS_DDL = ("CREATE TABLE sessions (id TEXT PRIMARY KEY, title TEXT, agent TEXT, model TEXT, "
                "workdir_hash TEXT, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL, summary TEXT, "
                "summary_seq INTEGER, summary_images_json TEXT, handoff_seq INTEGER, handoff_plan TEXT, "
                "skill TEXT, task_type TEXT NOT NULL DEFAULT 'parent', requirement TEXT, plan_snapshot TEXT, "
                "plan_input_count INTEGER NOT NULL DEFAULT 0, autopilot_mode TEXT, harness_runtime TEXT, kind TEXT)")
MESSAGES_DDL = ("CREATE TABLE messages (seq INTEGER PRIMARY KEY AUTOINCREMENT, id TEXT NOT NULL, "
                "session_id TEXT NOT NULL REFERENCES sessions (id) ON DELETE CASCADE, role TEXT NOT NULL, "
                "agent TEXT, model TEXT, blocks_json TEXT NOT NULL, provider_state_json TEXT, "
                "usage_json TEXT NOT NULL, created_at INTEGER NOT NULL, synthetic INTEGER NOT NULL DEFAULT 0, "
                "display TEXT, mode TEXT, summary INTEGER NOT NULL DEFAULT 0)")
EVENTS_DDL = ("CREATE TABLE session_events (seq INTEGER PRIMARY KEY AUTOINCREMENT, "
              "session_id TEXT NOT NULL REFERENCES sessions (id) ON DELETE CASCADE, type TEXT NOT NULL, "
              "payload_json TEXT NOT NULL, sse_kind TEXT, ts INTEGER NOT NULL)")
SESSION = '01M81P2R36EVDKFFMP9NJ3NSKW'
SUB_SESSION = 'sub-01M84603JZAS8KMV7BWND6V0CF'
BASE_TS = 1795435949376


ALPHABET = '0123456789ABCDEFGHJKMNPQRSTVWXYZ'


def ulid(n):
    return '01M84860W3TBPDXN5Y93ZBF' + ALPHABET[n % 32] + ALPHABET[(n // 32) % 32] + 'Z'


def build(path):
    conn = sqlite3.connect(path)
    conn.executescript(';\n'.join([SESSIONS_DDL, MESSAGES_DDL, EVENTS_DDL,
                                   'CREATE INDEX idx_messages_session ON messages (session_id, seq)',
                                   'CREATE INDEX idx_events_session ON session_events (session_id, seq)']) + ';')
    conn.executemany('INSERT INTO sessions(id,created_at,updated_at,task_type) VALUES(?,?,?,?)', [
        (SESSION, BASE_TS, BASE_TS, 'parent'), (SUB_SESSION, BASE_TS, BASE_TS, 'subagent')])
    for n in range(60):
        session = SUB_SESSION if n % 3 == 0 else SESSION
        blocks = '[{"type":"text","text":"' + 'x' * (n * 300) + '"}]'
        conn.execute('INSERT INTO messages(id,session_id,role,blocks_json,usage_json,created_at) VALUES(?,?,?,?,?,?)',
                     (ulid(n), session, 'user' if n % 2 else 'assistant', blocks, '{"input":1}', BASE_TS + n))
    for n in range(1500):
        conn.execute('INSERT INTO session_events(session_id,type,payload_json,sse_kind,ts) VALUES(?,?,?,?,?)',
                     (SESSION, 'text_delta', '{"text":"a"}', 'text_delta', BASE_TS + n))
    conn.commit()
    roots = {name: conn.execute('SELECT rootpage FROM sqlite_master WHERE name=?', (name,)).fetchone()[0]
             for name in ('messages', 'session_events')}
    conn.execute('PRAGMA journal_mode=DELETE')
    conn.close()
    return roots


def orphan_trees(path, roots):
    """Overwrite both root pages with an index-interior header, detaching every leaf."""
    page_size = 4096
    with open(path, 'r+b') as handle:
        for root in roots.values():
            handle.seek((root - 1) * page_size)
            page = bytearray(page_size)
            page[0] = 0x02
            struct.pack_into('>H', page, 3, 6)
            handle.write(page)


def rows(conn, sql):
    return conn.execute(sql).fetchall()


class RepairTest(unittest.TestCase):
    def test_rebuild_restores_orphaned_messages_and_events(self):
        if shutil.which('sqlite3') is None:
            self.skipTest('sqlite3 shell not installed')
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        source, output = Path(tmp.name) / 'opencoder.db', Path(tmp.name) / 'out' / 'opencoder.db'
        roots = build(source)
        original = sqlite3.connect(source)
        messages = rows(original, 'SELECT * FROM messages ORDER BY seq')
        events = rows(original, 'SELECT * FROM session_events ORDER BY seq')
        original.close()
        orphan_trees(source, roots)
        broken = sqlite3.connect(source)
        with self.assertRaises(sqlite3.DatabaseError):
            rows(broken, 'SELECT count(*) FROM messages NOT INDEXED')
        broken.close()
        report = repair.repair(source, output)
        self.assertTrue(report['dump_patched'])
        self.assertEqual(report['carved']['messages_with_seq'], len(messages))
        self.assertEqual(report['carved']['events_inserted'], len(events))
        self.assertEqual(report['stub_sessions'], [])
        rebuilt = sqlite3.connect(output)
        self.assertEqual(rows(rebuilt, 'SELECT * FROM messages ORDER BY seq'), messages)
        self.assertEqual(rows(rebuilt, 'SELECT * FROM session_events ORDER BY seq'), events)
        self.assertEqual(rows(rebuilt, 'SELECT count(*) FROM sessions')[0][0], 2)
        self.assertEqual(report['verify']['integrity'], 'ok')
        self.assertEqual(report['verify']['messages_max_seq'], len(messages))
        rebuilt.close()

    def test_scrub_rewrites_text_that_is_not_utf8(self):
        conn = sqlite3.connect(':memory:')
        conn.execute(SESSIONS_DDL)
        conn.execute("INSERT INTO sessions(id,created_at,updated_at,title) VALUES(?,?,?,CAST(x'e4bda0e5a5bde6' AS TEXT))",
                     (SESSION, BASE_TS, BASE_TS))
        conn.execute("INSERT INTO sessions(id,created_at,updated_at,title) VALUES(?,?,?,?)",
                     (SUB_SESSION, BASE_TS, BASE_TS, 'ok'))
        self.assertEqual(repair.scrub_invalid_text(conn), {'sessions': {'title': 1}})
        titles = [row[0] for row in conn.execute('SELECT title FROM sessions ORDER BY id')]
        self.assertEqual(titles[1], 'ok')
        self.assertTrue(titles[0].startswith('\u4f60\u597d'))
        self.assertEqual(repair.scrub_invalid_text(conn), {})
        self.assertEqual(conn.execute('SELECT title FROM sessions WHERE id=?', (SESSION,)).fetchone()[0], titles[0])
        conn.close()

    def test_finalize_dump_only_patches_the_error_rollback(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        path = Path(tmp.name) / 'dump.sql'
        path.write_bytes(b'BEGIN TRANSACTION;\nINSERT INTO t VALUES(1);\nROLLBACK; -- due to errors\n')
        self.assertTrue(repair.finalize_dump(path))
        self.assertTrue(path.read_bytes().endswith(b'COMMIT;\n'))
        self.assertFalse(repair.finalize_dump(path))
        path.write_bytes(b'PRAGMA foreign_keys=OFF;\nBEGIN TRANSACTION;\nCOMMIT;\n')
        self.assertFalse(repair.finalize_dump(path))
        path.write_bytes(b'PRAGMA foreign_keys=OFF;\nCREATE TABLE t(x);\n')
        with self.assertRaises(RuntimeError):
            repair.finalize_dump(path)



    def test_assign_orphan_seqs_pairs_each_session_with_its_own_index_seqs(self):
        def row(mid, session, created):
            return (None, mid, session, 'user', None, None, '[{}]', None, '{}', created, 0, None, None, 0)
        orphans = {
            # Written after A, but its index entry names the lower seq.
            'B': row('B', SUB_SESSION, BASE_TS + 2),
            'A': row('A', SESSION, BASE_TS + 1),
            'C': row('C', SESSION, BASE_TS + 3),
            'known': row('known', SESSION, BASE_TS),  # already carved with its rowid
        }
        used = {5, 9}
        candidates = {SESSION: [5, 7], SUB_SESSION: [6, 8]}
        buffer = repair.assign_orphan_seqs(orphans, {'known'}, used, candidates)
        # A takes 7 (5 belongs to a table row), B its session's 6, C a fresh seq above 9.
        self.assertEqual([(r[1], r[0]) for r in buffer], [('A', 7), ('B', 6), ('C', 10)])

    def test_repair_refuses_to_overwrite_the_source_database(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        source = Path(tmp.name) / 'opencoder.db'
        build(source)
        before = source.read_bytes()
        with self.assertRaises(RuntimeError):
            repair.repair(source, source, overwrite=True)
        self.assertEqual(source.read_bytes(), before)
        self.assertFalse((Path(tmp.name) / ('.repair-' + source.name)).exists())

    def test_failed_repair_keeps_the_scratch_copy(self):
        false = shutil.which('false')
        if false is None:
            self.skipTest('false(1) not installed')
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        source, output = Path(tmp.name) / 'opencoder.db', Path(tmp.name) / 'out' / 'opencoder.db'
        roots = build(source)
        orphan_trees(source, roots)
        with self.assertRaises(RuntimeError):
            repair.repair(source, output, sqlite3_bin=false)
        workdir = output.parent / ('.repair-' + output.name)
        self.assertTrue((workdir / 'opencoder.db').exists(), 'scratch copy kept for inspection')
        self.assertFalse(output.exists())


if __name__ == '__main__':
    unittest.main()
