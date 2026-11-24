"""Rebuild a damaged opencoder database: dump what sqlite3 can read, carve the rest.

The source database is only ever copied; every write goes to ``output``.
``messages`` and ``session_events`` rows that the dump missed because their
b-tree pages were orphaned are carved from the page-level scan and re-inserted
with their original ``seq`` (rowid).
"""
import os
from contextlib import nullcontext
from pathlib import Path
import shutil
import sqlite3
import subprocess

from . import carve

DUMP_TAIL_ROLLBACK = b'ROLLBACK; -- due to errors'
MESSAGE_INSERT = ('INSERT OR IGNORE INTO messages(seq,id,session_id,role,agent,model,'
                  'blocks_json,provider_state_json,usage_json,created_at,synthetic,display,mode,summary)'
                  ' VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?)')
EVENT_INSERT = ('INSERT OR IGNORE INTO session_events(seq,session_id,type,payload_json,sse_kind,ts)'
                ' VALUES(?,?,?,?,?,?)')
SESSION_STUB = ('INSERT OR IGNORE INTO sessions(id,title,created_at,updated_at,task_type,plan_input_count)'
                ' VALUES(?,NULL,?,?,?,0)')
VALID_TASK_TYPES = ('parent', 'subagent', 'agent_step', 'todo_workflow', 'todo', 'project', 'node')
SIDECARS = ('-wal', '-shm', '-tshm', '-journal')
BATCH = 20000


def snapshot(source, workdir):
    """Copy the database (and its WAL) into the scratch directory; never write to the source."""
    source = Path(source)
    target = workdir / source.name
    shutil.copy2(source, target)
    wal = source.with_name(source.name + '-wal')
    if wal.exists():
        shutil.copy2(wal, target.with_name(target.name + '-wal'))
    return target


def run_cli(sqlite3_bin, args, stdin_path=None):
    """Run the sqlite3 shell; ``stdin_path`` is streamed in when given."""
    with open(stdin_path, 'rb') if stdin_path else nullcontext(subprocess.DEVNULL) as stdin:
        proc = subprocess.run([sqlite3_bin, *map(str, args)], stdin=stdin, capture_output=True)
    if proc.returncode != 0:
        raise RuntimeError('sqlite3 %s failed (%s): %s' % (
            args[0], proc.returncode, proc.stderr.decode(errors='replace').strip()))
    return proc.stderr.decode(errors='replace')


def dump_database(db_path, dump_path, sqlite3_bin):
    """``.dump`` folds the WAL into the scratch copy; unreadable rows are reported on stderr."""
    with open(dump_path, 'wb') as out:
        proc = subprocess.run([sqlite3_bin, str(db_path)], input=b'.dump\n', stdout=out,
                              stderr=subprocess.PIPE)
    stderr = proc.stderr.decode(errors='replace')
    if proc.returncode != 0 or not dump_path.stat().st_size:
        raise RuntimeError('sqlite3 .dump failed (%s): %s' % (proc.returncode, stderr.strip()))
    return stderr


def checkpoint(db_path, sqlite3_bin):
    run_cli(sqlite3_bin, [db_path, 'PRAGMA wal_checkpoint(TRUNCATE);'])


def finalize_dump(dump_path):
    """Turn the error-terminated dump into a committable script; True when patched."""
    with open(dump_path, 'rb') as handle:
        head = handle.read(64)
        size = handle.seek(0, 2)
        handle.seek(max(0, size - 4096))
        tail = handle.read()
    if b'BEGIN TRANSACTION;' not in head:
        raise RuntimeError('dump does not start with BEGIN TRANSACTION')
    stripped = tail.rstrip()
    if not stripped.endswith(DUMP_TAIL_ROLLBACK):
        return False
    patched = stripped[:-len(DUMP_TAIL_ROLLBACK)] + b'COMMIT;\n'
    with open(dump_path, 'r+b') as handle:
        handle.seek(size - len(tail))
        handle.write(patched)
        handle.truncate()
    return True


def rewrite_inserts(dump_path):
    """Make duplicate rows (stale page copies) resolve to the last scanned copy instead of aborting.

    A damaged table b-tree can hold the same key twice while its unique index holds
    one entry; ``INSERT OR REPLACE`` keeps the load going and lets the later copy win.
    """
    patched = dump_path.with_name(dump_path.name + '.patched')
    replaced = 0
    with open(dump_path, 'rb') as src, open(patched, 'wb') as out:
        for line in src:
            if line.startswith(b'INSERT INTO '):
                line = b'INSERT OR REPLACE INTO ' + line[len(b'INSERT INTO '):]
                replaced += 1
            out.write(line)
    patched.replace(dump_path)
    return replaced


def remove_database(output):
    output = Path(output)
    for suffix in ('',) + SIDECARS:
        sidecar = output.with_name(output.name + suffix)
        if sidecar.exists():
            sidecar.unlink()


def load_dump(dump_path, output, sqlite3_bin):
    remove_database(output)
    run_cli(sqlite3_bin, ['-bail', output], stdin_path=dump_path)


def index_sequences(db_path):
    """(session_id, seq) pairs recorded by the (possibly stale) messages index; empty when unreadable."""
    try:
        conn = sqlite3.connect('file:%s?mode=ro' % db_path, uri=True)
        try:
            return {(row[0], row[1]) for row in conn.execute(
                'SELECT session_id, seq FROM messages INDEXED BY idx_messages_session')}
        finally:
            conn.close()
    except sqlite3.Error:
        return set()


def rowid_bitmap(conn, table):
    """Bitmap of the rowids already present in ``table``."""
    bitmap = bytearray()
    for (seq,) in conn.execute('SELECT seq FROM %s' % table):
        if len(bitmap) < seq // 8 + 1:
            bitmap.extend(bytes(seq // 8 + 1 - len(bitmap)))
        bitmap[seq // 8] |= 1 << (seq % 8)
    return bitmap


def present(bitmap, seq):
    return seq // 8 < len(bitmap) and bitmap[seq // 8] & (1 << (seq % 8))


def flush(conn, sql, buffer):
    """executemany the buffer; returns the number of rows actually inserted."""
    if not buffer:
        return 0
    before = conn.total_changes
    conn.executemany(sql, buffer)
    buffer.clear()
    return conn.total_changes - before


def keep_longest(orphans, values):
    """Index-leaf copies carry no rowid; keep the richest copy per message id."""
    previous = orphans.get(values[1])
    orphans[values[1]] = values if previous is None or len(previous[6]) <= len(values[6]) else previous


def insert_carved(conn, snapshot_path):
    """One page scan: re-insert messages (with and without seq) and missing events."""
    stats = dict(message_pages=set(), event_pages=set(), messages_with_seq=0, messages_without_seq=0,
                 message_duplicates=0, events_seen=0, events_inserted=0)
    events = rowid_bitmap(conn, 'session_events')
    known_ids, orphans, messages, pending = set(), {}, [], []
    for page_no, seq, values in carve.carve(snapshot_path, carve.is_message_or_event):
        if carve.is_message(values):
            stats['message_pages'].add(page_no)
            if seq is None:
                stats['message_duplicates'] += values[1] in orphans
                keep_longest(orphans, values)
            else:
                known_ids.add(values[1])
                messages.append((seq, *values[1:]))
                if len(messages) >= BATCH:
                    stats['messages_with_seq'] += flush(conn, MESSAGE_INSERT, messages)
            continue
        stats['event_pages'].add(page_no)
        stats['events_seen'] += 1
        if seq is not None and not present(events, seq):
            pending.append((seq, *values[1:]))
            if len(pending) >= BATCH:
                stats['events_inserted'] += flush(conn, EVENT_INSERT, pending)
    stats['messages_with_seq'] += flush(conn, MESSAGE_INSERT, messages)
    stats['events_inserted'] += flush(conn, EVENT_INSERT, pending)
    stats['message_pages'], stats['event_pages'] = len(stats['message_pages']), len(stats['event_pages'])
    stats['messages_without_seq'] = insert_orphans(conn, snapshot_path, orphans, known_ids)
    return stats


def assign_orphan_seqs(orphans, known_ids, used, candidates):
    """Rows without a rowid take their own session's unused index seqs in created_at order.

    ``candidates`` maps session_id -> seqs the index lists but the table lacks; a row
    whose session has no candidate left (or none at all) gets a fresh seq above
    everything seen. Returns MESSAGE_INSERT tuples.
    """
    rows = [v for mid, v in orphans.items() if mid not in known_ids]
    rows.sort(key=lambda v: (v[9], v[1]))
    used = set(used)
    next_free = max(used | {seq for seqs in candidates.values() for seq in seqs}, default=0) + 1
    offset = dict()
    buffer = []
    for values in rows:
        pool = candidates.get(values[2], ())
        position = offset.get(values[2], 0)
        seq = None
        while position < len(pool):
            candidate = pool[position]
            position += 1
            if candidate not in used:
                seq = candidate
                break
        offset[values[2]] = position
        if seq is None:
            seq = next_free
            while seq in used:
                seq += 1
        used.add(seq)
        buffer.append((seq, *values[1:]))
    return buffer


def insert_orphans(conn, snapshot_path, orphans, known_ids):
    """Index-leaf rows (no rowid) inherit the seq their index entry still names."""
    used = {row[0] for row in conn.execute('SELECT seq FROM messages')}
    candidates = {}
    for session_id, seq in index_sequences(snapshot_path):
        if seq not in used:
            candidates.setdefault(session_id, []).append(seq)
    for seqs in candidates.values():
        seqs.sort()
    return flush(conn, MESSAGE_INSERT, assign_orphan_seqs(orphans, known_ids, used, candidates))


def stub_sessions(conn):
    """Sessions whose row is gone but whose messages survive get a minimal row (keeps the FK valid)."""
    rows = conn.execute('SELECT m.session_id, MIN(m.created_at), MAX(m.created_at), '
                        "CASE WHEN m.session_id LIKE 'sub-%' THEN 'subagent' ELSE 'parent' END "
                        'FROM messages m LEFT JOIN sessions s ON s.id = m.session_id '
                        'WHERE s.id IS NULL GROUP BY m.session_id').fetchall()
    conn.executemany(SESSION_STUB, rows)
    return [row[0] for row in rows]


def scrub_invalid_text(conn):
    """Lossy-decode TEXT values that are not valid UTF-8 (truncated overflow chains).

    Turso rejects any row holding such a value, which would make the whole table unreadable.
    """
    fixed = {}
    conn.text_factory = bytes
    try:
        names = [row[0].decode() for row in conn.execute(
            "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'")]
        for name in names:
            if name.startswith('__turso'):
                continue
            columns = [row[1].decode() for row in conn.execute('PRAGMA table_info("%s")' % name)]
            for column in columns:
                updates = []
                for rowid, value in conn.execute('SELECT rowid, "%s" FROM "%s" WHERE typeof("%s")=\'text\'' % (column, name, column)):
                    try:
                        value.decode('utf-8')
                    except UnicodeDecodeError:
                        updates.append((value.decode('utf-8', 'replace'), rowid))
                if updates:
                    conn.executemany('UPDATE "%s" SET "%s"=? WHERE rowid=?' % (name, column), updates)
                    fixed.setdefault(name, {})[column] = len(updates)
    finally:
        conn.text_factory = str
    return fixed


def normalize_sessions(conn):
    """Repair known-garbage columns on session rows that sqlite3 could still read."""
    placeholders = ','.join('?' * len(VALID_TASK_TYPES))
    fixed_type = conn.execute(
        "UPDATE sessions SET task_type = CASE WHEN id LIKE 'sub-%%' THEN 'subagent' ELSE 'parent' END "
        'WHERE task_type IS NULL OR task_type NOT IN (%s)' % placeholders, VALID_TASK_TYPES).rowcount
    fixed_kind = conn.execute("UPDATE sessions SET kind = NULL WHERE kind = ''").rowcount
    return dict(task_type=fixed_type, kind=fixed_kind)


def sync_sqlite_sequence(conn):
    """Keep the AUTOINCREMENT high-water mark above every re-inserted rowid."""
    synced = {}
    for table in ('messages', 'session_events'):
        cursor = conn.execute('UPDATE sqlite_sequence SET seq = MAX(seq, '
                              '(SELECT COALESCE(MAX(seq), 0) FROM %s)) WHERE name = ?' % table, (table,))
        if cursor.rowcount:
            synced[table] = conn.execute('SELECT seq FROM sqlite_sequence WHERE name = ?', (table,)).fetchone()[0]
    return synced


def verify(conn):
    report = {}
    report['integrity'] = conn.execute('PRAGMA integrity_check').fetchone()[0]
    for table in ('sessions', 'messages', 'session_events'):
        report[table] = conn.execute('SELECT COUNT(*) FROM %s' % table).fetchone()[0]
    report['messages_max_seq'] = conn.execute('SELECT COALESCE(MAX(seq), 0) FROM messages').fetchone()[0]
    report['events_max_seq'] = conn.execute('SELECT COALESCE(MAX(seq), 0) FROM session_events').fetchone()[0]
    report['messages_without_session'] = conn.execute(
        'SELECT COUNT(*) FROM messages m LEFT JOIN sessions s ON s.id = m.session_id WHERE s.id IS NULL'
    ).fetchone()[0]
    report['events_without_session'] = conn.execute(
        'SELECT COUNT(*) FROM session_events e LEFT JOIN sessions s ON s.id = e.session_id WHERE s.id IS NULL'
    ).fetchone()[0]
    return report


def repair(source, output, sqlite3_bin='sqlite3', dump=None, overwrite=False, keep_workdir=False, log=print):
    """Copy ``source``, dump + carve it and write a fresh database to ``output``."""
    source, output = Path(source), Path(output)
    if not source.exists():
        raise RuntimeError('source database not found: %s' % source)
    if output.exists() and not overwrite:
        raise RuntimeError('output exists: %s (pass --overwrite to replace it)' % output)
    # The output (and its -wal/-shm sidecars) is deleted before the dump is loaded;
    # that must never be the file being repaired.
    if output.exists() and os.path.samefile(output, source):
        raise RuntimeError('output must not be the source database: %s' % output)
    workdir = output.parent / ('.repair-' + output.name)
    if workdir.exists():
        shutil.rmtree(workdir)
    workdir.mkdir(parents=True)
    report = dict(source=str(source), output=str(output))
    try:
        work_db = snapshot(source, workdir)
        dump_path = Path(dump) if dump else workdir / 'dump.sql'
        if dump:
            checkpoint(work_db, sqlite3_bin)
        else:
            report['dump_stderr'] = dump_database(work_db, dump_path, sqlite3_bin).strip().splitlines()[-5:]
        report['dump_patched'] = finalize_dump(dump_path)
        report['dump_inserts'] = rewrite_inserts(dump_path)
        log('loading dump into %s' % output)
        load_dump(dump_path, output, sqlite3_bin)
        conn = sqlite3.connect(output)
        try:
            conn.execute('PRAGMA journal_mode=WAL')
            conn.execute('PRAGMA synchronous=OFF')
            report['carved'] = insert_carved(conn, work_db)
            report['stub_sessions'] = stub_sessions(conn)
            report['scrubbed_text'] = scrub_invalid_text(conn)
            report['normalized_sessions'] = normalize_sessions(conn)
            report['sqlite_sequence'] = sync_sqlite_sequence(conn)
            conn.commit()
            report['verify'] = verify(conn)
        finally:
            conn.close()
        checkpoint(output, sqlite3_bin)
    except BaseException:
        # The scratch copy is the only snapshot of the source state this run took.
        log('repair failed; the scratch copy and dump are kept in %s' % workdir)
        raise
    else:
        if not keep_workdir:
            shutil.rmtree(workdir, ignore_errors=True)
    return report
