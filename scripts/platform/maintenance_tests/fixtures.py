import copy
import json
from pathlib import Path
import sqlite3
from unittest.mock import patch
from contextlib import ExitStack
from rolling.config import Settings
from rolling.deployment import record_for
from rolling.state import Journal
from rolling.maintenance import flow, configuration


def legacy_database(path):
    path.parent.mkdir(parents=True, exist_ok=True)
    with sqlite3.connect(path) as conn:
        conn.executescript('''
        CREATE TABLE schema_version(version INTEGER NOT NULL);
        INSERT INTO schema_version VALUES (31);
        CREATE TABLE platform_users(name TEXT PRIMARY KEY,token_hash BLOB);
        INSERT INTO platform_users VALUES ('fixture',X'000102');
        CREATE TABLE project_milestones(id TEXT PRIMARY KEY,kind TEXT,title TEXT);
        INSERT INTO project_milestones VALUES ('i','initiative','before');
        CREATE TABLE project_todos(id TEXT PRIMARY KEY,milestone_id TEXT);
        INSERT INTO project_todos VALUES ('todo','i');
        CREATE INDEX idx_project_todos_milestone ON project_todos(milestone_id);
        ''')


def bundle_manifest(identifier, version):
    return {'release_id': identifier, 'commit': identifier, 'files': {}, 'protocol_version': 10,
            'compatibility': {'protocol': {'min': 1, 'max': 1},
                              'data_format': {'min': version, 'max': version}}}


class Fixture:
    def __init__(self, root):
        self.settings = Settings(root / 'state', root / 'server-work', root / 'server',
                                 root / 'agent-work', root / 'token', server_user='root',
                                 systemd_dir=root / 'units', nginx_include=root / 'nginx.conf',
                                 bin_dir=root / 'bin', min_memory_mb=1)
        self.settings.state_dir.mkdir()
        self.settings.systemd_dir.mkdir()
        self.settings.nginx_include.write_text('old ingress\n')
        (self.settings.state_dir / 'services').mkdir()
        (self.settings.state_dir / 'services/opencoder-resources').write_bytes(b'old resources')
        (self.settings.systemd_dir / 'opencoder-resources.service').write_text('old resource unit\n')
        (self.settings.state_dir / 'host').mkdir()
        (self.settings.state_dir / 'host/node-id').write_text('node')
        with sqlite3.connect(self.settings.state_dir / 'host/host.db') as conn:
            conn.executescript('''CREATE TABLE handoff(id TEXT);
                CREATE TABLE host_runtimes(id TEXT PRIMARY KEY,release_id TEXT,config TEXT,mode TEXT);
                CREATE TABLE fleet_definitions(kind TEXT,id TEXT,body TEXT,PRIMARY KEY(kind,id));
                CREATE TABLE capacity_queue(ticket TEXT,phase TEXT);''')
            conn.execute('INSERT INTO host_runtimes VALUES (?,?,?,?)',
                         ('old', 'old', json.dumps({'endpoint': 'http://127.0.0.1:3001',
                                                  'unit': 'opencoder-runtime-old.service'}), 'active'))
        self.db = self.settings.server_data / 'definitions.db'
        legacy_database(self.db)
        self.old = record_for(self.settings, bundle_manifest('old', 1), 0)
        journal = Journal(self.settings.state_dir)
        journal.data.update(current='old', phase='complete', releases={'old': self.old})
        journal.save()
        for key in ('server_unit', 'host_unit', 'runtime_unit'):
            (self.settings.systemd_dir / self.old[key]).write_text('old unit\n')
        self.bundle = root / 'bundle'
        (self.bundle / 'bin').mkdir(parents=True)
        for name in ('opencoder-server', 'opencoder-agent', 'dag-runner', 'agent-step-runner'):
            (self.bundle / 'bin' / name).write_bytes(b'new binary')
        self.rootfs = root / 'rootfs'
        (self.rootfs / 'usr/bin').mkdir(parents=True)
        (self.rootfs / 'workspace').mkdir()
        for workdir in (self.settings.agent_workdir, self.settings.server_workdir):
            workdir.mkdir()
            (workdir / 'opencoder.json').write_text(json.dumps({'dag': {'binary_dir': '/old/binaries'},
                'llm': {'api_key': 'private-fixture-credential'}}))
        self.desired = ({'dag': {'rootfs_dir': str(self.rootfs), 'binary_dir': '/mnt/binary',
                                'workspace_dir': '/mnt/workspace'}, 'agent': {'agents_dir': '/mnt/agents'}},
                        {'storage': {'backend': 'libsql'}, 'dag': {'binary_dir': str(root / 'exports/binary'),
                         'workspace_dir': str(root / 'fixture-source')}, 'agent': {'agents_dir': str(root / 'exports/agents')}})
        (root / 'fixture-source').mkdir()
        self.candidate = bundle_manifest('new', 2)
        self.calls = []
        self.active = {self.old[k] for k in ('server_unit', 'host_unit', 'runtime_unit')}
        self.active.add('opencoder-resources.service')
        self.fail_internal = False

    def migrate(self):
        with sqlite3.connect(self.db) as conn:
            if conn.execute('SELECT version FROM schema_version').fetchone()[0] == 32:
                return
            conn.executescript('''CREATE TABLE project_initiatives(id TEXT PRIMARY KEY,title TEXT);
                INSERT INTO project_initiatives SELECT id,title FROM project_milestones;
                DROP TABLE project_milestones;
                ALTER TABLE project_todos RENAME COLUMN milestone_id TO initiative_id;
                DROP INDEX idx_project_todos_milestone;
                CREATE TABLE project_tags(id TEXT PRIMARY KEY);
                UPDATE schema_version SET version=32;''')

    def run(self, *args):
        self.calls.append(args)
        if args[:3] == ('systemctl', '--no-block', 'stop'):
            self.active.difference_update(args[3:])
        elif args[:2] == ('systemctl', 'stop'):
            self.active.difference_update(args[2:])
        if args[:2] == ('systemctl', 'start'):
            if self.old['host_unit'] in args[2:] and self.old['runtime_unit'] not in self.active:
                raise AssertionError('restored Host started before its Runtime')
            self.active.update(args[2:])
            if any('host-new' in name for name in args[2:]):
                with sqlite3.connect(self.settings.state_dir / 'host/host.db') as conn:
                    assert conn.execute("SELECT mode FROM host_runtimes WHERE id='old'").fetchone() == ('retired',)
                    assert conn.execute("SELECT body FROM fleet_definitions WHERE kind='runtime_sleep' AND id='old'").fetchone()
            if any('server-new' in name for name in args[2:]):
                if self.old['server_unit'] in self.active:
                    raise AssertionError('old Server is still writing during migration')
                state = Journal(self.settings.state_dir).data['maintenance']
                if not (Path(state['backup']) / 'manifest.json').exists():
                    raise AssertionError('migration ran without a complete backup')
                self.migrate()

    def output(self, *args):
        if args[0] == 'findmnt':
            return '{"filesystems": []}'
        if args[0] == 'chroot':
            return '{}'
        unit = args[2]
        active = 'active' if unit in self.active else 'inactive'
        if '--value' in args:
            return 'loaded' if 'LoadState' in args else active
        return f'LoadState=loaded\nActiveState={active}\nMainPID={1 if active == "active" else 0}\nUnitFileState=enabled\n'

    def http(self, base, path, method='GET', body=None):
        self.calls.append((base, path, method))
        if path == '/inventory':
            return {'runtime_id': 'old', 'registration': {'id': 'node'}, 'can_hibernate': True,
                    'owned_processes': 0, 'indexes': [], 'snapshot': {
                        'active_runs': 0, 'pending_runs': 0, 'active_agent_loops': 0}}
        if path == '/api/health':
            return {'ok': True, 'role': 'resources', 'build': {
                'protocol_version': 10, 'release_compatibility': self.candidate['compatibility']}}
        if path == '/api/nodes':
            return {'nodes': [{'id': 'node', 'online': True}]}
        if path == '/api/admin/drain':
            return {'drained': True, 'server': {'mode': 'frozen', 'inflight_admissions': 0},
                    'nodes': [{'node_id': 'node', 'status': 200,
                               'body': {'mode': 'frozen', 'active_runs': 0, 'owned_processes': 0}}],
                    'offline_nodes': []}
        if path == '/api/admin/release':
            return {'instance_release': 'new'}
        if path == '/api/project/overview':
            old = base.endswith(':' + str(self.old['server_port'])) or base == self.settings.public_url
            # Public checks after opening inspect the new schema.
            if base == self.settings.public_url:
                old = Journal(self.settings.state_dir).data['current'] == 'old'
            with sqlite3.connect(self.db) as conn:
                conn.execute('SELECT ' + ('milestone_id' if old else 'initiative_id') + ' FROM project_todos').fetchall()
            if not old and self.fail_internal:
                raise RuntimeError('candidate verification failed')
        return {'ok': True}

    def wait(self, check, seconds):
        result = check()
        if not result:
            raise TimeoutError('fixture wait failed')
        return result

    def ingress_workers(self):
        return [{'pid': 1, 'start_ticks': 1}]

    def ingress_switched(self, workers):
        return True

    def patches(self):
        stack = ExitStack()
        stack.enter_context(patch.object(flow.manifest, 'verify', return_value=self.candidate))
        stack.enter_context(patch.object(flow.manifest, 'resources'))
        stack.enter_context(patch.object(flow.manifest._installer, 'build_info', return_value={}))
        stack.enter_context(patch.object(flow.preflight, 'check', return_value={'rootfs': str(self.rootfs),
            'release_id': 'new', 'configuration_hashes': configuration.hashes(self.desired)}))
        stack.enter_context(patch.object(configuration, 'desired_configs', return_value=self.desired))
        stack.enter_context(patch.object(flow.units, 'prepare'))
        stack.enter_context(patch.object(flow.units, 'validate'))
        stack.enter_context(patch.object(flow.units, 'activate_launchers'))
        stack.enter_context(patch.object(flow.probes, 'resources'))
        stack.enter_context(patch.object(flow.probes, 'candidate', return_value='node'))
        stack.enter_context(patch.object(flow.probes, 'public'))
        return stack
