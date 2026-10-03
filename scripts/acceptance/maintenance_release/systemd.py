"""Real systemd acceptance confined to this run's unique units and mount paths."""
import hashlib
import os
from pathlib import Path
import shlex
import subprocess
import time
from rolling import units
from rolling.state import atomic_bytes, write
from operations import PrivateOperations, show_arguments, systemctl_command, unit_fields


class Process:
    def __init__(self, owner, name, args):
        self.owner, self.name, self.args = owner, name, args

    @property
    def pid(self):
        return int(self.owner.property(self.name, 'MainPID') or 0)

    def poll(self):
        return None if self.owner.alive(self.name) else 0


class RealOperations(PrivateOperations):
    def __init__(self, settings, root, nginx):
        super().__init__(settings, root, nginx)
        self.physical = {}
        self.created = set()
        self.mount_plans = {}

    def physical_name(self, name):
        if name not in self.physical:
            if '@' in name:
                stem, instance = name[:-len('.service')].split('@', 1)
                template = stem + '@.service'
                physical = ('opencoder-test-' + self.root.name + '-' +
                            hashlib.sha256(template.encode()).hexdigest()[:12] + '@.service')
                self.physical[name] = physical.replace('@.service', '@' + instance + '.service')
            else:
                self.physical[name] = name if name.endswith('.mount') else (
                    'opencoder-test-' + self.root.name + '-' + hashlib.sha256(name.encode()).hexdigest()[:12] + '.service')
        return self.physical[name]

    def owned(self, name):
        if '@' in name and not (self.settings.systemd_dir / name).exists():
            stem, _ = name.split('@', 1)
            return super().owned(stem + '@.service')
        return super().owned(name)

    def property(self, name, key):
        return subprocess.check_output(['/usr/bin/systemctl', 'show', self.physical_name(name),
                                        '-p', key, '--value'], text=True).strip()

    def alive(self, name):
        return self.property(name, 'ActiveState') in ('active', 'activating', 'deactivating')

    def materialize(self):
        sources = [*self.settings.systemd_dir.glob('*.service'), *self.settings.systemd_dir.glob('*.mount')]
        for source in sources:
            self.physical_name(source.name)
        for source in sources:
            content = source.read_text()
            lines = []
            for line in content.splitlines():
                if line.startswith(('After=', 'Wants=', 'Requires=')):
                    key, value = line.split('=', 1)
                    line = key + '=' + ' '.join(self.physical.get(item, item) for item in value.split())
                lines.append(line)
                if line in ('Type=simple', 'Type=oneshot'):
                    lines.extend('Environment=' + units.argument(key + '=' + self.env[key])
                                 for key in ('HOME', 'XDG_CONFIG_HOME', 'XDG_DATA_HOME', 'PATH'))
                    lines.extend(['StandardOutput=append:' + str(self.root / (source.name + '.log')),
                                  'StandardError=append:' + str(self.root / (source.name + '.log'))])
            if source.suffix == '.mount':
                fields = unit_fields(source)
                where = Path(fields['Where'])
                if not where.is_relative_to(self.root):
                    raise ValueError('refusing a mount outside the owned evidence directory')
                port = int(next(option.split('=', 1)[1] for option in fields['Options'].split(',')
                                if option.startswith('port=')))
                self.mount_plans[source.name] = {'path': str(where), 'port': port}
            destination = Path('/etc/systemd/system') / self.physical_name(source.name)
            if destination.exists() and destination.name not in self.created:
                raise ValueError('refusing to replace a preexisting host unit')
            atomic_bytes(destination, ('\n'.join(lines) + '\n').encode(), 0o644)
            self.created.add(destination.name)
        subprocess.run(['/usr/bin/systemctl', 'daemon-reload'], check=True)

    def start(self, name):
        self.owned(name)
        self.materialize()
        subprocess.run(['/usr/bin/systemctl', 'start', self.physical_name(name)], check=True, timeout=150)
        if name.endswith('.service'):
            process = Process(self, name, shlex.split(unit_fields(self.settings.systemd_dir / name)['ExecStart']))
            self.children[name] = process
            self.processes.append((name, process))
        else:
            target = Path(unit_fields(self.settings.systemd_dir / name)['Where'])
            if target not in self.mounted:
                self.mounted.append(target)

    def stop(self, name):
        subprocess.run(['/usr/bin/systemctl', '--no-block', 'stop', self.physical_name(name)], check=True)

    def supervise(self):
        # Restart policies are enforced by the real service manager.
        pass

    def run(self, *args):
        args = tuple(map(str, args))
        self.guard()
        self.commands.append({'args': list(args), 'at': time.time_ns()})
        write(self.root / 'commands.json', self.commands)
        if Path(args[0]).name != 'systemctl':
            if args[0] == 'nginx':
                self.nginx_command(*args[1:])
            else:
                subprocess.run(args, check=True, timeout=180, env=self.env)
            return
        action, values = systemctl_command(args[1:])
        if action == 'reload' and values == ['nginx']:
            self.nginx_command('-s', 'reload')
            return
        if action == 'daemon-reload':
            self.materialize()
            return
        if action in ('start', 'enable'):
            self.materialize()
        translated = [self.physical_name(value) if not value.startswith('--') else value for value in values]
        prefix = [flag for flag in ('--no-block', '--no-reload') if flag in args]
        subprocess.run(['/usr/bin/systemctl', *prefix, action, *translated], check=True, timeout=150)
        if action == 'start' or (action == 'enable' and '--now' in values):
            for name in (value for value in values if not value.startswith('--')):
                if name.endswith('.service'):
                    process = Process(self, name, shlex.split(unit_fields(self.owned(name))['ExecStart']))
                    self.children[name] = process
                    self.processes.append((name, process))
                else:
                    target = Path(unit_fields(self.settings.systemd_dir / name)['Where'])
                    if target not in self.mounted:
                        self.mounted.append(target)
        self.guard()

    def output(self, *args):
        args = tuple(map(str, args))
        if Path(args[0]).name != 'systemctl':
            return subprocess.run(args, check=True, capture_output=True, text=True, timeout=180, env=self.env).stdout
        action, values = systemctl_command(args[1:])
        if action != 'show':
            raise ValueError('only private systemctl show is readable')
        name, properties = show_arguments(values)
        if name == 'nginx':
            return super().output(*args)
        options = [value for key in properties for value in ('-p', key)]
        if '--value' in values:
            options.append('--value')
        return subprocess.check_output(['/usr/bin/systemctl', 'show', self.physical_name(name), *options], text=True)

    def close(self):
        from rolling.maintenance.services import stop_unit
        self.closing = True
        errors = []
        names = [name for name in self.physical if not name.endswith('@.service') and
                 (self.physical[name] in self.created or name in self.children)]
        order = sorted(names, key=lambda n: (3 if n == 'opencoder-resources.service'
                       else 2 if n.endswith('.mount') else 0 if 'server' in n else 1, n))
        for name in order:
            try:
                stop_unit(name, self, 60)
                if name.endswith('.mount'):
                    from rolling.maintenance.mounts import clear
                    clear(self.mount_plans[name], self)
                subprocess.run(['/usr/bin/systemctl', 'disable', self.physical_name(name)], check=True, capture_output=True)
            except Exception as error:
                errors.append(str(error))
        active = [name for name in names if self.alive(name)]
        for name in names:
            if name not in active:
                (Path('/etc/systemd/system') / self.physical_name(name)).unlink(missing_ok=True)
        if not active:
            for name in self.created:
                if name.endswith('@.service'):
                    (Path('/etc/systemd/system') / name).unlink(missing_ok=True)
        subprocess.run(['/usr/bin/systemctl', 'daemon-reload'], check=True)
        if (self.root / 'nginx.pid').exists():
            self.nginx_command('-s', 'quit')
            self.wait(lambda: not (self.root / 'nginx.pid').exists(), 30)
        remaining = [line for line in Path('/proc/self/mountinfo').read_text().splitlines()
                     if line.split()[4].startswith(str(self.root) + '/')]
        return {'passed': not errors and not active and not remaining, 'errors': errors,
                'remaining_units': active, 'remaining_mounts': remaining,
                'real_systemd_units': sorted(self.created), 'data_and_evidence_preserved': True}
