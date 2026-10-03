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


def drained(operations, endpoint, node_id):
    status = operations.http(endpoint, '/api/admin/drain')
    server = status.get('server', {})
    nodes = status.get('nodes', [])
    # Offline registrations and historical idle indexes do not write the
    # shared Server database. Require the sole managed writer's live freeze
    # acknowledgement; flow also validates every retained Runtime inventory
    # (including pending work and owned processes) before stopping writers.
    # Recheck identity here so an external node reconnecting after preflight
    # cannot silently enter a maintenance scope the controller cannot stop.
    return (server.get('mode') == 'frozen'
            and server.get('inflight_admissions') == 0
            and len(nodes) == 1 and nodes[0].get('node_id') == node_id
            and 200 <= nodes[0].get('status', 0) < 300
            and nodes[0].get('body', {}).get('mode') == 'frozen'
            and nodes[0]['body'].get('active_runs') == 0
            and nodes[0]['body'].get('owned_processes') == 0)
