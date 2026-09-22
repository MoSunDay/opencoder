"""Find a free loopback port block without contacting another service."""
from contextlib import ExitStack
import errno
import socket


def available(start, count):
    with ExitStack() as stack:
        for port in range(start, start + count):
            listener = stack.enter_context(socket.socket(socket.AF_INET, socket.SOCK_STREAM))
            try:
                listener.bind(('127.0.0.1', port))
            except OSError as error:
                if error.errno in (errno.EADDRINUSE, errno.EACCES):
                    return False
                raise
    return True


def first_available(start, count, probe=available):
    if start < 1 or count < 1:
        raise ValueError('invalid release port range')
    for port in range(start, 65537 - count):
        if probe(port, count):
            return port
    raise ValueError('no ports available for release instances')
