"""Verify immutable inputs; debug packaging never claims release provenance."""
import shutil
from rolling import manifest
from rolling.state import write
from rolling.maintenance.archive import digest


NAMES = manifest._installer.NAMES


def binary_inventory(directory, names=NAMES):
    result = {}
    infos = []
    for name in names:
        path = directory / name
        manifest._installer.regular_file(path)
        result['bin/' + name] = {'bytes': path.stat().st_size, 'sha256': digest(path)}
        infos.append(manifest._installer.build_info(path))
    if any(info != infos[0] for info in infos):
        raise ValueError('input binaries have different build metadata')
    return result, infos[0]


def debug_manifest(files, info):
    return {'schema_version': 1, 'release_id': 'debug-' + info['git_commit'],
            'commit': info['git_commit'], 'version': info['version'],
            'version_long': info['version_long'], 'protocol_version': info['protocol_version'],
            'brain_schema_version': info['brain_schema_version'], 'spa_sha256': info['spa_sha256'],
            'compatibility': info['release_compatibility'], 'files': files}


def package_debug(directory, output):
    files, info = binary_inventory(directory)
    (output / 'bin').mkdir(parents=True)
    for name in NAMES:
        shutil.copy2(directory / name, output / 'bin' / name)
    value = debug_manifest(files, info)
    write(output / 'manifest.json', value)
    members = ['manifest.json', *sorted(files)]
    (output / 'SHA256SUMS').write_text(''.join(digest(output / name) + '  ' + name + '\n'
                                            for name in members))
    write(output.parent / 'debug-input.json', {'release_bundle': False, 'build': info, 'manifest': value})
    return value, info


def old_input(bundle):
    value = manifest._installer.verify_bundle(bundle)
    info = manifest._installer.build_info(bundle / 'bin/opencoder-server')
    if info['release_compatibility']['data_format'] != {'min': 1, 'max': 1}:
        raise ValueError('--old-bundle must contain an actual data-format1 Server')
    return value, info


def corrective_input(bundle, candidate):
    value = manifest.verify(bundle)
    if value['commit'] == candidate['commit'] or value['release_id'] == candidate['release_id']:
        raise ValueError('corrective bundle must contain a different compiled commit and release ID')
    manifest.compatible(value, [candidate])
    files, info = binary_inventory(bundle / 'bin')
    return value, files, info


def image_input(rootfs):
    if rootfs.is_symlink() or not rootfs.is_dir():
        raise ValueError('--rootfs must be a real directory')
    result = {}
    for name in ('dag-runner', 'agent-step-runner'):
        path = rootfs / 'usr/bin' / name
        manifest._installer.regular_file(path)
        result[name] = manifest._installer.build_info(path)
    return result
