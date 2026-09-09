"""Record the whole source tree without refresh locks or intrusive write probes."""
from pathlib import Path
import gzip
import json
import os
import stat
from common import SOURCE, git, read, sha, write


def capture(root, label):
    destination = root / 'evidence' / f'workspace-{label}.jsonl.gz'
    errors = []
    count = 0
    with gzip.open(destination, 'wt') as stream:
        for directory, dirs, files in os.walk(SOURCE, followlinks=False):
            dirs.sort()
            for name in sorted([*dirs, *files]):
                file = Path(directory) / name
                try:
                    value = file.lstat()
                    row = [str(file.relative_to(SOURCE)), value.st_mode, value.st_size,
                           value.st_mtime_ns, value.st_ctime_ns, value.st_ino, value.st_dev]
                    if stat.S_ISLNK(value.st_mode):
                        row.append(os.readlink(file))
                    stream.write(json.dumps(row, ensure_ascii=False) + '\n')
                    count += 1
                except FileNotFoundError:
                    errors.append(str(file))  # An external writer changed a path during the scan.
    repositories = [SOURCE]
    for category in ['repos', 'dep_repos']:
        repositories.extend(p for p in (SOURCE / category).iterdir() if (p / '.git').exists())
    states = {}
    for repo in repositories:
        status = git(repo, 'status', '--porcelain=v1', '-z', '--untracked-files=all')
        states[str(repo)] = {'head': git(repo, 'rev-parse', 'HEAD'), 'status': status}
    write(root / 'evidence' / f'git-{label}.json', states)
    write(root / 'evidence' / f'audit-{label}.json', {'entries': count, 'scanChanges': errors,
          'manifestSha256': sha(destination), 'repositories': len(states)})
    return count


def compare(root):
    def rows(label):
        with gzip.open(root / 'evidence' / f'workspace-{label}.jsonl.gz', 'rt') as stream:
            return {row[0]: row[1:] for row in map(json.loads, stream)}
    before, after = rows('before'), rows('after')
    changed = [p for p in before.keys() | after.keys() if before.get(p) != after.get(p)]
    git_before = read(root / 'evidence/git-before.json')
    git_after = read(root / 'evidence/git-after.json')
    git_changes = [p for p in git_before.keys() | git_after.keys() if git_before.get(p) != git_after.get(p)]
    result = {'metadataChanges': sorted(changed), 'originalTreeUnchanged': not changed,
              'gitStateChanges': git_changes,
              'method': 'all paths: type, size, mtime, ctime, inode, device, symlink target; all repository HEAD/status',
              'changesNotReverted': True}
    write(root / 'evidence' / 'workspace-comparison.json', result)
    return result
