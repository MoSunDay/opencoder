"""Retire backends only after ingress can no longer route earlier requests."""
from pathlib import Path


def identity(stat):
    fields = stat.rsplit(') ', 1)[1].split()
    return fields[0], int(fields[19])


def snapshot(master_pid, proc=Path('/proc')):
    if master_pid <= 0:
        raise ValueError('Nginx master is not running')
    children = (proc / str(master_pid) / 'task' / str(master_pid) / 'children').read_text()
    workers = []
    for pid in map(int, children.split()):
        root = proc / str(pid)
        try:
            command = (root / 'cmdline').read_bytes()
            if not command.startswith(b'nginx: worker process'):
                continue
            state, ticks = identity((root / 'stat').read_text())
        except FileNotFoundError:
            continue
        if state != 'Z':
            workers.append({'pid': pid, 'start_ticks': ticks})
    if not workers:
        raise ValueError('Nginx has no observable live workers')
    return sorted(workers, key=lambda worker: worker['pid'])


def drained(workers, proc=Path('/proc')):
    for worker in workers:
        try:
            state, ticks = identity((proc / str(worker['pid']) / 'stat').read_text())
        except FileNotFoundError:
            continue
        if state != 'Z' and ticks == worker['start_ticks']:
            return False
    return True


def retire(operations, base, workers, successor_port):
    status = operations.http(base, '/api/admin/release')
    if status.get('retirement_protocol', 1) >= 2:
        operations.http(base, '/api/admin/release/retire', 'POST', {'ingress_workers': workers, 'successor_port': successor_port})
        return True
    # A legacy Server closes its listener at retirement. Preserve it while
    # any earlier ingress worker could still deliver an accepted request.
    if not operations.ingress_drained(workers):
        return False
    operations.http(base, '/api/admin/release/retire', 'POST', {})
    return True
