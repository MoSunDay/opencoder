"""Add the ordered session-list index without changing application rows."""
import hashlib
import json
from pathlib import Path
import sqlite3
import time

NAME = 'idx_sessions_activity_visible'
DDL = ('CREATE INDEX ' + NAME + ' ON sessions(MAX(updated_at,created_at) DESC,id DESC) '
       "WHERE (kind IS NULL OR kind<>'operator')")
QUERY = """SELECT s.id,s.kind,s.created_at,s.updated_at FROM sessions s
WHERE ((s.task_type='parent' AND NOT EXISTS
(SELECT 1 FROM subagent_tasks st WHERE st.child_session_id=s.id)) OR s.task_type='agent_step')
AND (s.kind IS NULL OR s.kind<>'operator')
ORDER BY MAX(s.updated_at,s.created_at) DESC,s.id DESC LIMIT 500"""
TABLES = {'sessions', 'messages', 'subagent_tasks', 'sqlite_master', 'sqlite_stat1'}


def authorize(action, first, second, database, trigger):
    if database not in (None, 'main') or trigger is not None:
        return sqlite3.SQLITE_DENY
    if action in (sqlite3.SQLITE_SELECT, sqlite3.SQLITE_TRANSACTION, sqlite3.SQLITE_FUNCTION):
        return sqlite3.SQLITE_OK
    if action == sqlite3.SQLITE_READ and first in TABLES:
        return sqlite3.SQLITE_OK
    if action in (sqlite3.SQLITE_INSERT, sqlite3.SQLITE_UPDATE, sqlite3.SQLITE_DELETE):
        return sqlite3.SQLITE_OK if first in ('sqlite_master', 'sqlite_stat1') else sqlite3.SQLITE_DENY
    if action == sqlite3.SQLITE_CREATE_INDEX and first == NAME and second == 'sessions':
        return sqlite3.SQLITE_OK
    if action == sqlite3.SQLITE_CREATE_TABLE and first == 'sqlite_stat1':
        return sqlite3.SQLITE_OK
    if action == sqlite3.SQLITE_ANALYZE and first == 'sessions':
        return sqlite3.SQLITE_OK
    if action == sqlite3.SQLITE_REINDEX and first == NAME:
        return sqlite3.SQLITE_OK
    if action == sqlite3.SQLITE_PRAGMA and first == 'journal_mode' and second is None:
        return sqlite3.SQLITE_OK
    return sqlite3.SQLITE_DENY


def identity(path, db):
    stat = path.stat()
    rows = db.execute("SELECT type,name,tbl_name,sql FROM sqlite_master WHERE "
                      "tbl_name IN ('sessions','messages','subagent_tasks') AND name<>? "
                      "ORDER BY type,name", (NAME,)).fetchall()
    if {r[1] for r in rows if r[0] == 'table'} != {'sessions', 'messages', 'subagent_tasks'}:
        raise ValueError('required session tables are missing')
    return {'path': str(path), 'device': stat.st_dev, 'inode': stat.st_ino,
            'schema_sha256': hashlib.sha256(json.dumps(rows).encode()).hexdigest()}


def definition(db):
    row = db.execute('SELECT sql FROM sqlite_master WHERE type=\'index\' AND name=?', (NAME,)).fetchone()
    if row is not None and row[0] != DDL:
        raise ValueError('existing session index definition differs')
    return row is not None


def maintain(database, expected=None, timeout_seconds=20):
    """Default read-only preview; an exact preview enables the bounded DDL transaction."""
    if expected is not None and (not isinstance(expected, dict)
                                or not isinstance(expected.get('identity'), dict)
                                or type(expected.get('at_ms')) is not int):
        raise ValueError('valid saved preview required')
    path = Path(database).resolve(strict=True)
    if not path.is_file() or not 0 < timeout_seconds <= 60:
        raise ValueError('regular database and bounded deadline required')
    marker = Path(str(path) + '.engine.json')
    if marker.exists() or marker.is_symlink():
        raise ValueError('SQLite index maintenance cannot open an engine-marked database; use its native engine')
    started = time.monotonic()
    db = sqlite3.connect(path.as_uri() + ('?mode=rw' if expected else '?mode=ro'),
                         uri=True, timeout=min(1, timeout_seconds), isolation_level=None)
    db.set_authorizer(authorize)
    db.set_progress_handler(lambda: time.monotonic() - started >= timeout_seconds, 1000)
    try:
        db.execute('BEGIN IMMEDIATE' if expected else 'BEGIN')
        current = identity(path, db)
        present = definition(db)
        before_plan = [r[3] for r in db.execute('EXPLAIN QUERY PLAN ' + QUERY)]
        if expected:
            if expected['identity'] != current or not 0 <= time.time_ns() // 1_000_000 - expected['at_ms'] <= 900_000:
                raise ValueError('preview identity changed or expired')
            if db.execute('PRAGMA journal_mode').fetchone()[0] != 'wal':
                raise ValueError('live maintenance requires WAL mode')
            if not present:
                db.execute(DDL)
            # Only statistics for the session table are refreshed.
            db.execute('ANALYZE sessions')
            if not definition(db) or identity(path, db) != current:
                raise ValueError('session index changed the source schema')
        after_plan = [r[3] for r in db.execute('EXPLAIN QUERY PLAN ' + QUERY)]
        if expected and not any('USING INDEX ' + NAME in r for r in after_plan):
            raise ValueError('ordered session index was not selected')
        result = {'at_ms': time.time_ns() // 1_000_000, 'identity': current,
                  'applied': expected is not None, 'created': expected is not None and not present,
                  'index_present': definition(db), 'before_plan': before_plan,
                  'after_plan': after_plan, 'elapsed_ms': round((time.monotonic() - started) * 1000)}
        db.execute('COMMIT')
        return result
    except BaseException:
        db.set_progress_handler(None, 0)
        if db.in_transaction:
            db.execute('ROLLBACK')
        raise
    finally:
        db.close()
