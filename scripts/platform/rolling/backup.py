"""SQLite online backups; independent files are never labelled one snapshot."""
from pathlib import Path
from contextlib import closing
import hashlib
import json
import os
import shutil
import sqlite3
import tempfile
from .state import write


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


def snapshot(settings, output, stopped=False):
    if output.exists():
        raise ValueError("backup destination must be new")
    output.parent.mkdir(parents=True, exist_ok=True)
    # Interrupted attempts remain separate from a completed backup. A retry
    # starts a new staging directory and never edits or deletes the old copy.
    destination = output
    output = Path(tempfile.mkdtemp(prefix=f".{output.name}.incomplete-", dir=output.parent))
    roots = {"server": settings.server_data}
    if settings.legacy_agent_data:
        roots["legacy-node"] = settings.legacy_agent_data
    host = settings.state_dir / "host"
    if host.exists():
        roots["host"] = host
    runtimes = settings.state_dir / "runtimes"
    if runtimes.exists():
        roots.update({f"runtimes/{path.name}": path for path in runtimes.iterdir() if path.is_dir()})
    for name, root in roots.items():
        if output.is_relative_to(root):
            raise ValueError("backup output cannot be inside a source tree")
        if stopped:
            shutil.copytree(root, output / name, symlinks=True,
                ignore=shutil.ignore_patterns("*.db", "*.db-wal", "*.db-shm", "*.lock"))
        for source in root.rglob("*.db"):
            if not source.is_symlink():
                database(source, output / name / source.relative_to(root))
    files = {}
    for path in output.rglob("*"):
        if path.is_file() and not path.is_symlink():
            digest = hashlib.sha256()
            with path.open("rb") as stream:
                for block in iter(lambda: stream.read(1024 * 1024), b""):
                    digest.update(block)
                os.fsync(stream.fileno())
            files[str(path.relative_to(output))] = digest.hexdigest()
    for directory in sorted((p for p in output.rglob("*") if p.is_dir() and not p.is_symlink()),key=lambda p:len(p.parts),reverse=True):
        fd = os.open(directory,os.O_RDONLY | os.O_DIRECTORY)
        try:
            os.fsync(fd)
        finally:
            os.close(fd)
    write(output / "backup-manifest.json", {
        "kind": "stopped-consistent-copy" if stopped else "independent-online-database-backups",
        "cross_database_snapshot": stopped, "files": files})
    os.rename(output, destination)
    with_parent = os.open(destination.parent, os.O_RDONLY | os.O_DIRECTORY)
    try:
        os.fsync(with_parent)
    finally:
        os.close(with_parent)
