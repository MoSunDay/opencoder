#!/usr/bin/env python3
"""Verify and atomically activate one three-binary platform bundle."""

from __future__ import annotations

import argparse
import fcntl
import hashlib
import json
import os
import pathlib
import shutil
import subprocess
import sys
import time

NAMES = ("opencoder", "opencoder-server", "opencoder-agent")
EXPECTED_SUMS = {"manifest.json", *(f"bin/{name}" for name in NAMES)}
VERSIONS = ".opencoder-platform-versions"
CURRENT = ".opencoder-platform-current"
MANIFEST_LINK = ".opencoder-platform-manifest.json"


class InstallError(RuntimeError):
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


def regular_file(path: pathlib.Path) -> None:
    if path.is_symlink() or not path.is_file():
        raise InstallError(f"required regular file missing: {path}")


def load_json(path: pathlib.Path) -> dict:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise InstallError(f"invalid {path}: {error}") from error
    if not isinstance(value, dict):
        raise InstallError(f"invalid {path}: expected object")
    return value


def verify_sums(bundle: pathlib.Path) -> None:
    path = bundle / "SHA256SUMS"
    regular_file(path)
    found: dict[str, str] = {}
    for line in path.read_text(encoding="utf-8").splitlines():
        parts = line.split()
        if len(parts) != 2 or len(parts[0]) != 64:
            raise InstallError("invalid SHA256SUMS line")
        relative = parts[1].lstrip("*")
        if relative not in EXPECTED_SUMS or relative in found:
            raise InstallError(f"unexpected SHA256SUMS entry: {relative}")
        found[relative] = parts[0].lower()
    if set(found) != EXPECTED_SUMS:
        raise InstallError("SHA256SUMS must cover manifest and all three binaries")
    for relative, expected in found.items():
        file = bundle / relative
        regular_file(file)
        if sha256(file) != expected:
            raise InstallError(f"checksum mismatch: {relative}")


def build_info(binary: pathlib.Path) -> dict:
    try:
        result = subprocess.run(
            [str(binary), "--build-info"],
            check=True,
            capture_output=True,
            text=True,
            timeout=15,
        )
        info = json.loads(result.stdout)
    except (OSError, subprocess.SubprocessError, json.JSONDecodeError) as error:
        raise InstallError(f"cannot read build metadata from {binary}: {error}") from error
    if not isinstance(info, dict):
        raise InstallError(f"invalid build metadata from {binary}")
    return info


def verify_bundle(bundle: pathlib.Path) -> dict:
    if bundle.is_symlink() or not bundle.is_dir():
        raise InstallError(f"bundle is not a real directory: {bundle}")
    if (bundle / "bin").is_symlink() or not (bundle / "bin").is_dir():
        raise InstallError("bundle bin is not a real directory")
    if {entry.name for entry in bundle.iterdir()} != {"manifest.json", "SHA256SUMS", "bin"}:
        raise InstallError("bundle root contains unexpected members")
    if {entry.name for entry in (bundle / "bin").iterdir()} != set(NAMES):
        raise InstallError("bundle bin must contain exactly the three platform binaries")
    verify_sums(bundle)
    manifest = load_json(bundle / "manifest.json")
    required = {
        "schema_version",
        "commit",
        "version",
        "version_long",
        "protocol_version",
        "spa_sha256",
        "files",
    }
    if not required.issubset(manifest) or manifest["schema_version"] != 1:
        raise InstallError("unsupported platform manifest")
    protocol = manifest["protocol_version"]
    if not isinstance(protocol, int) or protocol <= 0:
        raise InstallError("invalid protocol_version")
    if any(not isinstance(manifest[key], str) or not manifest[key] for key in ("version", "version_long")):
        raise InstallError("manifest version fields must be non-empty strings")
    commit = manifest["commit"]
    if not isinstance(commit, str) or len(commit) != 40 or any(c not in "0123456789abcdef" for c in commit):
        raise InstallError("manifest commit must be a full hexadecimal hash")
    spa = manifest["spa_sha256"]
    if not isinstance(spa, str) or len(spa) != 64 or any(c not in "0123456789abcdef" for c in spa):
        raise InstallError("invalid spa_sha256")
    files = manifest["files"]
    expected_files = {f"bin/{name}" for name in NAMES}
    if not isinstance(files, dict) or set(files) != expected_files:
        raise InstallError("manifest files must contain exactly the three platform binaries")
    infos = []
    for name in NAMES:
        relative = f"bin/{name}"
        binary = bundle / relative
        regular_file(binary)
        entry = files[relative]
        if not isinstance(entry, dict) or entry.get("sha256") != sha256(binary):
            raise InstallError(f"manifest checksum mismatch: {relative}")
        byte_count = entry.get("bytes")
        if not isinstance(byte_count, int) or isinstance(byte_count, bool) or byte_count != binary.stat().st_size:
            raise InstallError(f"manifest byte count mismatch: {relative}")
        infos.append(build_info(binary))
    if infos[1:] != infos[:-1]:
        raise InstallError("platform binary build metadata differs")
    info = infos[0]
    expected = {
        "git_commit": commit,
        "git_dirty": False,
        "version": manifest["version"],
        "version_long": manifest["version_long"],
        "protocol_version": protocol,
        "spa_sha256": spa,
    }
    if any(info.get(key) != value for key, value in expected.items()):
        raise InstallError("binary build metadata does not match manifest")
    return manifest


def copy_durable(source: pathlib.Path, target: pathlib.Path, mode: int) -> None:
    with source.open("rb") as reader, target.open("xb") as writer:
        shutil.copyfileobj(reader, writer)
        writer.flush()
        os.fchmod(writer.fileno(), mode)
        os.fsync(writer.fileno())


def relative_current(version_name: str) -> str:
    return f"{VERSIONS}/{version_name}"


def replace_symlink(directory: pathlib.Path, name: str, target: str) -> None:
    temporary = directory / f".{name}.new.{os.getpid()}.{time.time_ns()}"
    try:
        os.symlink(target, temporary)
        os.replace(temporary, directory / name)
        fsync_dir(directory)
    except BaseException:
        if temporary.is_symlink() and os.readlink(temporary) == target:
            temporary.unlink()
        raise


def stage_bundle(bundle: pathlib.Path, versions: pathlib.Path, commit: str) -> pathlib.Path:
    target = versions / commit
    if target.exists():
        verify_bundle(target)
        if sha256(target / "manifest.json") != sha256(bundle / "manifest.json"):
            raise InstallError(f"version directory conflicts with bundle: {target}")
        return target
    stage = versions / f".stage-{commit}-{os.getpid()}"
    if stage.exists() or stage.is_symlink():
        raise InstallError(f"staging path already exists: {stage}")
    try:
        stage.mkdir()
        (stage / "bin").mkdir()
        for name in NAMES:
            copy_durable(bundle / "bin" / name, stage / "bin" / name, 0o755)
        copy_durable(bundle / "manifest.json", stage / "manifest.json", 0o644)
        copy_durable(bundle / "SHA256SUMS", stage / "SHA256SUMS", 0o644)
        fsync_dir(stage / "bin")
        fsync_dir(stage)
        verify_bundle(stage)
        os.replace(stage, target)
        fsync_dir(versions)
        return target
    except BaseException:
        shutil.rmtree(stage, ignore_errors=True)
        raise


def expected_launcher(name: str) -> str:
    return f"{CURRENT}/bin/{name}"


def valid_version_name(name: str) -> bool:
    commit = len(name) == 40 and all(character in "0123456789abcdef" for character in name)
    legacy = name.startswith("legacy-") and name.removeprefix("legacy-").isdigit()
    return commit or legacy


def inspect_current(dest: pathlib.Path) -> str | None:
    current = dest / CURRENT
    if not current.exists() and not current.is_symlink():
        return None
    if not current.is_symlink():
        raise InstallError(f"platform current pointer is not a symlink: {current}")
    target = os.readlink(current)
    parts = pathlib.PurePath(target).parts
    if (
        len(parts) != 2
        or parts[0] != VERSIONS
        or not valid_version_name(parts[1])
        or target != relative_current(parts[1])
    ):
        raise InstallError(f"platform current pointer escapes version store: {target}")
    resolved = dest / target
    if not resolved.is_dir() or resolved.is_symlink():
        raise InstallError(f"platform current target is invalid: {resolved}")
    return target


def legacy_version(dest: pathlib.Path, versions: pathlib.Path) -> str | None:
    present = [name for name in NAMES if (dest / name).exists()]
    if not present:
        return None
    for name in present:
        regular_file(dest / name)
    if len(present) > 1:
        infos = [build_info(dest / name) for name in present]
        if infos[1:] != infos[:-1]:
            raise InstallError("existing platform binaries have mixed build metadata")
    name = f"legacy-{time.time_ns()}"
    stage = versions / f".stage-{name}-{os.getpid()}"
    try:
        stage.mkdir()
        (stage / "bin").mkdir()
        for binary in present:
            copy_durable(dest / binary, stage / "bin" / binary, 0o755)
        fsync_dir(stage / "bin")
        fsync_dir(stage)
        os.replace(stage, versions / name)
        fsync_dir(versions)
    except BaseException:
        shutil.rmtree(stage, ignore_errors=True)
        raise
    return relative_current(name)


def prepare_launchers(dest: pathlib.Path, versions: pathlib.Path, current: str | None) -> None:
    for name in NAMES:
        path = dest / name
        if path.is_symlink() and os.readlink(path) != expected_launcher(name):
            raise InstallError(f"unexpected platform launcher symlink: {path}")
        if path.exists() and not path.is_symlink() and not path.is_file():
            raise InstallError(f"platform launcher is not a regular file: {path}")
    manifest = dest / MANIFEST_LINK
    if manifest.exists() or manifest.is_symlink():
        if not manifest.is_symlink() or os.readlink(manifest) != f"{CURRENT}/manifest.json":
            raise InstallError(f"unexpected installed manifest path: {manifest}")
    if current is None:
        current = legacy_version(dest, versions)
        if current is not None:
            replace_symlink(dest, CURRENT, current)
    elif (dest / current / "bin").is_dir():
        for name in NAMES:
            path = dest / name
            previous = dest / current / "bin" / name
            if path.exists() and not path.is_symlink():
                if not previous.is_file() or sha256(path) != sha256(previous):
                    raise InstallError(f"existing launcher differs from current platform: {path}")
    for name in NAMES:
        path = dest / name
        if not path.is_symlink():
            replace_symlink(dest, name, expected_launcher(name))
    if not manifest.is_symlink():
        replace_symlink(dest, MANIFEST_LINK, f"{CURRENT}/manifest.json")


def export_rollback(dest: pathlib.Path, current: str | None) -> pathlib.Path | None:
    if current is None:
        return None
    source = dest / current
    try:
        manifest = verify_bundle(source)
    except InstallError:
        return None
    root = dest / ".opencoder-platform-rollbacks"
    if root.is_symlink() or (root.exists() and not root.is_dir()):
        raise InstallError(f"rollback store is not a real directory: {root}")
    root.mkdir(exist_ok=True)
    fsync_dir(dest)
    target = root / f"{time.time_ns()}-{str(manifest['commit'])[:12]}"
    stage = root / f".rollback-{os.getpid()}-{time.time_ns()}"
    try:
        stage.mkdir()
        (stage / "bin").mkdir()
        for name in NAMES:
            copy_durable(source / "bin" / name, stage / "bin" / name, 0o755)
        copy_durable(source / "manifest.json", stage / "manifest.json", 0o644)
        copy_durable(source / "SHA256SUMS", stage / "SHA256SUMS", 0o644)
        fsync_dir(stage / "bin")
        fsync_dir(stage)
        verify_bundle(stage)
        os.replace(stage, target)
        fsync_dir(root)
        return target
    except BaseException:
        shutil.rmtree(stage, ignore_errors=True)
        raise


def install_bundle(
    bundle: pathlib.Path,
    dest: pathlib.Path,
    keep_backup: bool,
    fail_before_switch: bool = False,
) -> pathlib.Path | None:
    manifest = verify_bundle(bundle)
    if dest.is_symlink() or not dest.is_dir():
        raise InstallError(f"destination is not a real directory: {dest}")
    lock_path = dest / ".opencoder-platform-install.lock"
    if lock_path.is_symlink():
        raise InstallError(f"install lock cannot be a symlink: {lock_path}")
    with lock_path.open("a+b") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        versions = dest / VERSIONS
        if versions.is_symlink():
            raise InstallError(f"version store cannot be a symlink: {versions}")
        versions.mkdir(exist_ok=True)
        fsync_dir(dest)
        current = inspect_current(dest)
        target = stage_bundle(bundle, versions, manifest["commit"])
        prepare_launchers(dest, versions, current)
        rollback = export_rollback(dest, current) if keep_backup else None
        if fail_before_switch:
            raise InstallError("injected failure before atomic version switch")
        previous = inspect_current(dest)
        replace_symlink(dest, CURRENT, relative_current(target.name))
        try:
            installed = verify_bundle(target)
            if installed != manifest:
                raise InstallError("installed manifest changed after activation")
            for name in NAMES:
                if build_info(dest / name).get("git_commit") != manifest["commit"]:
                    raise InstallError(f"installed self-check failed: {name}")
        except BaseException:
            if previous is not None:
                replace_symlink(dest, CURRENT, previous)
            elif (dest / CURRENT).is_symlink():
                (dest / CURRENT).unlink()
                fsync_dir(dest)
            raise
        return rollback


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--bundle", required=True, type=pathlib.Path)
    parser.add_argument("--dest-dir", required=True, type=pathlib.Path)
    parser.add_argument("--backup", action="store_true")
    args = parser.parse_args()
    try:
        rollback = install_bundle(args.bundle, args.dest_dir, args.backup)
    except (InstallError, OSError) as error:
        print(f"platform install failed: {error}", file=sys.stderr)
        return 4
    print(f"installed platform bundle {args.bundle} -> {args.dest_dir}")
    if rollback:
        print(f"rollback bundle: {rollback}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
