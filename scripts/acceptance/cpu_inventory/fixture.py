"""Private Server, Host and Runtime with synthetic historical assignments."""
import hashlib
import json
import os
from pathlib import Path
import secrets
import socket
import sqlite3
import subprocess
import time
import urllib.error
import urllib.request

NODE = 'node-cpu-fixture'


def port():
    with socket.socket() as listener:
        listener.bind(('127.0.0.1', 0))
        return listener.getsockname()[1]


def write(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, separators=(',', ':')))


class Fixture:
    def __init__(self, binaries, root, count, payload_count, payload_bytes):
        self.binaries, self.root = binaries.resolve(), root.resolve()
        self.root.mkdir(parents=True, exist_ok=False)
        self.count = count
        self.token = secrets.token_hex(24)
        self.children = {}
        self.ports = {role: port() for role in ('server', 'host', 'runtime')}
        if len(set(self.ports.values())) != 3:
            raise RuntimeError('fixture listener ports collided')
        self.opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
        self.environment = {'PATH': os.environ['PATH'], 'HOME': str(self.root / 'home'),
                            'XDG_CONFIG_HOME': str(self.root / 'home/config'),
                            'XDG_DATA_HOME': str(self.root / 'home/data'),
                            'NO_PROXY': '127.0.0.1,localhost', 'LANG': 'C.UTF-8',
                            'RUST_LOG': 'info'}
        for role in ('server', 'host', 'runtime', 'home'):
            write(self.root / role / '.opencoder/config.json', {
                'agent': {'agents_dir': None},
                'local_memory': False})
            write(self.root / role / '.opencoder/ap.json', {'mode': 'off'})
        self.data = self.root / 'runtime-data'
        host_data = self.root / 'host-data'
        for data in (self.data, host_data):
            data.mkdir()
            (data / 'node-id').write_text(NODE)
        write(self.data / 'host-binding.json',
              {'database': str(host_data / 'host.db'), 'runtime_id': 'cpu-fixture'})
        self.assignments = []
        for index in range(count):
            identifier = f'agent-history-{index:06}'
            record = {'id': identifier, 'kind': 'agent', 'node_id': NODE,
                      'status': 'done', 'created_at': index + 1}
            assignment = {'index': record, 'request': {'id': identifier, 'kind': 'agent',
                          'input': {}}, 'runtime': None}
            if index < payload_count:
                assignment['runtime'] = {'fixture_payload': 'x' * payload_bytes}
                self.assignments.append((identifier, json.dumps(assignment)))
            write(self.data / 'agent' / identifier / 'execution.json', {
                'assignment': assignment, 'annotations': {}, 'result': {'marker': index},
                'error': None, 'events': []})

    def http(self, role, path, body=None):
        request = urllib.request.Request(f'http://127.0.0.1:{self.ports[role]}{path}',
            data=None if body is None else json.dumps(body).encode(),
            headers={'Authorization': 'Bearer ' + self.token, 'Content-Type': 'application/json'})
        try:
            with self.opener.open(request, timeout=30) as response:
                return json.load(response)
        except urllib.error.HTTPError as error:
            error.msg += ': ' + error.read(2048).decode(errors='replace')
            raise

    def until(self, check, label, seconds=300):
        deadline, error = time.monotonic() + seconds, None
        while time.monotonic() < deadline:
            self.check_alive()
            try:
                value = check()
                if value:
                    return value
            except (OSError, urllib.error.URLError) as caught:
                error = str(caught)
            time.sleep(.2)
        raise TimeoutError(f'{label}: {error}')

    def check_alive(self):
        for role, child in self.children.items():
            if child.poll() is not None:
                raise RuntimeError(f'{role} exited: see {self.root / (role + ".log")}')

    def start(self, role):
        common = ['--workdir', str(self.root / role), '--data-dir',
                  str(self.root / (role + '-data')), '--token', self.token]
        if role == 'server':
            binary = 'opencoder-server'
            args = common + ['--host', '127.0.0.1', '--port', str(self.ports[role])]
        else:
            binary = 'opencoder-agent'
            args = common + ['--name', 'cpu-fixture', '--no-dag']
            if role == 'host':
                args += ['--remote', f'http://127.0.0.1:{self.ports["server"]}',
                         'host', '--port', str(self.ports[role]), '--standby']
            else:
                args += ['runtime', '--port', str(self.ports[role])]
        with (self.root / (role + '.log')).open('ab') as log:
            self.children[role] = subprocess.Popen([str(self.binaries / binary), *args],
                env=self.environment, stdout=log, stderr=log)

    def stop(self, role):
        child = self.children.pop(role, None)
        if child is None:
            return
        child.terminate()
        try:
            child.wait(timeout=30)
        except subprocess.TimeoutExpired:
            # These are private fixture children, never production services.
            child.kill()
            child.wait(timeout=10)

    def open(self):
        self.start('server')
        self.until(lambda: self.http('server', '/api/health'), 'initial Server')
        self.stop('server')
        # Seed only the stopped, private fixture database. No production paths
        # or authentication rows are accessed, even during a baseline run.
        database = self.root / 'server-data/control.db'
        with sqlite3.connect(database.as_uri() + '?mode=rw', uri=True) as connection:
            connection.executemany('INSERT INTO execution_assignments VALUES(?,?)', self.assignments)
        self.assignments.clear()
        self.start('server')
        self.until(lambda: self.http('server', '/api/health'), 'Server')
        self.start('host')
        self.until(lambda: self.http('host', '/status'), 'Host')
        self.http('host', '/runtimes', {'id': 'cpu-fixture', 'release_id': 'cpu-fixture',
            'mode': 'staged', 'config': {'endpoint': f'http://127.0.0.1:{self.ports["runtime"]}',
            'data_dir': str(self.data), 'unit': 'opencoder-runtime-cpu-fixture.service'}})
        self.stop('host')
        # Model an already active historical Runtime in stopped fixture data.
        # This measures reporting, not deployment activation; real DAG probe
        # and activation gates remain covered by the separate release exercise.
        database = self.root / 'host-data/host.db'
        with sqlite3.connect(database.as_uri() + '?mode=rw', uri=True) as connection:
            connection.execute("UPDATE host_runtimes SET mode='active' WHERE id='cpu-fixture'")
        self.start('runtime')
        self.until(lambda: self.http('runtime', '/inventory')['snapshot']['ready'], 'Runtime')
        self.stop('runtime')
        # A completed Agent has a session row as well as its journal. Without
        # it, Inspect exercises the failed-initialization fallback instead of
        # a normal historical read, even though its outer RPC status is 200.
        database = self.data / 'runtime.db'
        with sqlite3.connect(database.as_uri() + '?mode=rw', uri=True) as connection:
            connection.executemany(
                "INSERT INTO sessions(id,agent,created_at,updated_at,kind) VALUES(?,'act',?,?,'agent')",
                ((f'agent-history-{index:06}', index + 1, index + 1)
                 for index in range(self.count)))
        self.start('runtime')
        self.until(lambda: self.http('runtime', '/inventory')['snapshot']['ready'], 'seeded Runtime')
        self.start('host')
        self.until(lambda: self.http('host', '/status'), 'prepared Host')
        self.http('host', '/activate-host', {})
        self.until(lambda: self.http('server', '/api/ready')['ready_nodes'] == 1, 'inventory commit')
        self.verify()

    def verify(self):
        values = []
        cursor = ''
        while True:
            page = self.http('server', '/api/executions?limit=500' + cursor)
            values.extend(page['executions'])
            next_page = page.get('next_cursor')
            if not next_page:
                break
            cursor = f'&cursor_created_at={next_page["created_at"]}&cursor_id={next_page["id"]}'
        if len(values) != self.count or len({value['id'] for value in values}) != self.count:
            raise AssertionError('Server inventory omitted or duplicated history')
        if any(value['node_id'] != NODE or value['status'] != 'done' for value in values):
            raise AssertionError('Server inventory changed history ownership or status')
        return hashlib.sha256(json.dumps(sorted(values, key=lambda r: r['id']),
                                        sort_keys=True).encode()).hexdigest()

    def close(self):
        for role in ('host', 'runtime', 'server'):
            self.stop(role)
