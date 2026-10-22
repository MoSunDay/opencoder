"""Local fixture processes; never uses production configuration or service units."""
import json
import os
from pathlib import Path
import select
import socket
import subprocess
import time
import urllib.request

TOKEN = 'release-compatibility-fixture'


def port():
    with socket.socket() as sock:
        sock.bind(('127.0.0.1', 0))
        return sock.getsockname()[1]


def request(endpoint, path, body=None):
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    req = urllib.request.Request(endpoint + path,
        data=None if body is None else json.dumps(body).encode(),
        headers={'Authorization': 'Bearer ' + TOKEN, 'Content-Type': 'application/json'})
    with opener.open(req, timeout=10) as response:
        return json.load(response)


def wait_http(child, endpoint, path='/api/ontology/environments'):
    deadline = time.monotonic() + 90
    while time.monotonic() < deadline:
        if child.poll() is not None:
            raise RuntimeError('fixture exited; inspect its process log')
        try:
            return request(endpoint, path)
        except OSError:
            time.sleep(.1)
    raise TimeoutError('fixture HTTP readiness')


class Processes:
    def __init__(self, root):
        self.root, self.children, self.logs = root, [], []

    def start(self, label, args, env=None):
        log = (self.root / (label + '.log')).open('w')
        self.logs.append(log)
        child = subprocess.Popen([str(arg) for arg in args], stdout=log, stderr=log, env=env)
        self.children.append(child)
        return child

    def server(self, label, binary, work, data, listen, release=None, resources=False):
        args = [binary, '--workdir', work, '--data-dir', data, '--port', listen,
                '--token', TOKEN, '--auth-enabled', 'false']
        if release:
            args += ['--release-config', release]
        if resources:
            args += ['--resources']
        return self.start(label, args)

    def runtime(self, label, binary, data, work, home, listen):
        env = {'PATH': os.environ['PATH'], 'HOME': str(home), 'RUST_LOG': 'error',
               'NO_PROXY': '127.0.0.1,localhost', 'LANG': 'C.UTF-8'}
        return self.start(label, [binary, '--name', 'compatibility-fixture', '--data-dir', data,
            '--workdir', work, '--token', TOKEN, '--no-dag', 'runtime', '--port', listen], env)

    @staticmethod
    def stop(child):
        if child.poll() is None:
            child.terminate()
            try:
                child.wait(timeout=10)
            except subprocess.TimeoutExpired:
                child.kill()
                child.wait(timeout=10)

    def close(self):
        for child in reversed(self.children):
            self.stop(child)
        for log in self.logs:
            log.close()


class NativeSql:
    def __init__(self, binary, database, root):
        self.log = (root / 'sql.log').open('w')
        self.child = subprocess.Popen([str(binary), 'storage', 'serve', '--database', str(database)],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=self.log, text=True)

    def query(self, sql):
        self.child.stdin.write(json.dumps({'sql': sql, 'parameters': []}) + '\n')
        self.child.stdin.flush()
        if not select.select([self.child.stdout], [], [], 15)[0]:
            raise TimeoutError('native SQL fixture')
        result = json.loads(self.child.stdout.readline())
        assert result['ok'], result
        return result['rows']

    def close(self):
        self.child.stdin.close()
        try:
            self.child.wait(timeout=5)
        except subprocess.TimeoutExpired:
            Processes.stop(self.child)
        self.log.close()


def configure(root):
    work = root / 'work'
    for name in ['work', 'agents', 'share/todo', 'files', 'server', 'home']:
        (root / name).mkdir(parents=True, exist_ok=True)
    config = {
        'agent': {'agents_dir': str(root / 'agents'), 'share_dir': str(root / 'share'), 'nfs': {'enabled': False}},
        'dag': {'binary_dir': str(root / 'binary'), 'workspace_dir': str(root / 'workspace'),
                'nfs': {'enabled': False}, 'workspace_nfs': {'enabled': False}},
        'ontology': {'files_dir': str(root / 'files'), 'nfs': {'enabled': False}},
    }
    (work / 'opencoder.json').write_text(json.dumps(config))
    return work
