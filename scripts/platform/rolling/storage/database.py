"""Select a live database's owning engine before opening it."""
from contextlib import closing, contextmanager
import json
import os
from pathlib import Path
import shutil
import sqlite3
import subprocess


@contextmanager
def using(executable):
    previous = os.environ.get('OPENCODER_STORAGE_CLI')
    os.environ['OPENCODER_STORAGE_CLI'] = str(executable)
    try:
        yield
    finally:
        if previous is None:
            os.environ.pop('OPENCODER_STORAGE_CLI', None)
        else:
            os.environ['OPENCODER_STORAGE_CLI'] = previous


def engine(path):
    marker = Path(str(Path(path).resolve()) + '.engine.json')
    if not marker.exists():
        return 'libsql'
    if json.loads(marker.read_text()) != {'engine': 'turso', 'format': 1}:
        raise ValueError(f'unsupported database engine marker: {marker}')
    return 'turso'


def executable():
    path = os.environ.get('OPENCODER_STORAGE_CLI') or shutil.which('opencoder-cli')
    if not path:
        raise ValueError('Turso database maintenance requires opencoder-cli')
    info = json.loads(subprocess.check_output([path, '--build-info'], text=True))
    if info.get('storage_engine') != 'turso':
        raise ValueError('installed opencoder-cli cannot open Turso databases')
    return path


def native(*arguments):
    result = subprocess.run([executable(), 'storage', *map(str, arguments)],
                            check=False, text=True, capture_output=True)
    if result.returncode:
        raise RuntimeError('native database maintenance failed: ' + result.stderr.strip()[:4096])
    return json.loads(result.stdout)


def query(path, sql, parameters=(), *, offline=False):
    path = Path(path)
    if not offline and engine(path) == 'turso':
        return native('query', '--database', path, '--sql', sql,
                      '--params', json.dumps(parameters))['rows']
    uri = path.resolve().as_uri() + ('?mode=ro&immutable=1' if offline else '?mode=ro')
    with closing(sqlite3.connect(uri, uri=True)) as connection:
        connection.execute('PRAGMA query_only=ON')
        return [list(row) for row in connection.execute(sql, parameters)]


def backup(source, target):
    if engine(source) != 'turso':
        return False
    native('backup', '--database', source, '--output', target)
    return True
