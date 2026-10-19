"""Mixed engines are rejected before any service can overlap."""
from copy import deepcopy
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from rolling import manifest
from rolling.storage import database


def release(engine):
    version = 11 if engine == 'turso' else 10
    return {'release_id': engine, 'storage_engine': engine, 'protocol_version': 10,
            'execution_io': {'reader': 1, 'writer': 1}, 'compatibility': {
                'protocol': {'min': 1, 'max': 1},
                'data_format': {'min': version, 'max': version}}}


class StorageEngineTests(unittest.TestCase):
    def test_engine_switch_rejects_both_rolling_directions(self):
        for engine in ('turso', 'libsql'):
            own = release(engine)
            manifest.compatible(own, [own])
            other = release('libsql' if engine == 'turso' else 'turso')
            with self.assertRaisesRegex(ValueError, 'storage engine.*maintenance'):
                manifest.compatible(own, [other])

    def test_format_cannot_hide_an_engine_and_program_association_is_separate(self):
        value = release('turso')
        del value['storage_engine']
        with self.assertRaisesRegex(ValueError, 'storage engine'):
            manifest.io_format(value)
        original = release('libsql')
        saved = deepcopy(original)
        journal = {'releases': {'old': {'manifest': original,
                   'runtime_program': {'manifest': release('turso')}}}}
        manifest.compatible(release('turso'), manifest.overlapping(journal))
        self.assertEqual(original, saved)

    def test_live_marked_files_never_open_with_sqlite(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'store.db'
            Path(str(path) + '.engine.json').write_text(json.dumps({'engine': 'turso', 'format': 1}))
            with patch.object(database, 'native', return_value={'rows': [[7]]}) as native, \
                    patch.object(database.sqlite3, 'connect', side_effect=AssertionError('mixed engine')):
                self.assertEqual(database.query(path, 'SELECT ?1', [7]), [[7]])
                self.assertTrue(database.backup(path, path.with_name('copy.db')))
            self.assertEqual(native.call_count, 2)
            Path(str(path) + '.engine.json').write_text('{}')
            with self.assertRaisesRegex(ValueError, 'engine marker'):
                database.query(path, 'SELECT 1')


if __name__ == '__main__':
    unittest.main()
