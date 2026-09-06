#!/usr/bin/env python3
"""Consistent backup and isolated restore for platform control/node data."""

from __future__ import annotations

import argparse
import contextlib
import fcntl
import hashlib
import json
import os
import pathlib
import re
import shutil
import sqlite3
import sys
import time

STATUSES = {"pending", "running", "idle", "cancelling", "interrupted", "done", "error", "cancelled"}
ACTIVE = {"pending", "running", "idle", "cancelling"}
NODE_NAME = re.compile(r"^[A-Za-z0-9._-]+$")


class ArchiveError(RuntimeError):
    pass


def sha256(path: pathlib.Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def fsync_dir(path: pathlib.Path) -> None:
    fd = os.open(path, os.O_RDONLY | os.O_DIRECTORY)
    try:
        os.fsync(fd)
    finally:
        os.close(fd)


def copy_file(source: pathlib.Path, target: pathlib.Path) -> None:
    with source.open("rb") as reader, target.open("xb") as writer:
        shutil.copyfileobj(reader, writer)
        writer.flush()
        os.fchmod(writer.fileno(), os.fstat(reader.fileno()).st_mode & 0o777)
        os.fsync(writer.fileno())


def real_directory(path: pathlib.Path, label: str) -> pathlib.Path:
    path = path.absolute()
    cursor = pathlib.Path(path.anchor)
    for part in path.parts[1:]:
        cursor /= part
        if cursor.is_symlink():
            raise ArchiveError(f"{label} contains a symlink: {cursor}")
    if not path.is_dir():
        raise ArchiveError(f"{label} is not a directory: {path}")
    return path.resolve(strict=True)


def new_output(path: pathlib.Path, label: str) -> pathlib.Path:
    path = path.absolute()
    if path.name in {"", ".", ".."}:
        raise ArchiveError(f"invalid {label}: {path}")
    path = real_directory(path.parent, f"{label} parent") / path.name
    if path.exists() or path.is_symlink():
        raise ArchiveError(f"{label} already exists: {path}")
    return path


def frozen(path: pathlib.Path, label: str) -> None:
    if path.is_symlink() or not path.is_file():
        raise ArchiveError(f"{label} admission state is missing or linked")
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise ArchiveError(f"invalid {label} admission state: {error}") from error
    if value != {"version": 1, "mode": "frozen"}:
        raise ArchiveError(f"{label} must be durably frozen before backup")


def require_drained_status(status: object, label: str) -> None:
    if not isinstance(status, str) or status not in STATUSES:
        raise ArchiveError(f"{label} has unknown status: {status!r}")
    if status in ACTIVE:
        raise ArchiveError(f"{label} is not drained ({status})")


def node_records_drained(node: pathlib.Path) -> None:
    records = []
    for kind in node.iterdir():
        if kind.is_symlink():
            raise ArchiveError(f"node data contains a symlink: {kind}")
        if not kind.is_dir():
            continue
        if kind.name == "executions":
            records.extend(path for path in kind.iterdir() if path.suffix == ".json")
            continue
        for execution in kind.iterdir():
            if execution.is_symlink():
                raise ArchiveError(f"node data contains a symlink: {execution}")
            if execution.is_dir():
                record = execution / "execution.json"
                if record.exists() or record.is_symlink():
                    records.append(record)
    for path in records:
        if path.is_symlink() or not path.is_file():
            raise ArchiveError(f"node execution record is not a regular file: {path}")
        try:
            status = json.loads(path.read_text(encoding="utf-8"))["assignment"]["index"]["status"]
        except (OSError, json.JSONDecodeError, KeyError, TypeError) as error:
            raise ArchiveError(f"invalid execution record {path}: {error}") from error
        require_drained_status(status, f"node execution {path}")


def copy_tree(source: pathlib.Path, target: pathlib.Path, skip: set[str] | None = None) -> None:
    skip = skip or set()
    target.mkdir()
    for entry in sorted(source.iterdir(), key=lambda item: item.name):
        if entry.name in skip:
            continue
        if entry.is_symlink():
            raise ArchiveError(f"backup source contains a symlink: {entry}")
        destination = target / entry.name
        if entry.is_dir():
            copy_tree(entry, destination)
        elif entry.is_file():
            copy_file(entry, destination)
        else:
            raise ArchiveError(f"backup source contains a special file: {entry}")
    fsync_dir(target)


def open_locked_database(stack: contextlib.ExitStack, path: pathlib.Path) -> sqlite3.Connection:
    if path.is_symlink() or not path.is_file():
        raise ArchiveError(f"database is missing or linked: {path}")
    connection = sqlite3.connect(path, timeout=0, isolation_level=None)
    stack.callback(connection.close)
    try:
        connection.execute("PRAGMA busy_timeout=0")
        connection.execute("BEGIN IMMEDIATE")
    except sqlite3.Error as error:
        raise ArchiveError(f"database is busy; refuse inconsistent backup: {path}: {error}") from error
    stack.callback(lambda: connection.execute("ROLLBACK"))
    return connection


def sqlite_backup(source_path: pathlib.Path, target: pathlib.Path) -> None:
    mode = source_path.stat().st_mode & 0o777
    source_uri = source_path.resolve(strict=True).as_uri() + "?mode=ro"
    source = sqlite3.connect(source_uri, uri=True, timeout=0)
    destination = sqlite3.connect(target)
    try:
        source.execute("PRAGMA busy_timeout=0")
        source.backup(destination)
        destination.commit()
        result = destination.execute("PRAGMA quick_check").fetchone()
        if result != ("ok",):
            raise ArchiveError(f"database quick_check failed: {target}: {result}")
    finally:
        destination.close()
        source.close()
    with target.open("rb") as stream:
        os.fchmod(stream.fileno(), mode)
        os.fsync(stream.fileno())


def lock_node(stack: contextlib.ExitStack, node: pathlib.Path) -> None:
    path = node / "node.lock"
    if path.is_symlink() or not path.is_file():
        raise ArchiveError(f"node lock is missing or linked: {path}")
    stream = path.open("rb")
    stack.callback(stream.close)
    try:
        fcntl.flock(stream, fcntl.LOCK_EX | fcntl.LOCK_NB)
    except BlockingIOError as error:
        raise ArchiveError(f"node is still running: {node}") from error


def require_server_executions_drained(connection: sqlite3.Connection) -> None:
    exists = connection.execute(
        "SELECT 1 FROM sqlite_master WHERE type='table' AND name='execution_index'"
    ).fetchone()
    if not exists:
        raise ArchiveError("control database has no execution_index")
    for execution_id, status in connection.execute("SELECT id,status FROM execution_index"):
        require_drained_status(status, f"server execution index {execution_id}")


def archive_files(root: pathlib.Path) -> dict[str, dict[str, int | str]]:
    files = {}
    manifest = root / "backup-manifest.json"
    for path in sorted(root.rglob("*")):
        if path.is_symlink():
            raise ArchiveError(f"archive contains a symlink: {path}")
        if path.is_file() and path != manifest:
            relative = path.relative_to(root).as_posix()
            files[relative] = {"sha256": sha256(path), "bytes": path.stat().st_size}
    return files


def write_manifest(stage: pathlib.Path, node_names: list[str]) -> None:
    value = {
        "schema_version": 1,
        "kind": "opencoder-platform-data-backup",
        "created_at": int(time.time()),
        "nodes": node_names,
        "files": archive_files(stage),
    }
    path = stage / "backup-manifest.json"
    with path.open("x", encoding="utf-8") as stream:
        json.dump(value, stream, indent=2, sort_keys=True)
        stream.write("\n")
        stream.flush()
        os.fsync(stream.fileno())
    fsync_dir(stage)


def verify_archive(root: pathlib.Path) -> dict:
    root = real_directory(root, "backup")
    manifest = root / "backup-manifest.json"
    if manifest.is_symlink() or not manifest.is_file():
        raise ArchiveError("backup manifest is missing or linked")
    try:
        value = json.loads(manifest.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise ArchiveError(f"invalid backup manifest: {error}") from error
    if not isinstance(value, dict) or value.get("schema_version") != 1:
        raise ArchiveError("unsupported backup manifest")
    if value.get("kind") != "opencoder-platform-data-backup":
        raise ArchiveError("invalid backup kind")
    files = value.get("files")
    if not isinstance(files, dict) or files != archive_files(root):
        raise ArchiveError("backup file inventory or checksum does not match")
    nodes = value.get("nodes")
    if not isinstance(nodes, list) or any(not isinstance(name, str) for name in nodes):
        raise ArchiveError("invalid backup node inventory")
    if len(nodes) != len(set(nodes)) or any(not NODE_NAME.fullmatch(name) for name in nodes):
        raise ArchiveError("invalid backup node inventory")
    if {path.name for path in (root / "nodes").iterdir()} != set(nodes):
        raise ArchiveError("backup node directories do not match manifest")
    for database in [root / "server/control.db", root / "server/definitions.db"] + [
        root / "nodes" / name / "runtime.db" for name in nodes
    ]:
        uri = database.resolve(strict=True).as_uri() + "?mode=ro&immutable=1"
        connection = sqlite3.connect(uri, uri=True)
        try:
            if connection.execute("PRAGMA quick_check").fetchone() != ("ok",):
                raise ArchiveError(f"database quick_check failed: {database}")
        finally:
            connection.close()
    if files != archive_files(root):
        raise ArchiveError("backup changed while it was being verified")
    return value


def backup(server: pathlib.Path, nodes: dict[str, pathlib.Path], output: pathlib.Path) -> None:
    server = real_directory(server, "server data")
    nodes = {name: real_directory(path, f"node {name}") for name, path in nodes.items()}
    output = new_output(output, "backup output")
    for source in [server, *nodes.values()]:
        if output.is_relative_to(source):
            raise ArchiveError("backup output cannot be inside a source directory")
    frozen(server / "admission.json", "server")
    for name, node in nodes.items():
        frozen(node / "admission.json", f"node {name}")
        node_records_drained(node)
    stage = output.parent / f".{output.name}.tmp-{os.getpid()}"
    if stage.exists() or stage.is_symlink():
        raise ArchiveError(f"backup staging path already exists: {stage}")
    try:
        with contextlib.ExitStack() as stack:
            for node in nodes.values():
                lock_node(stack, node)
            control = open_locked_database(stack, server / "control.db")
            definitions = open_locked_database(stack, server / "definitions.db")
            require_server_executions_drained(control)
            stage.mkdir()
            (stage / "server").mkdir()
            (stage / "nodes").mkdir()
            copy_file(server / "admission.json", stage / "server/admission.json")
            sqlite_backup(server / "control.db", stage / "server/control.db")
            sqlite_backup(server / "definitions.db", stage / "server/definitions.db")
            for name, node in nodes.items():
                target = stage / "nodes" / name
                copy_tree(
                    node,
                    target,
                    {"node.lock", "runtime.db", "runtime.db-wal", "runtime.db-shm"},
                )
                runtime = open_locked_database(stack, node / "runtime.db")
                sqlite_backup(node / "runtime.db", target / "runtime.db")
            frozen(server / "admission.json", "server")
            for name, node in nodes.items():
                frozen(node / "admission.json", f"node {name}")
            write_manifest(stage, sorted(nodes))
            verify_archive(stage)
            os.replace(stage, output)
            fsync_dir(output.parent)
    except BaseException:
        shutil.rmtree(stage, ignore_errors=True)
        raise


def restore(source: pathlib.Path, output: pathlib.Path) -> None:
    source = real_directory(source, "backup")
    verify_archive(source)
    output = new_output(output, "restore output")
    if output.is_relative_to(source):
        raise ArchiveError("restore output cannot be inside the backup")
    stage = output.parent / f".{output.name}.tmp-{os.getpid()}"
    if stage.exists() or stage.is_symlink():
        raise ArchiveError(f"restore staging path already exists: {stage}")
    try:
        copy_tree(source, stage)
        verify_archive(stage)
        os.replace(stage, output)
        fsync_dir(output.parent)
    except BaseException:
        shutil.rmtree(stage, ignore_errors=True)
        raise


def parse_nodes(values: list[str]) -> dict[str, pathlib.Path]:
    nodes = {}
    for value in values:
        name, separator, raw_path = value.partition("=")
        if not separator or not NODE_NAME.fullmatch(name) or name in nodes:
            raise ArchiveError(f"invalid or duplicate --node NAME=DIR: {value}")
        nodes[name] = pathlib.Path(raw_path)
    if not nodes:
        raise ArchiveError("at least one --node NAME=DIR is required")
    return nodes


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Create or restore a verified platform data snapshot.",
        epilog=(
            "Backup requires durable Frozen admission on Server and every Node, "
            "a stopped Worker (exclusive node.lock), and no active Server index. "
            "SQLite write locks define one consistent snapshot boundary. Restore "
            "always creates a new isolated directory."
        ),
    )
    subcommands = parser.add_subparsers(dest="command", required=True)
    create = subcommands.add_parser("backup")
    create.add_argument("--server-data", required=True, type=pathlib.Path)
    create.add_argument("--node", action="append", default=[])
    create.add_argument("--output", required=True, type=pathlib.Path)
    recover = subcommands.add_parser("restore")
    recover.add_argument("--backup", required=True, type=pathlib.Path)
    recover.add_argument("--output", required=True, type=pathlib.Path)
    args = parser.parse_args()
    try:
        if args.command == "backup":
            backup(args.server_data, parse_nodes(args.node), args.output)
        else:
            restore(args.backup, args.output)
    except (ArchiveError, OSError, sqlite3.Error) as error:
        print(f"platform data {args.command} failed: {error}", file=sys.stderr)
        return 4
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
