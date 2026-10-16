"""TODO export bindings and backup coordination; never inspect credentials."""
from contextlib import contextmanager
from pathlib import Path
import fcntl
import json
import os


def files_root(data):
    binding = Path(data) / 'todo-storage.json'
    if not binding.exists():
        return None
    if binding.is_symlink():
        raise ValueError('TODO storage binding must be a regular file')
    value = json.loads(binding.read_text())['files_root']
    root = Path(value)
    if not root.is_absolute() or any(p.is_symlink() for p in [root, *root.parents]):
        raise ValueError('TODO files root must be an absolute directory without symlinks')
    return root


def settings(config):
    return config.get('project', {}).get('todo_storage', {})


def validate(config, data):
    storage = settings(config)
    requested = storage.get('files_dir')
    bound = files_root(data)
    if bound is not None and (requested is None or Path(requested) != bound):
        raise ValueError('retain the bound TODO files_dir; migrate files before changing it')
    if requested is None:
        return
    root = Path(requested)
    if not root.is_absolute() or any(p.is_symlink() for p in [root, *root.parents]):
        raise ValueError('TODO files_dir must be absolute without symlinks')
    protected = [config.get('agent', {}).get('agents_dir'), config.get('dag', {}).get('binary_dir'),
                 config.get('dag', {}).get('workspace_dir'), config.get('ontology', {}).get('files_dir')]
    for candidate in [root, root.with_name(root.name + '.state')]:
        if Path(data).resolve().is_relative_to(candidate.resolve()) or candidate.resolve().is_relative_to(Path(data).resolve()):
            raise ValueError('TODO files cannot contain Server databases')
        for other in filter(None, protected):
            other = Path(other).resolve()
            if candidate.resolve().is_relative_to(other) or other.is_relative_to(candidate.resolve()):
                raise ValueError('TODO files overlap another resource root')
    limit = storage.get('max_file_bytes', 512 * 1024 * 1024)
    if type(limit) is not int or limit <= 0:
        raise ValueError('TODO max_file_bytes must be positive')


def port(config):
    nfs = settings(config).get('nfs', {})
    if set(nfs) - {'enabled', 'host', 'port'}:
        raise ValueError('unsupported TODO NFS fields; export is always read-only')
    if nfs.get('enabled'):
        return nfs.get('port', 2053)
    return None


@contextmanager
def backup_lock(data):
    root = files_root(data)
    if root is None:
        yield None
        return
    state = root.with_name(root.name + '.state')
    if state.is_symlink() or not state.is_dir():
        raise ValueError('TODO storage coordination directory is unavailable')
    path = state / '.global.lock'
    descriptor = os.open(path, os.O_CREAT | os.O_RDWR | os.O_NOFOLLOW, 0o600)
    try:
        fcntl.flock(descriptor, fcntl.LOCK_EX)
        yield root
    finally:
        os.close(descriptor)
