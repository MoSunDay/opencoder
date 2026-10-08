"""Authenticated live acceptance I/O; credentials never enter evidence."""
import hashlib
import json
import re
import secrets
import subprocess
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[3] / 'platform'))
from rolling.config import load
from rolling.io import Operations
from rolling.state import Journal, write


def require(condition, message):
    if not condition:
        raise AssertionError(message)


class Environment(Operations):
    def __init__(self, config, output, commit):
        self.settings = load(Path(config))
        super().__init__(self.settings.token_file)
        self.output = Path(output).resolve()
        self.output.mkdir(parents=True, exist_ok=False, mode=0o700)
        self.commit = commit
        self.tag = 'brain-e2e-' + secrets.token_hex(6)
        self.counter = 0
        self.created = []
        self.execution_directories = {}
        self.record = self.release()
        self.runtime_url = 'http://127.0.0.1:' + str(self.record['runtime_port'])
        self.node_id = (Path(self.record['runtime_data']) / 'node-id').read_text().strip()
        require(self.node_id.startswith('node-'), 'invalid persisted Runtime identity')
        self.save_suite()
        self.save('baseline', self.verify_release())

    def save_suite(self):
        source = Path(__file__).resolve().parent
        digests = {}
        for path in sorted(source.rglob('*')):
            if not path.is_file() or path.suffix not in ('.py', '.js', '.md'):
                continue
            relative = path.relative_to(source)
            data = path.read_bytes()
            target = self.output / 'suite-source' / relative
            target.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
            target.write_bytes(data)
            digests[str(relative)] = hashlib.sha256(data).hexdigest()
        self.save('suite', {'files': digests, 'entry': 'main.py'})

    def save(self, name, value):
        require(re.fullmatch(r'[a-zA-Z0-9_/.-]+', name) and '..' not in name,
                'unsafe evidence name')
        write(self.output / (name + '.json'), value)

    def release(self):
        state = Journal(self.settings.state_dir).data
        require(state['current'] == 'rel-' + self.commit and state['phase'] == 'complete',
                'release changed or deployment is in progress')
        return state['releases'][state['current']]

    def verify_release(self):
        record = self.release()
        builds = {}
        for kind in ('server', 'runtime'):
            unit = record[kind + '_unit']
            pid = int(subprocess.check_output(['systemctl', 'show', unit, '-p', 'MainPID', '--value']))
            require(pid > 0, kind + ' is not running')
            executable = Path(f'/proc/{pid}/exe')
            info = json.loads(subprocess.check_output([str(executable), '--build-info']))
            require(info['git_commit'] == self.commit and not info['git_dirty'], 'unexpected build')
            builds[kind] = {'pid': pid, 'unit': unit, 'build': info,
                            'sha256': hashlib.sha256(executable.read_bytes()).hexdigest()}
        status, ready = self.request('GET', '/api/ready')
        return {'commit': self.commit, 'processes': builds,
                'ready_status': status, 'ready': ready, 'at': time.time()}

    def request(self, method, path, body=None, base=None):
        self.counter += 1
        name = f'http/{self.counter:05d}'
        started = time.monotonic()
        status, value = 0, None
        try:
            request = urllib.request.Request((base or self.settings.public_url) + path,
                data=None if body is None else json.dumps(body).encode(), method=method,
                headers={'Authorization': 'Bearer ' + self.token, 'Content-Type': 'application/json'})
            try:
                response = self.opener.open(request, timeout=90)
            except urllib.error.HTTPError as error:
                response = error
            with response:
                status = response.code
                raw = response.read()
                try:
                    value = json.loads(raw)
                except (ValueError, UnicodeDecodeError):
                    value = {'non_json_body': raw[:4096].decode(errors='replace')}
            return status, value
        finally:
            self.save(name, {'method': method, 'path': path, 'request': body,
                'status': status, 'response': value, 'elapsed_seconds': time.monotonic() - started})

    def api(self, method, path, body=None, expected=None):
        status, value = self.request(method, path, body)
        require(status == expected if expected is not None else 200 <= status < 300,
                f'{method} {path}: HTTP {status}: {value}')
        return value

    def rpc(self, operation, expected=200):
        status, reply = self.request('POST', '/rpc', operation, base=self.runtime_url)
        require(status == 200 and reply['status'] == expected, f'Runtime RPC: {reply}')
        return reply['body']

    def brain_rpc(self, identifier, action, value=None, expected=200):
        require(identifier in self.created, 'RPC target is not owned by this acceptance')
        return self.rpc({'operation': 'brain', 'execution': {'id': identifier, 'kind': 'brain'},
                         'action': action, 'input': value}, expected)

    def create(self, label, plan, inputs=None, **extra):
        identifier = self.tag + '-' + label
        self.created.append(identifier)
        self.save('owned', self.created)
        request = {'id': identifier, 'node_id': self.node_id, 'schema_version': 7,
                   'plan': plan, 'inputs': inputs or {}, **extra}
        self.save('requests/' + identifier, request)
        for attempt in range(1, 4):
            try:
                status, reply = self.request('POST', '/api/brain/runs', request)
                if status == 202:
                    return identifier
                retry = status in (408, 423, 429) or 500 <= status < 600
                require(retry and attempt < 3, f'POST /api/brain/runs: HTTP {status}: {reply}')
            except OSError:
                if attempt == 3:
                    raise
            time.sleep(attempt * 2)
        raise AssertionError('root admission retries exhausted')

    def view(self, identifier):
        view = self.api('GET', f'/api/brain/runs/{identifier}/layered')
        self.save('runs/' + identifier, view)
        if identifier in self.created and view['run']['phase'] in ('completed', 'blocked', 'failed'):
            self.capture_decisions(identifier)
        return view

    def capture_decisions(self, identifier):
        require(identifier in self.created, 'decision evidence target is not owned')
        directory = Path(self.record['runtime_data']) / 'brain' / identifier / 'activations'
        for path in sorted(directory.glob('*/decision.json')):
            name = 'decisions/' + identifier + '/' + path.parent.name
            if (self.output / (name + '.json')).exists():
                continue
            context = json.loads((path.parent / 'context.json').read_text())
            run = context.get('run') or {}
            # Activation config and frozen capability definitions can contain credentials.
            # Keep only the scheduling position/error and the final proposal for this generation.
            self.save(name, {'decision': json.loads(path.read_text()),
                'context': {key: context.get(key) for key in ('layer', 'total_layers', 'guidance_only')},
                'run': {key: run.get(key) for key in ('phase', 'layer', 'round', 'generation', 'error')}})

    def wait(self, predicate, seconds=300, label='condition'):
        deadline = time.monotonic() + seconds
        while time.monotonic() < deadline:
            result = predicate()
            if result:
                return result
            time.sleep(1)
        raise TimeoutError(label + ' timed out')

    def terminal(self, identifier, seconds=1200):
        def inspect():
            view = self.view(identifier)
            return view if view['run']['phase'] in ('completed', 'failed', 'blocked', 'cancelled') else None
        return self.wait(inspect, seconds, identifier)

    def detail(self, operation):
        identifier = operation['execution_id']
        detail = self.api('GET', '/api/executions/' + identifier)
        self.save('executions/' + identifier, detail)
        return detail

    def cleanup(self, identifiers=None, receipt_name='cleanup'):
        identifiers = self.created if identifiers is None else identifiers
        require(all(identifier in self.created for identifier in identifiers), 'cleanup target is not owned')
        results = []
        for identifier in identifiers:
            try:
                self.capture_decisions(identifier)
                view = self.cleanup_snapshot(identifier)
                if view is None:
                    results.append({'id': identifier, 'absent': True})
                    continue
                if view['run']['phase'] not in ('completed', 'cancelled', 'failed'):
                    self.brain_rpc(identifier, 'cancel')
                def settled():
                    current = self.cleanup_snapshot(identifier)
                    require(current is not None, 'owned run disappeared during cleanup')
                    return current if current['run']['phase'] in ('completed', 'cancelled', 'failed') and all(
                        op['status'] in ('done', 'error', 'cancelled') for op in current['operations']) else None
                final = self.wait(settled, 180, 'owned children settle after cleanup')
                results.append({'id': identifier, 'phase_before_cleanup': view['run']['phase'],
                                'phase': final['run']['phase'], 'children_settled': True})
            except Exception as error:
                results.append({'id': identifier, 'error': str(error)})
        self.save(receipt_name, results)
        require(not any('error' in row for row in results), 'acceptance cleanup incomplete')

    def cleanup_snapshot(self, identifier):
        # All roots are explicitly pinned to this Runtime. Its authoritative
        # snapshot remains usable when the public index is still synchronizing.
        require(identifier in self.created, 'cleanup target is not owned by this acceptance')
        status, reply = self.request('POST', '/rpc', {'operation': 'brain',
            'execution': {'id': identifier, 'kind': 'brain'}, 'action': 'snapshot', 'input': None},
            base=self.runtime_url)
        require(status == 200, f'cleanup RPC: HTTP {status}')
        if reply['status'] == 404:
            return None
        if reply['status'] == 409:
            # This release reports an unknown journal as unsupported schema.
            # Require a separate public 404 and no local journal before calling it absent.
            record = Path(self.record['runtime_data']) / 'brain' / identifier / 'execution.json'
            if not record.exists():
                public_status, _ = self.request('GET', f'/api/brain/runs/{identifier}/layered')
                if public_status == 404:
                    return None
        require(reply['status'] == 200, f'cleanup snapshot: {reply}')
        return reply['body']

    def execution_data(self, operation):
        require(operation['run_id'] in self.created, 'workspace target is not owned by this acceptance')
        identifier = operation['execution_id']
        if identifier not in self.execution_directories:
            from workspaces import owned_directory, runtime_directories
            detail = self.detail(operation)
            index = detail['execution']
            require(index['id'] == identifier and index['kind'] == operation['execution_kind'],
                    'execution detail identity mismatch')
            directories = runtime_directories() | {Path(self.record['runtime_data'])}
            self.execution_directories[identifier] = owned_directory(operation, index['node_id'], directories)
        return self.execution_directories[identifier]

    def release_gate(self, operation):
        require(operation['run_id'] in self.created, 'gate is not owned by this acceptance')
        identifier = operation['execution_id']
        journal = self.execution_data(operation) / 'dag' / identifier / 'execution.json'
        if not journal.exists():
            return False
        record = json.loads(journal.read_text())
        run = Path(record['annotations']['dag_parent']) / identifier
        container = run / 'container.json'
        if not container.exists():
            return False
        identity = json.loads(container.read_text())['id']
        result = subprocess.run(['runc', '--root', str(run / 'runc-state'), 'exec', identity,
            '/bin/sh', '-c', 'test -d /workspace/check && : > /workspace/check/release'],
            capture_output=True, timeout=10)
        return result.returncode == 0
