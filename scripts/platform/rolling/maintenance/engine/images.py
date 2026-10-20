"""Bind an explicit retained-runtime program upgrade to its original OCI image."""
import hashlib
import json
from pathlib import Path
from ...state import atomic_bytes

NAME = 'runtime-image-replacement.json'
SCHEMA = 'opencoder.runtime-image-replacement.v1'
RUNNERS = ('dag-runner', 'agent-step-runner')


def stage(runtime_data, source, target):
    runtime_data, source, target = (Path(p).resolve(strict=True) for p in (runtime_data, source, target))
    if not target.is_relative_to(runtime_data / 'programs') or source == target:
        raise ValueError('Retained runtime image replacement is outside its private program directory')
    sources = {str(source)}
    original = runtime_data / 'dag/rootfs'
    # Engine maintenance retains the first image at this fixed location even
    # when a previous program upgrade predates replacement receipts.
    if original.is_dir() and not original.is_symlink():
        sources.add(str(original.resolve(strict=True)))
    previous = source.parent / NAME
    if previous.exists():
        value = read(previous, runtime_data, source)
        sources.update(value['source_rootfs'])
    value = {'schema': SCHEMA, 'runtime_data': str(runtime_data),
             'source_rootfs': sorted(sources), 'target_rootfs': str(target),
             'runner_sha256': {name: hashlib.sha256((target / 'usr/bin' / name).read_bytes()).hexdigest()
                               for name in RUNNERS}}
    path = target.parent / NAME
    raw = (json.dumps(value, indent=2, sort_keys=True) + '\n').encode()
    if path.is_symlink() or path.exists() and path.read_bytes() != raw:
        raise ValueError('Retained runtime image replacement receipt changed')
    atomic_bytes(path, raw)
    return str(path)


def read(path, runtime_data, target):
    if path.is_symlink() or not path.is_file() or path.stat().st_size > 16384:
        raise ValueError('Retained runtime image replacement receipt is invalid')
    value = json.loads(path.read_text())
    if (value.get('schema') != SCHEMA or value.get('runtime_data') != str(runtime_data)
            or value.get('target_rootfs') != str(target)
            or not isinstance(value.get('source_rootfs'), list) or not value['source_rootfs']
            or any(not isinstance(p, str) or not Path(p).is_absolute() for p in value['source_rootfs'])
            or set(value.get('runner_sha256', {})) != set(RUNNERS)
            or any(hashlib.sha256((target / 'usr/bin' / name).read_bytes()).hexdigest()
                   != value['runner_sha256'][name] for name in RUNNERS)):
        raise ValueError('Retained runtime image replacement does not match its frozen image')
    return value
