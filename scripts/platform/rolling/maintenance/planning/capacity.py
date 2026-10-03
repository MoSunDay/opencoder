"""Account for concurrent copies on each filesystem before admission closes."""
import os
from pathlib import Path
import shutil
from ...backup_files.tree import kind

MARGIN = 256 * 1024 * 1024


def tree_bytes(root, omitted=(), block_size=1):
    root = Path(root)
    if root.is_symlink() or not root.exists():
        return 0
    entry = kind(root.lstat().st_mode)
    if entry in ('character', 'block', 'fifo'):
        return block_size if block_size > 1 else 0
    if root.is_file():
        return ((root.stat().st_size + block_size - 1) // block_size) * block_size
    total = block_size if block_size > 1 else 0
    pending = [(root, Path('.'))]
    while pending:
        directory, relative = pending.pop()
        with os.scandir(directory) as entries:
            for entry in entries:
                name = relative / entry.name
                if name.as_posix() in omitted:
                    continue
                if entry.is_symlink():
                    total += block_size if block_size > 1 else 0
                    continue
                if entry.is_dir(follow_symlinks=False):
                    total += block_size if block_size > 1 else 0
                    pending.append((Path(entry.path), name))
                elif entry.is_file(follow_symlinks=False):
                    size = entry.stat(follow_symlinks=False).st_size
                    total += ((size + block_size - 1) // block_size) * block_size
                else:
                    kind(entry.stat(follow_symlinks=False).st_mode)
                    total += block_size if block_size > 1 else 0
    return total


def existing(path):
    path = Path(path).resolve()
    while not path.exists():
        path = path.parent
    return path


def plan(settings, candidate, rootfs, original, scope):
    from ..archive import managed_units
    from ...native.ontology import files_root
    identifier = candidate['release_id']
    block_size = os.statvfs(existing(settings.state_dir)).f_frsize
    frozen = settings.state_dir / 'runtimes' / identifier / 'dag/rootfs'
    # Runner replacement can grow the source image. Count both packaged
    # runners as additional space, including the stopped backup's second copy.
    binaries = sum(item['bytes'] for name, item in candidate['files'].items()
                   if name in ('bin/dag-runner', 'bin/agent-step-runner'))
    image = tree_bytes(rootfs, {'dev', 'proc', 'sys', 'tmp', 'workspace/context'}, block_size) + binaries
    freeze = 0 if frozen.exists() else image
    roots = [settings.server_data, settings.state_dir / 'host', settings.state_dir / 'runtimes',
             settings.state_dir / 'resources']
    if settings.legacy_agent_data:
        roots.append(settings.legacy_agent_data)
    ontology = files_root(settings.server_data)
    if ontology:
        roots.append(ontology)
    backup = sum(tree_bytes(path, block_size=block_size) for path in roots) + freeze
    controls = [*managed_units(settings, original, scope), settings.nginx_include,
                settings.state_dir / 'services', settings.state_dir / 'resources',
                settings.state_dir / 'host/deployment.json']
    controls.extend(path / name for path in (settings.server_workdir, settings.agent_workdir)
                    for name in ('opencoder.json', '.opencoder'))
    controls.extend(settings.bin_dir / name for name in
                    ('opencoder', 'opencoder-cli', 'opencoder-server', 'opencoder-agent',
                     '.opencoder-platform-current', '.opencoder-platform-manifest.json'))
    control = sum(tree_bytes(path, block_size=block_size) for path in dict.fromkeys(controls))
    # Staging and installed bundle copies can coexist. Existing/incomplete
    # copies already consume available bytes; retries still reserve a full copy.
    packages = 2 * sum(item['bytes'] for item in candidate['files'].values())
    migrations = [(root, sum(path.stat().st_size for path in root.glob('*.db')
                            if path.is_file() and not path.is_symlink()))
                  for root in (settings.server_data, settings.state_dir / 'resources')]
    components = {'frozen_image': freeze, 'stopped_backup': backup, 'controls': control,
                  'packages': packages, 'margin': MARGIN}
    budgets = {}
    for path, amount in [(settings.state_dir, sum(components.values())),
                         *((root, size + MARGIN) for root, size in migrations if size)]:
        destination = existing(path)
        device = destination.stat().st_dev
        item = budgets.setdefault(device, {'path': str(destination), 'required': 0})
        item['required'] += amount
    return {'components': components, 'migration': sum(size for _, size in migrations),
            'filesystems': list(budgets.values())}


def check(budget):
    for item in budget['filesystems']:
        available = shutil.disk_usage(item['path']).free
        if available < item['required']:
            raise ValueError(f"insufficient space before maintenance: {item['path']} "
                             f"needs {item['required']} bytes, available {available}")
    return budget
