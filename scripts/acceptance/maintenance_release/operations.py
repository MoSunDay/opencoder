"""Execute generated private units without contacting the host systemd manager."""
import os
from pathlib import Path
import shlex
import signal
import subprocess
import threading
import time
from rolling.io import Operations
from rolling.state import write
from evidence import unchanged_source


def systemctl_command(args):
    values = list(args)
    while values and values[0].startswith('--'):
        if values.pop(0) != '--no-block':
            raise ValueError('unsupported systemctl prefix')
    if not values or values[0] not in ('show', 'start', 'stop', 'enable', 'disable', 'kill', 'daemon-reload', 'reload'):
        raise ValueError('unsupported private systemctl action')
    return values[0], values[1:]


def unit_fields(path):
    return dict(line.split('=', 1) for line in path.read_text().splitlines()
                if line.startswith(('ExecStart=', 'WorkingDirectory=', 'Where=', 'What=', 'Options=', 'Type=',
                                    'Restart=', 'RestartSec=')))


class PrivateOperations(Operations):
    def __init__(self, settings, root, nginx):
        super().__init__(settings.token_file)
        self.settings, self.root, self.nginx = settings, root, nginx
        self.children, self.enabled, self.mounted = {}, set(), []
        self.commands, self.processes, self.stopping = [], [], set()
        self.desired, self.restart_after, self.monitor_errors = set(), {}, []
        self.closing = False
        self.lock = threading.RLock()
        self.source = self.source_frozen = None
        self.env = {**os.environ, 'HOME': str(root / 'home'),
                    'XDG_CONFIG_HOME': str(root / 'home/.config'),
                    'XDG_DATA_HOME': str(root / 'home/.local/share')}
        self.config = root / 'nginx.conf'
        self.config.write_text(f'pid {root}/nginx.pid; error_log {root}/nginx.log; '
            f'events {{worker_connections 256;}} http {{access_log off; include {settings.nginx_include};}}')

    def guard(self):
        if self.source_frozen is not None:
            unchanged_source(self.source, self.source_frozen)

    def owned(self, name):
        if '/' in name or name in ('.', '..') or not name.endswith(('.service', '.mount')):
            raise ValueError('refusing unowned unit: ' + name)
        path = self.settings.systemd_dir / name
        if not path.is_file() and name not in self.children and name not in (
                self.settings.legacy_agent_unit, self.settings.legacy_server_unit):
            raise ValueError('refusing unowned unit: ' + name)
        return path

    def alive(self, name):
        child = self.children.get(name)
        return child is not None and child.poll() is None

    def start(self, name):
        path = self.owned(name)
        if self.closing:
            raise ValueError('fixture services are closing')
        self.desired.add(name)
        if name.endswith('.mount'):
            fields = unit_fields(path)
            target = Path(fields['Where'].replace('%%', '%').replace('\\\\', '\\'))
            if target.is_symlink() or not target.resolve().is_relative_to((self.root / 'mounts').resolve()) or fields['Type'] != 'nfs':
                raise ValueError('mount escapes private fixture')
            options = fields['Options'].split(',')
            if 'ro' not in options or 'rw' in options or fields['What'] != '127.0.0.1:/':
                raise ValueError('mount must use private read-only NFS')
            if target not in self.mounted:
                subprocess.run(['mount', '-t', 'nfs', '-o', fields['Options'], fields['What'], str(target)],
                               check=True, timeout=30, env=self.env)
                self.mounted.append(target)
            return
        if self.alive(name):
            return
        fields = unit_fields(path)
        command = [value.replace('%%', '%').replace('$$', '$')
                   for value in shlex.split(fields['ExecStart'])]
        workdir = fields.get('WorkingDirectory', '/').replace('%%', '%')
        if workdir != '/' and not Path(workdir).is_relative_to(self.root):
            raise ValueError('service workdir escapes fixture')
        with (self.root / (name + '.log')).open('ab') as log:
            child = subprocess.Popen(command, env=self.env, cwd=workdir, stdout=log, stderr=log)
        self.children[name] = child
        self.processes.append((name, child))
        self.stopping.discard(name)

    def stop(self, name):
        path = self.owned(name)
        self.desired.discard(name)
        if name.endswith('.mount'):
            target = Path(unit_fields(path)['Where'])
            if target in self.mounted:
                subprocess.run(['umount', str(target)], check=True, timeout=30)
                self.mounted.remove(target)
        elif self.alive(name):
            self.stopping.add(name)
            self.children[name].send_signal(signal.SIGTERM)

    def supervise(self):
        if self.closing:
            return
        for name in sorted(self.desired):
            child = self.children.get(name)
            if child is None or child.poll() in (None, 0):
                continue
            fields = unit_fields(self.settings.systemd_dir / name)
            if fields.get('Restart') != 'on-failure':
                continue
            deadline = self.restart_after.setdefault(name, time.monotonic() + float(fields.get('RestartSec', '1').rstrip('s')))
            if time.monotonic() >= deadline:
                self.start(name)
                self.restart_after.pop(name, None)

    def nginx_command(self, *args):
        subprocess.run([str(self.nginx), '-e', str(self.root / 'nginx.log'), '-p', str(self.root),
                        '-c', str(self.config), *args], check=True, timeout=30, env=self.env)

    def run(self, *args):
        args = tuple(map(str, args))
        self.guard()
        self.commands.append({'args': list(args), 'at': time.time_ns()})
        write(self.root / 'commands.json', self.commands)
        if Path(args[0]).name == 'systemctl':
            action, values = systemctl_command(args[1:])
            if action == 'daemon-reload':
                if values:
                    raise ValueError('unexpected daemon-reload arguments')
            elif action == 'reload' and values == ['nginx']:
                self.nginx_command('-s', 'reload')
            else:
                if action == 'kill' and set(value for value in values if value.startswith('--')) != {
                        '--kill-who=main', '--signal=SIGTERM'}:
                    raise ValueError('only graceful main-process signaling is allowed')
                for name in (value for value in values if not value.startswith('--')):
                    self.owned(name)
                    if action == 'enable':
                        self.enabled.add(name)
                        if '--now' in values:
                            self.start(name)
                    elif action == 'disable':
                        self.enabled.discard(name)
                        if '--now' in values:
                            self.stop(name)
                    elif action == 'start':
                        self.start(name)
                    elif action in ('stop', 'kill'):
                        self.stop(name)
                    else:
                        raise ValueError('unsupported private service operation')
        elif args[0] == 'nginx':
            self.nginx_command(*args[1:])
        else:
            subprocess.run(args, check=True, timeout=180, env=self.env)
        self.guard()

    def output(self, *args):
        args = tuple(map(str, args))
        if Path(args[0]).name != 'systemctl':
            return subprocess.run(args, check=True, capture_output=True, text=True,
                                  timeout=180, env=self.env).stdout
        action, values = systemctl_command(args[1:])
        if action != 'show' or not values:
            raise ValueError('only private systemctl show is readable')
        name = values[0]
        if '/' in name:
            raise ValueError('invalid unit name')
        path = self.settings.systemd_dir / name
        active = self.alive(name)
        if name.endswith('.mount') and path.exists():
            active = Path(unit_fields(path)['Where']) in self.mounted
        state = 'deactivating' if active and name in self.stopping else 'active' if active else 'inactive'
        properties = {'LoadState': 'loaded' if path.exists() or name in self.children else 'not-found',
                      'ActiveState': state, 'MainPID': str(self.children[name].pid if self.alive(name) else 0),
                      'UnitFileState': 'enabled' if name in self.enabled else 'disabled'}
        requested = [values[index + 1] for index, value in enumerate(values) if value == '-p']
        requested.extend(value.split('=', 1)[1] for value in values if value.startswith('--property='))
        if '--value' in values:
            return '\n'.join(properties[key] for key in requested) + '\n'
        return ''.join(key + '=' + properties[key] + '\n' for key in requested or properties)

    def close(self):
        self.closing = True
        errors = list(self.monitor_errors)
        for name in sorted(self.children, key=lambda value: (
                2 if value == 'opencoder-resources.service' else 0 if 'server' in value else 1, value)):
            child = self.children[name]
            try:
                self.stop(name)
                deadline = time.monotonic() + 45
                while child.poll() is None and time.monotonic() < deadline:
                    try:
                        child.send_signal(signal.SIGTERM)
                    except ProcessLookupError:
                        child.poll()
                    time.sleep(.2)
                if child.poll() is None:
                    raise TimeoutError('private process still draining: ' + name)
            except Exception as error:
                errors.append(str(error))
        for target in reversed(self.mounted.copy()):
            try:
                subprocess.run(['umount', str(target)], check=True, timeout=30)
                self.mounted.remove(target)
            except Exception as error:
                errors.append(str(error))
        if (self.root / 'nginx.pid').exists():
            try:
                self.nginx_command('-s', 'quit')
                self.wait(lambda: not (self.root / 'nginx.pid').exists(), 30)
            except Exception as error:
                errors.append(str(error))
        remaining = [line for line in Path('/proc/self/mountinfo').read_text().splitlines()
                     if line.split()[4].startswith(str(self.root) + '/')]
        processes = [{'unit': name, 'pid': child.pid, 'exit_code': child.poll()}
                     for name, child in self.processes]
        return {'passed': not errors and not remaining and all(item['exit_code'] is not None for item in processes),
                'errors': errors, 'processes': processes, 'remaining_mounts': remaining,
                'host_systemd_units_created': [], 'data_and_evidence_preserved': True}
