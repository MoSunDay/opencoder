"""Locate Ontology files from the database's effective Server storage root."""
from contextlib import closing
from pathlib import Path
import hashlib
import json
import sqlite3


def files_root(server_data):
    database = Path(server_data) / 'ontology.db'
    if not database.exists():
        return None
    with closing(sqlite3.connect(database.resolve().as_uri() + '?mode=ro', uri=True)) as reader:
        row = reader.execute('SELECT files_root FROM ontology_schema_version WHERE version=1').fetchone()
    if row is None or not Path(row[0]).is_absolute():
        raise ValueError('Ontology database lacks its effective files root')
    root = Path(row[0]).resolve()
    if Path(server_data).resolve().is_relative_to(root):
        raise ValueError('Ontology export cannot contain Server databases')
    return root


def verify_files(database, root):
    with closing(sqlite3.connect(database.resolve().as_uri() + '?mode=ro&immutable=1', uri=True)) as reader:
        for (body,) in reader.execute('SELECT body FROM text_revisions'):
            reference = json.loads(body)
            if reference.get('status') != 'ready' or reference.get('is_deleted'):
                continue
            path = root / reference['content_path']
            if not path.resolve().is_relative_to(root.resolve()):
                raise ValueError('Ontology backup contains an escaping text path')
            data = path.read_bytes()
            if len(data) != reference['bytes'] or hashlib.sha256(data).hexdigest() != reference['sha256']:
                raise ValueError('Ontology backup text differs from its database revision')
