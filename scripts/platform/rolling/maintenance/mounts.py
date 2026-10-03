"""Install native resource mounts after the old writers have stopped."""
from pathlib import Path
import json
import subprocess
from ..state import atomic_bytes


def unit(plan):
    port = plan['port']
    path = str(plan['path'])
    if not path.startswith('/') or any(c in path for c in '\n\r'):
        raise ValueError('mount path must be absolute without control characters')
    path = path.replace('\\', '\\\\').replace('%', '%%')
    return f'''[Unit]
Description=OpenCoder read-only native resource
Wants=network-online.target opencoder-resources.service
After=network-online.target opencoder-resources.service

[Mount]
What=127.0.0.1:/
Where={path}
Type=nfs
Options=ro,vers=3,tcp,port={port},mountport={port},nolock,soft,retrans=1,timeo=50,actimeo=0,lookupcache=none
TimeoutSec=30

[Install]
WantedBy=multi-user.target
'''


def name(path):
    return subprocess.check_output(['systemd-escape', '--path', '--suffix=mount', str(path)], text=True).strip()


def mounted(path, operations):
    try:
        raw = operations.output('findmnt', '-J', '--nocanonicalize', '--mountpoint', str(path),
                                '-o', 'TARGET,SOURCE,FSTYPE,OPTIONS')
    except subprocess.CalledProcessError as error:
        if error.returncode == 1 and not error.stdout and not error.stderr:
            return []
        raise
    return json.loads(raw).get('filesystems', [])


def validate(plan, rows):
    for row in rows:
        options = set(row.get('options', '').split(','))
        if (row.get('target') != plan['path'] or row.get('source') != '127.0.0.1:/'
                or row.get('fstype') not in ('nfs', 'nfs4') or 'ro' not in options or 'rw' in options
                or f"port={plan['port']}" not in options):
            raise ValueError('refusing an unexpected resource mount: ' + plan['path'])


def clear(plan, operations):
    rows = mounted(plan['path'], operations)
    while rows:
        validate(plan, rows)
        operations.run('umount', plan['path'])
        remaining = mounted(plan['path'], operations)
        if len(remaining) >= len(rows):
            raise ValueError('resource mount did not disappear: ' + plan['path'])
        rows = remaining


def install(settings, plans, operations):
    for plan in plans:
        path = Path(plan['path'])
        unit_name = name(path)
        content = unit(plan)
        destination = settings.systemd_dir / unit_name
        # Replace native mounts within the stopped window. Existing Agent
        # mounts keep their handles and are intentionally outside this list.
        if operations.output('systemctl', 'show', unit_name, '-p', 'ActiveState', '--value').strip() == 'active':
            if destination.exists() and destination.read_text() == content:
                from .preflight import mount
                actual = mount(path, operations)
                if f"port={plan['port']}" not in actual['options'].split(','):
                    raise ValueError('retained native resource mount changed port')
                continue
            operations.run('systemctl', 'stop', unit_name)
        # A failed mount job can leave a kernel mount behind. Do not trust
        # ActiveState alone or stack a new export above an orphaned mount.
        clear(plan, operations)
        path.mkdir(parents=True, exist_ok=True)
        atomic_bytes(destination, content.encode(), 0o644)
        operations.run('systemd-analyze', 'verify', str(settings.systemd_dir / unit_name))
        operations.run('systemctl', 'daemon-reload')
        operations.run('systemctl', 'enable', '--now', unit_name)


def stop_new(settings, plans, operations):
    for plan in plans:
        unit_name = name(plan['path'])
        if operations.output('systemctl', 'show', unit_name, '-p', 'LoadState', '--value').strip() != 'not-found':
            operations.run('systemctl', 'stop', unit_name)
            clear(plan, operations)
            operations.run('systemctl', 'disable', unit_name)
        else:
            clear(plan, operations)


def resume_existing(plans, operations):
    for plan in plans:
        prior = plan.get('prior', {})
        if prior.get('loaded'):
            unit_name = name(plan['path'])
            operations.run('systemctl', 'enable' if prior['enabled'] else 'disable', unit_name)
            if prior['active']:
                operations.run('systemctl', 'start', unit_name)
