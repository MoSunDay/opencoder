#!/usr/bin/env python3
"""Hermetic contract tests for the platform bundle installer."""

from __future__ import annotations

import hashlib
import json
import os
import pathlib
import tempfile
import unittest

import install_bundle as subject


def binary_bytes(name: str, info: dict) -> bytes:
    payload = json.dumps(info, separators=(",", ":"))
    return (
        "#!/usr/bin/env python3\n"
        "import sys\n"
        f"INFO={payload!r}\n"
        f"print(INFO if '--build-info' in sys.argv else {name!r})\n"
    ).encode()


def make_bundle(
    root: pathlib.Path,
    commit_char: str,
    mismatch: str | None = None,
    names: tuple[str, ...] | None = None,
) -> pathlib.Path:
    binaries = subject.NAMES if names is None else names
    commit = commit_char * 40
    bundle = root / f"bundle-{commit_char}"
    (bundle / "bin").mkdir(parents=True)
    files = {}
    for name in binaries:
        binary_commit = (mismatch * 40) if mismatch == name else commit
        info = {
            "version": "1.2.3",
            "version_long": f"1.2.3 ({binary_commit[:7]})",
            "git_commit": binary_commit,
            "git_dirty": False,
            "protocol_version": 4,
            "spa_sha256": "d" * 64,
        }
        path = bundle / "bin" / name
        path.write_bytes(binary_bytes(name, info))
        path.chmod(0o755)
        data = path.read_bytes()
        files[f"bin/{name}"] = {
            "sha256": hashlib.sha256(data).hexdigest(),
            "bytes": len(data),
        }
    manifest = {
        "schema_version": 1,
        "commit": commit,
        "version": "1.2.3",
        "version_long": f"1.2.3 ({commit[:7]})",
        "protocol_version": 4,
        "spa_sha256": "d" * 64,
        "files": files,
    }
    (bundle / "manifest.json").write_text(json.dumps(manifest), encoding="utf-8")
    lines = []
    for relative in ("manifest.json", *(f"bin/{name}" for name in binaries)):
        digest = hashlib.sha256((bundle / relative).read_bytes()).hexdigest()
        lines.append(f"{digest}  {relative}\n")
    (bundle / "SHA256SUMS").write_text("".join(lines), encoding="utf-8")
    return bundle


def installed_commits(dest: pathlib.Path, names: tuple[str, ...] | None = None) -> set[str]:
    binaries = subject.NAMES if names is None else names
    return {subject.build_info(dest / name)["git_commit"] for name in binaries}


class PlatformInstallTests(unittest.TestCase):
    def test_atomic_switch_failure_and_paired_rollback(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            root = pathlib.Path(raw)
            dest = root / "dest"
            dest.mkdir()
            first = make_bundle(root, "a")
            second = make_bundle(root, "b")
            subject.install_bundle(first, dest, False)
            self.assertEqual(installed_commits(dest), {"a" * 40})

            with self.assertRaises(subject.InstallError):
                subject.install_bundle(second, dest, False, fail_before_switch=True)
            self.assertEqual(installed_commits(dest), {"a" * 40})

            rollback = subject.install_bundle(second, dest, True)
            self.assertIsNotNone(rollback)
            self.assertEqual(installed_commits(dest), {"b" * 40})
            subject.install_bundle(rollback, dest, False)
            self.assertEqual(installed_commits(dest), {"a" * 40})
            self.assertEqual(
                {path.readlink().parts[0] for path in (dest / name for name in subject.NAMES)},
                {subject.CURRENT},
            )

    def test_bad_bundle_never_touches_installed_generation(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            root = pathlib.Path(raw)
            dest = root / "dest"
            dest.mkdir()
            first = make_bundle(root, "a")
            bad = make_bundle(root, "b")
            subject.install_bundle(first, dest, False)
            before = (dest / subject.CURRENT).readlink()
            (bad / "bin" / "opencoder-agent").write_text("tampered", encoding="utf-8")
            with self.assertRaises(subject.InstallError):
                subject.install_bundle(bad, dest, False)
            self.assertEqual((dest / subject.CURRENT).readlink(), before)
            self.assertEqual(installed_commits(dest), {"a" * 40})

    def test_mixed_build_and_unexpected_member_are_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            root = pathlib.Path(raw)
            dest = root / "dest"
            dest.mkdir()
            mixed = make_bundle(root, "a", mismatch="opencoder-agent")
            with self.assertRaises(subject.InstallError):
                subject.install_bundle(mixed, dest, False)
            self.assertEqual(list(dest.iterdir()), [])

            valid = make_bundle(root, "b")
            (valid / "bin" / "extra").write_text("x", encoding="utf-8")
            with self.assertRaises(subject.InstallError):
                subject.install_bundle(valid, dest, False)
            self.assertEqual(list(dest.iterdir()), [])

    def test_single_legacy_binary_stays_active_until_switch(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            root = pathlib.Path(raw)
            dest = root / "dest"
            dest.mkdir()
            legacy = dest / "opencoder"
            legacy.write_text("#!/bin/sh\necho legacy\n", encoding="utf-8")
            legacy.chmod(0o755)
            bundle = make_bundle(root, "a")
            with self.assertRaises(subject.InstallError):
                subject.install_bundle(bundle, dest, False, fail_before_switch=True)
            self.assertEqual(legacy.read_text(encoding="utf-8"), "#!/bin/sh\necho legacy\n")
            self.assertEqual(legacy.resolve().parent.parent.name.startswith("legacy-"), True)
            subject.install_bundle(bundle, dest, False)
            self.assertEqual(installed_commits(dest), {"a" * 40})

    def test_malformed_current_targets_are_rejected_without_external_access(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            root = pathlib.Path(raw)
            dest = root / "dest"
            dest.mkdir()
            first = make_bundle(root, "a")
            second = make_bundle(root, "b")
            subject.install_bundle(first, dest, False)
            old_manifest = (dest / subject.VERSIONS / ("a" * 40) / "manifest.json").read_bytes()
            outside = root / "outside"
            outside.mkdir()
            marker = outside / "marker"
            marker.write_text("untouched", encoding="utf-8")

            malformed = (
                f"{subject.VERSIONS}/..",
                f"{subject.VERSIONS}/.",
                f"{subject.VERSIONS}/{'a' * 39}",
                f"{subject.VERSIONS}/{'A' * 40}",
                f"{subject.VERSIONS}/{'a' * 40}/extra",
                f"{subject.VERSIONS}//{'a' * 40}",
            )
            for target in malformed:
                (dest / subject.CURRENT).unlink()
                os.symlink(target, dest / subject.CURRENT)
                with self.assertRaises(subject.InstallError, msg=target):
                    subject.install_bundle(second, dest, False)
                self.assertEqual(os.readlink(dest / subject.CURRENT), target)
                self.assertEqual(
                    (dest / subject.VERSIONS / ("a" * 40) / "manifest.json").read_bytes(),
                    old_manifest,
                )
                self.assertEqual(marker.read_text(encoding="utf-8"), "untouched")

    def test_two_binary_bundle_installs_and_launches(self) -> None:
        names = ("opencoder", "opencoder-server")
        with tempfile.TemporaryDirectory() as raw:
            root = pathlib.Path(raw)
            dest = root / "dest"
            dest.mkdir()
            bundle = make_bundle(root, "a", names=names)
            subject.install_bundle(bundle, dest, False)
            for name in names:
                self.assertTrue((dest / name).is_symlink())
                self.assertEqual(os.readlink(dest / name), f"{subject.CURRENT}/bin/{name}")
            self.assertFalse((dest / "opencoder-agent").exists())
            self.assertFalse((dest / "opencoder-agent").is_symlink())
            self.assertEqual(installed_commits(dest, names), {"a" * 40})

    def test_manifest_declared_set_is_enforced(self) -> None:
        names = ("opencoder", "opencoder-server")
        with tempfile.TemporaryDirectory() as raw:
            root = pathlib.Path(raw)
            dest = root / "dest"
            dest.mkdir()

            undeclared = make_bundle(root, "a", names=names)
            (undeclared / "bin" / "opencoder-agent").write_text("extra", encoding="utf-8")
            with self.assertRaises(subject.InstallError):
                subject.install_bundle(undeclared, dest, False)
            self.assertEqual(list(dest.iterdir()), [])

            missing = make_bundle(root, "b", names=names)
            kept = "".join(
                line
                for line in (missing / "SHA256SUMS").read_text(encoding="utf-8").splitlines(keepends=True)
                if "bin/opencoder-server" not in line
            )
            (missing / "SHA256SUMS").write_text(kept, encoding="utf-8")
            with self.assertRaises(subject.InstallError):
                subject.install_bundle(missing, dest, False)
            self.assertEqual(list(dest.iterdir()), [])

    def test_shrunk_binary_set_removes_dangling_launcher(self) -> None:
        names = ("opencoder", "opencoder-server")
        with tempfile.TemporaryDirectory() as raw:
            root = pathlib.Path(raw)
            dest = root / "dest"
            dest.mkdir()
            subject.install_bundle(make_bundle(root, "a"), dest, False)
            self.assertTrue((dest / "opencoder-agent").is_symlink())
            subject.install_bundle(make_bundle(root, "b", names=names), dest, False)
            self.assertFalse((dest / "opencoder-agent").is_symlink())
            self.assertFalse((dest / "opencoder-agent").exists())
            for name in names:
                self.assertEqual(os.readlink(dest / name), f"{subject.CURRENT}/bin/{name}")
            self.assertEqual(installed_commits(dest, names), {"b" * 40})


if __name__ == "__main__":
    unittest.main()
