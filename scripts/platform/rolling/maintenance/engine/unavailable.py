"""Keep unavailable terminal history without permitting an old program to wake."""
import json
from pathlib import Path
import subprocess
from ...state import Journal


def partition(settings, registered):
    releases = Journal(settings.state_dir).data['releases']
    retained, unavailable = {}, {}
    for identifier, item in registered.items():
        if identifier in releases:
            retained[identifier] = item
            continue
        verify(settings, identifier, item)
        unavailable[identifier] = item
    return retained, unavailable


def verify(settings, identifier, item):
    def require(condition):
        if not condition:
            raise ValueError(f'Registered Runtime lacks a release record and is not safely unavailable: {identifier}')

    require(bool(identifier) and '/' not in identifier and '\\' not in identifier)
    require(item['mode'] == 'retired' and bool(item.get('sleep')))
    config = json.loads(item['config'])
    data = Path(config['data_dir'])
    require(data.is_absolute() and not data.exists() and not data.is_symlink())
    require(config['unit'] == f'opencoder-runtime-{identifier}.service')
    unit = settings.systemd_dir / config['unit']
    # An explicit persistent mask is required; missing binaries alone do not
    # prevent a future restoration from waking an incompatible old program.
    require(unit.is_symlink() and unit.readlink() == Path('/dev/null'))
    state = subprocess.check_output(['systemctl', 'show', config['unit'],
        '-p', 'ActiveState', '-p', 'MainPID'], text=True)
    status = dict(line.split('=', 1) for line in state.splitlines() if '=' in line)
    require(status.get('ActiveState') in ('inactive', 'failed') and status.get('MainPID') == '0')
    inventory = json.loads(item['sleep'])
    node = (settings.state_dir / 'host/node-id').read_text().strip()
    require(inventory.get('runtime_id') == identifier
        and inventory.get('registration', {}).get('id') == node
        and inventory.get('can_hibernate') is True and inventory.get('owned_processes') == 0)
    snapshot = inventory.get('snapshot', {})
    require(all(snapshot.get(key) == 0 for key in ('active_runs', 'pending_runs', 'active_agent_loops')))
    require(all(index.get('status') in ('done', 'error', 'cancelled') for index in inventory.get('indexes', [])))
