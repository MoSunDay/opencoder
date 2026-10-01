"""Immutable stopped backups, including the old service/controller configuration."""
from contextlib import closing
import hashlib
import json
import os
from pathlib import Path
import shutil
import sqlite3
import tempfile
from .. import backup
from ..state import atomic_bytes, write


def digest(path):
    result = hashlib.sha256()
    with path.open('rb') as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b''):
            result.update(block)
    return result.hexdigest()


def inventory(root):
    files = {}
    for path in sorted(root.rglob('*')):
        name = path.relative_to(root).as_posix()
        if path.is_symlink():
            files[name] = {'link': os.readlink(path)}
        elif path.is_file():
            files[name] = {'sha256': digest(path), 'mode': path.stat().st_mode & 0o777}
    return files


def verify(root):
    if digest(root / 'manifest.json') != (root / 'manifest.sha256').read_text().strip():
        raise ValueError('maintenance backup manifest checksum differs')
    metadata = json.loads((root / 'manifest.json').read_text())
    actual = inventory(root)
    actual.pop('manifest.json', None)
    actual.pop('manifest.sha256', None)
    if actual != metadata['files']:
        raise ValueError('maintenance backup checksum or file set differs')
    return metadata


def managed_units(settings):
    paths = [*settings.systemd_dir.glob('opencoder*.service'),
                   *settings.systemd_dir.glob('opencoder*.service.d'),
                   *settings.systemd_dir.glob('mnt-opencoder*.mount')]
    paths.extend(settings.systemd_dir / suffix for name in
                 (settings.legacy_server_unit, settings.legacy_agent_unit)
                 for suffix in (name, name + '.d') if (settings.systemd_dir / suffix).exists())
    return sorted(set(paths))


def capture_file(stage, path, name):
    destination = stage / 'control' / name
    item = {'path': str(path), 'copy': str(destination.relative_to(stage)),
            'exists': path.exists() or path.is_symlink()}
    if item['exists']:
        members = [path, *(path.rglob('*') if path.is_dir() and not path.is_symlink() else [])]
        item['owners'] = {str(p.relative_to(path)): [p.lstat().st_uid, p.lstat().st_gid]
                          for p in members}
        destination.parent.mkdir(parents=True, exist_ok=True)
        if path.is_dir() and not path.is_symlink():
            shutil.copytree(path, destination, symlinks=True)
        else:
            shutil.copy2(path, destination, follow_symlinks=False)
    return item


def create(settings, output, original, scope):
    if output.exists():
        metadata = verify(output)
        if metadata['scope']['release_id'] != scope['release_id']:
            raise ValueError('maintenance backup belongs to another candidate')
        return metadata
    output.parent.mkdir(parents=True, exist_ok=True)
    stage = Path(tempfile.mkdtemp(prefix='.maintenance-incomplete-', dir=output.parent))
    backup.snapshot(settings, stage / 'data', stopped=True)
    paths = [*managed_units(settings), settings.nginx_include,
             settings.state_dir / 'services', settings.state_dir / 'resources',
             settings.state_dir / 'host/deployment.json']
    for workdir in (settings.server_workdir, settings.agent_workdir):
        paths.extend([workdir / 'opencoder.json', workdir / '.opencoder'])
    paths.extend(settings.bin_dir / name for name in
                 ('opencoder', 'opencoder-cli', 'opencoder-server', 'opencoder-agent',
                  '.opencoder-platform-current', '.opencoder-platform-manifest.json'))
    controls = [capture_file(stage, path, str(i)) for i, path in enumerate(dict.fromkeys(paths))]
    import base64
    for item in controls:
        if item['path'] == str(settings.nginx_include):
            item['exists'] = scope['nginx'] is not None
            saved = stage / item['copy']
            if item['exists']:
                atomic_bytes(saved, base64.b64decode(scope['nginx']), 0o644)
            elif saved.exists():
                saved.unlink()
    # Staging a signal job installed a new controller. Restore the template
    # captured before that installation, rather than the new job's own unit.
    pending = settings.state_dir / 'maintenance-controller.json'
    if pending.exists():
        prior = json.loads(pending.read_text())
        if prior['release_id'] != scope['release_id']:
            raise ValueError('controller backup belongs to another maintenance candidate')
        for item in controls:
            if item['path'] == prior['path']:
                saved = stage / item['copy']
                item['exists'] = prior['content'] is not None
                if item['exists']:
                    atomic_bytes(saved, base64.b64decode(prior['content']), 0o644)
                elif saved.exists():
                    saved.unlink()
    metadata = {'original': original, 'scope': scope, 'control': controls}
    for record in original['releases'].values():
        bundle = settings.state_dir / 'releases' / record['id'] / 'bundle'
        if bundle.is_dir():
            metadata.setdefault('bundles', {})[str(bundle)] = inventory(bundle)
    metadata['files'] = inventory(stage)
    write(stage / 'manifest.json', metadata)
    atomic_bytes(stage / 'manifest.sha256', (digest(stage / 'manifest.json') + '\n').encode())
    # Fsync files and directories before publishing the complete receipt.
    for path in stage.rglob('*'):
        if path.is_file() and not path.is_symlink():
            with path.open('rb') as stream:
                os.fsync(stream.fileno())
    for path in sorted([stage, *(p for p in stage.rglob('*') if p.is_dir() and not p.is_symlink())],
                       key=lambda p: len(p.parts), reverse=True):
        fd = os.open(path, os.O_RDONLY | os.O_DIRECTORY)
        try:
            os.fsync(fd)
        finally:
            os.close(fd)
    os.rename(stage, output)
    fd = os.open(output.parent, os.O_RDONLY | os.O_DIRECTORY)
    try:
        os.fsync(fd)
    finally:
        os.close(fd)
    return verify(output)


def quote(name):
    return '"' + name.replace('"', '""') + '"'


def project_table(name):
    return name.startswith('project_') or name == 'schema_version'


def unrelated(conn, schema='main'):
    """Fingerprint every non-project table, including authentication rows."""
    tables = conn.execute(f'SELECT name,sql FROM {schema}.sqlite_schema WHERE type=\'table\'').fetchall()
    result = {}
    for name, sql in tables:
        if project_table(name) or name.startswith('sqlite_'):
            continue
        rows = conn.execute(f'SELECT * FROM {schema}.{quote(name)}').fetchall()
        result[name] = hashlib.sha256(repr((sql, sorted(map(repr, rows)))).encode()).hexdigest()
    return result


def restore_projects(source, target):
    """Restore project DDL, indexes and schema version without writing auth data."""
    with closing(sqlite3.connect(target.resolve().as_uri() + '?mode=rw', uri=True)) as conn:
        conn.execute('ATTACH DATABASE ? AS saved', (source.resolve().as_uri() + '?mode=ro&immutable=1',))
        before = unrelated(conn)
        if before != unrelated(conn, 'saved'):
            raise ValueError('non-project data changed; refusing backup restoration')
        conn.execute('PRAGMA foreign_keys=OFF')
        conn.execute('BEGIN IMMEDIATE')
        try:
            tables = conn.execute("SELECT name FROM main.sqlite_schema WHERE type='table'").fetchall()
            for (name,) in tables:
                if project_table(name):
                    conn.execute(f'DROP TABLE {quote(name)}')
            objects = conn.execute('SELECT type,name,tbl_name,sql FROM saved.sqlite_schema WHERE sql IS NOT NULL').fetchall()
            for kind, name, table, sql in objects:
                if kind != 'table' or not project_table(name):
                    continue
                conn.execute(sql)
                columns = [row[1] for row in conn.execute(f'PRAGMA saved.table_xinfo({quote(name)})') if row[6] == 0]
                names = ','.join(map(quote, columns))
                conn.execute(f'INSERT INTO main.{quote(name)} ({names}) SELECT {names} FROM saved.{quote(name)}')
            for kind, name, table, sql in objects:
                if kind in ('index', 'trigger') and project_table(table):
                    conn.execute(sql)
            if unrelated(conn) != before:
                raise ValueError('restoration changed non-project data')
            if conn.execute('PRAGMA quick_check').fetchone() != ('ok',):
                raise ValueError('restored project database failed integrity check')
            conn.commit()
        except BaseException:
            conn.rollback()
            raise
