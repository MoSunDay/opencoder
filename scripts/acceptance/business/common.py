"""Private, bounded helpers for real business acceptance (no production writes)."""
from pathlib import Path
import hashlib
import json
import os
import subprocess
import time
import urllib.error
import urllib.request

HERE = Path(__file__).resolve().parent
SOURCE = Path('/root/workspace')
CONFIG = Path('/root/.config/eval-diagnose-api/config.json')
CASE = 'full-e2e-case5-20260909-002'
TARGET = '426116a5a42f9f530497e1c892ee1b6c7eb7bcba'
BASE = 'fddb16eaf4d3cb94ec5243cd137a8b1c97062e4f'
OPENER = urllib.request.build_opener(urllib.request.ProxyHandler({}))


class HTTPFailure(RuntimeError):
    def __init__(self, status, message):
        self.status = status
        super().__init__(message)


def read(path):
    return json.loads(Path(path).read_text())


def write(path, value):
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, ensure_ascii=False, indent=2) + '\n')
    path.chmod(0o600)


def sha(path):
    result = hashlib.sha256()
    with open(path, 'rb') as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b''):
            result.update(block)
    return result.hexdigest()


def run(args, *, cwd=None, timeout=600, capture=True, env=None):
    merged = dict(os.environ, GIT_OPTIONAL_LOCKS='0', GIT_TERMINAL_PROMPT='0',
                  GIT_CONFIG_GLOBAL='/dev/null', GIT_CONFIG_NOSYSTEM='1',
                  GIT_LFS_SKIP_SMUDGE='1')
    merged.update(env or {})
    result = subprocess.run([str(x) for x in args], cwd=cwd, env=merged,
                            timeout=timeout, capture_output=capture, text=True, stdin=subprocess.DEVNULL)
    if result.returncode:
        # Arguments can contain a private path, but never credential values.
        raise RuntimeError(f'{Path(args[0]).name} failed ({result.returncode}): '
                           + (result.stderr[-2500:] if capture else 'see private log'))
    return result.stdout if capture else ''


def git(directory, *args, timeout=1200):
    return run(['git', '-c', 'core.hooksPath=/dev/null', '-C', directory, *args], timeout=timeout).strip()


def http(base, token, path, method='GET', body=None, binary=False):
    req = urllib.request.Request(base + path, method=method,
        data=None if body is None else json.dumps(body).encode(),
        headers={'Authorization': 'Bearer ' + token, 'Content-Type': 'application/json'})
    try:
        with OPENER.open(req, timeout=90) as response:
            data = response.read()
            return data if binary else json.loads(data)
    except urllib.error.HTTPError as error:
        raise HTTPFailure(error.code, f'{method} {path.split("?")[0]} HTTP {error.code}: '
                          + error.read().decode()[:2500]) from None


def until(check, label, seconds=45):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        value = check()
        if value:
            return value
        time.sleep(.25)
    raise RuntimeError('Timed out: ' + label)


def emit(event, **values):
    print(json.dumps({'event': event, **values}, ensure_ascii=False), flush=True)
