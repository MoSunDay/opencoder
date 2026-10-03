"""OS file locks: released on process exit, shared across CLI invocations."""

import hashlib
import os
from contextlib import contextmanager
from pathlib import Path
from urllib.parse import urlsplit, urlunsplit


def target_lock_path(endpoint: str, root: Path) -> Path:
    parts = urlsplit(endpoint)
    host = (parts.hostname or "").lower()
    host = "localhost" if host in {"localhost", "127.0.0.1", "::1"} else host
    port = parts.port or (443 if parts.scheme == "https" else 80)
    canonical = urlunsplit((parts.scheme, f"{host}:{port}", parts.path.rstrip("/"), "", ""))
    digest = hashlib.sha256(canonical.encode()).hexdigest()
    return root / f"{digest}.lock"


@contextmanager
def exclusive_lock(path: Path):
    path.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
    descriptor = os.open(path, os.O_RDWR | os.O_CREAT, 0o600)
    with os.fdopen(descriptor, "r+b") as handle:
        locked = False
        try:
            if os.name == "nt":
                import msvcrt

                if path.stat().st_size == 0:
                    handle.write(b"0")
                    handle.flush()
                handle.seek(0)
                msvcrt.locking(handle.fileno(), msvcrt.LK_NBLCK, 1)
            else:
                import fcntl

                fcntl.flock(handle, fcntl.LOCK_EX | fcntl.LOCK_NB)
            locked = True
            yield
        except (BlockingIOError, PermissionError) as error:
            if not locked:
                raise RuntimeError("target or run is already in use") from error
            raise
        finally:
            if locked:
                if os.name == "nt":
                    handle.seek(0)
                    msvcrt.locking(handle.fileno(), msvcrt.LK_UNLCK, 1)
                else:
                    fcntl.flock(handle, fcntl.LOCK_UN)
