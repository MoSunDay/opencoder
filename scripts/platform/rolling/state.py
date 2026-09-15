"""Fsync-backed release journal; all switch operations are replayable."""
import contextlib
import fcntl
import json
import os
from pathlib import Path
import tempfile

PHASES = ("validated", "warming", "ready", "switching", "verifying", "complete")


def atomic_bytes(path, data, mode=0o600):
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    fd, temporary = tempfile.mkstemp(prefix=f".{path.name}.", dir=path.parent)
    try:
        with os.fdopen(fd, "wb") as stream:
            os.fchmod(stream.fileno(), mode)
            stream.write(data)
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary, path)
        directory = os.open(path.parent, os.O_RDONLY | os.O_DIRECTORY)
        try:
            os.fsync(directory)
        finally:
            os.close(directory)
    finally:
        if os.path.exists(temporary):
            os.unlink(temporary)


def write(path, value):
    atomic_bytes(path, (json.dumps(value, indent=2, sort_keys=True) + "\n").encode())


@contextlib.contextmanager
def locked(root):
    root.mkdir(parents=True, exist_ok=True)
    with (root / "deploy.lock").open("a+") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        yield


class Journal:
    def __init__(self, root):
        self.path = root / "release-state.json"
        self.data = json.loads(self.path.read_text()) if self.path.exists() else {
            "schema_version": 1, "current": None, "previous": None,
            "candidate": None, "phase": "unmigrated", "releases": {},
        }
        if self.data["schema_version"] != 1:
            raise ValueError("unsupported release journal")

    def save(self):
        # The business Server uses its existing service account to read status.
        # This journal contains release metadata, never credentials or inputs.
        atomic_bytes(self.path, (json.dumps(self.data, indent=2, sort_keys=True) + "\n").encode(), 0o644)

    def phase(self, phase):
        if phase not in PHASES and phase not in ("failed", "rolling_back", "rolled_back", "migrating"):
            raise ValueError(f"unknown release phase: {phase}")
        self.data["phase"] = phase
        self.save()

    def record(self, release_id):
        return self.data["releases"][release_id]

    def fail(self, error):
        self.data["failure"] = str(error)
        self.save()
