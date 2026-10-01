"""Install native resource mounts after the old writers have stopped."""
from pathlib import Path
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


def install(settings, plans, operations):
    for plan in plans:
        path = Path(plan['path'])
        name = operations.output('systemd-escape', '--path', '--suffix=mount', str(path)).strip()
        content = unit(plan)
        destination = settings.systemd_dir / name
        # Replace native mounts within the stopped window. Existing Agent
        # mounts keep their handles and are intentionally outside this list.
        if operations.output('systemctl', 'show', name, '-p', 'ActiveState', '--value').strip() == 'active':
            if destination.exists() and destination.read_text() == content:
                from .preflight import mount
                actual = mount(path, operations)
                if f"port={plan['port']}" not in actual['options'].split(','):
                    raise ValueError('retained native resource mount changed port')
                continue
            operations.run('systemctl', 'stop', name)
        path.mkdir(parents=True, exist_ok=True)
        atomic_bytes(destination, content.encode(), 0o644)
        operations.run('systemd-analyze', 'verify', str(settings.systemd_dir / name))
        operations.run('systemctl', 'daemon-reload')
        operations.run('systemctl', 'enable', '--now', name)
