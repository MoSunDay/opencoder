from contextlib import closing
from pathlib import Path
import hashlib
import json
import sqlite3
import sys
import tempfile
from types import SimpleNamespace
import unittest
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from rolling import backup
from rolling.maintenance import restore, preflight


class OntologyBackupTests(unittest.TestCase):
    def test_snapshot_includes_external_text_root_and_stopped_recovery_restores_both(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            data, files = root / 'server', root / 'text'
            data.mkdir(); files.mkdir()
            content = b'first immutable revision'
            (files / '1.md').write_bytes(content)
            with closing(sqlite3.connect(data / 'ontology.db')) as conn:
                conn.execute('CREATE TABLE ontology_schema_version(version INTEGER,files_root TEXT)')
                conn.execute('INSERT INTO ontology_schema_version VALUES(1,?)', (str(files),))
                conn.execute('CREATE TABLE text_revisions(body TEXT)')
                conn.execute('INSERT INTO text_revisions VALUES(?)', (json.dumps({'status':'ready',
                    'content_path':'1.md','bytes':len(content),'sha256':hashlib.sha256(content).hexdigest()}),))
                conn.commit()
            settings = SimpleNamespace(server_data=data, state_dir=root/'state', legacy_agent_data=None)
            for stopped in [False, True]:
                target = root / ('stopped' if stopped else 'online')
                backup.snapshot(settings, target, stopped=stopped)
                self.assertEqual((target/'ontology-files/1.md').read_bytes(), content)
                self.assertIn('server/ontology.db', json.loads((target/'backup-manifest.json').read_text())['files'])
            (files/'1.md').write_text('changed')
            (files/'later.md').write_text('later')
            with closing(sqlite3.connect(data/'ontology.db')) as conn:
                conn.execute('DELETE FROM text_revisions'); conn.commit()
            container=root/'maintenance'; container.mkdir()
            (root/'stopped').rename(container/'data')
            info = (data / 'ontology.db').stat()
            restore.data(settings, container, {'data_owners': {'server/ontology.db':
                [info.st_uid, info.st_gid, info.st_mode & 0o777]}})
            self.assertEqual((files/'1.md').read_bytes(),content)
            self.assertFalse((files/'later.md').exists())
            with closing(sqlite3.connect(data/'ontology.db')) as conn:
                self.assertEqual(conn.execute('SELECT count(*) FROM text_revisions').fetchone()[0],1)

    def test_four_export_ports_are_distinct_and_ontology_rejects_writable_options(self):
        agent={'agent':{'agents_dir':'/mount/agents'},'dag':{'binary_dir':'/mount/bin','workspace_dir':'/mount/work','rootfs_dir':'/image'}}
        server={'agent':{'agents_dir':'/source/agents','nfs':{'enabled':True}},
            'dag':{'binary_dir':'/source/bin','workspace_dir':'/source/work','nfs':{'enabled':True},'workspace_nfs':{'enabled':True}},
            'ontology':{'nfs':{'enabled':True,'port':2052}}}
        preflight.configuration(agent,server)
        server['ontology']['nfs']['port']=2051
        with self.assertRaisesRegex(ValueError,'distinct'): preflight.configuration(agent,server)
        server['ontology']['nfs']={'enabled':True,'port':2052,'read_only':False}
        with self.assertRaisesRegex(ValueError,'unsupported'): preflight.configuration(agent,server)
