"""Merge retry evidence on the host Python without copying private sandboxes."""
from pathlib import Path
import shutil

EXCLUDED = {'codex-home', 'private-executor', 'executor-output', 'rootfs', 'cache', 'layers'}


def copy_evidence(source, destination):
    source, destination = Path(source), Path(destination)
    if source.is_symlink() or destination.is_symlink():
        raise RuntimeError('Evidence copy cannot follow directory symlinks')
    destination.mkdir(parents=True, exist_ok=True)
    for file in source.iterdir():
        if file.name in EXCLUDED:
            continue
        target = destination / file.name
        if file.is_symlink() or target.is_symlink():
            raise RuntimeError('Evidence copy cannot follow symlinks')
        if file.is_dir():
            copy_evidence(file, target)
        else:
            shutil.copy2(file, target)
