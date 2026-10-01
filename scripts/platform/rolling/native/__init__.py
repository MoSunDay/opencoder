"""Immutable native probe publication and per-release runtime configuration."""
import base64
import hashlib
import json
from pathlib import Path
import urllib.error
from ..io import HttpFailure
from ..state import write


def effective_config(workdir, home=None):
    home = Path.home() if home is None else Path(home)
    result = {}
    def merge(target, source):
        for key, value in source.items():
            if isinstance(value, dict) and isinstance(target.get(key), dict):
                merge(target[key], value)
            else:
                target[key] = value
    for path in [home / '.config/opencoder/config.json',
                 home / '.opencoder/opencoder.json', home / '.opencoder/config.json',
                 workdir / 'opencoder.json', workdir / '.opencoder/config.json']:
        if path.exists():
            merge(result, json.loads(path.read_text()))
    return result


def runtime_config(settings, record, rootfs, home=None):
    workdir = Path(record['runtime_data']) / 'workdir'
    path = workdir / 'opencoder.json'
    if path.exists():
        config = json.loads(path.read_text())
        if config['dag']['rootfs_dir'] != str(rootfs):
            raise ValueError('retained runtime rootfs changed')
        return workdir
    config = effective_config(settings.agent_workdir, home=home)
    config.setdefault('dag', {}).update(rootfs_dir=str(rootfs),
        data_dir=str(Path(record['runtime_data']) / 'dag/runs'))
    write(path, config)
    path.chmod(0o600)
    write(workdir / '.opencoder/ap.json', {'mode': 'off'})
    return workdir


def publish_probe(settings, record, operations):
    binary = Path(record['runtime_data']) / 'dag/rootfs/usr/bin/dag-runner'
    data = binary.read_bytes()
    if data[:4] != b'\x7fELF' or len(data) > 32 * 1024 * 1024:
        raise ValueError('release probe must be a bounded native ELF executable')
    return publish_binary(settings.resource_url, 'release-probe', data, operations)


def publish_binary(base, prefix, data, operations):
    digest = hashlib.sha256(data).hexdigest()
    name = prefix + '-' + digest[:24]
    endpoint = '/api/dag/binaries/' + name
    try:
        metadata = operations.http(base, endpoint)
    except (HttpFailure, urllib.error.HTTPError) as error:
        if error.code != 404:
            raise
        try:
            operations.http(base, '/api/dag/binaries', 'POST', {
                'name': name, 'description': 'Immutable native execution resource',
                'binary_b64': base64.b64encode(data).decode()})
        except (HttpFailure, urllib.error.HTTPError) as collision:
            if collision.code != 409:
                raise
        metadata = operations.http(base, endpoint)
    versions = metadata.get('history', [])
    if len(versions) != 1 or versions[0]['sha256'] != digest or versions[0]['size_bytes'] != len(data):
        raise ValueError('release probe resource differs from its immutable binary')
    return name + '@v' + str(versions[0]['version'])
