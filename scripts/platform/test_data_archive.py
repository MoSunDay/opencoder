#!/usr/bin/env python3
"""Hermetic platform data backup/restore contract tests."""

from __future__ import annotations

import fcntl
import json
import pathlib
import sqlite3
import tempfile
import unittest

import data_archive as subject


def database(path: pathlib.Path, statements: list[str]) -> None:
    connection = sqlite3.connect(path)
    try:
        for statement in statements:
            connection.execute(statement)
        connection.commit()
    finally:
        connection.close()


def fixture(root: pathlib.Path, status: str = "done") -> tuple[pathlib.Path, pathlib.Path]:
    server = root / "server"
    node = root / "node"
    server.mkdir()
    node.mkdir()
    frozen = json.dumps({"version": 1, "mode": "frozen"})
    (server / "admission.json").write_text(frozen, encoding="utf-8")
    database(
        server / "control.db",
        [
            "CREATE TABLE execution_index(id TEXT PRIMARY KEY,created_at INTEGER NOT NULL,kind TEXT NOT NULL,node_id TEXT NOT NULL,status TEXT NOT NULL)",
            f"INSERT INTO execution_index VALUES('agent-a',1,'agent','node-a','{status}')",
        ],
    )
    database(server / "definitions.db", ["CREATE TABLE definitions(id TEXT PRIMARY KEY)"])
    (node / "node.lock").touch()
    (node / "admission.json").write_text(frozen, encoding="utf-8")
    (node / "node-id").write_text("node-a", encoding="utf-8")
    database(node / "runtime.db", ["CREATE TABLE sessions(id TEXT PRIMARY KEY)"])
    execution = node / "agent/agent-a"
    execution.mkdir(parents=True)
    (execution / "execution.json").write_text(
        json.dumps({"assignment": {"index": {"status": status}}}), encoding="utf-8"
    )
    resources = execution / "resources"
    resources.mkdir()
    (resources / "answer.txt").write_text("kept", encoding="utf-8")
    return server, node


class DataArchiveTests(unittest.TestCase):
    def test_backup_and_isolated_restore_preserve_all_sources(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            root = pathlib.Path(raw)
            server, node = fixture(root)
            output = root / "backup"
            subject.backup(server, {"node-a": node}, output)
            subject.verify_archive(output)
            restored = root / "restored"
            subject.restore(output, restored)
            subject.verify_archive(restored)
            self.assertEqual(
                (restored / "nodes/node-a/agent/agent-a/resources/answer.txt").read_text(),
                "kept",
            )
            self.assertEqual((node / "node-id").read_text(), "node-a")
            self.assertTrue((server / "control.db").is_file())

    def test_live_node_and_active_index_fail_without_partial_backup(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            root = pathlib.Path(raw)
            server, node = fixture(root)
            lock = (node / "node.lock").open("rb")
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            with self.assertRaises(subject.ArchiveError):
                subject.backup(server, {"node-a": node}, root / "live-backup")
            self.assertFalse((root / "live-backup").exists())
            lock.close()

            writer = sqlite3.connect(server / "control.db", isolation_level=None)
            writer.execute("BEGIN IMMEDIATE")
            with self.assertRaises(subject.ArchiveError):
                subject.backup(server, {"node-a": node}, root / "busy-backup")
            self.assertFalse((root / "busy-backup").exists())
            writer.execute("ROLLBACK")
            writer.close()

            connection = sqlite3.connect(server / "control.db")
            connection.execute("UPDATE execution_index SET status='idle'")
            connection.commit()
            connection.close()
            record = node / "agent/agent-a/execution.json"
            value = json.loads(record.read_text())
            value["assignment"]["index"]["status"] = "interrupted"
            record.write_text(json.dumps(value), encoding="utf-8")
            with self.assertRaises(subject.ArchiveError):
                subject.backup(server, {"node-a": node}, root / "active-backup")
            self.assertFalse((root / "active-backup").exists())

            connection = sqlite3.connect(server / "control.db")
            connection.execute("UPDATE execution_index SET status='corrupt'")
            connection.commit()
            connection.close()
            with self.assertRaises(subject.ArchiveError):
                subject.backup(server, {"node-a": node}, root / "corrupt-backup")
            self.assertFalse((root / "corrupt-backup").exists())

    def test_running_journal_and_tampered_archive_are_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            root = pathlib.Path(raw)
            server, node = fixture(root, "running")
            connection = sqlite3.connect(server / "control.db")
            connection.execute("UPDATE execution_index SET status='done'")
            connection.commit()
            connection.close()
            with self.assertRaises(subject.ArchiveError):
                subject.backup(server, {"node-a": node}, root / "bad")

            value = json.loads((node / "agent/agent-a/execution.json").read_text())
            value["assignment"]["index"]["status"] = "interrupted"
            (node / "agent/agent-a/execution.json").write_text(json.dumps(value))
            backup = root / "backup"
            subject.backup(server, {"node-a": node}, backup)
            (backup / "nodes/node-a/node-id").write_text("tampered")
            with self.assertRaises(subject.ArchiveError):
                subject.restore(backup, root / "restore")
            self.assertFalse((root / "restore").exists())

    def test_untrusted_node_inventory_cannot_escape_backup(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            root = pathlib.Path(raw)
            server, node = fixture(root)
            backup = root / "backup"
            subject.backup(server, {"node-a": node}, backup)
            manifest = backup / "backup-manifest.json"
            value = json.loads(manifest.read_text())
            value["nodes"] = ["../outside"]
            manifest.write_text(json.dumps(value))
            with self.assertRaises(subject.ArchiveError):
                subject.restore(backup, root / "restore")
            self.assertFalse((root / "restore").exists())

    def test_restore_never_overwrites_existing_output(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            root = pathlib.Path(raw)
            server, node = fixture(root)
            backup = root / "backup"
            subject.backup(server, {"node-a": node}, backup)
            output = root / "existing"
            output.mkdir()
            sentinel = output / "keep"
            sentinel.write_text("original")
            with self.assertRaises(subject.ArchiveError):
                subject.restore(backup, output)
            self.assertEqual(sentinel.read_text(), "original")

    def test_alias_paths_cannot_place_output_inside_source(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            root = pathlib.Path(raw)
            server, node = fixture(root)
            alias = root / "alias"
            alias.mkdir()
            node_output = node / "nested-backup"
            with self.assertRaises(subject.ArchiveError):
                subject.backup(server, {"node-a": alias / ".." / "node"}, node_output)
            self.assertFalse(node_output.exists())
            self.assertEqual(list(node.glob(".nested-backup.tmp-*")), [])
            self.assertEqual((node / "node-id").read_text(), "node-a")

            backup = root / "backup"
            subject.backup(server, {"node-a": node}, backup)
            restore_output = backup / "nested-restore"
            with self.assertRaises(subject.ArchiveError):
                subject.restore(alias / ".." / "backup", restore_output)
            self.assertFalse(restore_output.exists())
            self.assertTrue((backup / "backup-manifest.json").is_file())

    def test_nested_manifest_name_is_hashed_and_tampering_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            root = pathlib.Path(raw)
            server, node = fixture(root)
            nested = node / "agent/agent-a/resources/backup-manifest.json"
            nested.write_text("original", encoding="utf-8")
            backup = root / "backup"
            subject.backup(server, {"node-a": node}, backup)
            relative = "nodes/node-a/agent/agent-a/resources/backup-manifest.json"
            manifest = json.loads((backup / "backup-manifest.json").read_text())
            self.assertIn(relative, manifest["files"])
            (backup / relative).write_text("tampered-untracked", encoding="utf-8")
            with self.assertRaises(subject.ArchiveError):
                subject.restore(backup, root / "restore")
            self.assertFalse((root / "restore").exists())
            self.assertEqual(nested.read_text(), "original")

    def test_database_permissions_survive_backup_and_restore(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            root = pathlib.Path(raw)
            server, node = fixture(root)
            sources = [server / "control.db", server / "definitions.db", node / "runtime.db"]
            for path in sources:
                path.chmod(0o600)
            backup = root / "backup"
            subject.backup(server, {"node-a": node}, backup)
            archived = [
                backup / "server/control.db",
                backup / "server/definitions.db",
                backup / "nodes/node-a/runtime.db",
            ]
            self.assertEqual([path.stat().st_mode & 0o777 for path in archived], [0o600] * 3)
            restored = root / "restored"
            subject.restore(backup, restored)
            restored_databases = [
                restored / "server/control.db",
                restored / "server/definitions.db",
                restored / "nodes/node-a/runtime.db",
            ]
            self.assertEqual(
                [path.stat().st_mode & 0o777 for path in restored_databases], [0o600] * 3
            )

    def test_wal_mode_archive_verification_has_no_side_effect_files(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            root = pathlib.Path(raw)
            server, node = fixture(root)
            for path in [server / "control.db", server / "definitions.db", node / "runtime.db"]:
                connection = sqlite3.connect(path)
                self.assertEqual(connection.execute("PRAGMA journal_mode=WAL").fetchone(), ("wal",))
                connection.close()
            backup = root / "backup"
            subject.backup(server, {"node-a": node}, backup)
            before = sorted(path.relative_to(backup).as_posix() for path in backup.rglob("*"))
            subject.verify_archive(backup)
            subject.verify_archive(backup)
            after = sorted(path.relative_to(backup).as_posix() for path in backup.rglob("*"))
            self.assertEqual(after, before)
            self.assertFalse(any(path.endswith(("-wal", "-shm")) for path in after))
            restored = root / "restored"
            subject.restore(backup, restored)
            subject.verify_archive(restored)

    def test_database_uri_paths_are_encoded_for_backup_and_restore(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            root = pathlib.Path(raw) / "space ? hash# percent%"
            root.mkdir()
            server, node = fixture(root)
            backup = root / "encoded backup"
            subject.backup(server, {"node-a": node}, backup)
            subject.verify_archive(backup)
            restored = root / "encoded restore"
            subject.restore(backup, restored)
            subject.verify_archive(restored)
            self.assertTrue((restored / "server/control.db").is_file())
            self.assertTrue((restored / "nodes/node-a/runtime.db").is_file())


if __name__ == "__main__":
    unittest.main()
