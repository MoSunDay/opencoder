"""Private configuration snapshots; shared configuration changes after backup."""
import hashlib
import json
from pathlib import Path
import pwd
import os
from ..native import effective_config
from ..state import atomic_bytes


def actual_configs(settings):
    home = Path(pwd.getpwnam(settings.server_user).pw_dir)
    return effective_config(settings.agent_workdir), effective_config(settings.server_workdir, home)


def overlay(base, path):
    if path is None:
        return base
    incoming = json.loads(path.read_text())
    # DAG configuration is a complete current declaration. Preserve provider
    # credentials/settings while dropping obsolete DAG fields in the snapshot.
    result = dict(base)
    for key, value in incoming.items():
        if key == 'dag' or not isinstance(value, dict) or not isinstance(result.get(key), dict):
            result[key] = value
        else:
            result[key] = {**result[key], **value}
    return result


def desired_configs(settings):
    agent, server = actual_configs(settings)
    return overlay(agent, settings.agent_config), overlay(server, settings.server_config)


def encoded(config):
    return (json.dumps(config, sort_keys=True, indent=2) + '\n').encode()


def hashes(configs):
    return {name: hashlib.sha256(encoded(config)).hexdigest()
            for name, config in zip(('agent', 'server'), configs)}


def freeze(settings, identifier, expected):
    configs = desired_configs(settings)
    if hashes(configs) != expected:
        raise ValueError('maintenance configuration changed after preflight')
    result = {}
    for name, config in zip(('agent', 'server'), configs):
        path = settings.state_dir / 'candidate-configs' / identifier / (name + '.json')
        data = encoded(config)
        if path.exists():
            if path.read_bytes() != data:
                raise ValueError('candidate configuration is immutable')
        else:
            atomic_bytes(path, data)
        result[name] = {'path': str(path), 'sha256': expected[name]}
    return result


def install(settings, snapshots):
    verified = {}
    for name, item in snapshots.items():
        data = Path(item['path']).read_bytes()
        if hashlib.sha256(data).hexdigest() != item['sha256']:
            raise ValueError('candidate configuration checksum differs')
        verified[name] = data
    for name, workdir in [('agent', settings.agent_workdir), ('server', settings.server_workdir)]:
        path = workdir / 'opencoder.json'
        owner = path.stat() if path.exists() else None
        atomic_bytes(path, verified[name])
        if name == 'server':
            account = pwd.getpwnam(settings.server_user)
            os.chown(path, account.pw_uid, account.pw_gid)
        elif owner:
            os.chown(path, owner.st_uid, owner.st_gid)
