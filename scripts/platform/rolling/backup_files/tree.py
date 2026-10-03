"""Never open devices or pipes while copying a stopped filesystem tree."""
import hashlib
import os
from pathlib import Path
import shutil
import stat


def kind(mode):
    for predicate, name in ((stat.S_ISREG, 'regular'), (stat.S_ISDIR, 'directory'),
                            (stat.S_ISLNK, 'symlink'), (stat.S_ISCHR, 'character'),
                            (stat.S_ISBLK, 'block'), (stat.S_ISFIFO, 'fifo')):
        if predicate(mode):
            return name
    raise ValueError('unsupported backup filesystem entry; stop its owning process first')


def copy_leaf(source, target):
    source, target = Path(source), Path(target)
    info = source.lstat()
    entry = kind(info.st_mode)
    if entry == 'regular':
        return shutil.copy2(source, target, follow_symlinks=False)
    if entry not in ('character', 'block', 'fifo'):
        raise ValueError('backup leaf must be a regular file, device or pipe')
    os.mknod(target, info.st_mode, info.st_rdev)
    shutil.copystat(source, target, follow_symlinks=False)
    return str(target)


def copy_tree(source, target, **options):
    return shutil.copytree(source, target, symlinks=True, copy_function=copy_leaf, **options)


def metadata(path):
    path = Path(path)
    info = path.lstat()
    entry = kind(info.st_mode)
    if entry == 'directory':
        return None
    if entry == 'symlink':
        return {'link': os.readlink(path)}
    result = {'mode': stat.S_IMODE(info.st_mode)}
    if entry == 'regular':
        digest = hashlib.sha256()
        with path.open('rb') as stream:
            for block in iter(lambda: stream.read(1024 * 1024), b''):
                digest.update(block)
        result['sha256'] = digest.hexdigest()
    else:
        result['kind'] = entry
        if entry in ('character', 'block'):
            result['device'] = info.st_rdev
    return result


def inventory(root):
    root = Path(root)
    return {path.relative_to(root).as_posix(): item
            for path in sorted(root.rglob('*')) if (item := metadata(path)) is not None}
