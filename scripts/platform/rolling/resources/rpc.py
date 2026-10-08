"""Real read-only NFS/ACL requests, including a duplicate backend XID."""
import secrets
import socket
import struct


def read(stream, count):
    result = bytearray()
    while len(result) < count:
        chunk = stream.recv(count - len(result))
        if not chunk:
            raise ConnectionError('NFS reply ended early')
        result.extend(chunk)
    return bytes(result)


def receive(stream):
    result = bytearray()
    while True:
        header = struct.unpack('!I', read(stream, 4))[0]
        if len(result) + (header & 0x7fffffff) > 8 * 1024 * 1024:
            raise ValueError('NFS response exceeds record limit')
        result.extend(read(stream, header & 0x7fffffff))
        if header & 0x80000000:
            return bytes(result)


def send(stream, xid, program):
    payload = struct.pack('!10I', xid, 0, 2, program, 3, 0, 0, 0, 0, 0)
    stream.sendall(struct.pack('!I', 0x80000000 | len(payload)) + payload)


def check_retransmission(port):
    xid = secrets.randbelow(0xfffffffc)
    with socket.create_connection(('127.0.0.1', port), timeout=3) as stream:
        send(stream, xid, 100003)
        first = receive(stream)
        if len(first) != 24 or struct.unpack('!I', first[:4])[0] != xid:
            raise ValueError('NFS NULL reply differs')
        send(stream, xid, 100003)
        send(stream, xid + 1, 100003)
        send(stream, xid + 2, 100227)
        responses = [receive(stream), receive(stream)]
        if ({struct.unpack('!I', reply[:4])[0] for reply in responses} != {xid + 1, xid + 2}
                or any(len(reply) != 24 or struct.unpack('!I', reply[20:])[0] != 0 for reply in responses)):
            raise ValueError('retransmission blocked or corrupted later replies')
