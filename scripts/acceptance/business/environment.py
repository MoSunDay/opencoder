"""Launch private services and register the installed business runners unchanged."""
from pathlib import Path
import base64
import json
import os
import re
import secrets
import shutil
import socket
import subprocess
from common import CONFIG, HERE, emit, http, read, run, sha, until, write


def port():
    with socket.socket() as sock:
        sock.bind(('127.0.0.1', 0))
        return sock.getsockname()[1]


class Environment:
    @classmethod
    def attach(cls, root):
        value = cls.__new__(cls)
        value.root, value.runtime = root, root / 'runtime'
        value.prepared = read(root / 'prepared.json') if (root / 'prepared.json').exists() else {}
        value.units = read(value.runtime / 'units.json') if (value.runtime / 'units.json').exists() else []
        value.mount, value.mounted = value.runtime / 'nfs', os.path.ismount(value.runtime / 'nfs')
        value.token = (value.runtime / 'control-token').read_text().strip() if (value.runtime / 'control-token').exists() else None
        value.api_token = (value.runtime / 'api-token').read_text().strip() if (value.runtime / 'api-token').exists() else None
        endpoints = read(value.runtime / 'endpoints.json') if (value.runtime / 'endpoints.json').exists() else {}
        value.base, value.business, value.node = (endpoints.get(key) for key in ['server', 'api', 'node'])
        value.env = {key: str(value.runtime / sub) for key, sub in {
            'HOME': 'home', 'TMPDIR': 'tmp', 'XDG_CONFIG_HOME': 'home/.config',
            'XDG_DATA_HOME': 'home/.local/share', 'XDG_CACHE_HOME': 'home/.cache'}.items()}
        return value

    def __init__(self, root, prepared):
        self.root, self.prepared = root, prepared
        self.runtime = root / 'runtime'
        self.units = []
        self.mount = self.runtime / 'nfs'
        self.mounted = False
        self.token = secrets.token_hex(32)
        self.base = None
        self.node = None
        self.env = {'HOME': str(self.runtime / 'home'), 'TMPDIR': str(self.runtime / 'tmp'),
            'XDG_CONFIG_HOME': str(self.runtime / 'home/.config'),
            'XDG_DATA_HOME': str(self.runtime / 'home/.local/share'),
            'XDG_CACHE_HOME': str(self.runtime / 'home/.cache'),
            'GIT_OPTIONAL_LOCKS': '0', 'GIT_CONFIG_GLOBAL': '/dev/null',
            'PATH': str(self.runtime / 'bin') + ':/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin:/root/.local/bin'}
        (self.runtime / 'bin').mkdir(exist_ok=True)
        shutil.copy2(HERE / 'systemd.py', self.runtime / 'bin/systemd-run')
        (self.runtime / 'bin/systemd-run').chmod(0o700)
        go_environment = (json.loads(run(['/usr/local/bin/go', 'env', '-json',
            'GOPROXY', 'GOPRIVATE', 'GONOSUMDB', 'GOSUMDB'])) if prepared.get('requiresGo', True) else {})
        go_environment.update({key: os.environ[key] for key in ['HTTP_PROXY', 'HTTPS_PROXY', 'NO_PROXY']
                               if key in os.environ})
        write(self.runtime / 'go-environment.json', go_environment)
        for key in ['HOME', 'TMPDIR', 'XDG_CONFIG_HOME', 'XDG_DATA_HOME', 'XDG_CACHE_HOME']:
            Path(self.env[key]).mkdir(parents=True, exist_ok=True)
        from dependencies.go import prepare_modules
        if prepared.get('requiresGo', True):
            prepare_modules(self.runtime, prepared['workspace'], go_environment)
        else:
            write(self.runtime / 'go-module-manifest.json', {})
        for name in ['.fornax-cli', '.lark-cli']:
            source = Path('/root') / name
            if source.exists():
                shutil.copytree(source, Path(self.env['HOME']) / name, symlinks=False)
        token = self.runtime / 'control-token'
        token.write_text(self.token)
        token.chmod(0o600)
        self.api_token = secrets.token_hex(32)
        token = self.runtime / 'api-token'
        token.write_text(self.api_token)
        token.chmod(0o600)

    def start(self, name, command, workdir):
        unit = f'oc-e2e-{self.root.name}-{name}.service'
        log = self.root / 'evidence' / f'{name}.log'
        args = ['systemd-run', '--quiet', '--collect', '--unit', unit,
            '--property', 'KillMode=control-group', '--property', 'TimeoutStopSec=15',
            '--property', 'RuntimeMaxSec=10800', '--property', 'UMask=0077',
            '--property', f'WorkingDirectory={workdir}',
            '--property', f'StandardOutput=append:{log}',
            '--property', f'StandardError=append:{log}']
        for key, value in self.env.items():
            args.extend(['--setenv', f'{key}={value}'])
        # PID 1 otherwise loses mounts made in the acceptance process's private
        # namespace. Inherit it before making the service's own guarded copy.
        args.extend(['--', '/usr/bin/nsenter', f'--mount=/proc/{os.getpid()}/ns/mnt',
            f'--root=/proc/{os.getpid()}/root', f'--wd={workdir}', '--',
            '/usr/bin/python3', str(HERE / 'scope.py'), str(self.root), name, *map(str, command)])
        run(args)
        self.units.append(unit)
        write(self.runtime / 'units.json', self.units)
        return unit

    def api(self, path, method='GET', body=None, binary=False):
        return http(self.base, self.token, path, method, body, binary)

    def launch(self):
        server = self.runtime / 'server'
        server.mkdir(exist_ok=True)
        nfs_port = port()
        native = {'model': 'acceptance/unused', 'providers': {'acceptance': {
            'base_url': 'http://127.0.0.1:1/v1', 'api_key': 'unused-local-only', 'model': 'unused'}}}
        write(server / '.opencoder/config.json', {**native, 'agent': {
            'agents_dir': self.prepared['resources'], 'nfs': {
                'enabled': False, 'host': '127.0.0.1', 'port': nfs_port, 'read_only': True}}})
        self.start('server', [self.prepared['platformBinaries']['opencoder-server'], '--host', '127.0.0.1',
            '--port', '0', '--workdir', server, '--data-dir', self.runtime / 'server-data',
            '--token-file', self.runtime / 'control-token'], server)
        def listening():
            file = self.root / 'evidence/server.log'
            match = re.search(r'listening on (http://\S+)', file.read_text() if file.exists() else '')
            return match[1] if match else None
        self.base = until(listening, 'temporary Server')
        self.api('/api/agents/nfs', 'POST', {'enabled': True})
        self.mount.mkdir(exist_ok=True)
        run(['mount', '-t', 'nfs', '-o', f'ro,vers=3,tcp,port={nfs_port},mountport={nfs_port},nolock,soft,retrans=1,timeo=50,actimeo=0,lookupcache=none',
             '127.0.0.1:/', self.mount])
        self.mounted = True
        workspace = Path(self.prepared['workspace'])
        write(workspace / '.opencoder/config.json', {**native, 'agent': {'agents_dir': str(self.mount)}})
        owner = self.start('node', [self.prepared['platformBinaries']['opencoder-agent'], '--remote', self.base,
            '--name', 'business-e2e', '--workdir', workspace, '--data-dir', self.runtime / 'node-data',
            '--max-runs', '1', '--token-file', self.runtime / 'control-token'], workspace)
        def ready():
            return next((n for n in self.api('/api/nodes')['nodes'] if n.get('snapshot', {}).get('ready')), None)
        self.node = until(ready, 'temporary Node')['id']
        self.register(owner)
        self.start('api', ['/usr/bin/node', HERE / 'api.mjs', self.root, self.prepared['release']], workspace)
        until(lambda: (self.runtime / 'api-ready.json').exists(), 'business API')
        self.business = f"http://127.0.0.1:{read(self.runtime / 'api-ready.json')['port']}"
        emit('private_services_ready', node=self.node, url=self.base)
        write(self.runtime / 'endpoints.json', {'server': self.base, 'api': self.business, 'node': self.node})

    def register(self, owner):
        config = read(self.prepared.get('configFile', CONFIG))
        config.pop('opencoder', None)
        codex = self.runtime / 'codex-bin'
        shutil.copytree(Path(config['codexCommand']).parent, codex,
                        ignore=shutil.ignore_patterns('codex-home', '*.log', '*.jsonl'))
        # The copied foreground wrapper still executes the installed real Codex binary.
        config.update({key: self.prepared[key] for key in ['repositories', 'contextFiles', 'contextCaptureFile', 'codexHome']})
        config.update({'host': '127.0.0.1', 'port': port(), 'dataDir': str(self.runtime / 'jobs'),
            'apiTokenFile': str(self.runtime / 'api-token'), 'codexCommand': str(codex / 'codex'),
            'executorIsolation': 'systemd', 'maxConcurrency': 1, 'analysisTimeoutMs': 3600000,
            'regression': {'workspaceRoot': self.prepared['workspace'], 'maxConcurrency': 1},
            'manager': {'baseUrl': 'http://127.0.0.1:1', 'robotId': 'e2e-local-robot',
                'tokenFile': str(self.runtime / 'api-token'), 'members': ['peijunying', 'heyang.amos']}})
        config['publicBaseUrl'] = f"http://127.0.0.1:{config['port']}"
        runner_config = self.runtime / 'runner.json'
        write(runner_config, config)
        profile = {'executable': config['codexCommand'], 'model': 'gpt-6-astra',
            'reasoning_effort': 'xhigh', 'approval_policy': 'never', 'sandbox_mode': 'danger-full-access',
            'auth_slot': config['codexAuthSlot'], 'envs': {**self.env, 'CODEX_HOME': config['codexHome']}}
        release = Path(self.prepared['release'])
        paths = [p for p in release.rglob('*') if p.is_file()]
        paths += [p for p in codex.rglob('*') if p.is_file()]
        paths += [HERE / 'scope.py', self.runtime / 'bin/systemd-run', self.runtime / 'go-environment.json',
                  self.runtime / 'go-module-manifest.json',
                  runner_config, Path('/usr/bin/python3').resolve(),
                  Path('/usr/bin/node'), Path('/usr/local/libexec/codext.real')]
        if self.prepared.get('requiresGo', True): paths.append(Path('/usr/local/bin/go').resolve())
        inventory = {str(p): sha(p) for p in paths}
        resources = self.complete_resources()
        self.validate_nfs_resources()
        for name in ['eval-diagnose', 'regression-test']:
            self.api('/api/harnesses/codex/profiles/' + name, 'PUT', profile)
            self.api('/api/runners/' + name, 'PUT', {'parent_unit': owner,
                'command': ['/usr/bin/python3', str(HERE / 'scope.py'), str(self.root), 'runner-' + name,
                    '/usr/bin/node', str(release / 'dist/cli.js'), 'runner', '--config', str(runner_config)],
                'workdir': self.prepared['workspace'], 'envs': self.env, 'files': inventory})
            self.api('/api/dag/defs', 'POST', {'spec': {'name': name, 'steps': [{
                'name': 'workflow', 'kind': {'type': 'runner', 'runner': name, 'agent': name}, 'timeout_secs': 3600}]}})
        config['opencoder'] = {'url': self.base, 'tokenFile': str(self.runtime / 'control-token'), 'nodeId': self.node}
        write(self.runtime / 'business.json', config)
        write(self.root / 'evidence/registration.json', {'profile': {k: v for k, v in profile.items() if k != 'envs'},
            'runnerFiles': inventory, 'node': self.node, 'maxRuns': 1, 'queueOrder': 'fifo',
            'completeSkills': resources})

    def complete_resources(self):
        result = {}
        for agent, skill in [('eval-diagnose', 'eval-diagnose'), ('regression-test', 'code-regression-review')]:
            source = Path(self.prepared['release']) / 'skills' / skill
            files = [{'path': skill + '/' + str(p.relative_to(source)),
                      'content_b64': base64.b64encode(p.read_bytes()).decode()}
                     for p in source.rglob('*') if p.is_file()]
            result[agent] = self.api('/api/agents/resources/skills/' + agent, 'PUT', {'name': agent, 'files': files})
        write(self.root / 'evidence/skill-publication.json', result)
        return result

    def validate_nfs_resources(self):
        verified = {}
        for agent, skill in [('eval-diagnose', 'eval-diagnose'), ('regression-test', 'code-regression-review')]:
            version = self.api('/api/agents/resources/skills/' + agent + '/meta')['meta']['current']
            source = Path(self.prepared['release']) / 'skills' / skill
            for file in source.rglob('*'):
                if not file.is_file():
                    continue
                relative = f'skills/{agent}/v{version}/{skill}/' + str(file.relative_to(source))
                target = self.mount / relative
                if not target.is_file() or sha(target) != sha(file):
                    raise RuntimeError('NFS skill package is incomplete: ' + relative)
                verified[relative] = sha(target)
        write(self.root / 'evidence/nfs-package-hashes.json', verified)

    def stop(self):
        def stop(unit):
            # --collect unloads a failed transient service before cleanup sees it.
            loaded = run(['systemctl', 'show', unit, '-p', 'LoadState', '--value']).strip()
            if loaded != 'not-found':
                run(['systemctl', 'stop', unit], timeout=45)
        for unit in reversed(self.units[1:]):
            stop(unit)
        if self.mounted:
            run(['umount', self.mount], timeout=30)
            self.mounted = False
        if self.units:
            stop(self.units[0])
        active = [u for u in self.units if subprocess.run(['systemctl', 'is-active', '--quiet', u]).returncode == 0]
        assert not active, active
        write(self.root / 'evidence/services-cleanup.json', {'units': self.units, 'active': active, 'nfsUnmounted': True})
