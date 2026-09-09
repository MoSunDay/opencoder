#!/usr/bin/python3
"""Mount a read-only source boundary before executing any acceptance process."""
from pathlib import Path
import json
import os
import subprocess
import sys
import time


def main():
    root, role, *command = sys.argv[1:]
    if role != '--inside':
        os.execv('/usr/bin/unshare', ['unshare', '--mount', '--propagation', 'private',
            '--', '/usr/bin/python3', __file__, root, '--inside', role, *command])
    role, *command = command
    source = '/root/workspace'
    subprocess.run(['mount', '--bind', source, source], check=True)
    subprocess.run(['mount', '-o', 'remount,bind,ro', source], check=True)
    # All existing submounts must also be read-only inside this namespace.
    rows = [line.split() for line in Path('/proc/self/mountinfo').read_text().splitlines()]
    nested = sorted({r[4] for r in rows if r[4].startswith(source + '/')}, key=len, reverse=True)
    for point in nested:
        subprocess.run(['mount', '-o', 'remount,bind,ro', point], check=True)
    probe = Path(source) / 'agents.md'
    try:
        fd = os.open(probe, os.O_WRONLY)  # No truncate/create, even if the boundary is broken.
    except OSError as error:
        if error.errno != 30:
            raise
    else:
        os.close(fd)
        raise RuntimeError('Original workspace is writable; refusing execution')
    directory = Path(root) / 'evidence' / 'isolation'
    directory.mkdir(parents=True, exist_ok=True)
    (directory / f'{role}-{os.getpid()}.json').write_text(json.dumps({
        'role': role, 'pid': os.getpid(), 'time': time.time(), 'source': source,
        'writeOpen': 'EROFS', 'mounts': [line for line in Path('/proc/self/mountinfo')
        .read_text().splitlines() if source in line]}, indent=2))
    os.execvpe(command[0], command, os.environ)


if __name__ == '__main__':
    main()
