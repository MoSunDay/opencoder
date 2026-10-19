"""Small synchronous adapter for existing maintenance transactions over Turso."""
import json
from pathlib import Path
import sqlite3
import subprocess
import tempfile
from . import database


class Row(tuple):
    def __new__(cls, names, values):
        row = super().__new__(cls, values)
        row.names = names
        return row

    def keys(self):
        return self.names

    def __getitem__(self, key):
        return super().__getitem__(self.names.index(key) if isinstance(key, str) else key)


def encoded(value):
    if isinstance(value, bytes):
        return {'bytes': list(value)}
    raise TypeError('unsupported SQL parameter')


class Cursor:
    def __init__(self, connection, result):
        self.description = [(name, None, None, None, None, None, None) for name in result['columns']]
        self.rowcount = -1 if self.description else result['changes']
        self.lastrowid = result['last_insert_rowid']
        rows = []
        for values in result['rows']:
            values = [bytes(value['bytes']) if isinstance(value, dict) and set(value) == {'bytes'} else value
                      for value in values]
            rows.append(Row(result['columns'], values) if connection.row_factory else tuple(values))
        self.rows = iter(rows)

    def __iter__(self):
        return self.rows

    def fetchone(self):
        return next(self.rows, None)

    def fetchall(self):
        return list(self.rows)


class Connection:
    def __init__(self, path, readonly=False, isolation_level='DEFERRED', timeout=30):
        self.path = Path(path)
        self.row_factory = None
        self.isolation_level = isolation_level
        self.in_transaction = False
        self.total_changes = 0
        self.readonly = readonly
        command = [database.executable(), 'storage', 'serve', '--database', str(path)]
        if readonly:
            command.append('--read-only')
        self.process = subprocess.Popen(command, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                        stderr=subprocess.PIPE, text=True)
        self.execute(f'PRAGMA busy_timeout={int(timeout * 1000)}')

    def execute(self, sql, parameters=()):
        first = sql.lstrip().split(None, 1)[0].upper() if sql.strip() else ''
        if (first in ('INSERT', 'UPDATE', 'DELETE', 'REPLACE') and not self.in_transaction
                and self.isolation_level is not None and not self.readonly):
            self.execute('BEGIN')
        if self.process is None:
            raise sqlite3.ProgrammingError('database connection is closed')
        self.process.stdin.write(json.dumps({'sql': sql, 'parameters': parameters}, default=encoded) + '\n')
        self.process.stdin.flush()
        line = self.process.stdout.readline()
        if not line:
            raise sqlite3.OperationalError('native database process exited: ' + self.process.stderr.read(4096))
        result = json.loads(line)
        if 'autocommit' in result:
            self.in_transaction = not result['autocommit']
        if not result.get('ok'):
            kind = sqlite3.IntegrityError if 'constraint' in result['error'].lower() else sqlite3.OperationalError
            raise kind(result['error'])
        self.total_changes = result['total_changes']
        return Cursor(self, result)

    def executemany(self, sql, parameters):
        result = None
        for values in parameters:
            result = self.execute(sql, values)
        return result

    def commit(self):
        if self.in_transaction:
            self.execute('COMMIT')

    def rollback(self):
        if self.in_transaction:
            self.execute('ROLLBACK')

    def backup(self, destination, **kwargs):
        # SQLite only reads the completed offline snapshot; it never opens
        # the live Turso source. Keep existing SQLite destination APIs usable.
        with tempfile.TemporaryDirectory() as directory:
            snapshot = Path(directory) / 'snapshot.db'
            database.native('backup', '--database', self.path, '--output', snapshot)
            source = sqlite3.connect(snapshot.as_uri() + '?mode=ro&immutable=1', uri=True)
            try:
                source.backup(destination, **kwargs)
            finally:
                source.close()

    def close(self):
        process, self.process = self.process, None
        if process is None:
            return
        process.stdin.close()
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            process.terminate()
            process.wait(timeout=5)
        finally:
            process.stdout.close()
            process.stderr.close()

    def __del__(self):
        if getattr(self, 'process', None) is not None:
            self.close()

    def __enter__(self):
        return self

    def __exit__(self, kind, *_):
        self.rollback() if kind else self.commit()


def connect(path, *, readonly=False, isolation_level='DEFERRED', timeout=30):
    if database.engine(path) == 'turso':
        return Connection(path, readonly, isolation_level, timeout)
    uri = Path(path).resolve().as_uri() + ('?mode=ro' if readonly else '?mode=rw')
    return sqlite3.connect(uri, uri=True, timeout=timeout, isolation_level=isolation_level)
