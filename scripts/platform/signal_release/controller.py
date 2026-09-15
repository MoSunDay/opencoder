"""Install a content-addressed controller and stage a verified release."""
import hashlib
import contextlib
import fcntl
import json
import os
from pathlib import Path
import sys
from rolling import manifest
from rolling.state import Journal, atomic_bytes, write
from rolling.units import argument


@contextlib.contextmanager
def request_lock(settings):
    settings.state_dir.mkdir(parents=True, exist_ok=True)
    with (settings.state_dir / "signal-trigger.lock").open("a+") as stream:
        fcntl.flock(stream, fcntl.LOCK_EX | fcntl.LOCK_NB)
        yield


def prefix(settings):
    digest = hashlib.sha256(str(settings.state_dir).encode()).hexdigest()[:16]
    return "opencoder-release-" + digest


def install(settings, config_path, operations):
    source = Path(__file__).resolve().parents[1]
    files = sorted([*source.glob("rolling/*.py"), *source.glob("signal_release/*.py"),
                    source / "install_bundle.py", source / "rolling_cli.py"])
    contents = {str(path.relative_to(source)): path.read_bytes() for path in files}
    digest = hashlib.sha256()
    for name, content in sorted(contents.items()):
        digest.update(name.encode() + b"\0" + content + b"\0")
    target = settings.state_dir / "controllers" / digest.hexdigest()
    for name, content in contents.items():
        path = target / name
        if path.exists():
            if path.read_bytes() != content:
                raise ValueError("installed release controller has been modified")
        else:
            atomic_bytes(path, content, 0o644)
    (settings.state_dir / "signal-receipts").mkdir(exist_ok=True)
    # Persist the new directory entries as well as their individual files
    # before publishing a unit that depends on this controller snapshot.
    for directory in (target.parent, settings.state_dir):
        fd = os.open(directory, os.O_RDONLY | os.O_DIRECTORY)
        try:
            os.fsync(fd)
        finally:
            os.close(fd)
    command = " ".join(argument(value) for value in [sys.executable, "-m",
        "signal_release.runner", "--config", config_path.resolve(), "--instance"])
    unit = f"""[Unit]
Description=OpenCoder signal release %i
After=network-online.target

[Service]
Type=oneshot
User=root
WorkingDirectory={str(target).replace('%', '%%')}
ExecStart={command} "%i"
TimeoutStartSec=infinity
TimeoutStopSec=infinity
SendSIGKILL=no
Restart=no
"""
    path = settings.systemd_dir / (prefix(settings) + "@.service")
    atomic_bytes(path, unit.encode(), 0o644)
    operations.run("systemd-analyze", "verify", str(path))
    operations.run("systemctl", "daemon-reload")
    write(settings.state_dir / "host/deployment.json", {
        "state_dir": str(settings.state_dir), "unit_prefix": prefix(settings)})
    return {"unit_prefix": prefix(settings), "controller": str(target)}


def stage(settings, bundle):
    candidate = manifest.verify(bundle)
    journal = Journal(settings.state_dir).data
    if not journal["current"]:
        raise ValueError("first migration is required before staging a signal release")
    if journal["candidate"] not in (None, candidate["release_id"]):
        raise ValueError("another release is unfinished; resume it or roll it back first")
    manifest.compatible(candidate, [r["manifest"] for r in journal["releases"].values()])
    manifest.resources(settings, candidate)
    versions = settings.state_dir / "staged"
    versions.mkdir(parents=True, exist_ok=True)
    installed = manifest._installer.stage_bundle(bundle, versions, candidate)
    intent = {"bundle": str(installed), "release_id": candidate["release_id"]}
    write(settings.state_dir / "signal-pending.json", intent)
    return intent


def status(settings):
    pending = settings.state_dir / "signal-pending.json"
    receipts = settings.state_dir / "signal-receipts"
    return {"pending": json.loads(pending.read_text()) if pending.exists() else None,
        "receipts": [json.loads(path.read_text()) for path in sorted(receipts.glob("*.json"))]}
