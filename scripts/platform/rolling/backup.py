"""SQLite online backups; independent files are never labelled one snapshot."""
from pathlib import Path
from contextlib import closing
from concurrent.futures import ThreadPoolExecutor
import json
import os
import shutil
import sqlite3
import tempfile
from .state import write
from .native import ontology
from .storage import todos
from .backup_files import opaque, scope, tree


def database(source, target):
    target.parent.mkdir(parents=True, exist_ok=True)
    if target.exists():
        raise ValueError(f"backup output already exists: {target}")
    with closing(sqlite3.connect(source.resolve().as_uri() + "?mode=ro", uri=True)) as reader:
        # Pin one read snapshot so writes between backup steps cannot restart
        # the copy indefinitely. WAL writers may continue while it is copied.
        reader.execute("BEGIN")
        reader.execute("SELECT count(*) FROM sqlite_schema").fetchone()
        with closing(sqlite3.connect(target)) as writer:
            reader.backup(writer, pages=256, sleep=0.01)
            if writer.execute("PRAGMA quick_check").fetchone() != ("ok",):
                raise ValueError(f"backup verification failed: {source}")
        reader.rollback()
    with target.open("rb") as stream:
        os.fsync(stream.fileno())


def roots(settings, runtime_data=True):
    result = {"server": settings.server_data}
    for name in ("resources", "host"):
        path = settings.state_dir / name
        if path.exists():
            result[name] = path
    if runtime_data:
        if settings.legacy_agent_data:
            result["legacy-node"] = settings.legacy_agent_data
        runtimes = settings.state_dir / "runtimes"
        if runtimes.exists():
            result.update({f"runtimes/{path.name}": path for path in runtimes.iterdir() if path.is_dir()})
    return result


def databases(root, output, runtime=False, opaque_files=None, process_files=False):
    """Snapshot actual DBs; online Runtime views omit immutable inputs only."""
    def failed(error):
        raise error

    selected = scope.RuntimeScope(root) if runtime else None
    for directory, directories, files in os.walk(root, onerror=failed, followlinks=False):
        if selected:
            selected.prune(directory, directories, files)
        for name in files:
            source = Path(directory) / name
            if name.endswith('.db') and not source.is_symlink():
                relative = source.relative_to(root)
                if ((runtime or process_files) and opaque.process_path(relative)
                        and not opaque.sqlite_header(source)):
                    copied = opaque.copy_stable(source, output / relative)
                    if opaque_files is not None:
                        opaque_files[relative.as_posix()] = copied
                else:
                    database(source, output / relative)
    return selected.excluded if selected else {}


def snapshot(settings, output, stopped=False, runtime_data=True):
    with todos.backup_lock(settings.server_data) as todo_root:
        return _snapshot(settings, output, stopped, runtime_data, todo_root)


def _snapshot(settings, output, stopped, runtime_data, todo_root):
    if not runtime_data and not stopped:
        raise ValueError("shared-data maintenance backup requires stopped writers")
    if output.exists():
        raise ValueError("backup destination must be new")
    output.parent.mkdir(parents=True, exist_ok=True)
    # Interrupted attempts remain separate from a completed backup. A retry
    # starts a new staging directory and never edits or deletes the old copy.
    destination = output
    output = Path(tempfile.mkdtemp(prefix=f".{output.name}.incomplete-", dir=output.parent))
    selected = roots(settings, runtime_data)
    # The independent service has no database; preserve ownership and rollback
    # metadata without placing the token file in this metadata copy.
    todo_service = settings.state_dir / 'services/todo-resources/installation.json'
    for name, source in [('installation.json', todo_service),
                         ('policy.json', settings.state_dir / 'todo-files-policy.json')]:
        if source.exists():
            target = output / 'todo-service' / name
            target.parent.mkdir(parents=True, exist_ok=True)
            tree.copy_leaf(source, target)
    for name, root in selected.items():
        if output.is_relative_to(root):
            raise ValueError("backup output cannot be inside a source tree")
        if stopped:
            tree.copy_tree(root, output / name,
                ignore=shutil.ignore_patterns("*.db", "*.db-wal", "*.db-shm", "*.lock"))
    # Each database still has its own pinned read transaction and quick_check.
    # Bound parallel roots so cold historical directories cannot serialize the
    # entire backup; finish every copy before verifying or publishing output.
    opaque_files = {name: {} for name in selected}
    with ThreadPoolExecutor(max_workers=4) as executor:
        pending = {name: executor.submit(databases, root, output / name,
                   not stopped and (name == 'legacy-node' or name.startswith('runtimes/')),
                   opaque_files[name], name == 'legacy-node' or name.startswith('runtimes/'))
                   for name, root in selected.items()}
        excluded = {f'{name}/{path}': reason for name, future in pending.items()
                    for path, reason in future.result().items()}
    # Copy files after the pinned DB snapshot: immutable versions referenced by
    # that snapshot already exist. NFS path bindings must match their hashes too.
    ontology_root = ontology.files_root(output / 'server')
    if ontology_root is not None:
        if output.is_relative_to(ontology_root):
            raise ValueError('backup output cannot be inside Ontology files')
        tree.copy_tree(ontology_root, output / 'ontology-files')
        ontology.verify_files(output / 'server/ontology.db', output / 'ontology-files')
    if todo_root is not None:
        if output.is_relative_to(todo_root):
            raise ValueError('backup output cannot be inside TODO files')
        tree.copy_tree(todo_root, output / 'todo-files')
        # Online DB backups omit ordinary files; retain this binding explicitly.
        target = output / 'server/todo-storage.json'
        if not target.exists():
            target.parent.mkdir(parents=True, exist_ok=True)
            tree.copy_leaf(settings.server_data / 'todo-storage.json', target)
    files = {}
    for path in output.rglob("*"):
        item = tree.metadata(path)
        if item is not None:
            files[str(path.relative_to(output))] = item
        if path.is_file() and not path.is_symlink():
            with path.open("rb") as stream:
                os.fsync(stream.fileno())
    for directory in sorted((p for p in output.rglob("*") if p.is_dir() and not p.is_symlink()),key=lambda p:len(p.parts),reverse=True):
        fd = os.open(directory,os.O_RDONLY | os.O_DIRECTORY)
        try:
            os.fsync(fd)
        finally:
            os.close(fd)
    write(output / "backup-manifest.json", {
        "kind": "stopped-consistent-copy" if stopped else "independent-online-database-backups",
        "cross_database_snapshot": stopped, "files": files,
        "data_roots": {name: str(path) for name, path in selected.items()},
        "excluded_database_trees": excluded,
        "opaque_process_database_files": {f'{name}/{path}': metadata
            for name, copied in opaque_files.items() for path, metadata in copied.items()},
        "runtime_data_included": runtime_data,
        "ontology_files_root": str(ontology_root) if ontology_root else None,
        "todo_files_root": str(todo_root) if todo_root else None})
    os.rename(output, destination)
    with_parent = os.open(destination.parent, os.O_RDONLY | os.O_DIRECTORY)
    try:
        os.fsync(with_parent)
    finally:
        os.close(with_parent)
