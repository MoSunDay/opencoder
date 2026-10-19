"""Authenticated management requests; credentials are never persisted in evidence."""
import json
import os
from pathlib import Path
import urllib.error
import urllib.parse
import urllib.request


class Client:
    def __init__(self, base, token_file):
        self.base = base.rstrip('/')
        self.token = Path(token_file).read_text().strip()
        self.opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))

    def call(self, method, path, etag=None, raw=False):
        headers = {'Authorization': 'Bearer ' + self.token}
        if etag:
            headers['If-Match'] = etag
        request = urllib.request.Request(self.base + path, method=method, headers=headers)
        try:
            with self.opener.open(request, timeout=90) as response:
                data = response.read()
                return response.status, data if raw else json.loads(data), response.headers.get('ETag')
        except urllib.error.HTTPError as error:
            return error.code, json.loads(error.read()), None


def endpoint(kind, name):
    return ('/api/dag/defs/' if kind == 'dag' else '/api/dag/binaries/') + urllib.parse.quote(name, safe='')


def write(path, value):
    path = Path(path)
    temporary = path.with_suffix(path.suffix + '.tmp')
    with temporary.open('w') as stream:
        temporary.chmod(0o600)
        stream.write(json.dumps(value, ensure_ascii=False, indent=2) + '\n')
        stream.flush()
        os.fsync(stream.fileno())
    temporary.replace(path)
    sync_directory(path.parent)


def require(condition, message):
    if not condition:
        raise ValueError(message)


def sync_directory(path):
    descriptor = os.open(path, os.O_RDONLY | os.O_DIRECTORY)
    try:
        os.fsync(descriptor)
    finally:
        os.close(descriptor)


def sync_tree(path):
    for item in Path(path).rglob('*'):
        if item.is_file():
            with item.open('rb') as stream:
                os.fsync(stream.fileno())
    for item in sorted(Path(path).rglob('*'), reverse=True):
        if item.is_dir():
            sync_directory(item)
    sync_directory(path)
