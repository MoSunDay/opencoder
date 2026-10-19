"""Require a pinned retired Runtime when accepting historical Brain reads."""
import hashlib
import json
from pathlib import Path
import sqlite3
import subprocess


def required_owner(database, execution_id, runtime_id, current_runtime):
    if not execution_id or not runtime_id or not current_runtime or runtime_id == current_runtime:
        raise ValueError('history acceptance requires a pinned older Runtime')
    connection = sqlite3.connect(Path(database).resolve().as_uri() + '?mode=ro', uri=True)
    try:
        connection.execute('PRAGMA query_only=ON')
        rows = connection.execute(
            'SELECT r.id,r.mode,r.config FROM runtime_owners o '
            'JOIN host_runtimes r ON r.id=o.runtime_id WHERE o.execution_id=?',
            (execution_id,)).fetchall()
    finally:
        connection.close()
    if len(rows) != 1 or rows[0][0] != runtime_id or rows[0][1] != 'retired':
        raise ValueError('history execution must belong to the pinned retired Runtime')
    config = json.loads(rows[0][2])
    unit = config.get('unit')
    if not isinstance(unit, str) or not unit.startswith('opencoder-runtime-') or not unit.endswith('.service'):
        raise ValueError('historical Runtime unit is missing or invalid')
    return {'runtime': runtime_id, 'unit': unit,
            'config_sha256': hashlib.sha256(json.dumps(config, sort_keys=True).encode()).hexdigest()}


def process_identity(unit):
    identifier = subprocess.check_output(
        ['systemctl', 'show', '--property=MainPID', '--value', '--', unit], text=True).strip()
    if not identifier.isdecimal() or int(identifier) <= 0:
        raise ValueError('historical Runtime is not running after its detail read')
    root = Path('/proc') / identifier
    # The process name in parentheses may itself contain spaces.
    ticks = (root / 'stat').read_text().rsplit(')', 1)[1].split()[19]
    return {'pid': int(identifier), 'start_ticks': ticks,
            'executable': str((root / 'exe').resolve(strict=True))}


def snapshot(database, execution_id, runtime_id, current_runtime, schema_version,
             read, identify=process_identity):
    """A read never passes without matching ownership and a real live process.

    The caller pins this result throughout its observation. Failed reads are
    propagated, without switching execution IDs or silently retrying samples.
    """
    owner = required_owner(database, execution_id, runtime_id, current_runtime)
    response = read(execution_id)
    if response.get('schema_version') != schema_version or response.get('run', {}).get('run_id') != execution_id:
        raise ValueError('historical response does not match the pinned execution/schema')
    identity = identify(owner['unit'])
    if not isinstance(identity.get('pid'), int) or identity['pid'] <= 0 or not identity.get('start_ticks') or not identity.get('executable'):
        raise ValueError('historical Runtime process identity is incomplete')
    if required_owner(database, execution_id, runtime_id, current_runtime) != owner:
        raise ValueError('historical Runtime ownership changed during the read')
    return {**owner, **identity, 'execution_id': execution_id,
            'schema_version': schema_version}
