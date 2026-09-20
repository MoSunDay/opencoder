"""Hold a real Node WebSocket on the ingress worker being retired."""
import base64
import hashlib
import http.client
import json
import secrets
import struct
import threading
from urllib.parse import urlsplit


class NodeChannel:
    def __init__(self, env, host):
        target = urlsplit(env.settings.public_url)
        self.connection = http.client.HTTPConnection(target.hostname, target.port, timeout=10)
        self.closed = threading.Event()
        self.stop = threading.Event()
        self.errors = []
        self.lock = threading.Lock()
        key = base64.b64encode(secrets.token_bytes(16)).decode()
        self.connection.request('GET', '/api/nodes/channel?node_id=ingress-retirement-probe', headers={
            'Authorization': 'Bearer ' + env.token, 'Connection': 'Upgrade', 'Upgrade': 'websocket',
            'Sec-WebSocket-Version': '13', 'Sec-WebSocket-Key': key})
        response = self.connection.getresponse()
        assert response.status == 101, f'Node upgrade failed: {response.status}'
        expected = base64.b64encode(hashlib.sha1((key + '258EAFA5-E914-47DA-95CA-C5AB0DC85B11').encode()).digest()).decode()
        assert response.getheader('Sec-WebSocket-Accept') == expected
        self.socket = self.connection.sock
        status = env.http(host, '/status')
        registration = {**status['node'], 'id': 'ingress-retirement-probe', 'name': 'ingress retirement probe'}
        snapshot = {**status['snapshot'], 'generation': 'ingress-retirement-probe', 'ready': False}
        self.send(1, json.dumps({'type': 'hello', 'registration': registration, 'snapshot': snapshot}).encode())
        self.reader = threading.Thread(target=self.read, daemon=True)
        self.reader.start()
        self.pinger = threading.Thread(target=self.ping, daemon=True)
        self.pinger.start()

    def send(self, opcode, payload):
        mask = secrets.token_bytes(4)
        size = len(payload)
        length = bytes([size | 128]) if size < 126 else bytes([126 | 128]) + struct.pack('!H', size)
        with self.lock:
            self.socket.sendall(bytes([128 | opcode]) + length + mask + bytes(value ^ mask[i % 4] for i, value in enumerate(payload)))

    def read(self):
        try:
            stream = self.socket.makefile('rb')
            def exact(size):
                value = stream.read(size)
                if len(value) != size:
                    raise EOFError('Node channel ended without a retirement close frame')
                return value
            while not self.stop.is_set():
                first, second = exact(2)
                assert second & 128 == 0, 'server frame was masked'
                size = second & 127
                if size == 126:
                    size = struct.unpack('!H', exact(2))[0]
                elif size == 127:
                    size = struct.unpack('!Q', exact(8))[0]
                payload = exact(size)
                if first & 15 == 8:
                    assert not payload or struct.unpack('!H', payload[:2])[0] in [1000, 1001]
                    self.send(8, payload)
                    self.stop.set()
                    stream.close()
                    self.connection.close()
                    self.closed.set()
                    return
                if first & 15 == 9:
                    self.send(10, payload)
        except Exception as error:
            if not self.stop.is_set():
                self.errors.append(str(error))

    def ping(self):
        while not self.stop.wait(.5) and not self.closed.is_set():
            try:
                self.send(9, b'')
            except OSError as error:
                if not self.closed.is_set() and not self.stop.is_set():
                    self.errors.append(str(error))
                return

    def close(self):
        self.stop.set()
        self.connection.close()
