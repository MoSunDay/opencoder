"""Replace one sleeping retained Runtime's program after a compatible release."""
import argparse
from contextlib import contextmanager
import fcntl
import hashlib
import json
from pathlib import Path
import re
import sys

if __name__ == '__main__':
    sys.path.insert(0, str(Path(__file__).resolve().parents[3]))

from rolling import manifest
from rolling.state import Journal, atomic_bytes, locked, write
from rolling.storage.database import query
from rolling.maintenance import runtimes
from rolling.maintenance.recovery.capacity import stopped
from rolling.maintenance.engine import programs


@contextmanager
def exclusive(path):
    if path.is_symlink():
        raise ValueError('Runtime repair lock cannot be a symlink')
    with path.open('a+') as stream:
        fcntl.flock(stream, fcntl.LOCK_EX | fcntl.LOCK_NB)
        yield


def host_lock(settings, domain, identifier):
    name = hashlib.sha256((domain + '\0' + identifier).encode()).hexdigest()
    return settings.state_dir / 'host/host.locks' / name


def sleeping(settings, operations, record):
    """Require the Host's durable idle proof and no current reservation."""
    identifier = record['id']
    database = settings.state_dir / 'host/host.db'
    rows = query(database, "SELECT r.mode,r.config,d.body FROM host_runtimes r "
                 "LEFT JOIN fleet_definitions d ON d.kind='runtime_sleep' AND d.id=r.id "
                 "WHERE r.id=?", (identifier,))
    if len(rows) != 1 or rows[0][0] != 'retired' or not rows[0][2]:
        raise ValueError('Runtime must already be retired and hibernated')
    config = json.loads(rows[0][1])
    expected = {'data_dir': record['runtime_data'], 'unit': record['runtime_unit'],
                'endpoint': f"http://127.0.0.1:{record['runtime_port']}"}
    if any(config.get(key) != value for key, value in expected.items()):
        raise ValueError('Runtime registration differs from its release record')
    inventory = json.loads(rows[0][2])
    if not isinstance(inventory, dict):
        raise ValueError('Runtime has no durable hibernation inventory')
    node = (settings.state_dir / 'host/node-id').read_text().strip()
    runtimes.idle(identifier, node, inventory)
    if query(database, "SELECT ticket FROM capacity_queue WHERE runtime_id=? AND phase!='done'",
             (identifier,)):
        raise ValueError('Runtime still owns a reservation')
    stopped(operations, record['runtime_unit'])
    return rows[0]


def apply(settings, operations, journal, record, anchor, receipt):
    data = Path(record['runtime_data'])
    # Same lock order and names as Host hibernation. No Host wake or activation
    # can overlap the unit replacement; node.lock also excludes manual writers.
    with exclusive(host_lock(settings, 'release', 'activation')):
        with exclusive(host_lock(settings, 'runtime-use', record['id'])):
            with exclusive(data / 'node.lock'):
                if sleeping(settings, operations, record) != receipt['registration']:
                    raise ValueError('Runtime idle registration changed after staging')
                prior = (anchor / 'runtime-before.service').read_bytes()
                if hashlib.sha256(prior).hexdigest() != receipt['prior_unit_sha256']:
                    raise ValueError('Runtime repair backup changed')
                unit = settings.systemd_dir / record['runtime_unit']
                program = receipt['program']
                actual = hashlib.sha256(unit.read_bytes()).hexdigest()
                if actual not in (receipt['prior_unit_sha256'], program['unit_sha256']):
                    raise ValueError('Runtime unit changed after staging')
                original_program = receipt['prior_program']
                if record.get('runtime_program') not in (original_program, program):
                    raise ValueError('Runtime program changed after staging')
                try:
                    programs.install(settings, journal, {record['id']: program})
                    operations.run('systemctl', 'daemon-reload')
                except BaseException:
                    # All locks remain held while restoring the prior program.
                    # Retrying uses the same immutable preparation receipt.
                    atomic_bytes(unit, prior, 0o644)
                    if original_program is None:
                        record.pop('runtime_program', None)
                    else:
                        record['runtime_program'] = original_program
                    journal.save()
                    operations.run('systemctl', 'daemon-reload')
                    raise
                receipt['phase'] = 'installed'
                write(anchor / 'receipt.json', receipt)
    return {'runtime_id': record['id'], 'program_release': program['release_id'],
            'phase': 'installed', 'receipt': str(anchor / 'receipt.json'),
            'runtime_started': False, 'execution_recovery_verified': False}


def repair(settings, operations, identifier, release):
    for value in (identifier, release):
        if not re.fullmatch(r'[A-Za-z0-9_-]{1,64}', value):
            raise ValueError('Invalid Runtime or release identifier')
    with locked(settings.state_dir):
        journal = Journal(settings.state_dir)
        if (journal.data.get('phase') != 'complete' or journal.data.get('candidate')
                or journal.data.get('current') != release or identifier == release):
            raise ValueError('Repair requires a completed compatible current release')
        record = journal.record(identifier)
        bundle = settings.state_dir / 'releases' / release / 'bundle'
        candidate = manifest.verify(bundle)
        if candidate != journal.record(release)['manifest']:
            raise ValueError('Published bundle differs from its release record')
        previous = record.get('runtime_program', {}).get('manifest', record['manifest'])
        manifest.compatible(candidate, [previous])
        data = Path(record['runtime_data'])
        if (not data.is_dir() or data.is_symlink()
                or not data.resolve().is_relative_to((settings.state_dir / 'runtimes').resolve())):
            raise ValueError('Repair requires an existing managed Runtime directory')
        anchor = data / 'programs' / release / 'repair'
        path = anchor / 'receipt.json'
        if path.exists():
            receipt = json.loads(path.read_text())
            if receipt.get('runtime_id') != identifier or receipt.get('release_id') != release:
                raise ValueError('Runtime repair receipt belongs to another program')
            if receipt['phase'] == 'installed':
                program = receipt['program']
                unit = settings.systemd_dir / record['runtime_unit']
                if (record.get('runtime_program') != program
                        or hashlib.sha256(unit.read_bytes()).hexdigest() != program['unit_sha256']):
                    raise ValueError('Installed Runtime repair has changed')
                return {'runtime_id': identifier, 'program_release': release,
                        'phase': 'installed', 'receipt': str(path), 'replayed': True,
                        'execution_recovery_verified': False}
        else:
            registration = sleeping(settings, operations, record)
            unit = settings.systemd_dir / record['runtime_unit']
            prior = unit.read_bytes()
            anchor.mkdir(parents=True, exist_ok=True, mode=0o700)
            atomic_bytes(anchor / 'runtime-before.service', prior)
            receipt = {'runtime_id': identifier, 'release_id': release, 'phase': 'preparing',
                       'registration': registration, 'prior_program': record.get('runtime_program'),
                       'prior_unit_sha256': hashlib.sha256(prior).hexdigest()}
            write(path, receipt)
        if receipt['phase'] == 'preparing':
            program = programs.stage_one(settings, bundle, candidate, record, data / 'dag/rootfs')
            expected = manifest._installer.build_info(bundle / 'bin/opencoder-agent')
            programs.verify_image(Path(program['rootfs']), expected, operations)
            operations.output(str(bundle / 'bin/opencoder'), '--workdir',
                              program['workdir'], 'config', 'show')
            receipt.update(program=program, phase='prepared')
            write(path, receipt)
        if receipt['phase'] != 'prepared':
            raise ValueError('Unknown Runtime repair phase')
        return apply(settings, operations, journal, record, anchor, receipt)


def main():
    from rolling.config import load
    from rolling.io import Operations
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--config', type=Path, required=True)
    parser.add_argument('--runtime', required=True)
    parser.add_argument('--release', required=True)
    args = parser.parse_args()
    settings = load(args.config)
    print(json.dumps(repair(settings, Operations(settings.token_file), args.runtime, args.release)))


if __name__ == '__main__':
    main()
