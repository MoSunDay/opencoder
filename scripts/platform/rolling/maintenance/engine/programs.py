"""Stage separate programs and images without editing retained bundles or inputs."""
import hashlib
import json
from pathlib import Path
import shlex
from ... import units
from ...native import effective_config
from ...state import atomic_bytes, write
from ...storage.database import query


def registrations(settings, include_unavailable=False):
    rows = query(settings.state_dir / 'host/host.db',
                 "SELECT r.id,r.mode,r.config,d.body FROM host_runtimes r "
                 "LEFT JOIN fleet_definitions d ON d.kind='runtime_sleep' AND d.id=r.id")
    registered = {identifier: {'mode': mode, 'config': config, 'sleep': body}
                  for identifier, mode, config, body in rows}
    from .unavailable import partition
    retained, unavailable = partition(settings, registered)
    return registered if include_unavailable else retained


def stage(settings, bundle, candidate, original, registered, fallback, operations):
    result = {}
    for identifier, registration in registered.items():
        record = original['releases'].get(identifier)
        if record is None:
            raise ValueError(f'registered Runtime has no immutable release record: {identifier}')
        config = json.loads(registration['config'])
        if (config['data_dir'] != record['runtime_data'] or config['unit'] != record['runtime_unit']
                or config['endpoint'] != f"http://127.0.0.1:{record['runtime_port']}"):
            raise ValueError('Runtime registration differs from its release record')
        result[identifier] = stage_one(settings, bundle, candidate, record, fallback)
        # Check the copied configuration with the actual candidate before
        # closing admission. The command only displays redacted configuration.
        operations.output(str(bundle / 'bin/opencoder'), '--workdir',
                          result[identifier]['workdir'], 'config', 'show')
        from ...manifest import _installer
        expected = _installer.build_info(bundle / 'bin/opencoder-agent')
        image = Path(result[identifier]['rootfs'])
        verify_image(image, expected, operations)
    return result


def verify_image(image, expected, operations):
    names = units.image_programs(image)
    # The main launcher resolves /proc/self/exe before parsing arguments.
    # Match the native execution environment without mounting anything in the
    # host namespace. The private proc mount disappears when this check exits.
    script = ('mount -t proc proc "$1/proc"; root=$1; shift; '
              'for program do chroot "$root" "/usr/bin/$program" --build-info; done')
    output = operations.output('unshare', '--mount', '--propagation', 'private', '--',
                               'sh', '-ec', script, 'image-check', str(image), *names)
    actual = [json.loads(line) for line in output.splitlines() if line.strip()]
    if len(actual) != len(names) or any(info != expected for info in actual):
        raise ValueError('retained Runtime image program differs from the candidate bundle')


def stage_one(settings, bundle, candidate, record, fallback):
    runtime_data = Path(record['runtime_data'])
    unit = settings.systemd_dir / record['runtime_unit']
    content = unit.read_text()
    starts = [line for line in content.splitlines() if line.startswith('ExecStart=')]
    if len(starts) != 1:
        raise ValueError('Runtime unit must have one explicit ExecStart')
    for dropin in (settings.systemd_dir / (record['runtime_unit'] + '.d')).glob('*.conf'):
        if any(line.strip().startswith('ExecStart=') for line in dropin.read_text().splitlines()):
            raise ValueError('Runtime ExecStart override must be resolved before engine maintenance')
    arguments = shlex.split(starts[0].removeprefix('ExecStart='))
    old_agent = next((value for value in arguments if Path(value).name == 'opencoder-agent'), None)
    if old_agent is None or '--workdir' not in arguments:
        raise ValueError('Runtime unit lacks explicit program and workdir')
    old_workdir = Path(arguments[arguments.index('--workdir') + 1])
    config_file = old_workdir / 'opencoder.json'
    config = json.loads(config_file.read_text()) if config_file.exists() else effective_config(old_workdir)
    # Old programs without a frozen OCI image have no image to preserve. Their
    # execution records and resource snapshots remain at their original paths.
    source = record.get('runtime_program', {}).get('rootfs') or runtime_data / 'dag/rootfs'
    source = Path(source)
    if not source.is_dir():
        source = Path(fallback)
    managed = settings.state_dir / 'runtimes' / record['id']
    program_data = runtime_data
    if managed.is_dir() and managed.samefile(runtime_data):
        # Retained registrations can use an older bind-mount alias. Keep the
        # registration unchanged, but stage through one mount when possible.
        program_data = managed
        if source.is_relative_to(runtime_data):
            source = managed / source.relative_to(runtime_data)
    home = program_data / 'programs' / candidate['release_id']
    image = units.freeze_rootfs(record, source, bundle / 'bin', target=home / 'rootfs', link_source=True)
    config.setdefault('dag', {})['rootfs_dir'] = str(image)
    config['dag'].setdefault('data_dir', str(runtime_data / 'dag/runs'))
    # These fields belonged to removed execution backends. Only the new
    # program's private config changes; old configs and resource paths remain.
    for key in ('ops', 'wasm_dir'):
        config['dag'].pop(key, None)
    workdir = home / 'workdir'
    config_bytes = (json.dumps(config, indent=2, sort_keys=True) + '\n').encode()
    target = workdir / 'opencoder.json'
    if target.exists() and target.read_bytes() != config_bytes:
        raise ValueError('staged Runtime configuration changed')
    atomic_bytes(target, config_bytes)
    write(workdir / '.opencoder/ap.json', {'mode': 'off'})
    agent = settings.state_dir / 'releases' / candidate['release_id'] / 'bundle/bin/opencoder-agent'
    arguments[arguments.index(old_agent)] = str(agent)
    arguments[arguments.index('--workdir') + 1] = str(workdir)
    new_start = 'ExecStart=' + ' '.join(units.argument(arg) for arg in arguments)
    staged = home / 'runtime.service'
    atomic_bytes(staged, content.replace(starts[0], new_start).encode())
    return {'release_id': candidate['release_id'], 'manifest': candidate,
            'rootfs': str(image), 'workdir': str(workdir), 'unit_source': str(staged),
            'unit_sha256': hashlib.sha256(staged.read_bytes()).hexdigest(),
            'prior_unit_sha256': hashlib.sha256(unit.read_bytes()).hexdigest()}


def install(settings, journal, prepared):
    for identifier, program in prepared.items():
        record = journal.record(identifier)
        source = Path(program['unit_source']).read_bytes()
        target = settings.systemd_dir / record['runtime_unit']
        actual = hashlib.sha256(target.read_bytes()).hexdigest()
        if actual not in (program['prior_unit_sha256'], program['unit_sha256']):
            raise ValueError('Runtime unit changed after engine preflight')
        if hashlib.sha256(source).hexdigest() != program['unit_sha256']:
            raise ValueError('staged Runtime unit checksum differs')
        atomic_bytes(target, source, 0o644)
        record['runtime_program'] = program
    journal.save()
