import hashlib
import json
from pathlib import Path
import sqlite3
import tempfile
import unittest
from .model import select, dag_key, mentions, digest, stamp
from .apply import verify_pool
from .restore import restore


class SelectionTests(unittest.TestCase):
    now = 1_800_000_000

    def test_rust_nanosecond_timestamps_work_on_deployment_python(self):
        self.assertEqual(stamp('2026-10-08T10:00:31.567298806+00:00'), stamp('2026-10-08T10:00:31.567298+00:00'))

    def definition(self, name, days, prompt='same'):
        timestamp = (self.now - days * 86400) * 1000
        return {'name': name, 'created_at': timestamp, 'updated_at': timestamp,
                'spec': {'name': name, 'steps': [{'name': 'run', 'kind': {'type': 'agent', 'prompt': prompt}}]}}

    def test_recent_duplicate_can_be_removed_but_reference_and_keeper_remain(self):
        objects = {name: self.definition(name, 1) for name in ['first', 'second', 'used']}
        selected, _ = select(objects, {'used': {'schedule:daily'}}, {}, {}, self.now, 7, dag_key)
        self.assertEqual({r['name'] for r in selected}, {'first', 'second'})
        self.assertEqual({r['keep'] for r in selected}, {'used'})

    def test_idle_retention_boundary_recent_usage_and_unknown_metadata(self):
        objects = {name: self.definition(name, days, prompt=name) for name, days in [
            ('old', 8), ('boundary', 7), ('recent', 1), ('used', 8), ('unknown', 8)]}
        objects['unknown'].pop('created_at')
        selected, _ = select(objects, {}, {'used': self.now - 86400}, {}, self.now, 7, dag_key)
        self.assertEqual([r['name'] for r in selected], ['old'])

    def test_old_duplicate_keeper_is_not_selected_as_an_idle_object(self):
        objects = {name: self.definition(name, 9) for name in ['first', 'second']}
        selected, _ = select(objects, {}, {}, {'second': self.now - 8 * 86400}, self.now, 7, dag_key)
        self.assertEqual([(r['name'], r['keep']) for r in selected], [('first', 'second')])

    def test_reference_tokens_distinguish_names_and_keep_pins_paths_and_old_ids(self):
        value = ['tool@v3', '/mnt/second/v1/binary.bin', 'tool-copy', 'dag-flow']
        self.assertEqual(mentions(value, {'tool', 'second', 'flow'}, True), {'tool', 'second', 'flow'})
        self.assertEqual(mentions('tool-copy', {'tool'}), set())
        self.assertEqual(digest({'b': 1, 'a': 2}), digest({'a': 2, 'b': 1}))


class BackupTests(unittest.TestCase):
    def test_pool_restore_checks_every_byte_and_refuses_reused_names(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            backup = root / 'backup'
            pool = backup / 'pool'
            (pool / 'v1').mkdir(parents=True)
            data = b'native test artifact'
            version = {'version': 1, 'sha256': hashlib.sha256(data).hexdigest(), 'size_bytes': len(data)}
            meta = {'name': 'tool', 'current': 1, 'history': [1]}
            snapshot = {**meta, 'history': [version]}
            (pool / 'meta.json').write_text(json.dumps(meta))
            (pool / 'v1/meta.json').write_text(json.dumps(version))
            (pool / 'v1/binary.bin').write_bytes(data)
            (backup / 'snapshot.json').write_text(json.dumps(snapshot))
            state = root / 'state'; state.mkdir(); (state / 'control.locks').mkdir()
            connection = sqlite3.connect(state / 'control.db')
            connection.execute('CREATE TABLE fleet_definitions(kind TEXT,id TEXT,body TEXT,PRIMARY KEY(kind,id))')
            connection.close()
            binaries = root / 'binaries'; binaries.mkdir()
            plan = {'data_dir': str(state), 'binary_root': str(binaries)}
            receipt = {'status': 'deleted', 'kind': 'binary', 'name': 'tool', 'backup': str(backup)}
            self.assertEqual(restore(plan, receipt)['status'], 'restored')
            verify_pool(binaries / 'tool', snapshot)
            self.assertEqual(restore(plan, receipt)['status'], 'restored')
            (binaries / 'tool/v1/binary.bin').write_bytes(b'changed')
            with self.assertRaisesRegex(ValueError, 'checksum'):
                restore(plan, receipt)

    def test_dag_restore_changes_only_the_exact_definition(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary); (root / 'control.locks').mkdir(); backup = root / 'backup'; backup.mkdir()
            snapshot = {'name': 'workflow', 'spec': {'name': 'workflow'}, 'updated_at': 123}
            (backup / 'snapshot.json').write_text(json.dumps(snapshot))
            connection = sqlite3.connect(root / 'control.db')
            connection.execute('CREATE TABLE fleet_definitions(kind TEXT,id TEXT,body TEXT,PRIMARY KEY(kind,id))')
            connection.execute("INSERT INTO fleet_definitions VALUES('unrelated','keep','{}')"); connection.commit()
            plan = {'data_dir': str(root)}
            receipt = {'status': 'deleted', 'kind': 'dag', 'name': 'workflow', 'backup': str(backup)}
            restore(plan, receipt)
            self.assertEqual(connection.execute("SELECT body FROM fleet_definitions WHERE kind='unrelated'").fetchone()[0], '{}')
            connection.execute("UPDATE fleet_definitions SET body='{}' WHERE kind='dag'"); connection.commit()
            with self.assertRaisesRegex(ValueError, 'reused'):
                restore(plan, receipt)
            connection.close()


if __name__ == '__main__':
    unittest.main()
