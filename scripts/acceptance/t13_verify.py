#!/usr/bin/env python3
"""Independent T13 installer and data archive acceptance tests."""

from __future__ import annotations

import hashlib
import json
import os
import pathlib
import sqlite3
import sys
import tempfile
import unittest
from unittest import mock

PLATFORM = pathlib.Path(__file__).resolve().parents[1] / "platform"
sys.path.insert(0, str(PLATFORM))

import data_archive as archive  # noqa: E402
import install_bundle as install  # noqa: E402


def sha256(path: pathlib.Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def write_sums(bundle: pathlib.Path) -> None:
    paths = ("manifest.json", *(f"bin/{name}" for name in install.NAMES))
    (bundle / "SHA256SUMS").write_text(
        "".join(f"{sha256(bundle / relative)}  {relative}\n" for relative in paths),
        encoding="utf-8",
    )


def bundle(root: pathlib.Path, digit: str) -> pathlib.Path:
    commit = digit * 40
    target = root / f"bundle-{digit}"
    (target / "bin").mkdir(parents=True)
    files = {}
    info = {
        "git_commit": commit,
        "git_dirty": False,
        "version": "9.9.9",
        "version_long": f"9.9.9 ({commit[:7]})",
        "protocol_version": 4,
        "spa_sha256": "d" * 64,
    }
    for name in install.NAMES:
        script = target / "bin" / name
        script.write_text(
            "#!/usr/bin/env python3\n"
            "import json,sys\n"
            f"INFO={info!r}\n"
            f"print(json.dumps(INFO) if '--build-info' in sys.argv else {name!r})\n",
            encoding="utf-8",
        )
        script.chmod(0o755)
        files[f"bin/{name}"] = {"sha256": sha256(script), "bytes": script.stat().st_size}
    manifest = {
        "schema_version": 1,
        "commit": commit,
        "version": info["version"],
        "version_long": info["version_long"],
        "protocol_version": info["protocol_version"],
        "spa_sha256": info["spa_sha256"],
        "files": files,
    }
    (target / "manifest.json").write_text(json.dumps(manifest), encoding="utf-8")
    write_sums(target)
    return target


def commits(dest: pathlib.Path) -> set[str]:
    return {install.build_info(dest / name)["git_commit"] for name in install.NAMES}


def database(path: pathlib.Path, statements: list[str]) -> None:
    connection = sqlite3.connect(path)
    try:
        for statement in statements:
            connection.execute(statement)
        connection.commit()
    finally:
        connection.close()


def data_fixture(root: pathlib.Path, node_status: str = "done") -> tuple[pathlib.Path, pathlib.Path]:
    server, node = root / "server", root / "node"
    server.mkdir()
    node.mkdir()
    frozen = json.dumps({"version": 1, "mode": "frozen"})
    (server / "admission.json").write_text(frozen, encoding="utf-8")
    (node / "admission.json").write_text(frozen, encoding="utf-8")
    (node / "node.lock").touch()
    (node / "node-id").write_text("verify-node", encoding="utf-8")
    database(
        server / "control.db",
        [
            "CREATE TABLE execution_index(id TEXT PRIMARY KEY,created_at INTEGER,kind TEXT,node_id TEXT,status TEXT)",
            "INSERT INTO execution_index VALUES('a',1,'agent','verify-node','done')",
        ],
    )
    database(server / "definitions.db", ["CREATE TABLE definitions(id TEXT PRIMARY KEY)"])
    database(node / "runtime.db", ["CREATE TABLE sessions(id TEXT PRIMARY KEY)"])
    execution = node / "agent/a"
    (execution / "resources").mkdir(parents=True)
    (execution / "execution.json").write_text(
        json.dumps({"assignment": {"index": {"status": node_status}}}), encoding="utf-8"
    )
    (execution / "resources/answer.txt").write_text("preserved", encoding="utf-8")
    return server, node


class InstallerAcceptance(unittest.TestCase):
    def test_manifest_and_post_switch_failures_preserve_active_generation(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            root = pathlib.Path(raw)
            dest = root / "dest"
            dest.mkdir()
            first, second = bundle(root, "a"), bundle(root, "b")
            install.install_bundle(first, dest, False)
            current = os.readlink(dest / install.CURRENT)

            bad = json.loads((second / "manifest.json").read_text())
            bad.pop("spa_sha256")
            (second / "manifest.json").write_text(json.dumps(bad), encoding="utf-8")
            write_sums(second)
            with self.assertRaises(install.InstallError):
                install.install_bundle(second, dest, False)
            self.assertEqual(os.readlink(dest / install.CURRENT), current)
            self.assertEqual(commits(dest), {"a" * 40})

            second = bundle(root, "c")
            original = install.build_info

            def fail_active(path: pathlib.Path) -> dict:
                if path == dest / "opencoder" and os.readlink(dest / install.CURRENT).endswith("c" * 40):
                    raise install.InstallError("injected installed self-check failure")
                return original(path)

            with mock.patch.object(install, "build_info", side_effect=fail_active):
                with self.assertRaises(install.InstallError):
                    install.install_bundle(second, dest, False)
            self.assertEqual(os.readlink(dest / install.CURRENT), current)
            self.assertEqual(commits(dest), {"a" * 40})

    def test_current_traversal_is_rejected_and_rollback_is_paired(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            root = pathlib.Path(raw)
            dest = root / "dest"
            dest.mkdir()
            first, second = bundle(root, "a"), bundle(root, "b")
            install.install_bundle(first, dest, False)
            old = os.readlink(dest / install.CURRENT)
            (dest / install.CURRENT).unlink()
            os.symlink(f"{install.VERSIONS}/..", dest / install.CURRENT)
            with self.assertRaises(install.InstallError):
                install.install_bundle(second, dest, False)
            self.assertEqual(os.readlink(dest / install.CURRENT), f"{install.VERSIONS}/..")
            (dest / install.CURRENT).unlink()
            os.symlink(old, dest / install.CURRENT)

            rollback = install.install_bundle(second, dest, True)
            self.assertEqual(commits(dest), {"b" * 40})
            install.install_bundle(rollback, dest, False)
            self.assertEqual(commits(dest), {"a" * 40})


class ArchiveAcceptance(unittest.TestCase):
    def test_aliases_cannot_put_output_inside_source(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            root = pathlib.Path(raw)
            server, node = data_fixture(root)
            (root / "alias").mkdir()
            with self.assertRaises(archive.ArchiveError):
                archive.backup(server, {"verify-node": root / "alias/../node"}, node / "inside")
            self.assertFalse((node / "inside").exists())

            backup = root / "backup"
            archive.backup(server, {"verify-node": node}, backup)
            with self.assertRaises(archive.ArchiveError):
                archive.restore(root / "alias/../backup", backup / "inside")
            self.assertFalse((backup / "inside").exists())

    def test_nested_manifest_named_resource_is_hashed_and_tamper_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            root = pathlib.Path(raw)
            server, node = data_fixture(root)
            resource = node / "agent/a/resources/backup-manifest.json"
            resource.write_text("original", encoding="utf-8")
            backup = root / "backup"
            archive.backup(server, {"verify-node": node}, backup)
            relative = "nodes/verify-node/agent/a/resources/backup-manifest.json"
            manifest = json.loads((backup / "backup-manifest.json").read_text())
            self.assertIn(relative, manifest["files"])
            (backup / relative).write_text("tampered", encoding="utf-8")
            with self.assertRaises(archive.ArchiveError):
                archive.restore(backup, root / "restore")
            self.assertFalse((root / "restore").exists())
            self.assertEqual(resource.read_text(), "original")

    def test_busy_or_running_sources_and_existing_restore_are_untouched(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            root = pathlib.Path(raw)
            server, node = data_fixture(root, "running")
            with self.assertRaises(archive.ArchiveError):
                archive.backup(server, {"verify-node": node}, root / "running-backup")
            record = node / "agent/a/execution.json"
            value = json.loads(record.read_text())
            value["assignment"]["index"]["status"] = "done"
            record.write_text(json.dumps(value), encoding="utf-8")
            writer = sqlite3.connect(server / "control.db", isolation_level=None)
            writer.execute("BEGIN IMMEDIATE")
            with self.assertRaises(archive.ArchiveError):
                archive.backup(server, {"verify-node": node}, root / "busy-backup")
            writer.rollback()
            writer.close()

            backup = root / "backup"
            archive.backup(server, {"verify-node": node}, backup)
            existing = root / "existing"
            existing.mkdir()
            (existing / "sentinel").write_text("untouched", encoding="utf-8")
            with self.assertRaises(archive.ArchiveError):
                archive.restore(backup, existing)
            self.assertEqual((existing / "sentinel").read_text(), "untouched")


if __name__ == "__main__":
    unittest.main()
