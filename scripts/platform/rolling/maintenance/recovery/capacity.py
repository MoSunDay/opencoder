"""Explicit cancellation recovery for a stopped Runtime's orphan reservation."""
import argparse
from contextlib import contextmanager, closing
import fcntl
import hashlib
import json
from pathlib import Path
import re
import sqlite3
import sys
import uuid

if __name__ == '__main__':
    sys.path.insert(0, str(Path(__file__).resolve().parents[3]))

from rolling import backup
from rolling.state import Journal, write
from rolling.state import atomic_bytes
from rolling.maintenance import services


@contextmanager
def lock(path):
    if path.is_symlink():
        raise ValueError('recovery lock must not be a symlink')
    with path.open('a+') as stream:
        fcntl.flock(stream.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
        yield


def kernel_owners(unit):
    owners = []
    for path in Path('/proc').glob('[0-9]*/cgroup'):
        try:
            rows = path.read_text().splitlines()
        except FileNotFoundError:
            continue
        if any(unit in row.split(':', 2)[-1].split('/') for row in rows):
            owners.append(int(path.parent.name))
    return owners


def stopped(operations, unit):
    raw = operations.output('systemctl', 'show', unit, '-p', 'ActiveState', '-p', 'MainPID')
    status = dict(line.split('=', 1) for line in raw.splitlines() if '=' in line)
    if status.get('ActiveState') not in ('inactive', 'failed') or status.get('MainPID') != '0':
        raise ValueError('Runtime service must be stopped before reservation recovery')
    if kernel_owners(unit):
        raise ValueError('Runtime still owns kernel processes; complete cleanup first')


def frozen(status, node):
    server, nodes = status.get('server', {}), status.get('nodes', [])
    return (server.get('mode') == 'frozen' and server.get('inflight_admissions') == 0
            and len(nodes) == 1 and nodes[0].get('node_id') == node
            and 200 <= nodes[0].get('status', 0) < 300
            and nodes[0].get('body', {}).get('mode') == 'frozen'
            and nodes[0]['body'].get('active_runs') in (0, 1)
            and nodes[0]['body'].get('owned_processes') == 0)


def reservation_scope(conn, runtime, execution, ticket):
    live = conn.execute("SELECT ticket,runtime_id,execution_id,phase FROM capacity_queue WHERE phase!='done'").fetchall()
    if live not in ([], [(ticket, runtime, execution, 'running')]):
        raise ValueError('another reservation remains; recovery cannot prove sole orphan ownership')


def recover(settings, operations, runtime, execution, ticket):
    for identifier in (runtime, execution, ticket):
        if not re.fullmatch(r'[A-Za-z0-9_-]+', identifier):
            raise ValueError('invalid recovery identifier')
    journal = Journal(settings.state_dir)
    if journal.data.get('maintenance', {}).get('writes_open'):
        raise ValueError('legacy cancellation recovery must precede maintenance write reopening')
    current = journal.record(journal.data['current'])
    node = (settings.state_dir / 'host/node-id').read_text().strip()
    endpoint = f"http://127.0.0.1:{current['server_port']}"
    # The orphan itself may be the one logical active run. Kernel/process and
    # exclusive locks prove its absence; demanding active_runs=0 here would
    # make recovery of the reservation that causes active_runs=1 impossible.
    if not frozen(operations.http(endpoint, '/api/admin/drain'), node):
        raise ValueError('Server and sole managed node must be frozen with no owned processes')
    host = settings.state_dir / 'host'
    database = host / 'host.db'
    key = hashlib.sha256(('runtime-use\0' + runtime).encode()).hexdigest()
    with lock(host / 'host.locks' / key):
        with closing(sqlite3.connect(database.resolve().as_uri() + '?mode=rw', uri=True)) as conn:
            row = conn.execute('SELECT config,mode FROM host_runtimes WHERE id=?', (runtime,)).fetchone()
            if row is None or row[1] != 'retired':
                raise ValueError('recovery requires the registered retired Runtime')
            reservation = conn.execute('SELECT runtime_id,execution_id,phase FROM capacity_queue WHERE ticket=?', (ticket,)).fetchone()
            if reservation is None or reservation[:2] != (runtime, execution) or reservation[2] not in ('running', 'done'):
                raise ValueError('reservation identity or phase differs')
            reservation_scope(conn, runtime, execution, ticket)
            config = json.loads(row[0])
            unit = config['unit']
            if not re.fullmatch(r'opencoder-runtime-[A-Za-z0-9_-]+\.service', unit):
                raise ValueError('recovery requires an explicit managed Runtime unit')
            data = Path(config['data_dir'])
            if not data.is_absolute() or data.is_symlink() or not data.resolve().is_relative_to((settings.state_dir / 'runtimes').resolve()):
                raise ValueError('Runtime data must be inside the managed runtime directory')
            if (data / 'node-id').read_text().strip() != node:
                raise ValueError('Runtime belongs to another node')
            binding = json.loads((data / 'host-binding.json').read_text())
            if binding != {'database': str(database), 'runtime_id': runtime}:
                raise ValueError('Runtime Host binding differs from recovery scope')
            stopped(operations, unit)
            with lock(data / 'node.lock'):
                return release(conn, settings, operations, database, runtime, execution, ticket, row)


def release(conn, settings, operations, database, runtime, execution, ticket, registration):
    anchor = settings.state_dir / 'maintenance' / 'cancel-reservations' / ticket
    anchor.mkdir(parents=True, exist_ok=True, mode=0o700)
    path = anchor / 'receipt.json'
    identity = {'runtime': runtime, 'execution': execution, 'ticket': ticket,
                'config_sha256': hashlib.sha256(registration[0].encode()).hexdigest()}
    receipt = json.loads(path.read_text()) if path.exists() else {**identity, 'stage': 'backup'}
    if any(receipt.get(key) != value for key, value in identity.items()):
        raise ValueError('recovery anchor belongs to another reservation')
    saved = anchor / 'host-before.db'
    admission = Path(json.loads(registration[0])['data_dir']) / 'admission.json'
    if receipt['stage'] == 'backup':
        write(path, receipt)
        if not saved.exists():
            temporary = anchor / ('.incomplete-' + uuid.uuid4().hex + '.db')
            backup.database(database, temporary)
            temporary.rename(saved)
        with closing(sqlite3.connect(saved.resolve().as_uri() + '?mode=ro&immutable=1', uri=True)) as snapshot:
            if snapshot.execute('PRAGMA quick_check').fetchone() != ('ok',):
                raise ValueError('reservation backup is incomplete')
        if admission.is_symlink():
            raise ValueError('Runtime admission must not be a symlink')
        prior = anchor / 'runtime-admission-before.json'
        if not prior.exists():
            # None records the original absence without introducing a live
            # default. Preserve this control anchor across matched retries.
            write(prior, json.loads(admission.read_text()) if admission.exists() else None)
        receipt.update(stage='releasing', backup_sha256=hashlib.sha256(saved.read_bytes()).hexdigest())
        receipt['admission_sha256'] = hashlib.sha256(prior.read_bytes()).hexdigest()
        write(path, receipt)
    if hashlib.sha256(saved.read_bytes()).hexdigest() != receipt['backup_sha256']:
        raise ValueError('reservation backup checksum differs')
    if hashlib.sha256((anchor / 'runtime-admission-before.json').read_bytes()).hexdigest() != receipt['admission_sha256']:
        raise ValueError('Runtime admission backup checksum differs')
    conn.execute('PRAGMA synchronous=FULL')
    conn.execute('BEGIN IMMEDIATE')
    try:
        actual = conn.execute('SELECT config,mode FROM host_runtimes WHERE id=?', (runtime,)).fetchone()
        if actual != registration:
            raise ValueError('Runtime registration changed during recovery')
        stopped(operations, json.loads(registration[0])['unit'])
        row = conn.execute('SELECT runtime_id,execution_id,phase FROM capacity_queue WHERE ticket=?', (ticket,)).fetchone()
        if row is None or row[:2] != (runtime, execution) or row[2] not in ('running', 'done'):
            raise ValueError('reservation identity or phase differs')
        reservation_scope(conn, runtime, execution, ticket)
        if admission.is_symlink():
            raise ValueError('Runtime admission must not be a symlink')
        # Hold node.lock until the offline freeze and release are durable.
        # The unchanged old binary then starts frozen, before any recovery
        # scheduler can resume historical Agent work.
        atomic_bytes(admission, b'{"version":1,"mode":"frozen"}\n')
        if row[2] == 'running':
            conn.execute("UPDATE capacity_queue SET phase='done' WHERE ticket=?", (ticket,))
        conn.commit()
    except BaseException:
        conn.rollback()
        raise
    receipt['stage'] = 'released'
    write(path, receipt)
    return {**receipt, 'backup': str(saved), 'next': 'start the unchanged old Runtime and cancel the execution through its API'}


def _live_rows(database):
    # Read WAL state as well; immutable connections intentionally ignore the
    # live WAL and could hand recovery an obsolete reservation set.
    with closing(sqlite3.connect(database.resolve().as_uri() + '?mode=ro', uri=True)) as conn:
        return conn.execute(
            "SELECT ticket,runtime_id,execution_id,phase FROM capacity_queue WHERE phase='running' ORDER BY sequence"
        ).fetchall()


def recover_running(settings, operations, wait_seconds=90):
    """Recover every proven stale running reservation across managed Runtimes.

    This is the boot-safe path. It never guesses which ticket is stale: all
    live rows are snapshotted, their Runtime bindings are checked, and the
    exact set is released in one transaction after every affected unit is
    stopped and has no kernel-owned processes.
    """
    host = settings.state_dir / 'host'
    database = host / 'host.db'
    rows = _live_rows(database)
    if not rows:
        return {'stage': 'complete', 'recovered': [], 'next': 'start managed services'}
    node = (host / 'node-id').read_text().strip()
    grouped = {}
    with closing(sqlite3.connect(database.resolve().as_uri() + '?mode=ro', uri=True)) as conn:
        for ticket, runtime, execution, phase in rows:
            registration = conn.execute(
                'SELECT config,mode FROM host_runtimes WHERE id=?', (runtime,)
            ).fetchone()
            if registration is None:
                raise ValueError(f'capacity Runtime {runtime} is not registered')
            config = json.loads(registration[0])
            data = Path(config['data_dir'])
            if not data.is_absolute() or data.is_symlink() or not data.resolve().is_relative_to((settings.state_dir / 'runtimes').resolve()):
                raise ValueError('Runtime data must be inside the managed runtime directory')
            if (data / 'node-id').read_text().strip() != node:
                raise ValueError('Runtime belongs to another node')
            binding = json.loads((data / 'host-binding.json').read_text())
            if binding != {'database': str(database), 'runtime_id': runtime}:
                raise ValueError('Runtime Host binding differs from recovery scope')
            grouped.setdefault(runtime, {'config': config, 'rows': []})['rows'].append(
                (ticket, runtime, execution, phase, registration)
            )
    for runtime, item in grouped.items():
        unit = item['config'].get('unit')
        if not re.fullmatch(r'opencoder-runtime-[A-Za-z0-9_-]+\.service', unit or ''):
            raise ValueError('recovery requires an explicit managed Runtime unit')
        services.stop_unit(unit, operations, wait_seconds)
        stopped(operations, unit)
    anchor_id = hashlib.sha256(json.dumps(rows, separators=(',', ':')).encode()).hexdigest()
    anchor = settings.state_dir / 'maintenance' / 'capacity-recovery' / anchor_id
    anchor.mkdir(parents=True, exist_ok=True, mode=0o700)
    saved = anchor / 'host-before.db'
    if not saved.exists():
        temporary = anchor / ('.incomplete-' + uuid.uuid4().hex + '.db')
        backup.database(database, temporary)
        temporary.rename(saved)
    with closing(sqlite3.connect(saved.resolve().as_uri() + '?mode=ro&immutable=1', uri=True)) as conn:
        if conn.execute('PRAGMA quick_check').fetchone() != ('ok',):
            raise ValueError('capacity recovery backup is incomplete')
    before = {}
    for runtime, item in grouped.items():
        admission = Path(item['config']['data_dir']) / 'admission.json'
        if admission.is_symlink():
            raise ValueError('Runtime admission must not be a symlink')
        path = anchor / f'{runtime}-admission-before.json'
        if not path.exists():
            write(path, json.loads(admission.read_text()) if admission.exists() else None)
        before[runtime] = str(path)
        atomic_bytes(admission, b'{"version":1,"mode":"frozen"}\n')
    with closing(sqlite3.connect(database)) as conn:
        conn.execute('PRAGMA synchronous=FULL')
        conn.execute('BEGIN IMMEDIATE')
        released = []
        try:
            for ticket, runtime, execution, phase in rows:
                current = conn.execute(
                    'SELECT runtime_id,execution_id,phase FROM capacity_queue WHERE ticket=?', (ticket,)
                ).fetchone()
                if current == (runtime, execution, 'done'):
                    continue
                if current != (runtime, execution, 'running'):
                    raise ValueError(
                        f'capacity reservation changed during recovery: {ticket} '
                        f'expected {(runtime, execution, "running")}, got {current}'
                    )
                conn.execute(
                    "UPDATE capacity_queue SET phase='done' WHERE ticket=? AND runtime_id=? AND execution_id=? AND phase='running'",
                    (ticket, runtime, execution),
                )
                released.append(ticket)
            conn.commit()
        except BaseException:
            conn.rollback()
            raise
    runtime_units = [
        {'id': runtime, 'unit': item['config']['unit'], 'endpoint': item['config'].get('endpoint')}
        for runtime, item in sorted(grouped.items())
    ]
    receipt = {'stage': 'released', 'anchor': str(anchor), 'backup': str(saved),
               'reservations': [{'ticket': t, 'runtime': r, 'execution': e,
                                'state': 'released' if t in released else 'already_done'}
                                for t, r, e, _ in rows],
               'admission_backups': before, 'runtime_units': runtime_units,
               'next': 'start affected Runtime units, Host, then Server'}
    write(anchor / 'receipt.json', receipt)
    return receipt


def main():
    from rolling.config import load
    from rolling.io import Operations
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--config', type=Path, required=True)
    parser.add_argument('--runtime')
    parser.add_argument('--execution')
    parser.add_argument('--ticket')
    parser.add_argument('--recover-running', action='store_true',
                        help='recover all proven stale running reservations')
    args = parser.parse_args()
    settings = load(args.config)
    operations = Operations(settings.token_file)
    if args.recover_running:
        print(json.dumps(recover_running(settings, operations), indent=2))
    else:
        if not (args.runtime and args.execution and args.ticket):
            parser.error('--runtime, --execution and --ticket are required without --recover-running')
        print(json.dumps(recover(settings, operations, args.runtime, args.execution, args.ticket), indent=2))


if __name__ == '__main__':
    main()
