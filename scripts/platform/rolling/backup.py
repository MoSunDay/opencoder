"""SQLite online backups; independent files are never labelled one snapshot."""
from pathlib import Path
from contextlib import closing
import json
import os
import shutil
import sqlite3
import tempfile
from .state import write
from .native import ontology
from .backup_files import tree


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


def snapshot(settings, output, stopped=False, runtime_data=True):
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
    for name, root in selected.items():
        if output.is_relative_to(root):
            raise ValueError("backup output cannot be inside a source tree")
        if stopped:
            tree.copy_tree(root, output / name,
                ignore=shutil.ignore_patterns("*.db", "*.db-wal", "*.db-shm", "*.lock"))
        for source in root.rglob("*.db"):
            if not source.is_symlink():
                database(source, output / name / source.relative_to(root))
    # Copy files after the pinned DB snapshot: immutable versions referenced by
    # that snapshot already exist. NFS path bindings must match their hashes too.
    ontology_root = ontology.files_root(output / 'server')
    if ontology_root is not None:
        if output.is_relative_to(ontology_root):
            raise ValueError('backup output cannot be inside Ontology files')
        tree.copy_tree(ontology_root, output / 'ontology-files')
        ontology.verify_files(output / 'server/ontology.db', output / 'ontology-files')
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
        "runtime_data_included": runtime_data,
        "ontology_files_root": str(ontology_root) if ontology_root else None})
    os.rename(output, destination)
    with_parent = os.open(destination.parent, os.O_RDONLY | os.O_DIRECTORY)
    try:
        os.fsync(with_parent)
    finally:
        os.close(with_parent)
