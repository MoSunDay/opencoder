"""Private Unix control socket for controller children and Host systemctl calls."""
import json
from pathlib import Path
import socket
import socketserver
import subprocess
import sys
import threading
from rolling.io import Operations
from rolling.state import atomic_bytes


def request(path, method, args):
    with socket.socket(socket.AF_UNIX) as connection:
        connection.connect(str(path))
        connection.sendall((json.dumps({'method': method, 'args': list(map(str, args))}) + '\n').encode())
        with connection.makefile('rb') as stream:
            response = json.loads(stream.readline())
    if response.get('error'):
        if 'returncode' in response:
            raise subprocess.CalledProcessError(response['returncode'], list(args),
                output=response.get('stdout'), stderr=response.get('stderr'))
        raise RuntimeError(response['error'])
    return response['value']


def client(path, args, command='systemctl'):
    try:
        method = 'output' if command == 'systemctl' and 'show' in args else 'run'
        value = request(path, method, [command, *args])
        if value:
            sys.stdout.write(value)
    except Exception as error:
        print(str(error), file=sys.stderr)
        raise SystemExit(1)


class RemoteOperations(Operations):
    def __init__(self, token_file, socket_path):
        super().__init__(token_file)
        self.socket_path = socket_path

    def run(self, *args):
        return request(self.socket_path, 'run', args)

    def output(self, *args):
        return request(self.socket_path, 'output', args)


class Control:
    def __init__(self, operations):
        self.path = operations.root / 'control.sock'
        if len(str(self.path).encode()) >= 108:
            raise ValueError('data parent is too long for a private Unix socket')
        class Handler(socketserver.StreamRequestHandler):
            def handle(self):
                try:
                    payload = json.loads(self.rfile.readline())
                    if payload['method'] not in ('run', 'output'):
                        raise ValueError('unsupported control operation')
                    with operations.lock:
                        value = getattr(operations, payload['method'])(*payload['args'])
                    response = {'value': value}
                except subprocess.CalledProcessError as error:
                    response = {'error': str(error), 'returncode': error.returncode,
                                'stdout': error.stdout, 'stderr': error.stderr}
                except Exception as error:
                    response = {'error': str(error)}
                self.wfile.write((json.dumps(response) + '\n').encode())
        self.server = socketserver.ThreadingUnixStreamServer(str(self.path), Handler)
        self.server.daemon_threads = True
        self.path.chmod(0o600)
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()
        self.stopped = threading.Event()
        def supervise():
            while not self.stopped.wait(.1):
                try:
                    with operations.lock:
                        operations.supervise()
                except Exception as error:
                    operations.monitor_errors.append(str(error))
                    return
        self.monitor = threading.Thread(target=supervise, daemon=True)
        self.monitor.start()
        tools = operations.root / 'tools'
        tools.mkdir()
        script = ('#!/usr/bin/python3\nimport sys\n' +
                  f'sys.path.insert(0, {str(Path(__file__).parent)!r})\n' +
                  f'sys.path.insert(0, {str(Path(__file__).resolve().parents[2] / "platform")!r})\n' +
                  'from control import client\n')
        for command in ('systemctl', 'nginx'):
            body = script + f'client({str(self.path)!r}, sys.argv[1:], {command!r})\n'
            atomic_bytes(tools / command, body.encode(), 0o755)
        operations.env['PATH'] = str(tools) + ':' + operations.env['PATH']

    def close(self):
        self.stopped.set()
        self.monitor.join(timeout=5)
        self.server.shutdown()
        self.server.server_close()
        self.thread.join(timeout=5)
        self.path.unlink(missing_ok=True)
