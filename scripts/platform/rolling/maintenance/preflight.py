"""Read-only checks before closing admission or stopping any service."""
import json
from pathlib import Path
import shutil
import sqlite3
import subprocess
from contextlib import closing
from . import configuration as configuration_files


def configs(settings):
    return configuration_files.desired_configs(settings)


def configuration(agent, server):
    if server.get('storage', {}).get('backend', 'libsql') != 'libsql':
        raise ValueError('maintenance requires verified local libsql recovery')
    paths = {}
    allowed = {'binary_dir', 'workspace_dir', 'data_dir', 'rootfs_dir',
               'knowledge_root', 'nfs', 'workspace_nfs'}
    for config in (agent, server):
        if set(config.get('dag', {})) - allowed:
            raise ValueError('remove obsolete DAG configuration before maintenance')
    for section, keys in [('dag', ['rootfs_dir', 'binary_dir', 'workspace_dir']),
                          ('agent', ['agents_dir'])]:
        for key in keys:
            value = agent.get(section, {}).get(key)
            if not isinstance(value, str) or not Path(value).is_absolute():
                raise ValueError(f'{section}.{key} must be an explicit absolute path')
            paths[key] = Path(value)
    for section, key in [('dag', 'binary_dir'), ('dag', 'workspace_dir'), ('agent', 'agents_dir')]:
        value = server.get(section, {}).get(key)
        if not isinstance(value, str) or not Path(value).is_absolute():
            raise ValueError(f'Server {section}.{key} must be an explicit absolute path')
    ports = []
    for section, key, default in [('agent', 'nfs', 2049), ('dag', 'nfs', 2050),
                                   ('dag', 'workspace_nfs', 2051), ('ontology', 'nfs', 2052)]:
        if section == 'ontology' and not server.get(section, {}).get(key, {}).get('enabled'):
            continue
        export = server.get(section, {}).get(key, {})
        allowed_export = {'enabled', 'host', 'port'}
        if key != 'workspace_nfs' and section != 'ontology':
            allowed_export.add('read_only')
        if set(export) - allowed_export:
            raise ValueError(f'unsupported Server {section}.{key} fields')
        if export.get('enabled') is not True or export.get('read_only', True) is not True:
            raise ValueError(f'Server {section}.{key} must enable a read-only NFS export')
        ports.append(export.get('port', default))
    if len(set(ports)) != len(ports) or any(type(p) is not int or not 0 < p < 65536 for p in ports):
        raise ValueError('resource NFS ports must be valid and distinct')
    return paths


def mount(root, operations):
    rows = json.loads(operations.output('findmnt', '-J', '-T', str(root),
                                       '-o', 'TARGET,SOURCE,FSTYPE,OPTIONS'))['filesystems']
    identities = {(row.get('target'), row.get('source'), row.get('fstype'),
                   frozenset(row.get('options', '').split(','))) for row in rows}
    if not rows or len(identities) != 1 or rows[0]['fstype'] not in ('nfs', 'nfs4'):
        raise ValueError(f'resource path must be a verified NFS mount: {root}')
    options = set(rows[0]['options'].split(','))
    if 'ro' not in options or 'rw' in options:
        raise ValueError(f'resource mount must be read-only: {root}')
    return rows[0]


def workspace_source(path, user, operations, managed_paths=()):
    if not path.is_dir():
        raise ValueError(f'Server source workspace must already exist: {path}')
    source = path.resolve()
    for destination in managed_paths:
        managed = destination.resolve()
        if managed.is_relative_to(source) or source.is_relative_to(managed):
            raise ValueError(f'managed paths must be outside Server source workspace: {destination}')
    try:
        for permission in ('-r', '-x'):
            operations.run('runuser', '-u', user, '--', 'test', permission, str(path))
    except subprocess.CalledProcessError as error:
        raise ValueError(f'Server source workspace must be readable by {user}: {path}') from error


def check(settings, candidate, operations):
    agent, server = configs(settings)
    paths = configuration(agent, server)
    ontology_binding(settings, server)
    if shutil.which('runc') is None:
        raise ValueError('runc is required before maintenance')
    operations.run('runc', '--version')
    managed_paths = [Path(server['dag']['binary_dir']), settings.state_dir, settings.server_data,
                     settings.bin_dir, settings.systemd_dir, settings.nginx_include,
                     settings.server_workdir / 'opencoder.json', settings.agent_workdir / 'opencoder.json',
                     settings.server_workdir / '.opencoder', settings.agent_workdir / '.opencoder',
                     *paths.values()]
    ontology_root = server.get('ontology', {}).get('files_dir')
    if ontology_root:
        if not Path(ontology_root).is_absolute():
            raise ValueError('Ontology files_dir must be absolute')
        ontology_root = Path(ontology_root).resolve()
        for protected in [settings.server_data, Path(server['agent']['agents_dir']), Path(server['dag']['binary_dir'])]:
            protected = protected.resolve()
            if (protected.is_relative_to(ontology_root)
                    or (protected != settings.server_data.resolve() and ontology_root.is_relative_to(protected))):
                raise ValueError('Ontology files overlap a protected resource or database root')
        managed_paths.append(ontology_root)
    if settings.legacy_agent_data:
        managed_paths.append(settings.legacy_agent_data)
    workspace_source(Path(server['dag']['workspace_dir']), settings.server_user, operations,
                     managed_paths)
    rootfs = paths.pop('rootfs_dir')
    if rootfs.is_symlink() or not rootfs.is_dir():
        raise ValueError('DAG rootfs must be a real directory')
    for name in ('usr/bin', 'workspace'):
        path = rootfs / name
        if path.is_symlink() or not path.is_dir():
            raise ValueError(f'DAG image is missing a real {name} directory')
    mounts = {}
    for key, path in paths.items():
        if path.is_symlink() or (path.exists() and not path.is_dir()):
            raise ValueError(f'resource mount path is not a real directory: {key}')
        if key == 'agents_dir':
            mounts[key] = mount(path, operations)
            next(path.iterdir(), None)
        else:
            # Old exporters do not offer the new native resources. Their
            # mounts are installed only after the producer has been upgraded.
            mounts[key] = {'planned': True, 'target': str(path)}
    mounts['native'] = [{'path': str(paths[key]), 'source': str(server['dag'][key]),
                         'port': server['dag'][export].get('port', default)}
                        for key, export, default in [('binary_dir', 'nfs', 2050),
                                                     ('workspace_dir', 'workspace_nfs', 2051)]]
    from .mounts import name as mount_name, mounted, validate
    for plan in mounts['native']:
        validate(plan, mounted(plan['path'], operations))
        raw = operations.output('systemctl', 'show', mount_name(plan['path']),
                                '-p', 'ActiveState', '-p', 'UnitFileState', '-p', 'LoadState')
        status = dict(line.split('=', 1) for line in raw.splitlines() if '=' in line)
        plan['prior'] = {'loaded': status.get('LoadState') != 'not-found',
                         'active': status.get('ActiveState') == 'active',
                         'enabled': status.get('UnitFileState') == 'enabled'}
    database = database_inventory(settings.server_data / 'definitions.db')
    from ..state import Journal
    from ..backup import roots as backup_roots
    from .planning import capacity
    budget = capacity.check(capacity.plan(settings, candidate, rootfs,
                                          Journal(settings.state_dir).data, {'mounts': mounts}))
    return {'rootfs': str(rootfs), 'mounts': mounts, 'release_id': candidate['release_id'],
            'database': database, 'backup_bytes': budget['components']['stopped_backup'], 'capacity': budget,
            'backup_roots': {name: str(path) for name, path in backup_roots(settings, runtime_data=False).items()},
            'retained_execution_roots': [str(settings.state_dir / 'runtimes')]
                + ([str(settings.legacy_agent_data)] if settings.legacy_agent_data else []),
            'configuration_hashes': configuration_files.hashes((agent, server))}


def ontology_binding(settings, desired):
    from ..native.ontology import files_root
    bound = files_root(settings.server_data)
    if bound is None:
        return
    requested = desired.get('ontology', {}).get('files_dir')
    if requested is not None:
        if not Path(requested).is_absolute() or Path(requested).resolve() != bound:
            raise ValueError('Ontology files_dir cannot change for an existing database')
    else:
        _, original = configuration_files.actual_configs(settings)
        if original.get('ontology', {}).get('files_dir') is not None:
            raise ValueError('retain the explicit Ontology files_dir for its existing database')


def database_inventory(path):
    """Expose migration size and source revision without reading credentials."""
    if not path.is_file():
        raise ValueError('maintenance requires an existing definitions database')
    with closing(sqlite3.connect(path.resolve().as_uri() + '?mode=ro', uri=True)) as conn:
        version = conn.execute('SELECT version FROM schema_version LIMIT 1').fetchone()
        if version is None or version[0] > 33:
            raise ValueError('unsupported definitions database schema')
        names = conn.execute("SELECT name FROM sqlite_schema WHERE type='table' AND name LIKE 'project_%'").fetchall()
        tables = {name: conn.execute('SELECT count(*) FROM "' + name.replace('"', '""') + '"').fetchone()[0]
                  for (name,) in names}
        indexes = conn.execute("SELECT count(*) FROM sqlite_schema WHERE type='index' AND tbl_name LIKE 'project_%'").fetchone()[0]
    return {'schema_version': version[0], 'bytes': path.stat().st_size,
            'tables': tables, 'indexes': indexes}


def receipt(settings, candidate, operations):
    from ..state import Journal
    journal = Journal(settings.state_dir)
    scope = check(settings, candidate, operations)
    return {**scope, 'current_release': journal.data['current'],
            'maintenance': 'wait for idle; close admission; stop writers; back up; upgrade resources and schema',
            'rollback': 'restore old projects and services before writes reopen; use a corrective release afterwards'}
