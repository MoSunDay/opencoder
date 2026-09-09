"""Verify a run has stopped while preserving its databases and evidence."""
from pathlib import Path
import os
import shutil
from common import read, run, write


def initialize_runtime(root):
    root = Path(root).resolve()
    runtime = root / 'runtime'
    owner = root / 'evidence/runtime-owner.json'
    if owner.exists() or runtime.exists() or runtime.is_symlink():
        raise RuntimeError('Choose a new acceptance root; an existing run cannot be overwritten')
    runtime.mkdir(mode=0o700)
    identity = runtime.stat()
    write(owner, {'runtime': str(runtime), 'identity': [identity.st_dev, identity.st_ino]})


def assert_owned(root):
    root = Path(root).resolve()
    runtime = root / 'runtime'
    owner = root / 'evidence/runtime-owner.json'
    if owner.is_symlink() or not owner.is_file():
        raise RuntimeError('Runtime ownership record missing; cannot finalize')
    expected = read(owner)
    if expected.get('runtime') != str(runtime) or runtime.is_symlink():
        raise RuntimeError('Runtime ownership mismatch; cannot finalize')
    if runtime.exists():
        actual = runtime.stat()
        if not runtime.is_dir() or expected.get('identity') != [actual.st_dev, actual.st_ino]:
            raise RuntimeError('Runtime identity changed; cannot finalize')
    return runtime


def references_runtime(path, runtime):
    return path == runtime or path.startswith(runtime + '/')


def occupants(runtime):
    """Include open files and private mount namespaces, even with an outside cwd."""
    runtime = str(runtime)
    processes, mounts, namespaces = [], [], set()
    for proc in Path('/proc').iterdir():
        if not proc.name.isdigit():
            continue
        try:
            paths = [os.readlink(proc / field) for field in ['cwd', 'root', 'exe']]
            for fd in (proc / 'fd').iterdir():
                try:
                    value = os.readlink(fd)
                    paths.append(value[:-10] if value.endswith(' (deleted)') else value)
                except FileNotFoundError:
                    pass
            if any(references_runtime(path, runtime) for path in paths):
                processes.append(int(proc.name))
            namespace = os.readlink(proc / 'ns/mnt')
            if namespace not in namespaces:
                namespaces.add(namespace)
                for line in (proc / 'mountinfo').read_text().splitlines():
                    fields = line.split()
                    if any(references_runtime(value.replace('\\040', ' '), runtime)
                           for value in fields[3:5]):
                        mounts.append({'namespace': namespace, 'mount': line})
        except (FileNotFoundError, ProcessLookupError):
            continue
        except PermissionError as error:
            # A permission gap cannot establish that removal is safe.
            raise RuntimeError(f'Cannot inspect process {proc.name} before finalization') from error
    return {'remainingRuntimeProcesses': processes, 'remainingRuntimeMounts': mounts}


def finalize_runtime(root, destroy=False):
    root = Path(root).resolve()
    runtime = assert_owned(root)
    if not runtime.exists():
        raise RuntimeError('Runtime is missing; cannot confirm retained databases')
    busy = occupants(runtime)
    write(root / 'evidence/process-cleanup.json', busy)
    if busy['remainingRuntimeProcesses'] or busy['remainingRuntimeMounts']:
        raise RuntimeError('Runtime is still referenced by processes or mounts; cannot finalize')
    size = int(run(['du', '-sB1', runtime]).split()[0])
    assert_owned(root)
    if destroy:
        shutil.rmtree(runtime)
    return {'runtimeRemoved': destroy, 'runtimeRetained': not destroy,
            'runtimeBytesBeforeCleanup': size}
