"""Only Runtime-owned immutable inputs are outside an online DB snapshot."""
import json
from pathlib import Path


KINDS = {'brain', 'agent', 'dag', 'team', 'todos', 'project',
         'maintenance', 'system', 'operator'}


class RuntimeScope:
    def __init__(self, root):
        self.root = root
        self.excluded = {}

    def exclude(self, path, reason):
        self.excluded[path.relative_to(self.root).as_posix()] = reason

    def prune(self, directory, directories, files):
        """Prune before scandir; never use these names as a global ignore list."""
        directory = Path(directory)
        parts = directory.relative_to(self.root).parts
        if parts and parts[0].startswith('.cleared-executions-'):
            parts = parts[1:]
        if not parts:
            # Reserved DirectoryLayout resource roots, not the platform's
            # independent resources service or an execution's writable HOME.
            for name in ('resources', 'project-resources'):
                if name in directories:
                    self.exclude(directory / name, 'pinned execution resources')
        if parts == ('dag',) and 'rootfs' in directories:
            self.exclude(directory / 'rootfs', 'immutable Runtime OCI image')
        # One archive parent preserves historical journals without treating
        # a user's nested agent/resources or rootfs-upper as platform data.
        if (len(parts) == 2 and parts[-2] in KINDS
                and {'execution.json', 'pending-create.json'}.intersection(files)):
            for name in directories:
                if name == 'resources' or name.startswith('resources.staging-'):
                    self.exclude(directory / name, 'pinned execution resources')
        # run_parent's date/definition/run hierarchy plus a committed OCI
        # receipt identify the real run. Writable task data is in upper and
        # workspace; neither is pruned. Unrecognized/writable OCI roots stay.
        if (len(parts) == 5 and parts[:2] == ('dag', 'runs')
                and 'container.json' in files):
            config = directory / 'bundle/config.json'
            if config.is_file():
                root = json.loads(config.read_text()).get('root', {})
                if root.get('path') == 'rootfs' and root.get('readonly') is True:
                    for name in ('rootfs-upper', 'rootfs-work', 'bundle/rootfs'):
                        self.exclude(directory / name, 'read-only DAG container root')
        directories[:] = [name for name in directories
                          if (directory / name).relative_to(self.root).as_posix()
                          not in self.excluded]
