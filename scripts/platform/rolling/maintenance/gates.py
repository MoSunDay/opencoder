"""Public maintenance gate; only the node channel remains available."""
from ..state import atomic_bytes


def close(settings, record, operations):
    # The private Server identity check reads Host status through its stable
    # loopback address; every Host write/RPC remains closed during migration.
    content = f'''server {{
    listen {settings.listen};
    location = /api/nodes/channel {{
        proxy_pass http://127.0.0.1:{record['server_port']};
        proxy_http_version 1.1;
        proxy_set_header Upgrade $http_upgrade;
        proxy_set_header Connection "upgrade";
        proxy_read_timeout 1d;
    }}
    location / {{ add_header Retry-After 60 always; return 503; }}
}}
server {{
    listen 127.0.0.1:{settings.host_port};
    location = /status {{ proxy_pass http://127.0.0.1:{record['host_port']}; }}
    location / {{ return 503; }}
}}
'''
    atomic_bytes(settings.nginx_include, content.encode(), 0o644)
    operations.run('nginx', '-t')
    operations.run('systemctl', 'reload', 'nginx')


def drained(operations, endpoint):
    status = operations.http(endpoint, '/api/admin/drain')
    if status.get('offline_nodes'):
        raise ValueError('offline writers prevent a consistent maintenance backup')
    return status.get('drained') is True and bool(status.get('nodes'))
