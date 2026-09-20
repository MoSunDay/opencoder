"""A request accepted by an old ingress worker before a backend connection."""
import http.client
import json
from pathlib import Path
import time
from urllib.parse import urlsplit


def queues(table, local_port, remote_port):
    local, remote = f'0100007F:{local_port:04X}', f'0100007F:{remote_port:04X}'
    for line in table.splitlines()[1:]:
        fields = line.split()
        if fields[1:4] == [local, remote, '01']:
            return tuple(int(value, 16) for value in fields[4].split(':'))
    return None


class LateRequest:
    def __init__(self, env, body):
        target = urlsplit(env.settings.public_url)
        self.connection = http.client.HTTPConnection(target.hostname, target.port, timeout=10)
        self.body = json.dumps(body).encode()
        try:
            # A completed keepalive request pins this socket to a known worker.
            self.connection.request('GET', '/api/health', headers={'Authorization': 'Bearer ' + env.token})
            response = self.connection.getresponse()
            assert response.status == 200
            response.read()
            sock = self.connection.sock
            assert sock is not None, 'ingress did not retain the fixture connection'
            self.local_port = sock.getsockname()[1]
            self.remote_port = target.port
            headers = (f'POST /api/executions HTTP/1.1\r\nHost: {target.netloc}\r\n'
                f'Authorization: Bearer {env.token}\r\nContent-Type: application/json\r\n'
                f'Content-Length: {len(self.body)}\r\nConnection: close\r\n')
            # Withhold the final blank line, so no upstream request can start.
            sock.sendall(headers.encode())
            deadline = time.monotonic() + 5
            while True:
                table = Path('/proc/net/tcp').read_text()
                client = queues(table, self.local_port, self.remote_port)
                server = queues(table, self.remote_port, self.local_port)
                if client == (0, 0) and server == (0, 0):
                    break
                if time.monotonic() >= deadline:
                    raise TimeoutError('old ingress worker did not consume the partial headers')
                time.sleep(.01)
        except BaseException:
            self.close()
            raise

    def finish(self):
        self.connection.sock.sendall(b'\r\n' + self.body)
        response = http.client.HTTPResponse(self.connection.sock)
        response.begin()
        body = response.read()
        self.connection.close()
        assert response.status == 202, f'accepted ingress request failed after retirement: {response.status}: {body[:512]!r}'
        return json.loads(body)

    def close(self):
        self.connection.close()
