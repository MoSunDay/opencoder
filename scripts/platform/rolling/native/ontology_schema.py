"""Inspect shared Ontology storage before staging or starting any candidate."""
import hashlib
import json
from pathlib import Path
import re
from ..storage.database import query


def version(manifest):
    value = manifest.get('ontology_catalog_schema', 0)
    if type(value) is not int or value not in (0, 1):
        raise ValueError('unsupported Ontology catalog schema')
    return value


def contract():
    result = {}
    for sql in Path(__file__).with_suffix('.sql').read_text().split(';'):
        sql = sql.strip()
        if sql:
            name = re.match(r'CREATE (?:UNIQUE )?(?:TABLE|INDEX) (\w+)', sql)[1]
            result[name] = sql
    return result


def normalized(sql):
    quoted, parts = False, []
    for char in sql:
        if char == "'":
            quoted = not quoted
        if quoted or char == "'":
            parts.append(char)
        elif not char.isspace() and char != ';':
            parts.append(char.lower())
    value = ''.join(parts)
    for prefix in ['createtable', 'createindex', 'createuniqueindex']:
        value = value.replace(prefix + 'ifnotexists', prefix, 1)
    return value


def inspect(database, *, files=None, offline=False):
    expected = contract()
    if not database.exists():
        return {'missing': list(expected), 'empty': True, 'files_root': None}
    objects = dict(query(database, "SELECT name,sql FROM sqlite_schema WHERE sql IS NOT NULL AND name NOT LIKE 'sqlite_%'", offline=offline))
    missing = []
    for name, sql in expected.items():
        if name not in objects:
            missing.append(name)
        elif normalized(objects[name]) != normalized(sql):
            raise ValueError(f'Ontology schema conflicts with {name}; existing objects cannot be replaced')
    binding = None
    if objects:
        if 'ontology_schema_version' not in objects:
            raise ValueError('Ontology database lacks its files binding')
        rows = query(database, 'SELECT version,files_root FROM ontology_schema_version', offline=offline)
        if len(rows) != 1 or rows[0][0] != 1 or not Path(rows[0][1]).is_absolute():
            raise ValueError('invalid Ontology files binding')
        binding = rows[0][1]
        if files is not None and Path(binding).resolve() != Path(files).resolve():
            raise ValueError('Ontology files directory differs from the database binding')
    if 'environments' in objects and not query(database, "SELECT 1 FROM environments WHERE env_key='debug'", offline=offline):
        missing.append('debug environment')
    return {'missing': missing, 'empty': not objects, 'files_root': binding}


def check(settings, candidate, *, maintenance=False):
    if not version(candidate):
        return None
    state = inspect(settings.server_data / 'ontology.db')
    if state['missing'] and not maintenance:
        raise ValueError('Ontology schema requires --maintenance before staging or starting a candidate: ' + ', '.join(state['missing']))
    return state


def authorize(settings, journal, record):
    """Called only after stopped backup verification, before Server startup."""
    from ..maintenance import archive
    state = journal.data['maintenance']
    if state['stage'] != 'installing' or state['writes_open'] or state['target'] != record['id']:
        raise ValueError('Ontology migration requires the unopened maintenance target')
    backup = Path(state['backup'])
    archive.verify(backup)
    database = settings.server_data / 'ontology.db'
    observed = inspect(database)
    state['ontology_migration'] = {
        'schema': 1, 'release_id': record['id'], 'server_port': record['server_port'],
        'database': str(database.resolve()), 'files_root': observed['files_root'],
        'backup_sha256': hashlib.sha256((backup / 'manifest.json').read_bytes()).hexdigest(),
        'backup_verified': True,
    }
    journal.save()


def completed(settings, journal, record):
    if not version(record['manifest']):
        return
    check(settings, record['manifest'])
    journal.data['maintenance']['ontology_schema_complete'] = 1
    journal.save()


def authorize_initial(settings, journal, record):
    from ..backup_files.tree import inventory
    if journal.data.get('migration_stage') != 'services' or journal.data.get('current'):
        raise ValueError('initial Ontology migration requires stopped legacy services')
    backup = settings.state_dir / 'backups/first-migration'
    receipt = backup / 'backup-manifest.json'
    metadata = json.loads(receipt.read_text())
    actual = inventory(backup)
    actual.pop('backup-manifest.json', None)
    if metadata.get('kind') != 'stopped-consistent-copy' or actual != metadata['files']:
        raise ValueError('initial migration backup is incomplete or changed')
    journal.data['ontology_initial_migration'] = {
        'schema': 1, 'release_id': record['id'], 'server_port': record['server_port'],
        'database': str((settings.server_data / 'ontology.db').resolve()),
        'backup_sha256': hashlib.sha256(receipt.read_bytes()).hexdigest(),
        'backup_verified': True,
    }
    journal.save()
