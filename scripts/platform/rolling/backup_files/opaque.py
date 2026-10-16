"""Preserve non-SQLite process stores without interpreting or modifying them."""
import shutil
import time
from .scope import KINDS


def process_path(relative):
    parts = relative.parts
    if parts and parts[0].startswith('.cleared-executions-'):
        parts = parts[1:]
    return ((len(parts) >= 3 and parts[0] in KINDS)
            or (len(parts) >= 2 and parts[0] in ('project-runs', 'workdir', 'workspace')))


def sqlite_header(path):
    with path.open('rb') as source:
        return source.read(16) == b'SQLite format 3\x00'


def stamp(path):
    info = path.stat()
    return info.st_dev, info.st_ino, info.st_size, info.st_mtime_ns, info.st_ctime_ns


def copy_stable(source, target):
    target.parent.mkdir(parents=True, exist_ok=True)
    if target.exists():
        raise ValueError(f'backup output already exists: {target}')
    for _ in range(3):
        before = stamp(source)
        shutil.copy2(source, target)
        if before == stamp(source):
            return {'kind': 'stable-byte-copy', 'sqlite': False}
        # Only the new, unpublished backup copy is retried. Source is untouched.
        target.unlink()
        time.sleep(.01)
    raise ValueError('non-SQLite process file changed during backup')
