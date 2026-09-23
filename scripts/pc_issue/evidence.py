"""Evidence hashes and build-source checks against actual local bytes."""
import hashlib
import json
import os
from pathlib import Path
import subprocess


def evidence(path):
    path = Path(path).resolve(strict=True)
    if not path.is_file():
        raise ValueError('Evidence must be a file')
    h = hashlib.sha256()
    size = 0
    with path.open('rb') as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b''):
            h.update(block)
            size += len(block)
    if not size:
        raise ValueError('Empty evidence')
    return {'path': str(path), 'sha256': h.hexdigest(), 'bytes': size}


def verify_source(source):
    worktree = Path(source['worktree']).resolve(strict=True)
    def git(*args):
        return subprocess.check_output(['git', '-C', str(worktree), *args], stderr=subprocess.PIPE).decode().strip()
    if Path(git('rev-parse', '--show-toplevel')).resolve() != worktree:
        raise ValueError('Worktree resolved to a parent repository')
    if git('rev-parse', 'HEAD') != source['commit']:
        raise ValueError('Build source commit differs from worktree HEAD')
    if git('ls-files','--others','--exclude-standard'):
        raise ValueError('Build worktree contains untracked files; freeze all changes before building')
    diff=subprocess.check_output(['git','-C',str(worktree),'diff','--binary','HEAD','--'],stderr=subprocess.PIPE)
    expected=hashlib.sha256(diff).hexdigest()
    primary=[c for c in source['changes'] if c.get('repo')==source['repo']]
    if len(primary)!=1 or primary[0]['diff_sha256']!=expected or not diff:
        raise ValueError('Primary build diff does not match the actual isolated worktree')
    for change in source['changes']:
        actual = evidence(change['diff_path'])
        if actual['sha256'] != change['diff_sha256']:
            raise ValueError('Build diff hash mismatch')
    return source


def freeze_source(path, destination):
    """Freeze full original case input; retries cannot silently replace it."""
    data = Path(path).read_bytes()
    json.loads(data)
    dest = Path(destination)
    dest.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
    try:
        with dest.open('xb') as stream:
            stream.write(data)
            stream.flush()
            os.fsync(stream.fileno())
        dest.chmod(0o600)
    except FileExistsError:
        if dest.read_bytes() != data:
            raise ValueError('Original case input changed for the same execution') from None
    return str(dest.resolve())
