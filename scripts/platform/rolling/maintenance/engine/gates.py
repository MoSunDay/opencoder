"""Freeze only this Server and its local Runtimes; remote nodes keep running."""
import json
import os
from pathlib import Path
import pwd
from ...state import atomic_bytes
from .. import gates as ingress, runtimes
from . import programs


def server_mode(settings, mode):
    path = settings.server_data / 'admission.json'
    if path.is_symlink():
        raise ValueError('Server admission state cannot be a symlink')
    account = pwd.getpwnam(settings.server_user)
    owner = (path.stat().st_uid, path.stat().st_gid) if path.exists() else (account.pw_uid, account.pw_gid)
    atomic_bytes(path, json.dumps({'version': 1, 'mode': mode}).encode(), owner=owner)


def runtime_call(operations, config, command):
    reply = operations.http(config['endpoint'], '/rpc', 'POST',
                            {'operation': 'admission', 'command': command})
    if reply.get('status') != 200:
        raise ValueError('local Runtime rejected admission transition')
    return reply['body']


def close(settings, record, operations, seconds):
    ingress.close(settings, record, operations, seconds)
    server_mode(settings, 'frozen')
    for registration in programs.registrations(settings).values():
        if registration['mode'] != 'staged' and not json.loads(registration['sleep'] or 'null'):
            runtime_call(operations, json.loads(registration['config']), 'freeze')


def drained(settings, record, operations):
    status = operations.http(f"http://127.0.0.1:{record['server_port']}", '/api/ready', accepted=(503,))
    if status.get('mode') != 'frozen' or status.get('inflight_admissions') != 0:
        return False
    try:
        return runtimes.capture(settings, operations)
    except ValueError:
        return False


def reopen(settings, record, operations, seconds):
    # Retired sleeping Runtimes read this durable file when their Host wakes
    # them. No remote admission RPC is issued by this maintenance flow.
    for item in programs.registrations(settings).values():
        config = json.loads(item['config'])
        if item['mode'] == 'staged':
            continue
        if json.loads(item['sleep'] or 'null'):
            path = Path(config['data_dir']) / 'admission.json'
            if path.is_symlink():
                raise ValueError('Runtime admission state cannot be a symlink')
            owner = (path.stat().st_uid, path.stat().st_gid) if path.exists() else None
            atomic_bytes(path, json.dumps({'version': 1, 'mode': 'open'}).encode(), owner=owner)
        else:
            runtime_call(operations, config, 'reopen')
    server_mode(settings, 'open')
    ingress.reopen(settings, record, operations, seconds)
