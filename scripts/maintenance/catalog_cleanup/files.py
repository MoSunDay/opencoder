"""Inspect every resource file; unknown data never proves absence of references."""
import os
from pathlib import Path
import stat

MAX_BYTES = 4 * 1024 * 1024


def resource_texts(root):
    root = Path(root)
    if not stat.S_ISDIR(root.lstat().st_mode):
        raise ValueError('agent resource root is not a directory')
    pending = [root]
    while pending:
        directory = pending.pop()
        with os.scandir(directory) as entries:
            for entry in entries:
                path = Path(entry.path)
                if entry.is_symlink():
                    raise ValueError('unverified agent link: ' + str(path))
                if entry.is_dir(follow_symlinks=False):
                    pending.append(path)
                    continue
                if not entry.is_file(follow_symlinks=False):
                    raise ValueError('unverified agent file: ' + str(path))
                with path.open('rb') as stream:
                    data = stream.read(MAX_BYTES + 1)
                if len(data) > MAX_BYTES or b'\0' in data:
                    raise ValueError('unverified agent file: ' + str(path))
                try:
                    text = data.decode('utf-8')
                except UnicodeDecodeError as error:
                    raise ValueError('unverified agent encoding: ' + str(path)) from error
                yield path.relative_to(root), text
