import base64
import hashlib
import json
import os
from pathlib import Path
import re
import secrets
import socket
import subprocess
import time
import urllib.request
import urllib.error


def wait(check, label, seconds=60):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        value = check()
        if value:
            return value
        time.sleep(.05)
    raise TimeoutError(label)


def port():
    with socket.socket() as listener:
        listener.bind(('127.0.0.1', 0))
        return listener.getsockname()[1]


def write(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value))
    path.chmod(0o600)


def inventory(root):
    result = {}
    for path in [root, *sorted(root.rglob('*'))]:
        stat = path.lstat()
        record = {'mode': stat.st_mode, 'uid': stat.st_uid, 'gid': stat.st_gid}
        if path.is_file():
            digest = hashlib.sha256()
            with path.open('rb') as source:
                for chunk in iter(lambda: source.read(65536), b''):
                    digest.update(chunk)
            record.update(size=stat.st_size, mtime_ns=stat.st_mtime_ns, sha256=digest.hexdigest())
        result[str(path.relative_to(root))] = record
    return result


class Runtime:
    def __init__(self, root, binaries, rootfs):
        self.root, self.binaries, self.rootfs = root, binaries, rootfs
        self.children, self.mounts = {}, []
        self.token = secrets.token_hex(32)
        self.base = None
        self.opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
        (root / 'evidence').mkdir(parents=True)
        for name in ['home', 'tmp', 'server', 'source/binaries', 'source/workspace']:
            (root / 'runtime' / name).mkdir(parents=True)
        self.source = root / 'runtime/source/workspace'
        for step in ['first', 'second']:
            directory = self.source / step
            directory.mkdir()
            (directory / 'seed.txt').write_text('immutable source')
            (directory / 'seed.txt').chmod(0o444)
            os.chown(directory / 'seed.txt', 65534, 65534)
        self.original = inventory(self.source)
        (root / 'runtime/token').write_text(self.token)
        (root / 'runtime/token').chmod(0o600)
        self.environment = {'HOME': str(root / 'runtime/home'), 'TMPDIR': str(root / 'runtime/tmp'),
            'PATH': '/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin',
            'XDG_CONFIG_HOME': str(root / 'runtime/home/.config'),
            'XDG_DATA_HOME': str(root / 'runtime/home/.local/share')}

    def start(self, name, command, directory):
        with open(self.root / 'evidence' / (name + '.log'), 'ab') as log:
            self.children[name] = subprocess.Popen(list(map(str, command)), cwd=directory,
                env=self.environment, stdin=subprocess.DEVNULL, stdout=log, stderr=log)

    def api(self, path, method='GET', body=None):
        request = urllib.request.Request(self.base + path, method=method,
            data=None if body is None else json.dumps(body).encode(),
            headers={'Authorization': 'Bearer ' + self.token, 'Content-Type': 'application/json'})
        try:
            with self.opener.open(request, timeout=60) as response:
                return json.load(response)
        except urllib.error.HTTPError as error:
            raise RuntimeError(f'{method} {path}: {error.code} {error.read(4096).decode()}') from error

    def launch(self):
        binary_port, workspace_port = port(), port()
        server = self.root / 'runtime/server'
        write(server / 'opencoder.json', {'dag': {
            'binary_dir': str(self.root / 'runtime/source/binaries'), 'workspace_dir': str(self.source),
            'nfs': {'enabled': True, 'host': '127.0.0.1', 'port': binary_port, 'read_only': True},
            'workspace_nfs': {'enabled': True, 'host': '127.0.0.1', 'port': workspace_port}}})
        self.start('server', [self.binaries['opencoder-server'], '--host', '127.0.0.1', '--port', '0',
            '--workdir', server, '--data-dir', self.root / 'runtime/server-data',
            '--token-file', self.root / 'runtime/token'], server)
        def listening():
            match = re.search(r'listening on (http://\S+)', (self.root / 'evidence/server.log').read_text())
            return match[1] if match else None
        self.base = wait(listening, 'Server startup')
        for name in ['node-a', 'node-b']:
            node = self.root / 'runtime' / name
            dag = {'rootfs_dir': str(self.rootfs), 'data_dir': str(node / 'state/dag/runs')}
            for key, listener in [('binary_dir', binary_port), ('workspace_dir', workspace_port)]:
                mount = node / key
                mount.mkdir(parents=True)
                subprocess.run(['mount', '-t', 'nfs', '-o',
                    f'ro,vers=3,tcp,port={listener},mountport={listener},nolock,soft,timeo=10,retrans=1,actimeo=0,lookupcache=none',
                    '127.0.0.1:/', str(mount)], check=True, timeout=30)
                self.mounts.append(mount)
                dag[key] = str(mount)
            write(node / 'opencoder.json', {'dag': dag})
            self.start(name, self.node_command(name), node)
        def ready():
            nodes = [node for node in self.api('/api/nodes')['nodes']
                     if node.get('online') and node.get('snapshot', {}).get('ready')]
            return {node['name']: node['id'] for node in nodes} if len(nodes) == 2 else None
        return wait(ready, 'Two actual Workers ready')

    def node_command(self, name):
        node = self.root / 'runtime' / name
        return [self.binaries['opencoder-agent'], '--remote', self.base, '--name', name,
            '--workdir', node, '--data-dir', node / 'state', '--max-runs', '1',
            '--token-file', self.root / 'runtime/token']

    def publish(self, name, source, update=False):
        directory = self.root / 'runtime/build'
        directory.mkdir(exist_ok=True)
        (directory / (name + '.c')).write_text(source)
        subprocess.run(['cc', '-O2', '-static', '-s', '-Wl,--build-id=none',
            str(directory / (name + '.c')), '-o', str(directory / name)], check=True, timeout=30)
        value = self.api('/api/dag/binaries' + ('/' + name if update else ''), 'PUT' if update else 'POST', {'name': name, 'description': 'native acceptance',
            'binary_b64': base64.b64encode((directory / name).read_bytes()).decode()})
        version = value.get('version', value.get('current', 1))
        write(self.root / 'evidence' / (name + '-publication-' + str(version) + '.json'), value)
        return name + '@v' + str(version)

    def restart(self, name):
        previous = next(node['snapshot']['generation'] for node in self.api('/api/nodes')['nodes'] if node['name'] == name)
        child = self.children[name]
        child.kill()
        child.wait(timeout=10)
        self.start(name, self.node_command(name), self.root / 'runtime' / name)
        def ready():
            return any(node['name'] == name and node['online'] and node.get('snapshot', {}).get('ready')
                       and node['snapshot']['generation'] != previous for node in self.api('/api/nodes')['nodes'])
        wait(ready, 'Worker restart ' + name)

    def run_root(self, name, identifier):
        journal = self.root / 'runtime' / name / 'state/dag' / identifier / 'execution.json'
        if not journal.is_file():
            return None
        value = json.loads(journal.read_text())
        return Path(value['annotations']['dag_parent']) / identifier

    def close(self):
        errors = []
        for journal in (self.root / 'runtime').glob('node-*/state/dag/*/execution.json'):
            identifier = journal.parent.name
            try:
                value = self.api('/api/executions/' + identifier)
                if value['execution']['status'] not in ('done', 'error', 'cancelled'):
                    self.api('/api/executions/' + identifier + '/commands', 'POST', {'action': 'cancel'})
                    wait(lambda: self.api('/api/executions/' + identifier)['execution']['status'] == 'cancelled', 'Cleanup cancel ' + identifier, 30)
            except Exception as error:
                errors.append(str(error))
        for name in ['node-a', 'node-b']:
            child = self.children.get(name)
            if child and child.poll() is None:
                child.terminate()
                try:
                    child.wait(timeout=30)
                except subprocess.TimeoutExpired:
                    child.kill()
                    child.wait(timeout=10)
                    errors.append(name + ' did not stop gracefully')
        for journal in (self.root / 'runtime').glob('node-*/state/dag/*/execution.json'):
            value = json.loads(journal.read_text())
            parent = value.get('annotations', {}).get('dag_parent')
            if not parent:
                continue
            root = Path(parent) / journal.parent.name
            if not str(root).startswith(str(journal.parents[3]) + '/'):
                errors.append('invalid cleanup ownership ' + str(root))
                continue
            state = root / 'runc-state'
            if (state / ('dag-run-' + journal.parent.name)).exists():
                subprocess.run(['runc', '--root', str(state), 'delete', '--force', 'dag-run-' + journal.parent.name], check=True, timeout=30)
            mounts = Path('/proc/self/mountinfo').read_text().splitlines()
            for target in [root / 'bundle/rootfs', root / 'workspace']:
                if any(line.split()[4] == str(target) for line in mounts):
                    subprocess.run(['umount', str(target)], check=True, timeout=30)
        for mount in reversed(self.mounts):
            subprocess.run(['umount', str(mount)], check=True, timeout=30)
        server = self.children.get('server')
        if server and server.poll() is None:
            server.terminate()
            server.wait(timeout=30)
        assert inventory(self.source) == self.original, 'Server source was modified'
        remaining = [line for line in Path('/proc/self/mountinfo').read_text().splitlines()
                     if str(self.root / 'runtime') in line]
        assert not remaining, remaining
        if errors:
            raise RuntimeError('; '.join(errors))
        return {'source_unchanged': True, 'remaining_mounts': remaining,
                'process_exit_codes': {name: child.poll() for name, child in self.children.items()}}
