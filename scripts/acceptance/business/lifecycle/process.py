"""Reattach only to this run's fixture process, guarding against PID reuse."""
from pathlib import Path
import os
import signal
import time
from common import read, write


def identity(pid):
    try:
        fields = Path(f'/proc/{pid}/stat').read_text().rsplit(')', 1)[1].split()
        return fields[19], fields[0]
    except (FileNotFoundError, ProcessLookupError):
        return None


def record_fixture(root, process):
    state = identity(process.pid)
    if state is None:
        raise RuntimeError('Fixture process exited before its ownership was recorded')
    write(Path(root) / 'evidence/fixture-process.json', {'pid': process.pid, 'startTime': state[0]})


def stop_fixture(root):
    marker = Path(root) / 'evidence/fixture-process.json'
    if not marker.exists():
        return
    owner = read(marker)
    pid = owner['pid']

    def alive():
        state = identity(pid)
        return state is not None and state[0] == owner['startTime'] and state[1] != 'Z'

    for sig in [signal.SIGTERM, signal.SIGKILL]:
        if not alive():
            return
        try:
            os.kill(pid, sig)
        except ProcessLookupError:
            return
        deadline = time.monotonic() + 5
        while time.monotonic() < deadline:
            try:
                os.waitpid(pid, os.WNOHANG)
            except ChildProcessError:
                pass
            if not alive():
                return
            time.sleep(0.05)
    raise RuntimeError('Owned fixture process did not stop')
