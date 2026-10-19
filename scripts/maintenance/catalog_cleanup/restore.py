"""Restore one verified catalog object, without replacing any database."""
from contextlib import ExitStack
import fcntl
import hashlib
import json
from pathlib import Path
import shutil
import sqlite3
import tempfile
from .apply import verify_pool
from .client import require, sync_tree, sync_directory


def restore(plan, receipt):
    require(receipt['status'] == 'deleted', 'only deleted objects can be restored')
    name, kind = receipt['name'], receipt['kind']
    backup = Path(receipt['backup'])
    snapshot = json.loads((backup / 'snapshot.json').read_text())
    require(snapshot.get('name', snapshot.get('id')) == name, 'backup name mismatch')
    database = Path(plan['data_dir']) / 'control.db'
    lock_dir = database.with_suffix('.locks')
    require(lock_dir.is_dir(), 'server lock directory is missing')
    lock_path = lock_dir / hashlib.sha256(b'catalog-maintenance\0registry').hexdigest()
    with ExitStack() as resources:
        require(not lock_path.is_symlink(), 'invalid catalog lock')
        lock = resources.enter_context(lock_path.open('a+'))
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        if kind == 'binary':
            require(name not in ('.', '..', '.locks') and '/' not in name, 'invalid pool name')
            pool_locks = Path(plan['binary_root']) / '.locks'
            require(not pool_locks.is_symlink(), 'invalid pool lock directory')
            pool_locks.mkdir(exist_ok=True)
            pool_lock_path = pool_locks / name
            require(not pool_lock_path.is_symlink(), 'invalid pool lock')
            pool_lock = resources.enter_context(pool_lock_path.open('a+'))
            fcntl.flock(pool_lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        connection = sqlite3.connect(database)
        if kind == 'dag':
            current = connection.execute("SELECT body FROM fleet_definitions WHERE kind='dag' AND id=?", (name,)).fetchone()
            require(current is None or json.loads(current[0]) == snapshot, 'name has been reused')
            if current is None:
                with connection:
                    connection.execute("INSERT INTO fleet_definitions(kind,id,body) VALUES('dag',?,?)",
                                       (name, json.dumps(snapshot, ensure_ascii=False)))
        else:
            require(kind == 'binary' and name not in ('.', '..') and '/' not in name, 'invalid pool name')
            verify_pool(backup / 'pool', snapshot)
            root = Path(plan['binary_root'])
            destination = root / name
            if destination.exists():
                verify_pool(destination, snapshot)
            else:
                stage = Path(tempfile.mkdtemp(prefix='.restore-', dir=root))
                try:
                    shutil.copytree(backup / 'pool', stage / 'pool')
                    verify_pool(stage / 'pool', snapshot)
                    sync_tree(stage / 'pool')
                    (stage / 'pool').rename(destination)
                    sync_directory(root)
                finally:
                    shutil.rmtree(stage)
        with connection:
            connection.execute("DELETE FROM fleet_definitions WHERE kind='catalog_retired' AND id=?", (kind + ':' + name,))
        connection.close()
    return {'kind': kind, 'name': name, 'status': 'restored'}
