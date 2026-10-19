import json
from pathlib import Path
import sqlite3
import tempfile
import time
import unittest
from unittest.mock import patch
from .files import resource_texts, MAX_BYTES
from .inventory import inventory


class ResourceFilesTests(unittest.TestCase):
    def test_every_extension_protects_pool_in_the_real_inventory(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            state = root / 'state'; state.mkdir()
            binary = root / 'binaries'; binary.mkdir()
            agents = root / 'agents'; agents.mkdir()
            with sqlite3.connect(state / 'control.db') as db:
                db.executescript('''CREATE TABLE fleet_definitions(kind TEXT,id TEXT,body TEXT);
                    CREATE TABLE brain_plan_versions(id TEXT,version INT,body TEXT);
                    CREATE TABLE execution_assignments(id TEXT,assignment TEXT);
                    CREATE TABLE execution_index(id TEXT,created_at INT,status TEXT);
                    CREATE TABLE dispatch_receipts(scope TEXT,id TEXT,payload TEXT,phase TEXT);''')
            with sqlite3.connect(state / 'definitions.db') as db:
                db.executescript('''CREATE TABLE schedules(id TEXT,target TEXT,params_json TEXT);
                    CREATE TABLE project_todos(id TEXT,capability_id TEXT,executor_ref TEXT,
                    executor_spec TEXT,plan_md TEXT,draft TEXT);''')
            config = root / 'config.json'; config.write_text('{}')
            pool = binary / 'native-tool'; (pool / 'v1').mkdir(parents=True)
            old = int((time.time() - 9 * 86400) * 1000)
            (pool / 'meta.json').write_text(json.dumps(dict(name='native-tool', current=1,
                history=[1], created_at=old, updated_at=old)))
            (pool / 'v1/meta.json').write_text(json.dumps(dict(version=1, sha256='digest', size_bytes=1)))
            for name in ('runner.cjs', 'runner.bash', 'runner.txt', 'runner.custom', 'runner'):
                with self.subTest(name=name):
                    path = agents / name
                    path.write_text('/mnt/binaries/native-tool/v1/binary.bin')
                    result = inventory(state, binary, agents, config)
                    self.assertEqual(result['binary'], [])
                    self.assertEqual(result['retained_binary'][0]['references'], ['agent-file:' + name])
                    path.unlink()
            self.assertEqual([r['name'] for r in inventory(state, binary, agents, config)['binary']], ['native-tool'])

    def test_unknown_data_roots_links_and_io_failures_are_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            path = root / 'unknown.custom'
            for data in (b'\xff\xfe', b'text\0binary', b'x' * (MAX_BYTES + 1)):
                path.write_bytes(data)
                with self.subTest(data_size=len(data)), self.assertRaises(ValueError):
                    list(resource_texts(root))
            path.unlink()
            path.symlink_to(root / 'missing')
            with self.assertRaisesRegex(ValueError, 'link'):
                list(resource_texts(root))
            with self.assertRaises(OSError):
                list(resource_texts(root / 'missing'))
            with patch('os.scandir', side_effect=PermissionError('unreadable')):
                with self.assertRaises(PermissionError):
                    list(resource_texts(root))
